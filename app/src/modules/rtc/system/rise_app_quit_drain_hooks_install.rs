use crate::core::desktop_platform::rise_app_quit_drain_hooks::{RiseAppQuitDrainHooks, set_rise_app_quit_drain_hooks};
use crate::modules::rtc::system::rise_rtc_call_system_coordinator::RiseRTCCallSystemCoordinator;

pub fn install_rise_app_quit_drain_hooks() {
    set_rise_app_quit_drain_hooks(RiseAppQuitDrainHooks {
        will_terminate: |cx| {
            RiseRTCCallSystemCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.leave_before_termination(cx));
        },
    });
}
