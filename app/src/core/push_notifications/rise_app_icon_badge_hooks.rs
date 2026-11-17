use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct RiseAppIconBadgeHooks {
    pub sync: Box<dyn Fn(bool, &mut App) + Send + Sync>,
}

pub(crate) static RISE_APP_ICON_BADGE_HOOKS: FeatureHookSlot<RiseAppIconBadgeHooks> = FeatureHookSlot::new("RISE_APP_ICON_BADGE_HOOKS");

impl RiseAppIconBadgeHooks {
    pub fn install(hooks: Self) {
        let _ = RISE_APP_ICON_BADGE_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        RISE_APP_ICON_BADGE_HOOKS.get()
    }

    /// syncs the app icon badge, not forced
    pub fn sync(cx: &mut App) {
        Self::sync_force(false, cx);
    }

    pub fn sync_force(force: bool, cx: &mut App) {
        if let Some(hooks) = Self::installed() {
            (hooks.sync)(force, cx);
        }
    }
}
