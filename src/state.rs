use std::sync::atomic::AtomicUsize;

use bytes::Bytes;
use memory_cache::MemoryCache;
use tokio::sync::Mutex;

pub struct AppState {
    pub cache: Mutex<MemoryCache<String, Bytes>>,

    /* pub up_count: AtomicUsize, */
}
