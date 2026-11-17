use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_date::{Date, TimeInterval};

use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCConnectionState, RiseRTCDirectCall, RiseRTCIncomingCall, RiseRTCParticipant, RiseRTCRaisedHand,
    RiseRTCReaction, RiseRTCRoomEviction, RiseRTCUserProfile,
};
use crate::modules::rtc::engine::rise_rtc_listener_volume_presentation::{
    RiseRTCListenerVolume, RiseRTCListenerVolumePresentation,
};
use crate::modules::rtc::engine::rise_rtc_presentation::{
    RiseRTCPresentation, RiseRTCPresentationSnapshot, RiseRTCRoomPresentation,
};
use crate::modules::rtc::shared::rise_rtc_navigation_gate::RiseRTCRoomTargetSafeLogKind;
use crate::modules::rtc::shared::rtc_guide_sandbox_hooks::RtcGuideSandboxHooks;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub struct RTCServicesStore {
    room_presentation_target: Option<RiseRTCRoomTarget>,
    is_room_presentation_visible: bool,
    focused_stream_user_id: Option<String>,
    last_raise_hand_at: Option<Date>,
    raise_hand_cooldown_revision: u64,

    raise_hand_cooldown_task: Option<Task<()>>,

    presentation: Entity<RiseRTCPresentation>,
    listener_volumes: Entity<RiseRTCListenerVolumePresentation>,
    user_services: Entity<UserServicesStore>,
}

impl RTCServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseRTCPresentation::shared(cx);
        let listener_volumes = RiseRTCListenerVolumePresentation::shared(cx);
        let user_services = UserServicesStore::shared(cx);
        cx.observe(&presentation, |_, _, cx| cx.notify()).detach();
        cx.observe(&listener_volumes, |_, _, cx| cx.notify()).detach();
        cx.observe(&user_services, |_, _, cx| cx.notify()).detach();
        Self {
            room_presentation_target: None,
            is_room_presentation_visible: false,
            focused_stream_user_id: None,
            last_raise_hand_at: None,
            raise_hand_cooldown_revision: 0,
            raise_hand_cooldown_task: None,
            presentation,
            listener_volumes,
            user_services,
        }
    }

    pub fn room_presentation_target(&self) -> Option<&RiseRTCRoomTarget> {
        self.room_presentation_target.as_ref()
    }

    pub fn is_room_presentation_visible(&self) -> bool {
        self.is_room_presentation_visible
    }

    pub fn focused_stream_user_id(&self) -> Option<&str> {
        self.focused_stream_user_id.as_deref()
    }

    pub fn last_raise_hand_at(&self) -> Option<Date> {
        self.last_raise_hand_at
    }

    pub fn raise_hand_cooldown_revision(&self) -> u64 {
        self.raise_hand_cooldown_revision
    }

    pub fn snapshot<'a>(&self, cx: &'a App) -> &'a RiseRTCPresentationSnapshot {
        self.presentation.read(cx).snapshot()
    }

    pub fn active_room<'a>(&self, cx: &'a App) -> Option<&'a RiseRTCRoomPresentation> {
        self.presentation.read(cx).current_active_room()
    }

    pub fn participants<'a>(&self, cx: &'a App) -> &'a [RiseRTCParticipant] {
        self.active_room(cx).map_or(&[], |room| room.participants.as_slice())
    }

    pub fn raised_hands<'a>(&self, cx: &'a App) -> &'a [RiseRTCRaisedHand] {
        self.active_room(cx).map_or(&[], |room| room.raised_hands.as_slice())
    }

    pub fn recent_calls<'a>(&self, cx: &'a App) -> &'a [RiseRTCDirectCall] {
        self.presentation.read(cx).current_recent_calls()
    }

    pub fn incoming_call<'a>(&self, cx: &'a App) -> Option<&'a RiseRTCIncomingCall> {
        self.presentation.read(cx).current_incoming_call()
    }

    pub fn reactions<'a>(&self, cx: &'a App) -> &'a [RiseRTCReaction] {
        self.presentation.read(cx).current_reactions()
    }

    pub fn connection_state(&self, cx: &App) -> RiseRTCConnectionState {
        self.presentation.read(cx).current_connection_state()
    }

    pub fn last_error_code<'a>(&self, cx: &'a App) -> Option<&'a str> {
        self.presentation.read(cx).current_last_error_code()
    }

    pub fn account_id<'a>(&self, cx: &'a App) -> Option<&'a str> {
        self.presentation.read(cx).current_account_id()
    }

    pub fn room_eviction<'a>(&self, cx: &'a App) -> Option<&'a RiseRTCRoomEviction> {
        self.presentation.read(cx).current_room_eviction()
    }

    pub fn verification_emojis<'a>(&self, call_id: Option<&str>, cx: &'a App) -> Option<&'a [String]> {
        let call_id = call_id?;
        self.presentation
            .read(cx)
            .verification_emojis_by_call()
            .get(call_id)
            .map(Vec::as_slice)
    }

    pub fn participant<'a>(&self, user_id: &str, cx: &'a App) -> Option<&'a RiseRTCParticipant> {
        self.participants(cx).iter().find(|participant| participant.user_id == user_id)
    }

    pub fn listener_volume_percent(&self, user_id: &str, cx: &App) -> f64 {
        RiseRTCListenerVolume::resolved_percent(
            self.listener_volumes.read(cx).snapshot().override_for(user_id),
            self.user_services.read(cx).is_totally_blocked(user_id, cx),
        )
    }

    pub fn is_muted_for_me(&self, user_id: &str, cx: &App) -> bool {
        RiseRTCListenerVolume::is_muted(self.listener_volume_percent(user_id, cx))
    }

    pub fn has_explicit_listener_volume(&self, user_id: &str, cx: &App) -> bool {
        self.listener_volumes.read(cx).snapshot().override_for(user_id).is_some()
    }

    pub fn call(&self, call_id: &str, cx: &App) -> Option<RiseRTCDirectCall> {
        if let Some(practice) = RtcGuideSandboxHooks::practice_call(call_id, cx) {
            return Some(practice);
        }
        self.recent_calls(cx).iter().find(|call| call.call_id == call_id).cloned()
    }

    pub fn practice_call_counterpart(&self, call_id: &str, cx: &App) -> Option<RiseRTCUserProfile> {
        RtcGuideSandboxHooks::practice_call_counterpart(call_id, cx)
    }

    pub fn present_room(&mut self, target: RiseRTCRoomTarget, cx: &mut Context<Self>) {
        let kind = target.safe_log_kind();
        self.room_presentation_target = Some(target);
        self.is_room_presentation_visible = true;
        cx.notify();
        self.report_room_facts(cx);
        Sai::log(
            "RTC",
            move || format!("room presentation state committed target={kind}"),
            SaiLogKind::Network,
            cx,
        );
    }

    pub fn minimize_room(&mut self, cx: &mut Context<Self>) {
        if self.room_presentation_target.is_none() {
            return;
        }
        self.is_room_presentation_visible = false;
        cx.notify();
        self.report_room_facts(cx);
    }

    fn report_room_facts(&self, cx: &mut Context<Self>) {
        GuideFacts::set(
            RiseGuideSignal::VoiceRoomOpened,
            self.room_presentation_target.is_some() && self.is_room_presentation_visible,
            cx,
        );
    }

    pub fn focus_stream(&mut self, user_id: Option<String>, cx: &mut Context<Self>) {
        self.focused_stream_user_id = user_id;
        cx.notify();
    }

    pub fn begin_raise_hand_cooldown(&mut self, now: Date, seconds: TimeInterval, cx: &mut Context<Self>) {
        self.raise_hand_cooldown_task = None;
        self.last_raise_hand_at = Some(now);
        cx.notify();
        if seconds <= 0.0 {
            return;
        }
        self.raise_hand_cooldown_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_secs_f64(seconds)).await;
            let _ = this.update(cx, |store, cx| {
                store.raise_hand_cooldown_revision = store.raise_hand_cooldown_revision.wrapping_add(1);
                store.raise_hand_cooldown_task = None;
                cx.notify();
            });
        }));
    }

    pub fn clear_raise_hand_cooldown(&mut self, cx: &mut Context<Self>) {
        self.raise_hand_cooldown_task = None;
        self.last_raise_hand_at = None;
        cx.notify();
    }

    pub fn acknowledge_room_eviction(&mut self, cx: &mut Context<Self>) {
        self.presentation
            .update(cx, |presentation, cx| presentation.acknowledge_eviction(cx));
    }

    pub fn dismiss_room_presentation(&mut self, cx: &mut Context<Self>) {
        self.room_presentation_target = None;
        self.is_room_presentation_visible = false;
        self.focused_stream_user_id = None;
        self.clear_raise_hand_cooldown(cx);
        self.report_room_facts(cx);
    }
}
