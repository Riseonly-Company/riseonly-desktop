use std::sync::{Arc, LazyLock, Weak};

use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::foundation::ns_cache::NSCache;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleLayoutSpec, AsyncTextBubblePreparedLayout, AsyncTextBubbleSeed,
};

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    seed: AsyncTextBubbleSeed,
    spec: AsyncTextBubbleLayoutSpec,
}

struct Storage {
    layouts: NSCache<Key, Arc<AsyncTextBubblePreparedLayout>>,
    epoch_value: Mutex<u64>,
}

impl Storage {
    fn remove_all(&self) {
        let mut epoch = self.epoch_value.lock();
        *epoch = epoch.wrapping_add(1);
        self.layouts.remove_all_objects();
    }
}

/// process wide cache for immutable text geometry
/// a message list keeps its own layouts but they die with the popped chat so a new list reuses
/// recent ones from here
pub struct AsyncTextBubblePreparedLayoutCache {
    pub total_cost_limit: usize,
    pub count_limit: usize,
    storage: Arc<Storage>,
    _memory_warning_observer: RiseSubscription,
}

static SHARED: LazyLock<AsyncTextBubblePreparedLayoutCache> = LazyLock::new(|| {
    AsyncTextBubblePreparedLayoutCache::new(
        AsyncTextBubblePreparedLayoutCache::default_total_cost_limit(),
        AsyncTextBubblePreparedLayoutCache::default_count_limit(),
    )
});

impl AsyncTextBubblePreparedLayoutCache {
    pub fn shared() -> &'static AsyncTextBubblePreparedLayoutCache {
        &SHARED
    }

    pub fn default_total_cost_limit() -> usize {
        usize::try_from(RiseLimits::client_value("async_text_bubble_layout_cache_bytes")).unwrap_or(0)
    }

    pub fn default_count_limit() -> usize {
        usize::try_from(RiseLimits::client_value("async_text_bubble_layout_cache_entries")).unwrap_or(0)
    }

    /// defaults are the canon byte and entry limits
    pub fn new(total_cost_limit: usize, count_limit: usize) -> Self {
        let total_cost_limit = total_cost_limit.max(1);
        let count_limit = count_limit.max(1);
        let layouts = NSCache::new();
        layouts.set_total_cost_limit(total_cost_limit);
        layouts.set_count_limit(count_limit);
        let storage = Arc::new(Storage {
            layouts,
            epoch_value: Mutex::new(0),
        });

        let weak: Weak<Storage> = Arc::downgrade(&storage);
        let observer = RiseAppEvents::default_center().observe(
            UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION,
            move |_| {
                if let Some(storage) = weak.upgrade() {
                    storage.remove_all();
                }
            },
        );

        Self {
            total_cost_limit,
            count_limit,
            storage,
            _memory_warning_observer: observer,
        }
    }

    pub fn layout(
        &self,
        seed: &AsyncTextBubbleSeed,
        spec: &AsyncTextBubbleLayoutSpec,
    ) -> Option<Arc<AsyncTextBubblePreparedLayout>> {
        self.storage.layouts.object(&Key {
            seed: seed.clone(),
            spec: *spec,
        })
    }

    /// current generation
    /// workers read it on start and insert with it so geometry finished after remove_all (account
    /// switch) never lands in the new account
    pub fn epoch(&self) -> u64 {
        *self.storage.epoch_value.lock()
    }

    pub fn insert(&self, layout: Arc<AsyncTextBubblePreparedLayout>) {
        let _epoch = self.storage.epoch_value.lock();
        self.store(layout);
    }

    pub fn insert_at_epoch(&self, layout: Arc<AsyncTextBubblePreparedLayout>, epoch: u64) {
        let current = self.storage.epoch_value.lock();
        if epoch != *current {
            return;
        }
        self.store(layout);
    }

    pub fn remove_all(&self) {
        self.storage.remove_all();
    }

    fn store(&self, layout: Arc<AsyncTextBubblePreparedLayout>) {
        let key = Key {
            seed: layout.seed.clone(),
            spec: layout.spec,
        };
        let cost = Self::estimated_cost(&layout);
        self.storage.layouts.set_object_cost(layout, key, cost);
    }

    fn estimated_cost(layout: &AsyncTextBubblePreparedLayout) -> usize {
        let text_bytes = layout.seed.text.len() * 4;
        let line_bytes = (layout.text_lines.len() + 1) * 1_024;
        (text_bytes + line_bytes).max(2_048)
    }
}
