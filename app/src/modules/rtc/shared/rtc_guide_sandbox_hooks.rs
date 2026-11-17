use gpui::{App, Task};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCameraPosition, RiseRTCDirectCall, RiseRTCModerationAction, RiseRTCStateError, RiseRTCUserProfile,
};

pub struct RtcGuideSandboxHooks {
    pub forum_chat_id: String,
    pub voice_room_id: Box<dyn Fn(&str) -> Option<String> + Send + Sync>,
    pub owns_voice_room: Box<dyn Fn(Option<&str>, &App) -> bool + Send + Sync>,
    pub owns_call_room: Box<dyn Fn(Option<&str>, &App) -> bool + Send + Sync>,
    pub owns_call: Box<dyn Fn(Option<&str>, &App) -> bool + Send + Sync>,
    pub owns_user: Box<dyn Fn(&str, &App) -> bool + Send + Sync>,
    pub join_voice_room: Box<dyn Fn(&str, &mut App) -> Result<(), RiseRTCStateError> + Send + Sync>,
    pub leave_voice_room: Box<dyn Fn(&mut App) + Send + Sync>,
    pub refresh_voice_room: Box<dyn Fn(&mut App) + Send + Sync>,
    pub promote_to_speaker: Box<dyn Fn(&str, &mut App) + Send + Sync>,
    pub move_to_audience: Box<dyn Fn(&str, &mut App) + Send + Sync>,
    pub moderate: Box<dyn Fn(&str, RiseRTCModerationAction, &mut App) + Send + Sync>,
    pub set_self_microphone: Box<dyn Fn(bool, &mut App) + Send + Sync>,
    pub set_self_deafened: Box<dyn Fn(bool, &mut App) + Send + Sync>,
    pub start_practice_call: Box<dyn Fn(&str, &mut App) -> Option<Uuid> + Send + Sync>,
    pub end_practice_call: Box<dyn Fn(&mut App) -> Option<RiseRTCDirectCall> + Send + Sync>,
    pub practice_call: Box<dyn Fn(&str, &App) -> Option<RiseRTCDirectCall> + Send + Sync>,
    pub practice_call_counterpart: Box<dyn Fn(&str, &App) -> Option<RiseRTCUserProfile> + Send + Sync>,
    pub set_practice_call_microphone: Box<dyn Fn(bool, &mut App) + Send + Sync>,
    pub set_practice_call_deafened: Box<dyn Fn(bool, &mut App) + Send + Sync>,
    pub set_practice_call_camera:
        Box<dyn Fn(bool, RiseRTCCameraPosition, &mut App) -> Task<anyhow::Result<()>> + Send + Sync>,
    pub switch_practice_camera: Box<dyn Fn(RiseRTCCameraPosition, &mut App) -> Task<anyhow::Result<()>> + Send + Sync>,
    pub set_practice_screen_share: Box<dyn Fn(bool, &mut App) + Send + Sync>,
}

pub(crate) static RTC_GUIDE_SANDBOX_HOOKS: FeatureHookSlot<RtcGuideSandboxHooks> = FeatureHookSlot::new("RTC_GUIDE_SANDBOX_HOOKS");

/// a second install is ignored, the first owner keeps the slot
pub fn set_rtc_guide_sandbox_hooks(hooks: RtcGuideSandboxHooks) {
    let _ = RTC_GUIDE_SANDBOX_HOOKS.set(hooks);
}

pub fn rtc_guide_sandbox_hooks() -> Option<&'static RtcGuideSandboxHooks> {
    RTC_GUIDE_SANDBOX_HOOKS.get()
}

impl RtcGuideSandboxHooks {
    pub fn forum_chat_id() -> Option<&'static str> {
        rtc_guide_sandbox_hooks().map(|hooks| hooks.forum_chat_id.as_str())
    }

    pub fn voice_room_id(topic_id: &str) -> Option<String> {
        rtc_guide_sandbox_hooks().and_then(|hooks| (hooks.voice_room_id)(topic_id))
    }

    pub fn owns_voice_room(room_id: Option<&str>, cx: &App) -> bool {
        rtc_guide_sandbox_hooks().is_some_and(|hooks| (hooks.owns_voice_room)(room_id, cx))
    }

    pub fn owns_call_room(room_id: Option<&str>, cx: &App) -> bool {
        rtc_guide_sandbox_hooks().is_some_and(|hooks| (hooks.owns_call_room)(room_id, cx))
    }

    pub fn owns_call(call_id: Option<&str>, cx: &App) -> bool {
        rtc_guide_sandbox_hooks().is_some_and(|hooks| (hooks.owns_call)(call_id, cx))
    }

    pub fn owns_user(user_id: &str, cx: &App) -> bool {
        rtc_guide_sandbox_hooks().is_some_and(|hooks| (hooks.owns_user)(user_id, cx))
    }

    /// an empty slot owns no room so joining one through it is the
    /// missing room refusal the sandbox throws for a room it doesnt own
    pub fn join_voice_room(room_id: &str, cx: &mut App) -> Result<(), RiseRTCStateError> {
        match rtc_guide_sandbox_hooks() {
            Some(hooks) => (hooks.join_voice_room)(room_id, cx),
            None => Err(RiseRTCStateError::MissingActiveRoom),
        }
    }

    pub fn leave_voice_room(cx: &mut App) {
        if let Some(hooks) = rtc_guide_sandbox_hooks() {
            (hooks.leave_voice_room)(cx);
        }
    }

    pub fn refresh_voice_room(cx: &mut App) {
        if let Some(hooks) = rtc_guide_sandbox_hooks() {
            (hooks.refresh_voice_room)(cx);
        }
    }

    pub fn promote_to_speaker(user_id: &str, cx: &mut App) {
        if let Some(hooks) = rtc_guide_sandbox_hooks() {
            (hooks.promote_to_speaker)(user_id, cx);
        }
    }

    pub fn move_to_audience(user_id: &str, cx: &mut App) {
        if let Some(hooks) = rtc_guide_sandbox_hooks() {
            (hooks.move_to_audience)(user_id, cx);
        }
    }

    pub fn moderate(user_id: &str, action: RiseRTCModerationAction, cx: &mut App) {
        if let Some(hooks) = rtc_guide_sandbox_hooks() {
            (hooks.moderate)(user_id, action, cx);
        }
    }

    pub fn set_self_microphone(enabled: bool, cx: &mut App) {
        if let Some(hooks) = rtc_guide_sandbox_hooks() {
            (hooks.set_self_microphone)(enabled, cx);
        }
    }

    pub fn set_self_deafened(deafened: bool, cx: &mut App) {
        if let Some(hooks) = rtc_guide_sandbox_hooks() {
            (hooks.set_self_deafened)(deafened, cx);
        }
    }

    pub fn start_practice_call(callee_id: &str, cx: &mut App) -> Option<Uuid> {
        rtc_guide_sandbox_hooks().and_then(|hooks| (hooks.start_practice_call)(callee_id, cx))
    }

    pub fn end_practice_call(cx: &mut App) -> Option<RiseRTCDirectCall> {
        rtc_guide_sandbox_hooks().and_then(|hooks| (hooks.end_practice_call)(cx))
    }

    pub fn practice_call(call_id: &str, cx: &App) -> Option<RiseRTCDirectCall> {
        rtc_guide_sandbox_hooks().and_then(|hooks| (hooks.practice_call)(call_id, cx))
    }

    pub fn practice_call_counterpart(call_id: &str, cx: &App) -> Option<RiseRTCUserProfile> {
        rtc_guide_sandbox_hooks().and_then(|hooks| (hooks.practice_call_counterpart)(call_id, cx))
    }

    pub fn set_practice_call_microphone(enabled: bool, cx: &mut App) {
        if let Some(hooks) = rtc_guide_sandbox_hooks() {
            (hooks.set_practice_call_microphone)(enabled, cx);
        }
    }

    pub fn set_practice_call_deafened(deafened: bool, cx: &mut App) {
        if let Some(hooks) = rtc_guide_sandbox_hooks() {
            (hooks.set_practice_call_deafened)(deafened, cx);
        }
    }

    pub fn set_practice_call_camera(
        enabled: bool,
        position: RiseRTCCameraPosition,
        cx: &mut App,
    ) -> Task<anyhow::Result<()>> {
        match rtc_guide_sandbox_hooks() {
            Some(hooks) => (hooks.set_practice_call_camera)(enabled, position, cx),
            None => Task::ready(Ok(())),
        }
    }

    pub fn switch_practice_camera(position: RiseRTCCameraPosition, cx: &mut App) -> Task<anyhow::Result<()>> {
        match rtc_guide_sandbox_hooks() {
            Some(hooks) => (hooks.switch_practice_camera)(position, cx),
            None => Task::ready(Ok(())),
        }
    }

    pub fn set_practice_screen_share(enabled: bool, cx: &mut App) {
        if let Some(hooks) = rtc_guide_sandbox_hooks() {
            (hooks.set_practice_screen_share)(enabled, cx);
        }
    }
}
