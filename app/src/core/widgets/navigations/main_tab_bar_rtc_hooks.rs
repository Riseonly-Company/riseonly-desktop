use gpui::{AnyView, App};

use crate::core::feature_hooks::FeatureHookSlot;

pub struct MainTabBarRtcHooks {
    pub local_speaking_ring: fn(&mut App) -> AnyView,
}

pub(crate) static MAIN_TAB_BAR_RTC_HOOKS: FeatureHookSlot<MainTabBarRtcHooks> =
    FeatureHookSlot::new("MAIN_TAB_BAR_RTC_HOOKS");

pub fn set_main_tab_bar_rtc_hooks(hooks: MainTabBarRtcHooks) {
    let _ = MAIN_TAB_BAR_RTC_HOOKS.set(hooks);
}

pub fn main_tab_bar_rtc_hooks() -> Option<&'static MainTabBarRtcHooks> {
    MAIN_TAB_BAR_RTC_HOOKS.get()
}
