use std::future::Future;
use std::sync::Arc;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;

use crate::modules::rtc::engine::rise_rtc_call_system_hooks::RiseRTCCallSystemHooks;
use crate::modules::rtc::engine::rise_rtc_domain::{
    RiseRTCActionExecutor, RiseRTCDomain, RiseRTCDomainRegistry, RiseRTCHangupAction,
};
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCAnswerOutcome, RiseRTCCallMediaMode, RiseRTCCameraPosition, RiseRTCDirectCall, RiseRTCModerationAction,
    RiseRTCParticipantRole, RiseRTCScreenShareQuality,
};

pub struct RTCActionsStore;

impl RTCActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self
    }

    pub fn new_idempotency_key() -> String {
        Uuid::new_v4().uuid_string().to_lowercase()
    }

    fn with_domain<T, F, Fut>(cx: &mut Context<Self>, work: F) -> Task<anyhow::Result<T>>
    where
        T: 'static,
        F: FnOnce(Arc<RiseRTCDomain>) -> Fut + 'static,
        Fut: Future<Output = anyhow::Result<T>> + 'static,
    {
        let domain = RiseRTCActionExecutor::require_domain(cx);
        cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| work(domain.await?).await)
    }

    fn with_optional_domain<F, Fut>(cx: &mut Context<Self>, work: F) -> Task<()>
    where
        F: FnOnce(Arc<RiseRTCDomain>) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let domain = RiseRTCActionExecutor::require_domain(cx);
        cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            work(domain).await;
        })
    }

    fn active_domain_for_session(expected_session_id: Uuid, cx: &mut App) -> Option<Arc<RiseRTCDomain>> {
        RiseRTCDomainRegistry::shared(cx)
            .read(cx)
            .active()
            .filter(|domain| domain.session_id == expected_session_id)
            .cloned()
    }

    pub fn end_direct_call_for_room_switch(
        &mut self,
        call_id: String,
        expected_session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let Some(domain) = Self::active_domain_for_session(expected_session_id, cx) else {
            return Task::ready(Err(CancellationError.into()));
        };
        cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| {
            domain.end_direct_call_for_room_switch(&call_id).await
        })
    }

    pub fn join_room(
        &mut self,
        room_id: String,
        display_name: Option<String>,
        preferred_role: Option<RiseRTCParticipantRole>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move {
            domain.join_room(&room_id, display_name.as_deref(), preferred_role).await
        })
    }

    pub fn leave_room(&mut self, cx: &mut Context<Self>) -> Task<()> {
        Self::with_optional_domain(cx, move |domain| async move { domain.leave_room().await })
    }

    pub fn leave_room_expected_room_id(&mut self, expected_room_id: String, cx: &mut Context<Self>) -> Task<()> {
        Self::with_optional_domain(cx, move |domain| async move {
            domain.leave_room_expected_room_id(&expected_room_id).await;
        })
    }

    pub fn leave_room_expected_room_id_session_id(
        &mut self,
        expected_room_id: String,
        expected_session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let Some(domain) = Self::active_domain_for_session(expected_session_id, cx) else {
            return Task::ready(());
        };
        cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| {
            domain.leave_room_expected_room_id(&expected_room_id).await;
        })
    }

    pub fn refresh_active_room(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.refresh_active_room().await })
    }

    pub fn load_more_participants(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.load_more_participants().await })
    }

    pub fn request_to_speak(&mut self, note: Option<String>, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.request_to_speak(note.as_deref()).await })
    }

    pub fn cancel_request_to_speak(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.cancel_request_to_speak().await })
    }

    pub fn promote_to_speaker(&mut self, user_id: String, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.promote_to_speaker(&user_id).await })
    }

    pub fn move_to_audience(&mut self, user_id: String, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.move_to_audience(&user_id).await })
    }

    pub fn moderate(
        &mut self,
        user_id: String,
        action: RiseRTCModerationAction,
        reason: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move {
            domain.moderate(&user_id, action, reason.as_deref()).await
        })
    }

    pub fn send_reaction(&mut self, emoji: String, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.send_reaction(&emoji).await })
    }

    pub fn set_microphone(&mut self, enabled: bool, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.set_microphone(enabled).await })
    }

    pub fn set_deafened(&mut self, deafened: bool, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.set_deafened(deafened).await })
    }

    pub fn preview_listener_volume_action(
        &mut self,
        user_id: String,
        percent: f64,
        sequence: u64,
        cx: &mut Context<Self>,
    ) {
        Self::with_optional_domain(cx, move |domain| async move {
            domain.preview_listener_volume(&user_id, percent, sequence).await;
        })
        .detach();
    }

    pub fn commit_listener_volume_action(
        &mut self,
        user_id: String,
        percent: f64,
        sequence: u64,
        cx: &mut Context<Self>,
    ) {
        Self::with_optional_domain(cx, move |domain| async move {
            domain.commit_listener_volume(&user_id, percent, sequence).await;
        })
        .detach();
    }

    pub fn reset_listener_volume_action(&mut self, user_id: String, sequence: u64, cx: &mut Context<Self>) {
        Self::with_optional_domain(cx, move |domain| async move {
            domain.reset_listener_volume(&user_id, sequence).await;
        })
        .detach();
    }

    pub fn refresh_listener_volumes_action(&mut self, cx: &mut Context<Self>) {
        Self::with_optional_domain(cx, move |domain| async move { domain.refresh_listener_volumes().await }).detach();
    }

    pub fn set_camera(
        &mut self,
        enabled: bool,
        position: RiseRTCCameraPosition,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.set_camera(enabled, position).await })
    }

    pub fn set_screen_share(
        &mut self,
        enabled: bool,
        include_device_audio: bool,
        quality: RiseRTCScreenShareQuality,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move {
            domain.set_screen_share(enabled, include_device_audio, quality).await
        })
    }

    pub fn switch_camera(&mut self, position: RiseRTCCameraPosition, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.switch_camera(position).await })
    }

    pub fn select_audio_input(&mut self, id: Option<String>, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.select_audio_input(id.as_deref()).await })
    }

    pub fn select_audio_output(&mut self, id: Option<String>, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move { domain.select_audio_output(id.as_deref()).await })
    }

    pub fn start_direct_call(
        &mut self,
        callee_id: String,
        chat_id: String,
        media_mode: RiseRTCCallMediaMode,
        idempotency_key: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        let idempotency_key = idempotency_key.unwrap_or_else(Self::new_idempotency_key);
        Self::with_domain(cx, move |domain| async move {
            domain
                .start_direct_call(&callee_id, &chat_id, media_mode, &idempotency_key)
                .await
        })
    }

    pub fn request_system_outgoing_call_action(
        &mut self,
        callee_id: String,
        chat_id: String,
        display_name: String,
        avatar_url: Option<String>,
        has_video: bool,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Uuid>> {
        RiseRTCCallSystemHooks::request_outgoing_call(callee_id, chat_id, display_name, avatar_url, has_video, cx)
    }

    pub fn request_system_call_end_action_call_uuid(
        &mut self,
        call_uuid: Uuid,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        RiseRTCCallSystemHooks::request_end_call_uuid(call_uuid, cx)
    }

    pub fn request_system_call_end_action_server_call_id(
        &mut self,
        server_call_id: String,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<bool>> {
        RiseRTCCallSystemHooks::request_end_server_call_id(server_call_id, cx)
    }

    pub fn request_system_call_answer_action(&mut self, server_call_id: String, cx: &mut Context<Self>) -> Task<bool> {
        RiseRTCCallSystemHooks::request_answer(server_call_id, cx)
    }

    pub fn request_system_call_muted_action_call_uuid(
        &mut self,
        is_muted: bool,
        call_uuid: Uuid,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        RiseRTCCallSystemHooks::request_muted_call_uuid(is_muted, call_uuid, cx)
    }

    pub fn request_system_call_muted_action_server_call_id(
        &mut self,
        is_muted: bool,
        server_call_id: String,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<bool>> {
        RiseRTCCallSystemHooks::request_muted_server_call_id(is_muted, server_call_id, cx)
    }

    pub fn answer_call(
        &mut self,
        call_id: String,
        display_name: Option<String>,
        installation_id: String,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCAnswerOutcome>> {
        Self::with_domain(cx, move |domain| async move {
            domain
                .answer_call(&call_id, display_name.as_deref(), &installation_id)
                .await
        })
    }

    pub fn rejoin_call(
        &mut self,
        call_id: String,
        installation_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        Self::with_domain(cx, move |domain| async move {
            domain.rejoin_call(&call_id, installation_id.as_deref()).await
        })
    }

    pub fn decline_call(
        &mut self,
        call_id: String,
        reason: Option<String>,
        idempotency_key: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        let idempotency_key = idempotency_key.unwrap_or_else(Self::new_idempotency_key);
        Self::with_domain(cx, move |domain| async move {
            domain.decline_call(&call_id, reason.as_deref(), &idempotency_key).await
        })
    }

    pub fn cancel_call(
        &mut self,
        call_id: String,
        reason: Option<String>,
        idempotency_key: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        let idempotency_key = idempotency_key.unwrap_or_else(Self::new_idempotency_key);
        Self::with_domain(cx, move |domain| async move {
            domain.cancel_call(&call_id, reason.as_deref(), &idempotency_key).await
        })
    }

    pub fn end_call(
        &mut self,
        call_id: String,
        reason: Option<String>,
        idempotency_key: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        let idempotency_key = idempotency_key.unwrap_or_else(Self::new_idempotency_key);
        Self::with_domain(cx, move |domain| async move {
            domain.end_call(&call_id, reason.as_deref(), &idempotency_key).await
        })
    }

    pub fn deliver_hangup_when_online(
        &mut self,
        action: RiseRTCHangupAction,
        call_id: String,
        reason: String,
        idempotency_key: String,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        Self::with_domain(cx, move |domain| async move {
            domain.deliver_hangup_when_online(action, &call_id, &reason, &idempotency_key);
            Ok(())
        })
    }

    pub fn set_call_recording(
        &mut self,
        call_id: String,
        on: bool,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        let idempotency_key = Self::new_idempotency_key();
        Self::with_domain(cx, move |domain| async move {
            if on {
                domain.start_call_recording(&call_id, &idempotency_key).await
            } else {
                domain.stop_call_recording(&call_id, &idempotency_key).await
            }
        })
    }
}
