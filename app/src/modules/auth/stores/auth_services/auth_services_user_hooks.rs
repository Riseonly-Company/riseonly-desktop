use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct AuthServicesUserHooks {
    pub prepare_contact_sync_after_registration: Box<dyn Fn(&mut App) + Send + Sync>,
    pub refresh_my_profile: Box<dyn Fn(&mut App) + Send + Sync>,
}

pub(crate) static AUTH_SERVICES_USER_HOOKS: FeatureHookSlot<AuthServicesUserHooks> = FeatureHookSlot::new("AUTH_SERVICES_USER_HOOKS");

impl AuthServicesUserHooks {
    pub fn install(hooks: Self) {
        let _ = AUTH_SERVICES_USER_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        AUTH_SERVICES_USER_HOOKS.get()
    }

    pub fn prepare_contact_sync_after_registration(cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.prepare_contact_sync_after_registration)(cx);
        }
    }

    pub fn refresh_my_profile(cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.refresh_my_profile)(cx);
        }
    }
}
