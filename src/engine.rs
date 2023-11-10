use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use archived::Archive;
use axum::extract::BodyStream;
use bytes::{BufMut, Bytes, BytesMut};
use rand::Rng;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{
        mpsc::{self, Receiver, Sender},
        RwLock,
    },
};
use tokio_stream::StreamExt;
use tracing::{debug, error, info};
use walkdir::WalkDir;

use crate::view::{ViewError, ViewSuccess};

/// breeze engine! this is the core of everything
pub struct Engine {
    // ------ STATE ------ //
    /// The in-memory cache that cached uploads are stored in.
    cache: RwLock<Archive>,

    /// Cached count of uploaded files.
    pub upl_count: AtomicUsize,

    // ------ CONFIG ------ //
    /// The base URL that the server will be accessed from.
    /// It is only used for formatting returned upload URLs.
    pub base_url: String,

    /// The path on disk that uploads are saved to.
    save_path: PathBuf,

    /// The authorisation key required for uploading new files.
    /// If it is empty, no key will be required.
    pub upload_key: String,

    /// The maximum size for an upload to be stored in cache.
    /// Anything bigger skips cache and is read/written to
    /// directly from disk.
    cache_max_length: usize,
}

impl Engine {
    /// Creates a new instance of the breeze engine.
    pub fn new(
        base_url: String,
        save_path: PathBuf,
        upload_key: String,
        cache_max_length: usize,
        cache_lifetime: Duration,
        cache_full_scan_freq: Duration, // how often the cache will be scanned for expired items
        cache_mem_capacity: usize,
    ) -> Self {
        Self {
            cache: RwLock::new(Archive::with_full_scan(
                cache_full_scan_freq,
                cache_lifetime,
                cache_mem_capacity,
            )),
            upl_count: AtomicUsize::new(WalkDir::new(&save_path).min_depth(1).into_iter().count()), // count the amount of files in the save path and initialise our cached count with it

            base_url,
            save_path,
            upload_key,

            cache_max_length,
        }
    }

    /// Returns if an upload would be able to be cached
    #[inline(always)]
    fn will_use_cache(&self, length: usize) -> bool {
        length <= self.cache_max_length
    }

    /// Check if an upload exists in cache or on disk
    pub async fn upload_exists(&self, path: &Path) -> bool {
        let cache = self.cache.read().await;

        // extract file name, since that's what cache uses
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_string();

        // check in cache
        if cache.contains_key(&name) {
            return true;
        }

        // check on disk
        if path.exists() {
            return true;
        }

        return false;
    }

    /// Generate a new save path for an upload.
    ///
    /// This will call itself recursively if it picks
    /// a name that's already used. (it is rare)
    #[async_recursion::async_recursion]
    pub async fn gen_path(&self, original_path: &PathBuf) -> PathBuf {
        // generate a 6-character alphanumeric string
        let id: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(6)
            .map(char::from)
            .collect();

        // extract the extension from the original path
        let original_extension = original_path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_string();

        // path on disk
        let mut path = self.save_path.clone();
        path.push(&id);
        path.set_extension(original_extension);

        if !self.upload_exists(&path).await {
            path
        } else {
            // we had a name collision! try again..
            self.gen_path(original_path).await
        }
    }

    /// Process an upload.
    /// This is called by the /new route.
    pub async fn process_upload(
        &self,
        path: PathBuf,
        name: String, // we already extract it in the route handler, and it'd be wasteful to do it in gen_path
        content_length: usize,
        mut stream: BodyStream,
    ) {
        // if the upload size is smaller than the specified maximum, we use the cache!
        let mut use_cache = self.will_use_cache(content_length);

        // if we're using cache, make some space to store the upload in
        let mut data = if use_cache {
            BytesMut::with_capacity(content_length)
        } else {
            BytesMut::new()
        };

        // start a task that handles saving files to disk (we can save to cache/disk in parallel that way)
        let (tx, mut rx): (Sender<Bytes>, Receiver<Bytes>) = mpsc::channel(1);

        tokio::spawn(async move {
            // create file to save upload to
            let mut file = File::create(path)
                .await
                .expect("could not open file! make sure your upload path is valid");

            // receive chunks and save them to file
            while let Some(chunk) = rx.recv().await {
                debug!("writing chunk to disk (length: {})", chunk.len());
                file.write_all(&chunk)
                    .await
                    .expect("error while writing file to disk");
            }
        });

        // read and save upload
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();

            // send chunk to io task
            debug!("sending data to io task");
            tx.send(chunk.clone())
                .await
                .expect("failed to send data to io task");

            if use_cache {
                debug!("receiving data into buffer");
                if data.len() + chunk.len() > data.capacity() {
                    error!("the amount of data sent exceeds the content-length provided by the client! caching will be cancelled for this upload.");

                    // if we receive too much data, drop the buffer and stop using cache (it is still okay to use disk, probably)
                    data = BytesMut::new();
                    use_cache = false;
                } else {
                    data.put(chunk);
                }
            }
        }

        // insert upload into cache if necessary
        if use_cache {
            let mut cache = self.cache.write().await;

            info!("caching upload!");
            cache.insert(name, data.freeze());
        }

        info!("finished processing upload!!");

        // if all goes well, increment the cached upload counter
        self.upl_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Read an upload from cache, if it exists.
    ///
    /// Previously, this would lock the cache as
    /// writable to renew the upload's cache lifespan.
    /// Locking the cache as readable allows multiple concurrent
    /// readers though, which allows me to handle multiple views concurrently.
    async fn read_cached_upload(&self, name: &String) -> Option<Bytes> {
        let cache = self.cache.read().await;

        // fetch upload data from cache
        cache.get(name).map(ToOwned::to_owned)
    }

    /// Reads an upload, from cache or on disk.
    pub async fn get_upload(&self, original_path: &Path) -> Result<ViewSuccess, ViewError> {
        // extract upload file name
        let name = original_path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_string();

        // path on disk
        let mut path = self.save_path.clone();
        path.push(&name);

        // check if the upload exists, if not then 404
        if !self.upload_exists(&path).await {
            return Err(ViewError::NotFound);
        }

        // attempt to read upload from cache
        let cached_data = self.read_cached_upload(&name).await;

        if let Some(data) = cached_data {
            info!("got upload from cache!");

            Ok(ViewSuccess::FromCache(data))
        } else {
            // we already know the upload exists by now so this is okay
            let mut file = File::open(&path).await.unwrap();

            // read upload length from disk
            let metadata = file.metadata().await;

            if metadata.is_err() {
                error!("failed to get upload file metadata!");
                return Err(ViewError::InternalServerError);
            }

            let metadata = metadata.unwrap();

            let length = metadata.len() as usize;

            debug!("read upload from disk, size = {}", length);

            // if the upload is okay to cache, recache it and send a fromcache response
            if self.will_use_cache(length) {
                // read file from disk
                let mut data = BytesMut::with_capacity(length);

                // read file from disk and if it fails at any point, return 500
                loop {
                    match file.read_buf(&mut data).await {
                        Ok(n) => {
                            if n == 0 {
                                break;
                            }
                        }
                        Err(_) => {
                            return Err(ViewError::InternalServerError);
                        }
                    }
                }

                let data = data.freeze();

                // re-insert it into cache
                let mut cache = self.cache.write().await;
                cache.insert(name, data.clone());

                info!("recached upload from disk!");

                return Ok(ViewSuccess::FromCache(data));
            }

            info!("got upload from disk!");

            Ok(ViewSuccess::FromDisk(file))
        }
    }
}
