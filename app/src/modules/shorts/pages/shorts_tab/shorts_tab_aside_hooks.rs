use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

/// what the shorts tab asks the right panel of the front window
pub struct ShortsTabAsideHooks {
    pub section_visits: fn(&App) -> u64,
}

pub(crate) static SHORTS_TAB_ASIDE_HOOKS: FeatureHookSlot<ShortsTabAsideHooks> =
    FeatureHookSlot::new("SHORTS_TAB_ASIDE_HOOKS");

pub fn set_shorts_tab_aside_hooks(hooks: ShortsTabAsideHooks) {
    let _ = SHORTS_TAB_ASIDE_HOOKS.set(hooks);
}

pub fn shorts_tab_aside_hooks() -> Option<&'static ShortsTabAsideHooks> {
    SHORTS_TAB_ASIDE_HOOKS.get()
}
