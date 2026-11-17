use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct AuthServicesGuidesHooks {
    pub note_account_created: Box<dyn Fn(&str, &mut App) + Send + Sync>,
}

pub(crate) static AUTH_SERVICES_GUIDES_HOOKS: FeatureHookSlot<AuthServicesGuidesHooks> = FeatureHookSlot::new("AUTH_SERVICES_GUIDES_HOOKS");

impl AuthServicesGuidesHooks {
    pub fn install(hooks: Self) {
        let _ = AUTH_SERVICES_GUIDES_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        AUTH_SERVICES_GUIDES_HOOKS.get()
    }

    pub fn note_account_created(user_id: &str, cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.note_account_created)(user_id, cx);
        }
    }
}
