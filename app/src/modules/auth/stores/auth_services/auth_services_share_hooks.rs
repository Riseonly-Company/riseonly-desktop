use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct AuthServicesShareHooks {
    pub reconcile_persisted_credential: fn(),
    pub synchronize_credential: fn(bool),
    pub write_snapshot_keeping_persisted_targets: fn(&mut App),
    pub sign_out: fn(&mut App),
}

pub(crate) static AUTH_SERVICES_SHARE_HOOKS: FeatureHookSlot<AuthServicesShareHooks> =
    FeatureHookSlot::new("AUTH_SERVICES_SHARE_HOOKS");

pub fn set_auth_services_share_hooks(hooks: AuthServicesShareHooks) {
    let _ = AUTH_SERVICES_SHARE_HOOKS.set(hooks);
}

pub fn auth_services_share_hooks() -> Option<&'static AuthServicesShareHooks> {
    AUTH_SERVICES_SHARE_HOOKS.get()
}
