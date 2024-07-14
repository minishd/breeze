use std::path::PathBuf;

use bytes::Bytes;
use tokio::{
    fs::File,
    io::{self, AsyncWriteExt},
    sync::mpsc::{self, Receiver, Sender},
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
    pub fn from_config(cfg: config::DiskConfig) -> Self {
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
        let mut p = self.cfg.save_path.clone();
        p.push(saved_name);

        p
    }

    /// Try to open a file on disk, and if we didn't find it,
    /// then return [`None`].
    pub async fn open(&self, saved_name: &str) -> Result<Option<File>, io::Error> {
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
    pub async fn len(&self, f: &File) -> Result<usize, io::Error> {
        Ok(f.metadata().await?.len() as usize)
    }

    /// Create a background I/O task
    pub async fn start_save(&self, saved_name: &str) -> Sender<Bytes> {
        // start a task that handles saving files to disk (we can save to cache/disk in parallel that way)
        let (tx, mut rx): (Sender<Bytes>, Receiver<Bytes>) = mpsc::channel(256);

        let p = self.path_for(saved_name);

        tokio::spawn(async move {
            // create file to save upload to
            let mut file = File::create(p)
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

        tx
    }
}
