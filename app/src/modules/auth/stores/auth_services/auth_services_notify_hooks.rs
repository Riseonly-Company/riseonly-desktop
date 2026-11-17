use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct AuthServicesNotifyHooks {
    pub note_account_created: Box<dyn Fn(&mut App) + Send + Sync>,
}

pub(crate) static AUTH_SERVICES_NOTIFY_HOOKS: FeatureHookSlot<AuthServicesNotifyHooks> = FeatureHookSlot::new("AUTH_SERVICES_NOTIFY_HOOKS");

impl AuthServicesNotifyHooks {
    pub fn install(hooks: Self) {
        let _ = AUTH_SERVICES_NOTIFY_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        AUTH_SERVICES_NOTIFY_HOOKS.get()
    }

    pub fn note_account_created(cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.note_account_created)(cx);
        }
    }
}
