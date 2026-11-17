use crate::modules::rtc::engine::rise_rtc_call_system_hooks::{RiseRTCCallSystemHooks, set_rise_rtc_call_system_hooks};
use crate::modules::rtc::system::rise_rtc_call_system_coordinator::RiseRTCCallSystemCoordinator;

pub(crate) fn install_rise_rtc_call_system_hooks() {
    set_rise_rtc_call_system_hooks(RiseRTCCallSystemHooks {
        report_remote_call_event: |server_call_id, event, cx| {
            RiseRTCCallSystemCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.report_remote_call_event(server_call_id, event, cx));
        },
        report_realtime_incoming_call: |envelope, cx| {
            RiseRTCCallSystemCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.report_realtime_incoming_call(envelope, cx));
        },
        report_media_connected: |server_call_id, cx| {
            RiseRTCCallSystemCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.report_media_connected(server_call_id, cx));
        },
        drives_system_audio: |server_call_id, cx| {
            rise_ui::shared::try_read::<RiseRTCCallSystemCoordinator>(cx)
                .is_some_and(|coordinator| coordinator.drives_system_audio(server_call_id))
        },
        request_outgoing_call: |callee_id, chat_id, display_name, avatar_url, has_video, cx| {
            RiseRTCCallSystemCoordinator::shared(cx).update(cx, |coordinator, cx| {
                coordinator.request_outgoing_call(callee_id, chat_id, display_name, avatar_url, has_video, cx)
            })
        },
        request_end_call_uuid: |call_uuid, cx| {
            RiseRTCCallSystemCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.request_end_call_uuid(call_uuid, cx))
        },
        request_end_server_call_id: |server_call_id, cx| {
            RiseRTCCallSystemCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.request_end_server_call_id(server_call_id, cx))
        },
        request_answer: |server_call_id, cx| {
            RiseRTCCallSystemCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.request_answer(server_call_id, cx))
        },
        request_muted_call_uuid: |is_muted, call_uuid, cx| {
            RiseRTCCallSystemCoordinator::shared(cx).update(cx, |coordinator, _| coordinator.request_muted_call_uuid(is_muted, call_uuid))
        },
        request_muted_server_call_id: |is_muted, server_call_id, cx| {
            RiseRTCCallSystemCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.request_muted_server_call_id(is_muted, server_call_id, cx))
        },
        retry_vo_ip_registration: |cx| {
            RiseRTCCallSystemCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.retry_vo_ip_registration(cx));
        },
    });
}
