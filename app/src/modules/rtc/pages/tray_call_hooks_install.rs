use std::rc::Rc;

use gpui::App;

use crate::core::desktop_platform::tray_call_hooks::{TrayCallHooks, set_tray_call_hooks};
use crate::modules::rtc::components::rise_rtc_rail_call_card::RiseRTCRailCallCard;
use crate::modules::rtc::pages::rise_rtc_room_presentation_host::RiseRTCRoomPresentationHost;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::rtc::system::rise_rtc_system_call_presentation::RiseRTCSystemCallPresentationStore;

pub fn install_tray_call_hooks() {
    set_tray_call_hooks(TrayCallHooks {
        state: RiseRTCRailCallCard::tray_state,
        observe: |callback, cx| {
            let callback: Rc<dyn Fn(&mut App)> = Rc::from(callback);
            let rtc = RTCServicesStore::shared(cx);
            let system_calls = RiseRTCSystemCallPresentationStore::shared(cx);
            let on_room = Rc::clone(&callback);
            vec![
                cx.observe(&rtc, move |_, cx| on_room(cx)),
                cx.observe(&system_calls, move |_, cx| callback(cx)),
            ]
        },
        toggle_microphone: RiseRTCRailCallCard::tray_toggle_microphone,
        toggle_deafened: RiseRTCRailCallCard::tray_toggle_deafened,
        open_call: RiseRTCRoomPresentationHost::present_call,
        leave_call: RiseRTCRailCallCard::tray_leave,
    });
}
