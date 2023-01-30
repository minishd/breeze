use std::time::{Duration, SystemTime};

/// Represents a set of eviction and expiration details for a specific cache entry.
pub(crate) struct CacheEntry<B> {
    /// Entry value.
    pub(crate) value: B,

    /// Expiration time.
    ///
    /// - [`None`] if the value must be kept forever.
    pub(crate) expiration_time: SystemTime,
}

impl<B> CacheEntry<B> {
    pub(crate) fn new(value: B, lifetime: Duration) -> Self {
        Self {
            expiration_time: SystemTime::now() + lifetime,
            value,
        }
    }

    /// Check if a entry is expired.
    pub(crate) fn is_expired(&self, current_time: SystemTime) -> bool {
        current_time >= self.expiration_time
    }
}
