use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, Weak};

/// thread safe cache bounded by count limit and total cost limit, empties on memory warning
/// values are cloned out so V is usually an Arc, a limit of 0 means no limit
pub struct NSCache<K, V>
where
    K: Eq + Hash + Clone + Send + 'static,
    V: Clone + Send + 'static,
{
    inner: Arc<NSCacheShared<K, V>>,
}

struct NSCacheShared<K, V> {
    state: Mutex<NSCacheState<K, V>>,
}

struct NSCacheEntry<V> {
    value: V,
    cost: usize,
    tick: u64,
}

struct NSCacheState<K, V> {
    name: String,
    count_limit: usize,
    total_cost_limit: usize,
    total_cost: usize,
    next_tick: u64,
    entries: HashMap<K, NSCacheEntry<V>>,
    recency: BTreeMap<u64, K>,
}

trait NSCachePurgeable: Send + Sync {
    fn purge(&self);
}

static NS_CACHE_REGISTRY: LazyLock<Mutex<Vec<Weak<dyn NSCachePurgeable>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

fn lock_ignoring_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// empties every live cache, called by the memory pressure seam
pub fn ns_cache_did_receive_memory_warning() {
    let live: Vec<Arc<dyn NSCachePurgeable>> = {
        let mut registry = lock_ignoring_poison(&NS_CACHE_REGISTRY);
        registry.retain(|weak| weak.strong_count() > 0);
        registry.iter().filter_map(Weak::upgrade).collect()
    };
    for cache in live {
        cache.purge();
    }
}

impl<K, V> NSCachePurgeable for NSCacheShared<K, V>
where
    K: Eq + Hash + Clone + Send + 'static,
    V: Clone + Send + 'static,
{
    fn purge(&self) {
        lock_ignoring_poison(&self.state).remove_all();
    }
}

impl<K, V> NSCacheState<K, V>
where
    K: Eq + Hash + Clone,
{
    fn remove_all(&mut self) {
        self.entries.clear();
        self.recency.clear();
        self.total_cost = 0;
    }

    fn remove(&mut self, key: &K) -> Option<NSCacheEntry<V>> {
        let entry = self.entries.remove(key)?;
        self.recency.remove(&entry.tick);
        self.total_cost = self.total_cost.saturating_sub(entry.cost);
        Some(entry)
    }

    fn touch(&mut self, key: &K) {
        let tick = self.next_tick;
        self.next_tick = self.next_tick.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(key) {
            self.recency.remove(&entry.tick);
            entry.tick = tick;
            self.recency.insert(tick, key.clone());
        }
    }

    fn is_over_limits(&self) -> bool {
        let over_count = self.count_limit > 0 && self.entries.len() > self.count_limit;
        let over_cost = self.total_cost_limit > 0 && self.total_cost > self.total_cost_limit;
        over_count || over_cost
    }

    fn evict_to_limits(&mut self) {
        while self.is_over_limits() {
            let Some((_, oldest)) = self.recency.pop_first() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.total_cost = self.total_cost.saturating_sub(entry.cost);
            }
        }
    }
}

impl<K, V> NSCache<K, V>
where
    K: Eq + Hash + Clone + Send + 'static,
    V: Clone + Send + 'static,
{
    pub fn new() -> Self {
        let inner = Arc::new(NSCacheShared {
            state: Mutex::new(NSCacheState {
                name: String::new(),
                count_limit: 0,
                total_cost_limit: 0,
                total_cost: 0,
                next_tick: 0,
                entries: HashMap::new(),
                recency: BTreeMap::new(),
            }),
        });
        let as_purgeable: Arc<dyn NSCachePurgeable> = inner.clone();
        let weak = Arc::downgrade(&as_purgeable);
        {
            let mut registry = lock_ignoring_poison(&NS_CACHE_REGISTRY);
            registry.retain(|entry| entry.strong_count() > 0);
            registry.push(weak);
        }
        Self { inner }
    }

    fn state(&self) -> MutexGuard<'_, NSCacheState<K, V>> {
        lock_ignoring_poison(&self.inner.state)
    }

    pub fn name(&self) -> String {
        self.state().name.clone()
    }

    pub fn set_name(&self, name: impl Into<String>) {
        self.state().name = name.into();
    }

    pub fn count_limit(&self) -> usize {
        self.state().count_limit
    }

    pub fn set_count_limit(&self, count_limit: usize) {
        let mut state = self.state();
        state.count_limit = count_limit;
        state.evict_to_limits();
    }

    pub fn total_cost_limit(&self) -> usize {
        self.state().total_cost_limit
    }

    pub fn set_total_cost_limit(&self, total_cost_limit: usize) {
        let mut state = self.state();
        state.total_cost_limit = total_cost_limit;
        state.evict_to_limits();
    }

    pub fn object(&self, for_key: &K) -> Option<V> {
        let mut state = self.state();
        let value = state.entries.get(for_key)?.value.clone();
        state.touch(for_key);
        Some(value)
    }

    pub fn set_object(&self, obj: V, for_key: K) {
        self.set_object_cost(obj, for_key, 0);
    }

    pub fn set_object_cost(&self, obj: V, for_key: K, cost: usize) {
        let mut state = self.state();
        state.remove(&for_key);
        let tick = state.next_tick;
        state.next_tick = state.next_tick.wrapping_add(1);
        state.total_cost = state.total_cost.saturating_add(cost);
        state.recency.insert(tick, for_key.clone());
        state.entries.insert(
            for_key,
            NSCacheEntry {
                value: obj,
                cost,
                tick,
            },
        );
        state.evict_to_limits();
    }

    pub fn remove_object(&self, for_key: &K) {
        self.state().remove(for_key);
    }

    pub fn remove_all_objects(&self) {
        self.state().remove_all();
    }

    /// count for tests and diagnostics only
    pub fn count(&self) -> usize {
        self.state().entries.len()
    }

    /// total cost for tests and diagnostics only
    pub fn total_cost(&self) -> usize {
        self.state().total_cost
    }
}

impl<K, V> Default for NSCache<K, V>
where
    K: Eq + Hash + Clone + Send + 'static,
    V: Clone + Send + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}
