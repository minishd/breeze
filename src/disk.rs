use std::path::{Path, PathBuf};

use bytes::Bytes;
use tokio::{
    fs::File,
    io::{self, AsyncWriteExt},
    sync::mpsc,
};
use tracing::debug;
use walkdir::WalkDir;

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
    pub fn count(&self) -> usize {
        WalkDir::new(&self.cfg.save_path)
            .min_depth(1)
            .into_iter()
            .count()
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
    pub fn start_save(&self, saved_name: &str) -> mpsc::UnboundedSender<Bytes> {
        // start a task that handles saving files to disk (we can save to cache/disk in parallel that way)
        let (tx, mut rx): (mpsc::UnboundedSender<Bytes>, mpsc::UnboundedReceiver<Bytes>) =
            mpsc::unbounded_channel();

        let p = self.path_for(saved_name);

        tokio::spawn(async move {
            // create file to save upload to
            let file = File::create(p).await;

            if let Err(err) = file {
                tracing::error!(%err, "could not open file! make sure your upload path is valid");
                return;
            }
            let mut file = file.unwrap();

            // receive chunks and save them to file
            while let Some(chunk) = rx.recv().await {
                debug!("writing chunk to disk (length: {})", chunk.len());
                if let Err(err) = file.write_all(&chunk).await {
                    tracing::error!(%err, "error while writing file to disk");
                }
            }
        });

        tx
    }
}
