use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Global};

#[derive(Default)]
struct SaiFetchMemoryCacheStorage(HashMap<String, Rc<dyn Any>>);

impl Global for SaiFetchMemoryCacheStorage {}

/// in memory box of values by key
/// reading a key filled with another type counts as a miss
pub enum SaiFetchMemoryCache {}

impl SaiFetchMemoryCache {
    pub fn value(cache_key: &str, cx: &App) -> Option<Rc<dyn Any>> {
        cx.try_global::<SaiFetchMemoryCacheStorage>()
            .and_then(|storage| storage.0.get(cache_key).cloned())
    }

    pub fn set_value(value: Rc<dyn Any>, cache_key: &str, cx: &mut App) {
        cx.default_global::<SaiFetchMemoryCacheStorage>()
            .0
            .insert(cache_key.to_owned(), value);
    }

    pub fn remove_value(cache_key: &str, cx: &mut App) {
        cx.default_global::<SaiFetchMemoryCacheStorage>().0.remove(cache_key);
    }

    pub fn clear_all(cx: &mut App) {
        cx.default_global::<SaiFetchMemoryCacheStorage>().0.clear();
    }
}
