use gpui::{AnyWindowHandle, App};

use crate::core::feature_hooks::FeatureHookSlot;

/// what the call windows need from the shell window and the room host
pub struct RiseRTCCallWindowHooks {
    pub app_id: fn() -> Option<String>,
    pub is_main_window: fn(AnyWindowHandle) -> bool,
    pub activate_call_window: fn(&mut App),
    pub uses_separate_window: fn(&App) -> bool,
}

pub(crate) static RISE_RTC_CALL_WINDOW_HOOKS: FeatureHookSlot<RiseRTCCallWindowHooks> =
    FeatureHookSlot::new("RISE_RTC_CALL_WINDOW_HOOKS");

pub fn set_rise_rtc_call_window_hooks(hooks: RiseRTCCallWindowHooks) {
    let _ = RISE_RTC_CALL_WINDOW_HOOKS.set(hooks);
}

pub fn rise_rtc_call_window_hooks() -> Option<&'static RiseRTCCallWindowHooks> {
    RISE_RTC_CALL_WINDOW_HOOKS.get()
}
