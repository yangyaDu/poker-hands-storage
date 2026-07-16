use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ByteCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub resident_estimated_bytes: usize,
    pub peak_resident_estimated_bytes: usize,
    pub evictions: u64,
    pub evicted_estimated_bytes: u64,
    pub oversized_skips: u64,
    pub cache_disabled_skips: u64,
}

impl ByteCacheStats {
    pub fn merge(&mut self, other: Self) {
        self.hits = self.hits.wrapping_add(other.hits);
        self.misses = self.misses.wrapping_add(other.misses);
        self.entries += other.entries;
        self.resident_estimated_bytes += other.resident_estimated_bytes;
        self.peak_resident_estimated_bytes += other.peak_resident_estimated_bytes;
        self.evictions = self.evictions.wrapping_add(other.evictions);
        self.evicted_estimated_bytes = self
            .evicted_estimated_bytes
            .wrapping_add(other.evicted_estimated_bytes);
        self.oversized_skips = self.oversized_skips.wrapping_add(other.oversized_skips);
        self.cache_disabled_skips = self
            .cache_disabled_skips
            .wrapping_add(other.cache_disabled_skips);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheInsertOutcome {
    Cached,
    Disabled,
    Oversized,
}

#[derive(Debug)]
pub(crate) struct ByteLru<K, V> {
    byte_budget: usize,
    entries: HashMap<K, CacheEntry<V>>,
    counter: u64,
    stats: ByteCacheStats,
}

#[derive(Debug)]
struct CacheEntry<V> {
    value: Arc<V>,
    last_access: u64,
    estimated_bytes: usize,
}

impl<K, V> ByteLru<K, V>
where
    K: Copy + Eq + Hash,
{
    pub(crate) fn new(byte_budget: usize) -> Self {
        Self {
            byte_budget,
            entries: HashMap::new(),
            counter: 0,
            stats: ByteCacheStats::default(),
        }
    }

    pub(crate) fn get(&mut self, key: K) -> Option<Arc<V>> {
        self.counter = self.counter.wrapping_add(1);
        match self.entries.get_mut(&key) {
            Some(entry) => {
                entry.last_access = self.counter;
                self.stats.hits = self.stats.hits.wrapping_add(1);
                Some(Arc::clone(&entry.value))
            }
            None => {
                self.stats.misses = self.stats.misses.wrapping_add(1);
                None
            }
        }
    }

    pub(crate) fn put(
        &mut self,
        key: K,
        value: Arc<V>,
        estimated_bytes: usize,
    ) -> CacheInsertOutcome {
        if self.byte_budget == 0 {
            self.stats.cache_disabled_skips = self.stats.cache_disabled_skips.wrapping_add(1);
            return CacheInsertOutcome::Disabled;
        }
        if estimated_bytes > self.byte_budget {
            self.stats.oversized_skips = self.stats.oversized_skips.wrapping_add(1);
            return CacheInsertOutcome::Oversized;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.stats.resident_estimated_bytes = self
                .stats
                .resident_estimated_bytes
                .saturating_sub(previous.estimated_bytes);
        }
        while self
            .stats
            .resident_estimated_bytes
            .checked_add(estimated_bytes)
            .is_none_or(|total| total > self.byte_budget)
        {
            let Some(lru_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(key, _)| *key)
            else {
                break;
            };
            let evicted = self.entries.remove(&lru_key).expect("existing LRU entry");
            self.stats.resident_estimated_bytes = self
                .stats
                .resident_estimated_bytes
                .saturating_sub(evicted.estimated_bytes);
            self.stats.evictions = self.stats.evictions.wrapping_add(1);
            self.stats.evicted_estimated_bytes = self
                .stats
                .evicted_estimated_bytes
                .wrapping_add(evicted.estimated_bytes as u64);
        }
        self.counter = self.counter.wrapping_add(1);
        self.stats.resident_estimated_bytes += estimated_bytes;
        self.stats.peak_resident_estimated_bytes = self
            .stats
            .peak_resident_estimated_bytes
            .max(self.stats.resident_estimated_bytes);
        self.entries.insert(
            key,
            CacheEntry {
                value,
                last_access: self.counter,
                estimated_bytes,
            },
        );
        CacheInsertOutcome::Cached
    }

    pub(crate) fn stats(&self) -> ByteCacheStats {
        ByteCacheStats {
            entries: self.entries.len(),
            ..self.stats
        }
    }
}
