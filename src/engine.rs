use std::{
    ops::Bound,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::body::BodyDataStream;
use bytes::{BufMut, Bytes, BytesMut};
use img_parts::{DynImage, ImageEXIF};
use rand::distributions::{Alphanumeric, DistString};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
};
use tokio_stream::StreamExt;
use tracing::{debug, error, info};

use crate::{cache, config, disk};

/// Various forms of upload data that can be sent to the client
pub enum UploadData {
    /// Send back the data from memory
    Cache(Bytes),
    /// Stream the file from disk to the client
    Disk(tokio::io::Take<File>),
}

pub struct UploadResponse {
    pub full_len: u64,
    pub range: (u64, u64),
    pub data: UploadData,
}

/// Non-error outcomes of an [`Engine::process`] call.
/// Some are rejections.
pub enum ProcessOutcome {
    /// The upload was successful.
    /// We give the user their file's URL
    Success(String),

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

/// breeze engine
pub struct Engine {
    /// Cached count of uploaded files
    pub upl_count: AtomicUsize,

    /// Engine configuration
    pub cfg: config::EngineConfig,

    /// The in-memory cache that cached uploads are stored in
    cache: Arc<cache::Cache>,

    /// An interface to the on-disk upload store
    disk: disk::Disk,
}

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

impl Engine {
    /// Creates a new instance of the engine
    pub fn with_config(cfg: config::EngineConfig) -> Self {
        let cache = cache::Cache::with_config(cfg.cache.clone());
        let disk = disk::Disk::with_config(cfg.disk.clone());

        let cache = Arc::new(cache);

        let cache_scanner = cache.clone();
        tokio::spawn(async move { cache_scanner.scanner().await });

        Self {
            // initialise our cached upload count. this doesn't include temp uploads!
            upl_count: AtomicUsize::new(disk.count()),

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
    ) -> anyhow::Result<GetOutcome> {
        let data = if let Some(u) = self.cache.get(saved_name) {
            u
        } else {
            // now, check if we have it on disk
            let mut f = if let Some(f) = self.disk.open(saved_name).await? {
                f
            } else {
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
                let (start, end) = if let Some(range) = resolve_range(range, full_len) {
                    range
                } else {
                    return Ok(GetOutcome::RangeNotSatisfiable);
                };

                let range_len = (end - start) + 1;

                f.seek(std::io::SeekFrom::Start(start)).await?;
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
        let (start, end) = if let Some(range) = resolve_range(range, full_len) {
            range
        } else {
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

    /// Generate a new saved name for an upload.
    ///
    /// If it picks a name that already exists, it will try again.
    pub async fn gen_saved_name(&self, ext: &str) -> String {
        loop {
            // generate a 6-character alphanumeric string
            let mut saved_name: String = Alphanumeric.sample_string(&mut rand::thread_rng(), 6);

            // if we have an extension, add it now
            if !ext.is_empty() {
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
    /// This is for deleting failed uploads only!!
    pub async fn remove(&self, saved_name: &str) -> anyhow::Result<()> {
        info!("!! removing upload: {saved_name}");

        self.cache.remove(saved_name);
        self.disk.remove(saved_name).await?;

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
    ) -> anyhow::Result<()> {
        // if we're using cache, make some space to store the upload in
        let mut data = if use_cache {
            BytesMut::with_capacity(provided_len.try_into()?)
        } else {
            BytesMut::new()
        };

        // don't begin a disk save if we're using temporary lifetimes
        let tx = if lifetime.is_none() {
            Some(self.disk.start_save(saved_name).await)
        } else {
            None
        };

        // whether or not we're gonna coalesce the data
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

        // read and save upload
        while let Some(chunk) = stream.next().await {
            // if we error on a chunk, fail out
            let chunk = chunk?;

            // if we have an i/o task, send it off
            // also cloning this is okay because it's a Bytes
            if !coalesce_and_strip {
                if let Some(ref tx) = tx {
                    debug!("sending chunk to i/o task");
                    tx.send(chunk.clone())?;
                }
            }

            if use_cache {
                debug!("receiving data into buffer");

                if data.len() + chunk.len() > data.capacity() {
                    info!("the amount of data sent exceeds the content-length provided by the client! caching will be cancelled for this upload.");

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
                tx.send(data.clone())?;
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

        Ok(())
    }

    pub async fn process(
        &self,
        ext: &str,
        provided_len: u64,
        stream: BodyDataStream,
        lifetime: Option<Duration>,
        keep_exif: bool,
    ) -> anyhow::Result<ProcessOutcome> {
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

        // If anything fails, delete the upload and return the error
        if save_result.is_err() {
            error!("failed processing upload!");

            self.remove(&saved_name).await?;
            save_result?;
        }

        // format and send back the url
        let url = format!("{}/p/{}", self.cfg.base_url, saved_name);

        // if all goes well, increment the cached upload counter
        self.upl_count.fetch_add(1, Ordering::Relaxed);

        info!("finished processing upload!");

        Ok(ProcessOutcome::Success(url))
    }
}
