use std::rc::Rc;

use gpui::{AnyElement, AnyView, App, Window};
use rise_theme::AppTheme;

use crate::core::feature_hooks::FeatureHookSlot;

/// title bar and window frame the call window borrows from the shell
pub struct RiseRTCCallWindowChromeHooks {
    pub title_bar: fn(Rc<dyn Fn(&mut Window, &mut App)>, &mut Window, &mut App) -> AnyView,
    pub set_title_bar_controls_theme: fn(&AnyView, Option<AppTheme>, &mut App),
    pub window_frame: fn(AnyElement, &mut Window, &App) -> AnyElement,
}

pub(crate) static RISE_RTC_CALL_WINDOW_CHROME_HOOKS: FeatureHookSlot<RiseRTCCallWindowChromeHooks> =
    FeatureHookSlot::new("RISE_RTC_CALL_WINDOW_CHROME_HOOKS");

pub fn set_rise_rtc_call_window_chrome_hooks(hooks: RiseRTCCallWindowChromeHooks) {
    let _ = RISE_RTC_CALL_WINDOW_CHROME_HOOKS.set(hooks);
}

pub fn rise_rtc_call_window_chrome_hooks() -> Option<&'static RiseRTCCallWindowChromeHooks> {
    RISE_RTC_CALL_WINDOW_CHROME_HOOKS.get()
}
