use crate::core::feature_hooks::FeatureHookSlot;

/// raw values of message realtime events, installed by the chat message repository
pub struct RiseCanonEventsParityMessageRealtimeHooks {
    pub all_raw_values: Box<dyn Fn() -> Vec<String> + Send + Sync>,
}

pub(crate) static MESSAGE_REALTIME_HOOKS: FeatureHookSlot<RiseCanonEventsParityMessageRealtimeHooks> = FeatureHookSlot::new("MESSAGE_REALTIME_HOOKS");

impl RiseCanonEventsParityMessageRealtimeHooks {
    pub fn install(hooks: Self) {
        let _ = MESSAGE_REALTIME_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        MESSAGE_REALTIME_HOOKS.get()
    }
}

/// raw values of chat realtime message kinds, installed by the chat models
pub struct RiseCanonEventsParityChatModelsHooks {
    pub all_raw_values: Box<dyn Fn() -> Vec<String> + Send + Sync>,
}

pub(crate) static CHAT_MODELS_HOOKS: FeatureHookSlot<RiseCanonEventsParityChatModelsHooks> = FeatureHookSlot::new("CHAT_MODELS_HOOKS");

impl RiseCanonEventsParityChatModelsHooks {
    pub fn install(hooks: Self) {
        let _ = CHAT_MODELS_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        CHAT_MODELS_HOOKS.get()
    }
}
