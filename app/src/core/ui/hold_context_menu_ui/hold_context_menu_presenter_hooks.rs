use std::time::Duration;

use gpui::{AnyWindowHandle, App, Window};

use crate::core::feature_hooks::FeatureHookSlot;

pub struct HoldContextMenuPresenterHooks {
    pub show: fn(Option<AnyWindowHandle>, &mut App),
    pub restore_source_key_window: fn(&mut Window, &mut App),
    pub source_window: fn(&mut App) -> Option<AnyWindowHandle>,
    pub hide: fn(Duration, &mut App),
}

pub(crate) static HOLD_CONTEXT_MENU_PRESENTER_HOOKS: FeatureHookSlot<HoldContextMenuPresenterHooks> =
    FeatureHookSlot::new("HOLD_CONTEXT_MENU_PRESENTER_HOOKS");

pub fn set_hold_context_menu_presenter_hooks(hooks: HoldContextMenuPresenterHooks) {
    let _ = HOLD_CONTEXT_MENU_PRESENTER_HOOKS.set(hooks);
}

pub fn hold_context_menu_presenter_hooks() -> Option<&'static HoldContextMenuPresenterHooks> {
    HOLD_CONTEXT_MENU_PRESENTER_HOOKS.get()
}
