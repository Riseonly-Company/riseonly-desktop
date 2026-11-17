use crate::core::widgets::navigations::main_tab_bar_rtc_hooks::{MainTabBarRtcHooks, set_main_tab_bar_rtc_hooks};
use crate::modules::rtc::components::rise_rtc_local_speaking_ring::RiseRTCLocalSpeakingRing;

pub fn install_main_tab_bar_rtc_hooks() {
    set_main_tab_bar_rtc_hooks(MainTabBarRtcHooks {
        local_speaking_ring: |cx| RiseRTCLocalSpeakingRing::shared(cx).into(),
    });
}
