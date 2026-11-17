use gpui::{App, Task};

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::rtc::shared::rtc_guide_sandbox_hooks::{RtcGuideSandboxHooks, set_rtc_guide_sandbox_hooks};

pub fn install_rtc_guide_sandbox_hooks() {
    set_rtc_guide_sandbox_hooks(RtcGuideSandboxHooks {
        forum_chat_id: GuideSandbox::forum_chat_id(),
        voice_room_id: Box::new(GuideSandbox::voice_room_id),
        owns_voice_room: Box::new(|id: Option<&str>, cx: &App| read(cx, |sandbox| sandbox.owns_voice_room_id(id))),
        owns_call_room: Box::new(|id: Option<&str>, cx: &App| read(cx, |sandbox| sandbox.owns_call_room_id(id))),
        owns_call: Box::new(|id: Option<&str>, cx: &App| read(cx, |sandbox| sandbox.owns_call_id(id))),
        owns_user: Box::new(|id: &str, cx: &App| read(cx, |sandbox| sandbox.owns_user_id(id))),
        join_voice_room: Box::new(|room_id: &str, cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.join_voice_room(room_id, cx))
        }),
        leave_voice_room: Box::new(|cx: &mut App| GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.leave_voice_room(cx))),
        refresh_voice_room: Box::new(|cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.refresh_voice_room(cx));
        }),
        promote_to_speaker: Box::new(|user_id: &str, cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.promote_to_speaker(user_id, cx));
        }),
        move_to_audience: Box::new(|user_id: &str, cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.move_to_audience(user_id, cx));
        }),
        moderate: Box::new(|user_id: &str, action, cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.moderate(user_id, action, cx));
        }),
        set_self_microphone: Box::new(|enabled: bool, cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_self_microphone(enabled, cx));
        }),
        set_self_deafened: Box::new(|deafened: bool, cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_self_deafened(deafened, cx));
        }),
        start_practice_call: Box::new(|callee_id: &str, cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.start_practice_call(callee_id, cx))
        }),
        end_practice_call: Box::new(|cx: &mut App| GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.end_practice_call(cx))),
        practice_call: Box::new(|call_id: &str, cx: &App| {
            GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).practice_call(call_id))
        }),
        practice_call_counterpart: Box::new(|call_id: &str, cx: &App| {
            GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).practice_call_counterpart(call_id))
        }),
        set_practice_call_microphone: Box::new(|enabled: bool, cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_practice_call_microphone(enabled, cx));
        }),
        set_practice_call_deafened: Box::new(|deafened: bool, cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_practice_call_deafened(deafened, cx));
        }),
        set_practice_call_camera: Box::new(|enabled: bool, position, cx: &mut App| -> Task<anyhow::Result<()>> {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_practice_call_camera(enabled, position, cx))
        }),
        switch_practice_camera: Box::new(|position, cx: &mut App| -> Task<anyhow::Result<()>> {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.switch_practice_camera(position, cx))
        }),
        set_practice_screen_share: Box::new(|enabled: bool, cx: &mut App| {
            GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_practice_screen_share(enabled, cx));
        }),
    });
}

fn read(cx: &App, owns: impl FnOnce(&GuideSandbox) -> bool) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| owns(sandbox.read(cx)))
}
