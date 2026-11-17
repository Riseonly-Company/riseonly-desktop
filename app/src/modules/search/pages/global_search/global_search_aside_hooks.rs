use gpui::App;

use crate::app::router::navigation_types::RootRoute;
use crate::core::feature_hooks::FeatureHookSlot;

/// what the search page asks the right panel of the front window
pub struct GlobalSearchAsideHooks {
    pub replace: fn(RootRoute, &mut App),
}

pub(crate) static GLOBAL_SEARCH_ASIDE_HOOKS: FeatureHookSlot<GlobalSearchAsideHooks> =
    FeatureHookSlot::new("GLOBAL_SEARCH_ASIDE_HOOKS");

pub fn set_global_search_aside_hooks(hooks: GlobalSearchAsideHooks) {
    let _ = GLOBAL_SEARCH_ASIDE_HOOKS.set(hooks);
}

pub fn global_search_aside_hooks() -> Option<&'static GlobalSearchAsideHooks> {
    GLOBAL_SEARCH_ASIDE_HOOKS.get()
}
