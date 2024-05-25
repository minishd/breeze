use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::extract::BodyStream;
use bytes::{BufMut, Bytes, BytesMut};
use img_parts::{DynImage, ImageEXIF};
use rand::distributions::{Alphanumeric, DistString};
use tokio::{fs::File, io::AsyncReadExt};
use tokio_stream::StreamExt;
use tracing::{debug, info};

use crate::{cache, config, disk};

/// Various forms of upload data that can be sent to the client
pub enum UploadData {
    /// Send back the data from memory
    Cache(Bytes),

    /// Stream the file from disk to the client
    Disk(File, usize),
}

/// Rejection outcomes of an [`Engine::process`] call
pub enum ProcessOutcome {
    /// The upload was successful.
    /// We give the user their file's URL
    Success(String),

    /// Occurs when a temporary upload is too big to fit in the cache.
    TemporaryUploadTooLarge,

    /// Occurs when the user-given lifetime is longer than we will allow
    TemporaryUploadLifetimeTooLong,
}

/// breeze engine! this is the core of everything
pub struct Engine {
    /// Cached count of uploaded files.
    pub upl_count: AtomicUsize,

    /// Engine configuration
    pub cfg: config::EngineConfig,

    /// The in-memory cache that cached uploads are stored in.
    cache: Arc<cache::Cache>,

    /// An interface to the on-disk upload store
    disk: disk::Disk,
}

impl Engine {
    /// Creates a new instance of the breeze engine.
    pub fn from_config(cfg: config::EngineConfig) -> Self {
        let cache = cache::Cache::from_config(cfg.cache.clone());
        let disk = disk::Disk::from_config(cfg.disk.clone());

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

    /// Fetch an upload
    /// 
    /// This will first try to read from cache, and then disk after.
    /// If an upload is eligible to be cached, it will be cached and
    /// sent back as a cache response instead of a disk response.
    pub async fn get(&self, saved_name: &str) -> anyhow::Result<Option<UploadData>> {
        // check the cache first
        if let Some(u) = self.cache.get(saved_name) {
            return Ok(Some(UploadData::Cache(u)));
        }

        // now, check if we have it on disk
        let mut f = if let Some(f) = self.disk.open(saved_name).await? {
            f
        } else {
            // file didn't exist
            return Ok(None);
        };

        let len = self.disk.len(&f).await?;

        // can this be recached?
        if self.cache.will_use(len) {
            // read file from disk
            let mut full = BytesMut::with_capacity(len);

            // read file from disk and if it fails at any point, return 500
            loop {
                match f.read_buf(&mut full).await {
                    Ok(n) => {
                        if n == 0 {
                            break;
                        }
                    }
                    Err(e) => Err(e)?,
                }
            }

            let full = full.freeze();

            // re-insert it into cache
            self.cache.add(saved_name, full.clone());

            return Ok(Some(UploadData::Cache(full)));
        }

        Ok(Some(UploadData::Disk(f, len)))
    }

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
    /// This will call itself recursively if it picks
    /// a name that's already used. (it is rare)
    #[async_recursion::async_recursion]
    pub async fn gen_saved_name(&self, ext: &str) -> String {
        // generate a 6-character alphanumeric string
        let id: String = Alphanumeric.sample_string(&mut rand::thread_rng(), 6);

        // path on disk
        let saved_name = format!("{}.{}", id, ext);

        if !self.has(&saved_name).await {
            saved_name
        } else {
            // we had a name collision! try again..
            info!("name collision! saved_name= {}", saved_name);
            self.gen_saved_name(ext).await
        }
    }

    /// Save a file to disk, and optionally cache.
    /// 
    /// This also handles custom file lifetimes and EXIF data removal.
    pub async fn save(
        &self,
        saved_name: &str,
        provided_len: usize,
        mut use_cache: bool,
        mut stream: BodyStream,
        lifetime: Option<Duration>,
        keep_exif: bool,
    ) -> Result<(), axum::Error> {
        // if we're using cache, make some space to store the upload in
        let mut data = if use_cache {
            BytesMut::with_capacity(provided_len)
        } else {
            BytesMut::new()
        };

        // don't begin a disk save if we're using temporary lifetimes
        let tx = if lifetime.is_none() {
            Some(self.disk.start_save(saved_name).await)
        } else {
            None
        };

        let tx: Option<&_> = tx.as_ref();

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
            && provided_len <= 16_777_216;

        // read and save upload
        while let Some(chunk) = stream.next().await {
            // if we error on a chunk, fail out
            let chunk = chunk?;

            // if we have an i/o task, send it off
            // also cloning this is okay because it's a Bytes
            if !coalesce_and_strip {
                debug!("sending chunk to i/o task");
                tx.map(|tx| tx.send(chunk.clone()));
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
                debug!("stripped exif data");
                data
            } else {
                data
            };

            // send what we did over to the i/o task, all in one chunk
            tx.map(|tx| tx.send(data.clone()));

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

        info!("finished processing upload!!");

        // if all goes well, increment the cached upload counter
        self.upl_count.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    pub async fn process(
        &self,
        ext: &str,
        provided_len: usize,
        stream: BodyStream,
        lifetime: Option<Duration>,
        keep_exif: bool,
    ) -> Result<ProcessOutcome, axum::Error> {
        // if the upload size is smaller than the specified maximum, we use the cache!
        let use_cache: bool = self.cache.will_use(provided_len);

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
        self.save(
            &saved_name,
            provided_len,
            use_cache,
            stream,
            lifetime,
            keep_exif,
        )
        .await?;

        // format and send back the url
        let url = format!("{}/p/{}", self.cfg.base_url, saved_name);

        Ok(ProcessOutcome::Success(url))
    }
}
