use gpui::App;
use serde_json::Value;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct SaiImageDiskCacheHooks {
    pub invalidation_token: Box<dyn Fn(Option<&str>) -> u64 + Send + Sync>,
    pub store: Box<dyn Fn(Vec<u8>, &str, Option<&str>, Option<u64>) + Send + Sync>,
    pub schedule_prefetch: Box<dyn Fn(&Value, &[String], &mut App) + Send + Sync>,
}

pub(crate) static SAI_IMAGE_DISK_CACHE_HOOKS: FeatureHookSlot<SaiImageDiskCacheHooks> = FeatureHookSlot::new("SAI_IMAGE_DISK_CACHE_HOOKS");

/// installed once, a second install keeps the first
pub fn set_sai_image_disk_cache_hooks(hooks: SaiImageDiskCacheHooks) {
    let _ = SAI_IMAGE_DISK_CACHE_HOOKS.set(hooks);
}

pub(crate) fn sai_image_disk_cache_hooks() -> Option<&'static SaiImageDiskCacheHooks> {
    SAI_IMAGE_DISK_CACHE_HOOKS.get()
}

/// schedules an image disk prefetch, does nothing without the image cache
pub(crate) fn sai_clever_image_disk_prefetch_schedule(
    value: &Value,
    key_patterns: &[String],
    cx: &mut App,
) {
    if let Some(hooks) = sai_image_disk_cache_hooks() {
        (hooks.schedule_prefetch)(value, key_patterns, cx);
    }
}
