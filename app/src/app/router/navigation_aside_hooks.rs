use gpui::App;

use crate::app::router::navigation_types::RootRoute;
use crate::core::feature_hooks::FeatureHookSlot;

/// what the navigation store and a page ask the right panel of the front window
pub struct NavigationAsideHooks {
    pub open: fn(RootRoute, &mut App),
    pub has_frontmost: fn(&App) -> bool,
}

pub(crate) static NAVIGATION_ASIDE_HOOKS: FeatureHookSlot<NavigationAsideHooks> =
    FeatureHookSlot::new("NAVIGATION_ASIDE_HOOKS");

pub fn set_navigation_aside_hooks(hooks: NavigationAsideHooks) {
    let _ = NAVIGATION_ASIDE_HOOKS.set(hooks);
}

pub fn navigation_aside_hooks() -> Option<&'static NavigationAsideHooks> {
    NAVIGATION_ASIDE_HOOKS.get()
}
