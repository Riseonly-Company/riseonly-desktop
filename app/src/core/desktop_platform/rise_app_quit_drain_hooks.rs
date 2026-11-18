use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct RiseAppQuitDrainHooks {
    pub will_terminate: fn(&mut App),
}

pub(crate) static RISE_APP_QUIT_DRAIN_HOOKS: FeatureHookSlot<RiseAppQuitDrainHooks> =
    FeatureHookSlot::new("RISE_APP_QUIT_DRAIN_HOOKS");

pub fn set_rise_app_quit_drain_hooks(hooks: RiseAppQuitDrainHooks) {
    let _ = RISE_APP_QUIT_DRAIN_HOOKS.set(hooks);
}

pub fn rise_app_quit_drain_hooks() -> Option<&'static RiseAppQuitDrainHooks> {
    RISE_APP_QUIT_DRAIN_HOOKS.get()
}
