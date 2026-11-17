use gpui::App;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiCacheCleanLayer {
    Everywhere,
    Memory,
    Disk,
}

pub enum SaiCacheCleaner {}

impl SaiCacheCleaner {
    /// cleans keys layer by layer, a key past the end of layers is cleaned everywhere
    pub fn clean_layers(cache_keys: &[String], layers: &[SaiCacheCleanLayer], cx: &mut App) {
        let keys: Vec<&str> = cache_keys
            .iter()
            .map(|key| trimming_whitespaces_and_newlines(key))
            .filter(|key| !key.is_empty())
            .collect();
        if keys.is_empty() {
            return;
        }

        for (index, key) in keys.into_iter().enumerate() {
            let layer = layers.get(index).copied().unwrap_or(SaiCacheCleanLayer::Everywhere);
            SaiWsResponseCache::remove(key, layer, cx);
        }
    }

    /// cleans keys in one layer, default layer is everywhere
    pub fn clean(cache_keys: &[String], layer: SaiCacheCleanLayer, cx: &mut App) {
        Self::clean_layers(cache_keys, &vec![layer; cache_keys.len()], cx);
    }

    pub fn clean_entries(entries: &[(String, SaiCacheCleanLayer)], cx: &mut App) {
        let cache_keys: Vec<String> = entries.iter().map(|(cache_key, _)| cache_key.clone()).collect();
        let layers: Vec<SaiCacheCleanLayer> = entries.iter().map(|(_, layer)| *layer).collect();
        Self::clean_layers(&cache_keys, &layers, cx);
    }
}
