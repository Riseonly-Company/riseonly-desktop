use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct MaintenanceAuthHooks {
    pub viewer_bypasses_maintenance: Box<dyn Fn(&App) -> bool + Send + Sync>,
}

pub(crate) static MAINTENANCE_AUTH_HOOKS: FeatureHookSlot<MaintenanceAuthHooks> = FeatureHookSlot::new("MAINTENANCE_AUTH_HOOKS");

impl MaintenanceAuthHooks {
    pub fn install(hooks: Self) {
        let _ = MAINTENANCE_AUTH_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        MAINTENANCE_AUTH_HOOKS.get()
    }

    /// false until auth installs the slot
    pub fn viewer_bypasses_maintenance(cx: &App) -> bool {
        Self::installed().is_some_and(|hooks| (hooks.viewer_bypasses_maintenance)(cx))
    }
}
