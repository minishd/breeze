use std::sync::{Mutex, Arc};

use bytes::Bytes;
use memory_cache::MemoryCache;

pub struct AppState {
    pub cache: Mutex<MemoryCache<String, Bytes>>
}