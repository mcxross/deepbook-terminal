use dashmap::DashMap;
use std::sync::Arc;

use super::{Counter, Pool};

pub static POOLS: std::sync::LazyLock<PoolStore> = std::sync::LazyLock::new(PoolStore::new);

pub struct PoolStore {
    inner: DashMap<Counter, Arc<Pool>>,
}

impl PoolStore {
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    pub fn get(&self, counter: &Counter) -> Option<Arc<Pool>> {
        self.inner.get(counter).map(|r| Arc::clone(r.value()))
    }

    pub fn mget(&self, counters: &[Counter]) -> Vec<Option<Arc<Pool>>> {
        counters.iter().map(|c| self.get(c)).collect()
    }

    pub fn insert(&self, pool: Pool) {
        let counter = pool.counter.clone();
        self.inner.insert(counter, Arc::new(pool));
    }

    pub fn modify<F>(&self, counter: Counter, f: F)
    where
        F: FnOnce(&mut Pool),
    {
        let mut pool = self
            .get(&counter)
            .map_or_else(|| Pool::new(counter.clone()), |s| (*s).clone());
        f(&mut pool);
        self.insert(pool);
    }

    pub fn remove(&self, counter: &Counter) {
        self.inner.remove(counter);
    }

    pub fn clear(&self) {
        self.inner.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl Default for PoolStore {
    fn default() -> Self {
        Self::new()
    }
}
