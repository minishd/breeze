use std::path::{Path, PathBuf};

use bytes::Bytes;
use tokio::{
    fs::File,
    io::{self, AsyncWriteExt},
    sync::mpsc,
};

use crate::config;

/// Provides an API to access the disk file store
/// like we access the cache.
pub struct Disk {
    cfg: config::DiskConfig,
}

impl Disk {
    pub fn with_config(cfg: config::DiskConfig) -> Self {
        Self { cfg }
    }

    /// Counts the number of files saved to disk we have
    pub fn count(&self) -> io::Result<usize> {
        std::fs::read_dir(&self.cfg.save_path)?.try_fold(0, |acc, x| {
            Ok(if x?.file_type()?.is_file() {
                acc + 1
            } else {
                acc
            })
        })
    }

    /// Formats the path on disk for a `saved_name`.
    fn path_for(&self, saved_name: &str) -> PathBuf {
        // try to prevent path traversal by ignoring everything except the file name
        let name = Path::new(saved_name).file_name().unwrap_or_default();

        let mut p: PathBuf = self.cfg.save_path.clone();
        p.push(name);

        p
    }

    /// Try to open a file on disk, and if we didn't find it,
    /// then return [`None`].
    pub async fn open(&self, saved_name: &str) -> io::Result<Option<File>> {
        let p = self.path_for(saved_name);

        match File::open(p).await {
            Ok(f) => Ok(Some(f)),
            Err(e) => match e.kind() {
                io::ErrorKind::NotFound => Ok(None),
                _ => Err(e)?, // some other error, send it back
            },
        }
    }

    /// Get the size of an upload's file
    pub async fn len(&self, f: &File) -> io::Result<u64> {
        Ok(f.metadata().await?.len())
    }

    /// Remove an upload from disk.
    pub async fn remove(&self, saved_name: &str) -> io::Result<()> {
        let p = self.path_for(saved_name);

        tokio::fs::remove_file(p).await
    }

    /// Create a background I/O task
    pub fn start_save<
        Fut: Future + Send + 'static,
        F: FnOnce(io::Error) -> Fut + Send + 'static,
    >(
        &self,
        saved_name: &str,
        fail_callback: F,
    ) -> mpsc::Sender<Bytes> {
        // start a task that handles saving files to disk (we can save to cache/disk in parallel that way)
        // a large buffer size is chosen so uploads can be received quickly,
        // but with less possibility of running out of memory.
        // (thats probably only possible w very high link speed tho......)
        let (tx, mut rx): (mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>) = mpsc::channel(30000);

        let p = self.path_for(saved_name);

        tokio::spawn(async move {
            // create file to save upload to
            let mut file = match File::create(p).await {
                Ok(f) => f,
                Err(err) => {
                    tracing::error!(%err, "could not open file! make sure your upload path is valid");
                    return;
                }
            };

            // receive chunks and save them to file
            while let Some(chunk) = rx.recv().await {
                tracing::debug!(length = chunk.len(), "writing chunk to disk");
                if let Err(err) = file.write_all(&chunk).await {
                    drop(rx);
                    fail_callback(err).await;
                    return;
                }
            }

            // flush to disk
            // this should catch "no space left on device" i hope...
            if let Err(err) = file.flush().await {
                fail_callback(err).await;
            }
        });

        tx
    }
}
