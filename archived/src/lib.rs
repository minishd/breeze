mod entry;

use bytes::Bytes;

use crate::entry::*;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

pub struct Archive {
    cache_table: HashMap<String, CacheEntry<Bytes>>,
    full_scan_frequency: Option<Duration>,
    created_time: SystemTime,
    last_scan_time: Option<SystemTime>,
    entry_lifetime: Duration,
    capacity: usize,
    length: usize,
}

impl Archive {
    /* pub fn new(capacity: usize) -> Self {
        Self {
            cache_table: HashMap::new(),
            full_scan_frequency: None,
            created_time: SystemTime::now(),
            last_scan_time: None,
            capacity,
            length: 0,
        }
    } */

    pub fn with_full_scan(full_scan_frequency: Duration, entry_lifetime: Duration, capacity: usize) -> Self {
        Self {
            cache_table: HashMap::with_capacity(256),
            full_scan_frequency: Some(full_scan_frequency),
            created_time: SystemTime::now(),
            last_scan_time: None,
            entry_lifetime,
            capacity,
            length: 0,
        }
    }

    pub fn contains_key(&self, key: &String) -> bool {
        let now = SystemTime::now();

        self.cache_table
            .get(key)
            .filter(|cache_entry| !cache_entry.is_expired(now))
            .is_some()
    }

    pub fn get_last_scan_time(&self) -> Option<SystemTime> {
        self.last_scan_time
    }

    pub fn get_full_scan_frequency(&self) -> Option<Duration> {
        self.full_scan_frequency
    }

    pub fn get(&self, key: &String) -> Option<&Bytes> {
        let now = SystemTime::now();

        self.cache_table
            .get(key)
            .filter(|cache_entry| !cache_entry.is_expired(now))
            .map(|cache_entry| &cache_entry.value)
    }

    pub fn get_or_insert<F>(
        &mut self,
        key: String,
        factory: F,
    ) -> &Bytes
    where
        F: Fn() -> Bytes,
    {
        let now = SystemTime::now();

        self.try_full_scan_expired_items(now);

        match self.cache_table.entry(key) {
            Entry::Occupied(mut occupied) => {
                if occupied.get().is_expired(now) {
                    occupied.insert(CacheEntry::new(factory(), self.entry_lifetime));
                }

                &occupied.into_mut().value
            }
            Entry::Vacant(vacant) => &vacant.insert(CacheEntry::new(factory(), self.entry_lifetime)).value,
        }
    }

    pub fn insert(
        &mut self,
        key: String,
        value: Bytes,
    ) -> Option<Bytes> {
        let now = SystemTime::now();

        self.try_full_scan_expired_items(now);

        if value.len() + self.length > self.capacity {
            return None;
        }

        self.length += value.len();

        self.cache_table
            .insert(key, CacheEntry::new(value, self.entry_lifetime))
            .filter(|cache_entry| !cache_entry.is_expired(now))
            .map(|cache_entry| cache_entry.value)
    }

    pub fn remove(&mut self, key: &String) -> Option<Bytes> {
        let now = SystemTime::now();

        self.try_full_scan_expired_items(now);

        let mut removed_len: usize = 0;
        let result = self
            .cache_table
            .remove(key)
            .filter(|cache_entry| !cache_entry.is_expired(now))
            .and_then(|o| {
                removed_len += o.value.len();
                return Some(o);
            })
            .map(|cache_entry| cache_entry.value);
        self.length -= removed_len;
        return result;
    }

    pub fn renew(&mut self, key: &String) -> Option<()> {
        let now = SystemTime::now();

        self.try_full_scan_expired_items(now);

        let entry = self.cache_table.get_mut(key);

        if entry.is_some() {
            let mut entry = entry.unwrap();

            entry.expiration_time = now + self.entry_lifetime;

            return Some(());
        } else {
            return None;
        }
    }

    fn try_full_scan_expired_items(&mut self, current_time: SystemTime) {
        if let Some(full_scan_frequency) = self.full_scan_frequency {
            let since = current_time
                .duration_since(self.last_scan_time.unwrap_or(self.created_time))
                .unwrap();

            if since >= full_scan_frequency {
                let mut removed_len = 0;
                self.cache_table.retain(|_, cache_entry| {
                    if cache_entry.is_expired(current_time) {
                        removed_len += cache_entry.value.len();
                        return false;
                    }
                    return true;
                });
                self.length -= removed_len;

                self.last_scan_time = Some(current_time);
            }
        }
    }
}
