use bytes::Bytes;
use memory_cache::MemoryCache;
use tokio::sync::Mutex;

pub struct AppState {
    pub cache: Mutex<MemoryCache<String, Bytes>>
}