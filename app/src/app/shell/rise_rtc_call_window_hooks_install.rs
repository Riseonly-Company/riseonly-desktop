use crate::app::root_view::RootView;
use crate::app::shell::window_presenter::WindowPresenter;
use crate::modules::rtc::pages::rise_rtc_room_presentation_host::RiseRTCRoomPresentationHost;
use crate::modules::rtc::system::rise_rtc_call_window_hooks::{
    RiseRTCCallWindowHooks, set_rise_rtc_call_window_hooks,
};

pub fn install_rise_rtc_call_window_hooks() {
    set_rise_rtc_call_window_hooks(RiseRTCCallWindowHooks {
        app_id: WindowPresenter::app_id,
        is_main_window: |window| window.downcast::<RootView>().is_some(),
        activate_call_window: RiseRTCRoomPresentationHost::activate_call_window,
        present_call: RiseRTCRoomPresentationHost::present_call,
        uses_separate_window: RiseRTCRoomPresentationHost::uses_separate_window,
    });
}
