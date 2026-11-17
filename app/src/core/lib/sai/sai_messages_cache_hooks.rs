use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct SaiMessagesCacheHooks {
    pub notify_messages_cache_did_change: Box<dyn Fn(&str, &mut App) + Send + Sync>,
}

pub(crate) static SAI_MESSAGES_CACHE_HOOKS: FeatureHookSlot<SaiMessagesCacheHooks> = FeatureHookSlot::new("SAI_MESSAGES_CACHE_HOOKS");

/// installed once, a second install keeps the first
pub fn set_sai_messages_cache_hooks(hooks: SaiMessagesCacheHooks) {
    let _ = SAI_MESSAGES_CACHE_HOOKS.set(hooks);
}

pub(crate) fn sai_messages_cache_hooks() -> Option<&'static SaiMessagesCacheHooks> {
    SAI_MESSAGES_CACHE_HOOKS.get()
}

/// tells the messages store a cache key changed, no-op without chat
pub(crate) fn sai_messages_cache_notify_did_change(cache_key: &str, cx: &mut App) {
    if let Some(hooks) = sai_messages_cache_hooks() {
        (hooks.notify_messages_cache_did_change)(cache_key, cx);
    }
}
