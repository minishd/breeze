use std::{
    io::SeekFrom,
    ops::Bound,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::body::BodyDataStream;
use base64::{Engine as _, prelude::BASE64_URL_SAFE_NO_PAD};
use bytes::{BufMut, Bytes, BytesMut};
use color_eyre::eyre::{self, WrapErr};
use hmac::Mac;
use img_parts::{DynImage, ImageEXIF};
use rand::distr::{Alphanumeric, SampleString};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
};
use tokio_stream::StreamExt;
use tracing::{debug, error, info};
use twox_hash::XxHash3_128;

use crate::{cache, config, disk};

/// Various forms of upload data that can be sent to the client
pub enum UploadData {
    /// Send back the data from memory
    Cache(Bytes),
    /// Stream the file from disk to the client
    Disk(tokio::io::Take<File>),
}

/// Upload data and metadata needed to build a view response
pub struct UploadResponse {
    pub full_len: u64,
    pub range: (u64, u64),
    pub data: UploadData,
}

/// Non-error outcomes of an [`Engine::process`] call.
/// Some are rejections.
pub enum ProcessOutcome {
    /// The upload was successful.
    /// We give the user their file's URL (and deletion URL if one was created)
    Success {
        url: String,
        deletion_url: Option<String>,
    },

    /// Occurs when an upload exceeds the chosen maximum file size.
    UploadTooLarge,

    /// Occurs when a temporary upload is too big to fit in the cache.
    TemporaryUploadTooLarge,

    /// Occurs when the user-given lifetime is longer than we will allow
    TemporaryUploadLifetimeTooLong,
}

/// Non-error outcomes of an [`Engine::get`] call.
pub enum GetOutcome {
    /// Successfully read upload.
    Success(UploadResponse),

    /// The upload was not found anywhere
    NotFound,

    /// A range was requested that exceeds an upload's bounds
    RangeNotSatisfiable,
}

/// Type alias to make using HMAC SHA256 easier
type HmacSha256 = hmac::Hmac<sha2::Sha256>;

/// breeze engine
pub struct Engine {
    /// Cached count of uploaded files
    pub upl_count: AtomicUsize,

    /// Engine configuration
    pub cfg: config::EngineConfig,

    /// HMAC state initialised with the deletion secret (if present)
    pub deletion_hmac: Option<HmacSha256>,

    /// The in-memory cache that cached uploads are stored in
    cache: Arc<cache::Cache>,

    /// An interface to the on-disk upload store
    disk: disk::Disk,
}

/// Try to parse a `Range` header into an easier format to work with
fn resolve_range(range: Option<headers::Range>, full_len: u64) -> Option<(u64, u64)> {
    let last_byte = full_len - 1;

    let (start, end) =
        if let Some((start, end)) = range.and_then(|r| r.satisfiable_ranges(full_len).next()) {
            // satisfiable_ranges will never return Excluded so this is ok
            let start = if let Bound::Included(start_incl) = start {
                start_incl
            } else {
                0
            };
            let end = if let Bound::Included(end_incl) = end {
                end_incl
            } else {
                last_byte
            };

            (start, end)
        } else {
            (0, last_byte)
        };

    // catch ranges we can't satisfy
    if end > last_byte || start > end {
        return None;
    }

    Some((start, end))
}

/// Calculate HMAC of field values.
pub fn update_hmac(hmac: &mut HmacSha256, saved_name: &str, hash: u128) {
    // mix deletion req fields into one buf
    let mut field_bytes = BytesMut::new();
    field_bytes.put(saved_name.as_bytes());
    field_bytes.put_u128(hash);

    // take the hmac
    hmac.update(&field_bytes);
}

/// How many bytes of a file should be used for hash calculation.
const SAMPLE_WANTED_BYTES: usize = 32768;

/// Format some info about an upload and hash it
///
/// This should not change between versions!!
/// That would break deletion urls
fn calculate_hash(len: u64, data_sample: Bytes) -> u128 {
    let mut buf = BytesMut::new();
    buf.put_u64(len);
    buf.put(data_sample);

    XxHash3_128::oneshot(&buf)
}

impl Engine {
    /// Creates a new instance of the engine
    pub fn with_config(cfg: config::EngineConfig) -> Self {
        let deletion_hmac = cfg
            .deletion_secret
            .as_ref()
            .map(|s| HmacSha256::new_from_slice(s.as_bytes()).unwrap());

        let cache = cache::Cache::with_config(cfg.cache.clone());
        let disk = disk::Disk::with_config(cfg.disk.clone());

        let cache = Arc::new(cache);

        let cache_scanner = cache.clone();
        tokio::spawn(async move { cache_scanner.scanner().await });

        Self {
            // initialise our cached upload count. this doesn't include temp uploads!
            upl_count: AtomicUsize::new(disk.count()),
            deletion_hmac,

            cfg,

            cache,
            disk,
        }
    }

    /// Fetch an upload.
    ///
    /// This will first try to read from cache, and then disk after.
    /// If an upload is eligible to be cached, it will be cached and
    /// sent back as a cache response instead of a disk response.
    ///
    /// If there is a range, it is applied at the very end.
    pub async fn get(
        &self,
        saved_name: &str,
        range: Option<headers::Range>,
    ) -> eyre::Result<GetOutcome> {
        let data = if let Some(u) = self.cache.get(saved_name) {
            u
        } else {
            // now, check if we have it on disk
            let Some(mut f) = self.disk.open(saved_name).await? else {
                // file didn't exist
                return Ok(GetOutcome::NotFound);
            };

            let full_len = self.disk.len(&f).await?;

            // if possible, recache and send a cache response
            // else, send a disk response
            if self.cache.will_use(full_len) {
                // read file from disk
                let mut data = BytesMut::with_capacity(full_len.try_into()?);

                // read file from disk and if it fails at any point, return 500
                loop {
                    match f.read_buf(&mut data).await {
                        Ok(n) => {
                            if n == 0 {
                                break;
                            }
                        }
                        Err(e) => Err(e)?,
                    }
                }

                let data = data.freeze();

                // re-insert it into cache
                self.cache.add(saved_name, data.clone());

                data
            } else {
                let Some((start, end)) = resolve_range(range, full_len) else {
                    return Ok(GetOutcome::RangeNotSatisfiable);
                };

                let range_len = (end - start) + 1;

                f.seek(SeekFrom::Start(start)).await?;
                let f = f.take(range_len);

                let res = UploadResponse {
                    full_len,
                    range: (start, end),
                    data: UploadData::Disk(f),
                };
                return Ok(GetOutcome::Success(res));
            }
        };

        let full_len = data.len() as u64;
        let Some((start, end)) = resolve_range(range, full_len) else {
            return Ok(GetOutcome::RangeNotSatisfiable);
        };

        // cut down to range
        let data = data.slice((start as usize)..=(end as usize));

        // build response
        let res = UploadResponse {
            full_len,
            range: (start, end),
            data: UploadData::Cache(data),
        };
        Ok(GetOutcome::Success(res))
    }

    /// Check if we have an upload stored anywhere.
    ///
    /// This is only used to prevent `saved_name` collisions!!
    /// It is not used to deliver "not found" errors.
    pub async fn has(&self, saved_name: &str) -> bool {
        if self.cache.has(saved_name) {
            return true;
        }

        // sidestep handling the error properly
        // that way we can call this in gen_saved_name easier
        if self.disk.open(saved_name).await.is_ok_and(|f| f.is_some()) {
            return true;
        }

        false
    }

    /// Try to read a file and calculate a hash for it.
    pub async fn get_hash(&self, saved_name: &str) -> eyre::Result<Option<u128>> {
        // readout sample data and full len
        let (data_sample, len) = if let Some(full_data) = self.cache.get(saved_name) {
            // we found it in cache! take as many bytes as we can
            let taking = full_data.len().min(SAMPLE_WANTED_BYTES);
            let data = full_data.slice(0..taking);

            let len = full_data.len() as u64;

            tracing::info!("data len is {}", data.len());

            (data, len)
        } else {
            // not in cache, so try disk
            let Some(mut f) = self.disk.open(saved_name).await? else {
                // not found there either so we just dont have it
                return Ok(None);
            };

            // find len..
            let len = f.seek(SeekFrom::End(0)).await?;
            f.rewind().await?;

            // only take wanted # of bytes for read
            let mut f = f.take(SAMPLE_WANTED_BYTES as u64);

            // try to read
            let mut data = Vec::with_capacity(SAMPLE_WANTED_BYTES);
            f.read_to_end(&mut data).await?;
            let data = Bytes::from(data);

            (data, len)
        };

        // calculate hash
        Ok(Some(calculate_hash(len, data_sample)))
    }

    /// Generate a new saved name for an upload.
    ///
    /// If it picks a name that already exists, it will try again.
    pub async fn gen_saved_name(&self, ext: Option<String>) -> String {
        loop {
            // generate a 6-character alphanumeric string
            let mut saved_name: String = Alphanumeric.sample_string(&mut rand::rng(), 6);

            // if we have an extension, add it now
            if let Some(ref ext) = ext {
                saved_name.push('.');
                saved_name.push_str(ext);
            }

            if !self.has(&saved_name).await {
                break saved_name;
            } else {
                // there was a name collision. loop and try again
                info!("name collision! saved_name= {}", saved_name);
            }
        }
    }

    /// Wipe out an upload from all storage.
    ///
    /// (Intended for deletion URLs and failed uploads)
    pub async fn remove(&self, saved_name: &str) -> eyre::Result<()> {
        info!(saved_name, "!! removing upload");

        self.cache.remove(saved_name);
        self.disk
            .remove(saved_name)
            .await
            .wrap_err("failed to remove file from disk")?;

        info!("!! successfully removed upload");

        Ok(())
    }

    /// Save a file to disk, and optionally cache.
    ///
    /// This also handles custom file lifetimes and EXIF data removal.
    pub async fn save(
        &self,
        saved_name: &str,
        provided_len: u64,
        mut use_cache: bool,
        mut stream: BodyDataStream,
        lifetime: Option<Duration>,
        keep_exif: bool,
    ) -> eyre::Result<(Bytes, u64)> {
        // if we're using cache, make some space to store the upload in
        let mut data = if use_cache {
            BytesMut::with_capacity(provided_len.try_into()?)
        } else {
            BytesMut::new()
        };

        // don't begin a disk save if we're using temporary lifetimes
        let tx = if lifetime.is_none() {
            Some(self.disk.start_save(saved_name))
        } else {
            None
        };

        // whether or not we are going to coalesce the data
        // in order to strip the exif data at the end,
        // instead of just sending it off to the i/o task
        let coalesce_and_strip = use_cache
            && matches!(
                std::path::Path::new(saved_name)
                    .extension()
                    .map(|s| s.to_str()),
                Some(Some("png" | "jpg" | "jpeg" | "webp" | "tiff"))
            )
            && !keep_exif
            && provided_len <= self.cfg.max_strip_len;

        // buffer of sampled data for the deletion hash
        let mut hash_sample = BytesMut::with_capacity(SAMPLE_WANTED_BYTES);
        // actual number of bytes processed
        let mut observed_len = 0;

        // read and save upload
        while let Some(chunk) = stream.next().await {
            // if we error on a chunk, fail out
            let chunk = chunk?;

            // if we have an i/o task, send it off
            // also cloning this is okay because it's a Bytes
            if !coalesce_and_strip {
                if let Some(ref tx) = tx {
                    debug!("sending chunk to i/o task");
                    tx.send(chunk.clone())
                        .wrap_err("failed to send chunk to i/o task!")?;
                }
            }

            // add to sample if we need to
            let wanted = SAMPLE_WANTED_BYTES - hash_sample.len();
            if wanted != 0 {
                // take as many bytes as we can ...
                let taking = chunk.len().min(wanted);
                hash_sample.extend_from_slice(&chunk[0..taking]);
            }
            // record new len
            observed_len += chunk.len() as u64;

            if use_cache {
                debug!("receiving data into buffer");

                if data.len() + chunk.len() > data.capacity() {
                    info!(
                        "the amount of data sent exceeds the content-length provided by the client! caching will be cancelled for this upload."
                    );

                    // if we receive too much data, drop the buffer and stop using cache (it is still okay to use disk, probably)
                    data = BytesMut::new();
                    use_cache = false;
                } else {
                    data.put(chunk);
                }
            }
        }

        let data = data.freeze();

        // we coalesced the data instead of streaming to disk,
        // strip the exif data and send it off now
        let data = if coalesce_and_strip {
            // strip the exif if we can
            // if we can't, then oh well
            let data = if let Ok(Some(data)) = DynImage::from_bytes(data.clone()).map(|o| {
                o.map(|mut img| {
                    img.set_exif(None);
                    img.encoder().bytes()
                })
            }) {
                info!("stripped exif data");
                data
            } else {
                info!("failed to strip exif data");
                data
            };

            // send what we did over to the i/o task, all in one chunk
            if let Some(ref tx) = tx {
                debug!("sending filled buffer to i/o task");
                tx.send(data.clone())
                    .wrap_err("failed to send coalesced buffer to i/o task!")?;
            }

            data
        } else {
            // or, we didn't do that
            // keep the data as it is
            data
        };

        // insert upload into cache if we're using it
        if use_cache {
            info!("caching upload!");
            match lifetime {
                Some(lt) => self.cache.add_with_lifetime(saved_name, data, lt, false),
                None => self.cache.add(saved_name, data),
            };
        }

        Ok((hash_sample.freeze(), observed_len))
    }

    pub async fn process(
        &self,
        ext: Option<String>,
        provided_len: u64,
        stream: BodyDataStream,
        lifetime: Option<Duration>,
        keep_exif: bool,
    ) -> eyre::Result<ProcessOutcome> {
        // if the upload size is greater than our max file size, deny it now
        if self.cfg.max_upload_len.is_some_and(|l| provided_len > l) {
            return Ok(ProcessOutcome::UploadTooLarge);
        }

        // if the upload size is smaller than the specified maximum, we use the cache!
        let use_cache = self.cache.will_use(provided_len);

        // if a temp file is too big for cache, reject it now
        if lifetime.is_some() && !use_cache {
            return Ok(ProcessOutcome::TemporaryUploadTooLarge);
        }

        // if a temp file's lifetime is too long, reject it now
        if lifetime.is_some_and(|lt| lt > self.cfg.max_temp_lifetime) {
            return Ok(ProcessOutcome::TemporaryUploadLifetimeTooLong);
        }

        // generate the file name
        let saved_name = self.gen_saved_name(ext).await;

        // save it
        let save_result = self
            .save(
                &saved_name,
                provided_len,
                use_cache,
                stream,
                lifetime,
                keep_exif,
            )
            .await;

        // handle result
        let (hash_sample, len) = match save_result {
            // Okay so just extract metadata
            Ok(m) => m,
            // If anything fails, delete the upload and return the error
            Err(err) => {
                error!("failed processing upload!");

                self.remove(&saved_name).await?;
                return Err(err);
            }
        };

        // if deletion urls are enabled, create one
        let deletion_url = self.deletion_hmac.clone().map(|mut hmac| {
            // calculate hash of file metadata
            let hash = calculate_hash(len, hash_sample);
            let mut hash_bytes = BytesMut::new();
            hash_bytes.put_u128(hash);
            let hash_b64 = BASE64_URL_SAFE_NO_PAD.encode(&hash_bytes);

            // take hmac
            update_hmac(&mut hmac, &saved_name, hash);
            let out = hmac.finalize().into_bytes();
            let out_b64 = BASE64_URL_SAFE_NO_PAD.encode(out);

            // format deletion url
            format!(
                "{}/del?name={saved_name}&hash={hash_b64}&hmac={out_b64}",
                self.cfg.base_url
            )
        });

        // format and send back the url
        let url = format!("{}/p/{saved_name}", self.cfg.base_url);

        // if all goes well, increment the cached upload counter
        self.upl_count.fetch_add(1, Ordering::Relaxed);

        info!("finished processing upload!");

        Ok(ProcessOutcome::Success { url, deletion_url })
    }
}
