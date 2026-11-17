use gpui::{App, Task};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCStateError;
use crate::modules::rtc::system::rise_rtc_system_models::{RiseRTCSystemIncomingEnvelope, RiseRTCSystemVoIPEvent};

const CALL_SYSTEM_UNAVAILABLE: &str = "rtc_callkit_unavailable";

/// members of the shared call system coordinator that the engine and stores call
pub struct RiseRTCCallSystemHooks {
    pub(crate) report_remote_call_event: fn(&str, RiseRTCSystemVoIPEvent, &mut App),
    pub(crate) report_realtime_incoming_call: fn(RiseRTCSystemIncomingEnvelope, &mut App),
    pub(crate) report_media_connected: fn(&str, &mut App),
    pub(crate) drives_system_audio: fn(&str, &App) -> bool,
    pub(crate) request_outgoing_call: fn(String, String, String, Option<String>, bool, &mut App) -> Task<anyhow::Result<Uuid>>,
    pub(crate) request_end_call_uuid: fn(Uuid, &mut App) -> Task<anyhow::Result<()>>,
    pub(crate) request_end_server_call_id: fn(String, &mut App) -> Task<anyhow::Result<bool>>,
    pub(crate) request_answer: fn(String, &mut App) -> Task<bool>,
    pub(crate) request_muted_call_uuid: fn(bool, Uuid, &mut App) -> Task<anyhow::Result<()>>,
    pub(crate) request_muted_server_call_id: fn(bool, String, &mut App) -> Task<anyhow::Result<bool>>,
    pub(crate) retry_vo_ip_registration: fn(&mut App),
}

pub(crate) static RISE_RTC_CALL_SYSTEM_HOOKS: FeatureHookSlot<RiseRTCCallSystemHooks> = FeatureHookSlot::new("RISE_RTC_CALL_SYSTEM_HOOKS");

/// a second install is ignored, the first owner keeps the slot
pub(crate) fn set_rise_rtc_call_system_hooks(hooks: RiseRTCCallSystemHooks) {
    let _ = RISE_RTC_CALL_SYSTEM_HOOKS.set(hooks);
}

pub(crate) fn rise_rtc_call_system_hooks() -> Option<&'static RiseRTCCallSystemHooks> {
    RISE_RTC_CALL_SYSTEM_HOOKS.get()
}

fn call_system_unavailable() -> anyhow::Error {
    RiseRTCStateError::Remote(CALL_SYSTEM_UNAVAILABLE.to_owned()).into()
}

impl RiseRTCCallSystemHooks {
    pub(crate) fn report_remote_call_event(server_call_id: &str, event: RiseRTCSystemVoIPEvent, cx: &mut App) {
        if let Some(hooks) = rise_rtc_call_system_hooks() {
            (hooks.report_remote_call_event)(server_call_id, event, cx);
        }
    }

    pub(crate) fn report_realtime_incoming_call(envelope: RiseRTCSystemIncomingEnvelope, cx: &mut App) {
        if let Some(hooks) = rise_rtc_call_system_hooks() {
            (hooks.report_realtime_incoming_call)(envelope, cx);
        }
    }

    pub(crate) fn report_media_connected(server_call_id: &str, cx: &mut App) {
        if let Some(hooks) = rise_rtc_call_system_hooks() {
            (hooks.report_media_connected)(server_call_id, cx);
        }
    }

    pub(crate) fn drives_system_audio(server_call_id: &str, cx: &App) -> bool {
        rise_rtc_call_system_hooks().is_some_and(|hooks| (hooks.drives_system_audio)(server_call_id, cx))
    }

    pub(crate) fn request_outgoing_call(
        callee_id: String,
        chat_id: String,
        display_name: String,
        avatar_url: Option<String>,
        has_video: bool,
        cx: &mut App,
    ) -> Task<anyhow::Result<Uuid>> {
        match rise_rtc_call_system_hooks() {
            Some(hooks) => (hooks.request_outgoing_call)(callee_id, chat_id, display_name, avatar_url, has_video, cx),
            None => Task::ready(Err(call_system_unavailable())),
        }
    }

    pub(crate) fn request_end_call_uuid(call_uuid: Uuid, cx: &mut App) -> Task<anyhow::Result<()>> {
        match rise_rtc_call_system_hooks() {
            Some(hooks) => (hooks.request_end_call_uuid)(call_uuid, cx),
            None => Task::ready(Err(call_system_unavailable())),
        }
    }

    pub fn request_end_server_call_id(server_call_id: String, cx: &mut App) -> Task<anyhow::Result<bool>> {
        match rise_rtc_call_system_hooks() {
            Some(hooks) => (hooks.request_end_server_call_id)(server_call_id, cx),
            None => Task::ready(Ok(false)),
        }
    }

    pub(crate) fn request_answer(server_call_id: String, cx: &mut App) -> Task<bool> {
        match rise_rtc_call_system_hooks() {
            Some(hooks) => (hooks.request_answer)(server_call_id, cx),
            None => Task::ready(false),
        }
    }

    pub(crate) fn request_muted_call_uuid(is_muted: bool, call_uuid: Uuid, cx: &mut App) -> Task<anyhow::Result<()>> {
        match rise_rtc_call_system_hooks() {
            Some(hooks) => (hooks.request_muted_call_uuid)(is_muted, call_uuid, cx),
            None => Task::ready(Err(call_system_unavailable())),
        }
    }

    pub fn request_muted_server_call_id(is_muted: bool, server_call_id: String, cx: &mut App) -> Task<anyhow::Result<bool>> {
        match rise_rtc_call_system_hooks() {
            Some(hooks) => (hooks.request_muted_server_call_id)(is_muted, server_call_id, cx),
            None => Task::ready(Ok(false)),
        }
    }

    pub(crate) fn retry_vo_ip_registration(cx: &mut App) {
        if let Some(hooks) = rise_rtc_call_system_hooks() {
            (hooks.retry_vo_ip_registration)(cx);
        }
    }
}
