use std::{
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
    time::Duration,
};

use bytes::Bytes;
use color_eyre::eyre::{self, bail};
use dashmap::{DashMap, mapref::one::Ref};
use tokio::time;

use crate::config;

#[cfg(not(test))]
use atomic_time::AtomicSystemTime;
#[cfg(not(test))]
use std::time::SystemTime;
#[cfg(test)]
use tests::{MockAtomicSystemTime as AtomicSystemTime, MockSystemTime as SystemTime};

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

    /// How many times the scanner has ran,
    /// for testing purposes
    scan_count: AtomicU64,

    /// How should it behave
    cfg: config::CacheConfig,
}

impl Cache {
    pub fn with_config(cfg: config::CacheConfig) -> eyre::Result<Self> {
        // Sanity check chosen limits
        if cfg.mem_capacity < cfg.max_length {
            bail!("`max_length` should not exceed `mem_capacity`");
        }

        // Return
        Ok(Self {
            map: DashMap::with_capacity(64),
            length: AtomicUsize::new(0),
            scan_count: AtomicU64::new(0),

            cfg,
        })
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
    fn get_(&self, key: &str) -> Option<Ref<'_, String, Entry>> {
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
    #[inline]
    pub fn will_use(&self, length: u64) -> bool {
        length <= (self.cfg.max_length as u64)
    }

    /// The background job that scans through the cache and removes inactive elements.
    ///
    /// TODO: see if this is actually less expensive than
    /// letting each entry keep track of expiry with its own task
    pub async fn scanner(&self) {
        let mut interval = time::interval(self.cfg.scan_freq);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        interval.tick().await; // Skip first tick

        loop {
            // We put this first so that it doesn't scan the instant the server starts
            interval.tick().await;
            self.scan_count.fetch_add(1, Ordering::Relaxed);

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

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };

    use bytes::Bytes;

    use crate::{cache::Cache, config::CacheConfig};

    thread_local! {
        static MOCK_CLOCK: AtomicU64 = AtomicU64::new(0);
    }
    fn get_clock() -> u64 {
        MOCK_CLOCK.with(|mc| mc.load(Ordering::Relaxed))
    }
    fn advance_clock(ms: u64) {
        MOCK_CLOCK.with(|mc| mc.fetch_add(ms, Ordering::Relaxed));
    }
    async fn advance_clock_async(ms: u64) {
        advance_clock(ms);
        tokio::time::advance(Duration::from_millis(ms)).await;
        tokio::task::yield_now().await; // make sure scanner tick runs
    }

    pub struct MockSystemTimeError;

    #[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
    pub(super) struct MockSystemTime(u64);
    impl MockSystemTime {
        pub fn now() -> Self {
            Self(get_clock())
        }

        pub fn duration_since(
            &self,
            earlier: MockSystemTime,
        ) -> Result<Duration, MockSystemTimeError> {
            if self.0 >= earlier.0 {
                Ok(Duration::from_millis(self.0 - earlier.0))
            } else {
                Err(MockSystemTimeError)
            }
        }

        pub fn elapsed(&self) -> Result<Duration, MockSystemTimeError> {
            Self::now().duration_since(*self)
        }
    }

    pub(super) struct MockAtomicSystemTime(AtomicU64);
    impl MockAtomicSystemTime {
        pub fn now() -> Self {
            Self(AtomicU64::new(get_clock()))
        }

        pub fn load(&self, order: Ordering) -> MockSystemTime {
            MockSystemTime(self.0.load(order))
        }
        pub fn store(&self, system_time: MockSystemTime, order: Ordering) {
            self.0.store(system_time.0, order);
        }
    }

    const KEY: &str = "abcdef.png";
    const VALUE: Bytes = Bytes::from_static(&[0, 1, 2, 3, 4, 5, 6, 7]);

    fn simple() -> Cache {
        return Cache::with_config(CacheConfig {
            max_length: 10_000_000,
            mem_capacity: 100_000_000,
            scan_freq: Duration::from_secs(5),
            upload_lifetime: Duration::from_secs(15),
        })
        .unwrap();
    }

    async fn scanning() -> Arc<Cache> {
        let cache = Arc::new(simple());

        tokio::spawn({
            let cache = cache.clone();
            async move { cache.scanner().await }
        });
        // allow 0ms scanner tick to run
        tokio::task::yield_now().await;

        cache
    }

    /// Make sure that cache use check
    /// decides properly for multiple lengths
    #[test]
    fn will_use() {
        let cache = simple();

        // use something
        assert!(cache.will_use(4_000_000));

        // don't use something
        assert!(!cache.will_use(12_000_001));

        // use something edge
        assert!(cache.will_use(10_000_000));

        // use something mini
        assert!(cache.will_use(0));
    }

    /// Make sure that [`Cache::add`]'s return value
    /// is `false` when an entry was replaced
    #[test]
    fn store_replacement() {
        let cache = simple();

        // store
        assert!(cache.add(KEY, VALUE));

        // store w replace
        assert!(!cache.add(KEY, VALUE));
    }

    /// Make sure that the scanner ticks at
    /// the right times, and removes entries
    /// when expected.
    #[tokio::test(start_paused = true)]
    async fn store_expire_on_hit_with_scanner() {
        let cache = scanning().await;

        // store
        assert!(cache.add(KEY, VALUE));

        // get again so that scanner timing
        // doesn't align w expiration
        advance_clock_async(4999).await;
        assert_eq!(cache.scan_count.load(Ordering::Relaxed), 0);
        assert_eq!(cache.get(KEY), Some(VALUE));

        // next scanner tick
        advance_clock_async(1).await;
        assert_eq!(cache.scan_count.load(Ordering::Relaxed), 1);

        // advance a bit more
        // make sure we don't expire early
        advance_clock_async(7000).await;
        assert_eq!(cache.scan_count.load(Ordering::Relaxed), 2);
        assert!(cache.map.get(KEY).is_some());

        // advance to next scanner tick
        advance_clock_async(3000).await;
        assert_eq!(cache.scan_count.load(Ordering::Relaxed), 3);

        // advance to after expiry
        advance_clock_async(4999).await;
        assert_eq!(cache.scan_count.load(Ordering::Relaxed), 3);

        // it should be there because we
        // offset ourselves by 1ms
        assert!(cache.map.get(KEY).is_some());
        assert_eq!(cache.get(KEY), None);
    }

    /// Make sure that the scanner removes
    /// expired entries.
    #[tokio::test(start_paused = true)]
    async fn store_expire_by_scanner() {
        let cache = scanning().await;

        // store
        assert!(cache.add(KEY, VALUE));

        // make sure we don't expire early
        advance_clock_async(6500).await;
        assert!(cache.map.get(KEY).is_some());

        // advance to after expiry
        advance_clock_async(8500).await;

        // it should get hit by scanner
        assert!(cache.map.get(KEY).is_none());
    }

    /// Make sure that entries expire on hit,
    /// even when there is no scanner
    #[test]
    fn store_get_expire_on_hit() {
        let cache = simple();

        // store, get
        let added_at = MockSystemTime::now();
        assert!(cache.add(KEY, VALUE));
        assert_eq!(cache.get(KEY), Some(VALUE));

        // get after delay
        // (upload gets used)
        advance_clock(2000);
        assert_eq!(cache.map.get(KEY).unwrap().last_used(), added_at);
        assert_eq!(cache.get(KEY), Some(VALUE));
        assert_eq!(
            cache.map.get(KEY).unwrap().last_used(),
            MockSystemTime::now()
        );

        // get after longer delay
        // (upload should have been used so no expire)
        advance_clock(14000);
        assert_eq!(cache.get(KEY), Some(VALUE));
        assert_eq!(
            cache.map.get(KEY).unwrap().last_used(),
            MockSystemTime::now()
        );

        // get after expiration
        advance_clock(15000);
        assert!(cache.get(KEY).is_none());
    }
}
