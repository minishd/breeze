use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, SystemTime},
};

use atomic_time::AtomicSystemTime;
use bytes::Bytes;
use dashmap::{mapref::one::Ref, DashMap};
use tokio::time;

use crate::config;

/// An entry stored in the cache.
///
/// It contains basic metadata and the actual value.
pub struct Entry {
    /// The data held
    value: Bytes,

    /// The last time this entry was read/written
    last_used: AtomicSystemTime,

    /// Whether or not `last_used` should be updated
    update_used: bool,

    /// How long the entry should last
    lifetime: Duration,
}

impl Entry {
    fn new(value: Bytes, lifetime: Duration, update_used: bool) -> Self {
        let now = AtomicSystemTime::now();

        Self {
            value,
            last_used: now,
            update_used,
            lifetime,
        }
    }

    fn last_used(&self) -> SystemTime {
        self.last_used.load(Ordering::Relaxed)
    }

    fn is_expired(&self) -> bool {
        match self.last_used().elapsed() {
            Ok(d) => d >= self.lifetime,
            Err(_) => false, // now > last_used
        }
    }
}

/// A concurrent cache with a maximum memory size (w/ LRU) and expiration.
///
/// It is designed to keep memory usage low.
pub struct Cache {
    /// Where elements are stored
    map: DashMap<String, Entry>,

    /// Total length of data stored in cache currently
    length: AtomicUsize,

    /// How should it behave
    cfg: config::CacheConfig,
}

impl Cache {
    pub fn with_config(cfg: config::CacheConfig) -> Self {
        Self {
            map: DashMap::with_capacity(64),
            length: AtomicUsize::new(0),

            cfg,
        }
    }

    /// Figure out who should be bumped out of cache next
    fn next_out(&self, length: usize) -> Vec<String> {
        let mut sorted: Vec<_> = self.map.iter().collect();

        // Sort by least recently used
        sorted.sort_unstable_by_key(|e| e.last_used());

        // Total bytes we would be removing
        let mut total = 0;

        // Pull entries until we have enough free space
        sorted
            .iter()
            .take_while(|e| {
                let need_more = total < length;

                if need_more {
                    total += e.value.len();
                }

                need_more
            })
            .map(|e| e.key().clone())
            .collect()
    }

    /// Remove an element from the cache
    ///
    /// Returns: [`Some`] if successful, [`None`] if element not found
    pub fn remove(&self, key: &str) -> Option<()> {
        // Skip expiry checks, we are removing it anyways
        // And also that could cause an infinite loop which would be pretty stupid.
        let e = self.map.get(key)?;

        // Atomically subtract from the total cache length
        self.length.fetch_sub(e.value.len(), Ordering::Relaxed);

        // Drop the entry lock so we can actually remove it
        drop(e);

        // Remove from map
        self.map.remove(key);

        Some(())
    }

    /// Add a new element to the cache with a specified lifetime.
    ///
    /// Returns: `true` if no value is replaced, `false` if a value was replaced
    pub fn add_with_lifetime(
        &self,
        key: &str,
        value: Bytes,
        lifetime: Duration,
        is_renewable: bool,
    ) -> bool {
        let e = Entry::new(value, lifetime, is_renewable);

        let len = e.value.len();
        let cur_total = self.length.load(Ordering::Relaxed);
        let new_total = cur_total + len;

        if new_total > self.cfg.mem_capacity {
            // How far we went above the limit
            let needed = new_total - self.cfg.mem_capacity;

            self.next_out(needed).iter().for_each(|k| {
                // Remove the element, and ignore the result
                // The only reason it should be failing is if it couldn't find it,
                // in which case it was already removed
                self.remove(k);
            });
        }

        // Atomically add to total cached data length
        self.length.fetch_add(len, Ordering::Relaxed);

        // Add to the map, return true if we didn't replace anything
        self.map.insert(key.to_string(), e).is_none()
    }

    /// Add a new element to the cache with the default lifetime.
    ///
    /// Returns: `true` if no value is replaced, `false` if a value was replaced
    pub fn add(&self, key: &str, value: Bytes) -> bool {
        self.add_with_lifetime(key, value, self.cfg.upload_lifetime, true)
    }

    /// Internal function for retrieving entries.
    ///
    /// Returns: same as [`DashMap::get`], for our purposes
    ///
    /// It exists so we can run the expiry check before
    /// actually working with any entries, so no weird bugs happen
    fn get_(&self, key: &str) -> Option<Ref<String, Entry>> {
        let e = self.map.get(key)?;

        // if the entry is expired get rid of it now
        if e.is_expired() {
            // drop the reference so we don't deadlock
            drop(e);

            // remove it
            self.remove(key);

            // and say we never had it
            return None;
        }

        Some(e)
    }

    /// Get an item from the cache, if it exists.
    pub fn get(&self, key: &str) -> Option<Bytes> {
        let e = self.get_(key)?;

        if e.update_used {
            e.last_used.store(SystemTime::now(), Ordering::Relaxed);
        }

        Some(e.value.clone())
    }

    /// Check if we have an item in cache.
    ///
    /// Returns: `true` if key exists, `false` if it doesn't
    ///
    /// We don't use [`DashMap::contains_key`] here because it would just do
    /// the exact same thing I do here, but without running the expiry check logic
    pub fn has(&self, key: &str) -> bool {
        self.get_(key).is_some()
    }

    /// Returns if an upload is able to be cached
    /// with the current caching rules
    #[inline(always)]
    pub fn will_use(&self, length: u64) -> bool {
        length <= self.cfg.max_length
    }

    /// The background job that scans through the cache and removes inactive elements.
    ///
    /// TODO: see if this is actually less expensive than
    /// letting each entry keep track of expiry with its own task
    pub async fn scanner(&self) {
        let mut interval = time::interval(self.cfg.scan_freq);

        loop {
            // We put this first so that it doesn't scan the instant the server starts
            interval.tick().await;

            // Save current timestamp so we aren't retrieving it constantly
            // If we don't do this it'll be a LOT of system api calls
            let now = SystemTime::now();

            // Collect a list of all the expired keys
            // If we fail to compare the times, it gets added to the list anyways
            let expired: Vec<_> = self
                .map
                .iter()
                .filter_map(|e| {
                    let elapsed = now.duration_since(e.last_used()).unwrap_or(Duration::MAX);
                    let is_expired = elapsed >= e.lifetime;

                    if is_expired {
                        Some(e.key().clone())
                    } else {
                        None
                    }
                })
                .collect();

            // If we have any, lock the map and drop all of them
            if !expired.is_empty() {
                // Use a retain call, should be less locks that way
                // (instead of many remove calls)
                self.map.retain(|k, _| !expired.contains(k));
            }
        }
    }
}
