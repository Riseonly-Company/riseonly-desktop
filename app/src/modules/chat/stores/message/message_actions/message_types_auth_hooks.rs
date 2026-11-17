use crate::core::feature_hooks::FeatureHookSlot;

pub struct MessageTypesAuthHooks {
    pub message_author_user_id_for_outgoing: fn() -> String,
    pub profile_name: fn() -> Option<String>,
    pub profile_logo: fn() -> Option<String>,
}

pub(crate) static MESSAGE_TYPES_AUTH_HOOKS: FeatureHookSlot<MessageTypesAuthHooks> = FeatureHookSlot::new("MESSAGE_TYPES_AUTH_HOOKS");

impl MessageTypesAuthHooks {
    pub(crate) fn install(hooks: Self) {
        let _ = MESSAGE_TYPES_AUTH_HOOKS.set(hooks);
    }

    pub(crate) fn installed() -> Option<&'static Self> {
        MESSAGE_TYPES_AUTH_HOOKS.get()
    }

    pub fn message_author_user_id_for_outgoing() -> String {
        Self::installed().map(|hooks| (hooks.message_author_user_id_for_outgoing)()).unwrap_or_default()
    }

    pub fn profile_name() -> Option<String> {
        Self::installed().and_then(|hooks| (hooks.profile_name)())
    }

    pub fn profile_logo() -> Option<String> {
        Self::installed().and_then(|hooks| (hooks.profile_logo)())
    }
}
