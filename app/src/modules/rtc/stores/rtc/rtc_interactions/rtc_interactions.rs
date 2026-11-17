use std::collections::HashMap;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::RiseGuideSignal;
use rise_canon::rise_key::rtc as rtc_key;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_i18n::tr;
use rise_ui::native_confirm_alert::NativeConfirmAlert;

use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::lib::rise_call_audio_ownership::RiseCallAudioOwnership;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::VoiceChannelConfig;
use crate::modules::rtc::engine::rise_rtc_domain::{RiseRTCDomainRegistry, RiseRTCHangupAction};
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCAnswerOutcome, RiseRTCCallMediaMode, RiseRTCCallStateExt, RiseRTCCameraPosition, RiseRTCDirectCall,
    RiseRTCModerationAction, RiseRTCParticipantRole, RiseRTCRoomEviction, RiseRTCRoomMode, RiseRTCScreenShareQuality,
    RiseRTCStateError,
};
use crate::modules::rtc::engine::rise_rtc_listener_volume_presentation::RiseRTCListenerVolume;
use crate::modules::rtc::engine::rise_rtc_presentation::RiseRTCPresentationSnapshot;
use crate::modules::rtc::shared::rise_rtc_navigation_gate::RiseRTCNavigationGate;
use crate::modules::rtc::shared::rise_rtc_raise_hand_cooldown::RiseRTCRaiseHandCooldown;
use crate::modules::rtc::shared::rise_rtc_room_locator::{RiseRTCRoomLocator, RiseRTCRoomTargetRoomID};
use crate::modules::rtc::shared::rtc_field_rules::RTCFieldRules;
use crate::modules::rtc::shared::rtc_guide_sandbox_hooks::RtcGuideSandboxHooks;
use crate::modules::rtc::stores::rtc::rtc_actions::rtc_actions::RTCActionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::rtc::system::rise_rtc_system_call_presentation::RiseRTCSystemCallPresentationStore;
use crate::modules::rtc::system::rise_rtc_system_models::RiseRTCSystemCallPhase;

pub struct RTCInteractionsStore {
    actions: Entity<RTCActionsStore>,
    service: Entity<RTCServicesStore>,

    listener_volume_sequence: u64,
    listener_volume_before_mute: HashMap<String, f64>,
    listener_volume_before_mute_scope: Option<String>,
    microphone_before_deafen: Option<(String, bool)>,
    is_switching_from_direct_call: bool,
    is_leaving_for_music: bool,
}

impl RTCInteractionsStore {
    const LISTENER_VOLUME_BEFORE_MUTE_LIMIT: usize = 64;
    const REACTION_CHARACTER_RANGE: std::ops::RangeInclusive<usize> = 1..=16;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let actions = RTCActionsStore::shared(cx);
        let service = RTCServicesStore::shared(cx);
        cx.observe(&service, |_, _, cx| cx.notify()).detach();
        Self {
            actions,
            service,
            listener_volume_sequence: 0,
            listener_volume_before_mute: HashMap::new(),
            listener_volume_before_mute_scope: None,
            microphone_before_deafen: None,
            is_switching_from_direct_call: false,
            is_leaving_for_music: false,
        }
    }

    pub fn actions(&self) -> Entity<RTCActionsStore> {
        self.actions.clone()
    }

    pub fn service(&self) -> Entity<RTCServicesStore> {
        self.service.clone()
    }

    pub fn snapshot<'a>(&self, cx: &'a App) -> &'a RiseRTCPresentationSnapshot {
        self.service.read(cx).snapshot(cx)
    }

    fn active_session_id(cx: &mut App) -> Option<Uuid> {
        RiseRTCDomainRegistry::shared(cx)
            .read(cx)
            .active()
            .map(|domain| domain.session_id)
    }

    fn active_room_id(&self, cx: &App) -> Option<String> {
        self.service
            .read(cx)
            .active_room(cx)
            .map(|room| room.config.room_id.clone())
    }

    fn active_room_joined_at_ms(&self, cx: &App) -> Option<i64> {
        self.service
            .read(cx)
            .active_room(cx)
            .and_then(|room| room.self_participant.as_ref())
            .map(|participant| participant.joined_at_ms)
    }

    fn notify_error(title: String, cx: &mut App) {
        Notifier::send(
            title,
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn request_music_playback(&mut self, retry: Box<dyn FnOnce(&mut App)>, cx: &mut Context<Self>) -> bool {
        if self.is_leaving_for_music {
            return false;
        }
        let call_id = self.active_direct_call_id(cx);
        let room_id = self.active_room_id(cx);
        let joined_at_ms = self.active_room_joined_at_ms(cx);
        if call_id.is_none() && room_id.is_none() {
            return !RiseCallAudioOwnership::is_call_active();
        }
        let Some(session_id) = Self::active_session_id(cx) else {
            return false;
        };
        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let this = cx.weak_entity();
        NativeConfirmAlert::present_delete(
            &tr("music_call_interrupt_title"),
            &tr("music_call_interrupt_message"),
            Some(tr("rtc_leave")),
            None,
            false,
            None,
            None,
            move |cx| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                if !SaiAccountCacheScope::is_current_account_epoch(account_id.as_deref(), account_epoch.as_deref())
                    || Self::active_session_id(cx) != Some(session_id)
                {
                    return;
                }
                this.update(cx, |store, cx| {
                    if store.active_direct_call_id(cx) != call_id {
                        return;
                    }
                    if call_id.is_none()
                        && (store.active_room_id(cx) != room_id || store.active_room_joined_at_ms(cx) != joined_at_ms)
                    {
                        return;
                    }
                    store.leave_for_music(call_id, room_id, session_id, account_id, account_epoch, retry, cx);
                });
            },
            cx,
        );
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn leave_for_music(
        &mut self,
        call_id: Option<String>,
        room_id: Option<String>,
        session_id: Uuid,
        account_id: Option<String>,
        account_epoch: Option<String>,
        retry: Box<dyn FnOnce(&mut App)>,
        cx: &mut Context<Self>,
    ) {
        self.is_leaving_for_music = true;
        let work: Task<anyhow::Result<()>> = if let Some(call_id) = call_id {
            self.actions.update(cx, |actions, cx| {
                actions.end_direct_call_for_room_switch(call_id, session_id, cx)
            })
        } else if let Some(room_id) = room_id {
            let target = self.service.read(cx).room_presentation_target().cloned();
            if let Some(target) = target
                && target.room_id().as_deref() == Some(room_id.as_str())
            {
                RiseRTCNavigationGate::mark_self_in_voice_channel(&target, false, cx);
            }
            let leave = self.actions.update(cx, |actions, cx| {
                actions.leave_room_expected_room_id_session_id(room_id, session_id, cx)
            });
            cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| {
                leave.await;
                Ok(())
            })
        } else {
            Task::ready(Ok(()))
        };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = work.await;
            let _ = this.update(cx, move |store, cx| {
                store.is_leaving_for_music = false;
                match result {
                    Ok(()) => {
                        let still_current = SaiAccountCacheScope::is_current_account_epoch(
                            account_id.as_deref(),
                            account_epoch.as_deref(),
                        ) && Self::active_session_id(cx) == Some(session_id)
                            && store.active_direct_call_id(cx).is_none()
                            && store.service.read(cx).active_room(cx).is_none()
                            && !RiseCallAudioOwnership::is_call_active();
                        if still_current {
                            retry(cx);
                        }
                    }
                    Err(error) if error.is::<CancellationError>() => {}
                    Err(_) => {
                        if SaiAccountCacheScope::is_current_account_epoch(
                            account_id.as_deref(),
                            account_epoch.as_deref(),
                        ) {
                            Self::notify_error(tr("rtc_action_failed"), cx);
                        }
                    }
                }
            });
        })
        .detach();
    }

    fn sandbox_owns_active_room(&self, cx: &App) -> bool {
        let room_id = self.active_room_id(cx);
        RtcGuideSandboxHooks::owns_voice_room(room_id.as_deref(), cx) || self.sandbox_owns_active_call(cx)
    }

    fn sandbox_owns_active_call(&self, cx: &App) -> bool {
        let room_id = self.active_room_id(cx);
        RtcGuideSandboxHooks::owns_call_room(room_id.as_deref(), cx)
    }

    pub fn present_room(&mut self, target: RiseRTCRoomTarget, cx: &mut Context<Self>) {
        if matches!(target, RiseRTCRoomTarget::DirectCall { .. }) {
            self.service.update(cx, |service, cx| service.present_room(target, cx));
            return;
        }
        if self.is_switching_from_direct_call {
            return;
        }
        let Some(call_id) = self.active_direct_call_id(cx) else {
            self.commit_voice_room_presentation(target, cx);
            return;
        };
        let Some(session_id) = Self::active_session_id(cx) else {
            return;
        };
        let this = cx.weak_entity();
        NativeConfirmAlert::present_delete(
            &tr("rtc_leave_call_title"),
            &tr("rtc_leave_call_for_voice_message"),
            Some(tr("rtc_leave")),
            None,
            false,
            None,
            None,
            move |cx| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                if Self::active_session_id(cx) != Some(session_id) {
                    return;
                }
                this.update(cx, |store, cx| {
                    if store.active_direct_call_id(cx).as_deref() != Some(call_id.as_str()) {
                        store.present_room(target, cx);
                        return;
                    }
                    store.switch_from_direct_call(call_id, session_id, target, cx);
                });
            },
            cx,
        );
    }

    fn switch_from_direct_call(
        &mut self,
        call_id: String,
        session_id: Uuid,
        target: RiseRTCRoomTarget,
        cx: &mut Context<Self>,
    ) {
        self.is_switching_from_direct_call = true;
        let work = self.actions.update(cx, |actions, cx| {
            actions.end_direct_call_for_room_switch(call_id, session_id, cx)
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = work.await;
            let _ = this.update(cx, move |store, cx| {
                store.is_switching_from_direct_call = false;
                match result {
                    Ok(()) => {
                        if Self::active_session_id(cx) == Some(session_id) && store.active_direct_call_id(cx).is_none() {
                            store.commit_voice_room_presentation(target, cx);
                        }
                    }
                    Err(error) if error.is::<CancellationError>() => {}
                    Err(_) => {
                        if Self::active_session_id(cx) == Some(session_id) {
                            Self::notify_error(tr("rtc_join_failed"), cx);
                        }
                    }
                }
            });
        })
        .detach();
    }

    fn active_direct_call_id(&self, cx: &mut App) -> Option<String> {
        let service = self.service.read(cx);
        if let Some(room) = service.active_room(cx)
            && room.config.mode == RiseRTCRoomMode::DirectCall
            && let Some(call_id) = RiseRTCRoomLocator::direct_call_id(&room.config.room_id)
            && service.call(&call_id, cx).map(|call| call.state.is_terminal()) != Some(true)
        {
            return Some(call_id);
        }
        RiseRTCSystemCallPresentationStore::shared(cx)
            .read(cx)
            .calls()
            .iter()
            .find(|call| matches!(call.phase, RiseRTCSystemCallPhase::Active | RiseRTCSystemCallPhase::Answering))
            .and_then(|call| call.server_call_id.clone())
    }

    fn commit_voice_room_presentation(&mut self, target: RiseRTCRoomTarget, cx: &mut Context<Self>) {
        RiseRTCNavigationGate::mark_self_in_voice_channel(&target, true, cx);
        self.service.update(cx, |service, cx| service.present_room(target, cx));
    }

    pub fn minimize_room(&mut self, cx: &mut Context<Self>) {
        self.service.update(cx, |service, cx| service.minimize_room(cx));
    }

    pub fn focus_stream(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        let user_id = user_id
            .map(trimming_whitespaces_and_newlines)
            .filter(|user_id| !user_id.is_empty())
            .map(str::to_owned);
        self.service.update(cx, |service, cx| service.focus_stream(user_id, cx));
    }

    pub fn dismiss_room_presentation(&mut self, cx: &mut Context<Self>) {
        self.service.update(cx, |service, cx| service.dismiss_room_presentation(cx));
    }

    pub fn room_eviction<'a>(&self, cx: &'a App) -> Option<&'a RiseRTCRoomEviction> {
        self.service.read(cx).room_eviction(cx)
    }

    pub fn acknowledge_room_eviction(&mut self, cx: &mut Context<Self>) {
        self.service.update(cx, |service, cx| service.acknowledge_room_eviction(cx));
    }

    pub fn join_room(
        &mut self,
        room_id: &str,
        display_name: Option<String>,
        preferred_role: Option<RiseRTCParticipantRole>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let room_id = match Self::normalized(room_id, rtc_key::ROOM_ID) {
            Ok(room_id) => room_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        if RtcGuideSandboxHooks::owns_voice_room(Some(&room_id), cx) {
            return Task::ready(RtcGuideSandboxHooks::join_voice_room(&room_id, cx).map_err(Into::into));
        }
        self.actions.update(cx, |actions, cx| {
            actions.join_room(room_id, display_name, preferred_role, cx)
        })
    }

    pub fn leave_room(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if self.sandbox_owns_active_room(cx) {
            RtcGuideSandboxHooks::leave_voice_room(cx);
            GuideSignalBus::emit(RiseGuideSignal::VoiceRoomLeft);
            return Task::ready(());
        }
        let leave = self.actions.update(cx, |actions, cx| actions.leave_room(cx));
        cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| {
            leave.await;
            GuideSignalBus::emit(RiseGuideSignal::VoiceRoomLeft);
        })
    }

    pub fn leave_room_expected_room_id(&mut self, expected_room_id: &str, cx: &mut Context<Self>) -> Task<()> {
        let Ok(room_id) = Self::normalized(expected_room_id, rtc_key::ROOM_ID) else {
            return Task::ready(());
        };
        if RtcGuideSandboxHooks::owns_voice_room(Some(&room_id), cx) {
            RtcGuideSandboxHooks::leave_voice_room(cx);
            GuideSignalBus::emit(RiseGuideSignal::VoiceRoomLeft);
            return Task::ready(());
        }
        let leave = self.actions.update(cx, |actions, cx| actions.leave_room_expected_room_id(room_id, cx));
        cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| {
            leave.await;
            GuideSignalBus::emit(RiseGuideSignal::VoiceRoomLeft);
        })
    }

    pub fn refresh_active_room(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        if self.sandbox_owns_active_room(cx) {
            RtcGuideSandboxHooks::refresh_voice_room(cx);
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| actions.refresh_active_room(cx))
    }

    pub fn load_more_participants(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        let has_more = self
            .service
            .read(cx)
            .active_room(cx)
            .is_some_and(|room| room.has_more_participants);
        if !has_more {
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| actions.load_more_participants(cx))
    }

    pub fn can_raise_hand(&self, cx: &App) -> bool {
        RiseRTCRaiseHandCooldown::allows_raise(
            self.service.read(cx).last_raise_hand_at(),
            RiseClock::now(),
            RiseRTCRaiseHandCooldown::window_seconds(),
        )
    }

    pub fn request_to_speak(&mut self, note: Option<&str>, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        if self.sandbox_owns_active_room(cx) || !self.can_raise_hand(cx) {
            return Task::ready(Ok(()));
        }
        self.service.update(cx, |service, cx| {
            service.begin_raise_hand_cooldown(RiseClock::now(), RiseRTCRaiseHandCooldown::window_seconds(), cx);
        });
        let note = note.map(|note| {
            RTCFieldRules::speak_note()
                .schema
                .as_ref()
                .map_or_else(|| note.to_owned(), |schema| schema.normalized_clamped(note))
        });
        self.actions.update(cx, |actions, cx| actions.request_to_speak(note, cx))
    }

    pub fn cancel_request_to_speak(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        if self.sandbox_owns_active_room(cx) {
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| actions.cancel_request_to_speak(cx))
    }

    pub fn take_stage(&mut self, user_id: &str, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        self.promote_to_speaker(user_id, cx)
    }

    pub fn promote_to_speaker(&mut self, user_id: &str, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        let user_id = match Self::normalized(user_id, rtc_key::TARGET_USER_ID) {
            Ok(user_id) => user_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        if self.sandbox_owns_active_room(cx) {
            RtcGuideSandboxHooks::promote_to_speaker(&user_id, cx);
            GuideSignalBus::emit(RiseGuideSignal::StageSpeakerPromoted);
            return Task::ready(Ok(()));
        }
        let promote = self.actions.update(cx, |actions, cx| actions.promote_to_speaker(user_id, cx));
        cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| {
            promote.await?;
            GuideSignalBus::emit(RiseGuideSignal::StageSpeakerPromoted);
            Ok(())
        })
    }

    pub fn move_to_audience(&mut self, user_id: &str, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        let user_id = match Self::normalized(user_id, rtc_key::TARGET_USER_ID) {
            Ok(user_id) => user_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        if self.sandbox_owns_active_room(cx) {
            RtcGuideSandboxHooks::move_to_audience(&user_id, cx);
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| actions.move_to_audience(user_id, cx))
    }

    pub fn moderate(
        &mut self,
        user_id: &str,
        action: RiseRTCModerationAction,
        reason: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let user_id = match Self::normalized(user_id, rtc_key::TARGET_USER_ID) {
            Ok(user_id) => user_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        let emits_mute = action == RiseRTCModerationAction::ForceMute;
        if self.sandbox_owns_active_room(cx) {
            RtcGuideSandboxHooks::moderate(&user_id, action, cx);
            if emits_mute {
                GuideSignalBus::emit(RiseGuideSignal::VoiceParticipantMuted);
            }
            return Task::ready(Ok(()));
        }
        let reason = reason.map(|reason| {
            RTCFieldRules::moderation_reason()
                .schema
                .as_ref()
                .map_or_else(|| reason.to_owned(), |schema| schema.normalized_clamped(reason))
        });
        let moderate = self.actions.update(cx, |actions, cx| actions.moderate(user_id, action, reason, cx));
        cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| {
            moderate.await?;
            if emits_mute {
                GuideSignalBus::emit(RiseGuideSignal::VoiceParticipantMuted);
            }
            Ok(())
        })
    }

    pub fn send_reaction(&mut self, emoji: &str, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        let emoji = trimming_whitespaces_and_newlines(emoji);
        if !Self::REACTION_CHARACTER_RANGE.contains(&character_count(emoji)) {
            return Task::ready(Err(RiseRTCStateError::Validation("invalid_reaction".to_owned()).into()));
        }
        if self.sandbox_owns_active_room(cx) {
            return Task::ready(Ok(()));
        }
        let emoji = emoji.to_owned();
        self.actions.update(cx, |actions, cx| actions.send_reaction(emoji, cx))
    }

    pub fn set_microphone(&mut self, enabled: bool, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        if self.sandbox_owns_active_call(cx) {
            RtcGuideSandboxHooks::set_practice_call_microphone(enabled, cx);
            return Task::ready(Ok(()));
        }
        if self.sandbox_owns_active_room(cx) {
            RtcGuideSandboxHooks::set_self_microphone(enabled, cx);
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| actions.set_microphone(enabled, cx))
    }

    pub fn set_deafened(&mut self, deafened: bool, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        if self.sandbox_owns_active_call(cx) {
            RtcGuideSandboxHooks::set_practice_call_deafened(deafened, cx);
            return Task::ready(Ok(()));
        }
        if self.sandbox_owns_active_room(cx) {
            RtcGuideSandboxHooks::set_self_deafened(deafened, cx);
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| actions.set_deafened(deafened, cx))
    }

    pub fn remember_microphone_before_deafen(&mut self, enabled: bool, cx: &App) {
        let Some(scope) = self.microphone_before_deafen_scope(cx) else {
            return;
        };
        self.microphone_before_deafen = Some((scope, enabled));
    }

    /// read once, taking the headphones off spends it and so does an explicit microphone tap
    pub fn take_microphone_before_deafen(&mut self, cx: &App) -> Option<bool> {
        let (scope, enabled) = self.microphone_before_deafen.take()?;
        (Some(scope) == self.microphone_before_deafen_scope(cx)).then_some(enabled)
    }

    pub fn forget_microphone_before_deafen(&mut self) {
        self.microphone_before_deafen = None;
    }

    fn microphone_before_deafen_scope(&self, cx: &App) -> Option<String> {
        let room_id = &self.service.read(cx).active_room(cx)?.config.room_id;
        Some(format!(
            "{}|{}|{room_id}",
            SaiAccountCacheScope::active_user_id().unwrap_or_default(),
            SaiAccountCacheScope::active_epoch().unwrap_or_default()
        ))
    }

    pub fn preview_listener_volume(&mut self, user_id: &str, percent: f64, cx: &mut Context<Self>) {
        let Some(user_id) = self.normalized_listener_target(user_id, cx) else {
            return;
        };
        let sequence = self.next_listener_volume_sequence();
        self.actions.update(cx, |actions, cx| {
            actions.preview_listener_volume_action(user_id, RiseRTCListenerVolume::clamped(percent), sequence, cx);
        });
    }

    pub fn commit_listener_volume(&mut self, user_id: &str, percent: f64, cx: &mut Context<Self>) {
        let Some(user_id) = self.normalized_listener_target(user_id, cx) else {
            return;
        };
        let sequence = self.next_listener_volume_sequence();
        self.actions.update(cx, |actions, cx| {
            actions.commit_listener_volume_action(user_id, RiseRTCListenerVolume::normalized(percent), sequence, cx);
        });
    }

    pub fn set_muted_for_me(&mut self, user_id: &str, muted: bool, cx: &mut Context<Self>) {
        let Some(user_id) = self.normalized_listener_target(user_id, cx) else {
            return;
        };
        let scope = format!(
            "{}|{}",
            SaiAccountCacheScope::active_user_id().unwrap_or_default(),
            SaiAccountCacheScope::active_epoch().unwrap_or_default()
        );
        if self.listener_volume_before_mute_scope.as_deref() != Some(scope.as_str()) {
            self.listener_volume_before_mute.clear();
            self.listener_volume_before_mute_scope = Some(scope);
        }
        if muted {
            let current = self.service.read(cx).listener_volume_percent(&user_id, cx);
            if !RiseRTCListenerVolume::is_muted(current) {
                if !self.listener_volume_before_mute.contains_key(&user_id)
                    && self.listener_volume_before_mute.len() >= Self::LISTENER_VOLUME_BEFORE_MUTE_LIMIT
                {
                    self.listener_volume_before_mute.clear();
                }
                self.listener_volume_before_mute.insert(user_id.clone(), current);
            }
            self.commit_listener_volume(&user_id, RiseRTCListenerVolume::MINIMUM_PERCENT, cx);
            return;
        }
        let restored = self.listener_volume_before_mute.remove(&user_id);
        self.commit_listener_volume(&user_id, restored.unwrap_or(RiseRTCListenerVolume::DEFAULT_PERCENT), cx);
    }

    pub fn reset_listener_volume(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let Some(user_id) = self.normalized_listener_target(user_id, cx) else {
            return;
        };
        let sequence = self.next_listener_volume_sequence();
        self.actions.update(cx, |actions, cx| actions.reset_listener_volume_action(user_id, sequence, cx));
    }

    fn next_listener_volume_sequence(&mut self) -> u64 {
        self.listener_volume_sequence = self.listener_volume_sequence.wrapping_add(1);
        if self.listener_volume_sequence == 0 {
            self.listener_volume_sequence = 1;
        }
        self.listener_volume_sequence
    }

    pub fn refresh_listener_volumes(&mut self, cx: &mut Context<Self>) {
        if self.sandbox_owns_active_room(cx) {
            return;
        }
        self.actions.update(cx, |actions, cx| actions.refresh_listener_volumes_action(cx));
    }

    fn normalized_listener_target(&self, user_id: &str, cx: &App) -> Option<String> {
        if self.sandbox_owns_active_room(cx) {
            return None;
        }
        let user_id = Self::normalized(user_id, rtc_key::TARGET_USER_ID).ok()?;
        if RtcGuideSandboxHooks::owns_user(&user_id, cx) {
            return None;
        }
        Some(user_id)
    }

    pub fn set_camera(
        &mut self,
        enabled: bool,
        position: RiseRTCCameraPosition,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        if self.sandbox_owns_active_call(cx) {
            return RtcGuideSandboxHooks::set_practice_call_camera(enabled, position, cx);
        }
        if self.sandbox_owns_active_room(cx) {
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| actions.set_camera(enabled, position, cx))
    }

    pub fn set_screen_share(
        &mut self,
        enabled: bool,
        include_device_audio: bool,
        quality: RiseRTCScreenShareQuality,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        if self.sandbox_owns_active_call(cx) {
            RtcGuideSandboxHooks::set_practice_screen_share(enabled, cx);
            return Task::ready(Ok(()));
        }
        if self.sandbox_owns_active_room(cx) {
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| {
            actions.set_screen_share(enabled, include_device_audio, quality, cx)
        })
    }

    fn viewer_is_premium(cx: &mut App) -> bool {
        AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .is_some_and(|profile| profile.more.is_premium)
    }

    pub fn is_high_stream_quality_available(&self, cx: &mut App) -> bool {
        Self::viewer_is_premium(cx)
    }

    pub fn select_stream_quality(&mut self, quality: RiseRTCScreenShareQuality, cx: &mut Context<Self>) -> bool {
        if quality != RiseRTCScreenShareQuality::High || self.is_high_stream_quality_available(cx) {
            return true;
        }
        PremiumUpsellStore::shared(cx).update(cx, |upsell, cx| {
            upsell.offer_premium_feature(tr("premium_upsell_reason_hd_stream"), "rtc_stream_quality", false, cx);
        });
        false
    }

    pub fn is_audio_bitrate_available(&self, kbps: i64, cx: &mut App) -> bool {
        kbps <= VoiceChannelConfig::audio_bitrate_ceiling_kbps(Self::viewer_is_premium(cx))
    }

    pub fn select_audio_bitrate(&mut self, kbps: i64, cx: &mut Context<Self>) -> bool {
        if self.is_audio_bitrate_available(kbps, cx) {
            return true;
        }
        PremiumUpsellStore::shared(cx).update(cx, |upsell, cx| {
            upsell.offer_premium_feature(tr("premium_upsell_reason_audio_bitrate"), "rtc_audio_bitrate", false, cx);
        });
        false
    }

    pub fn switch_camera(&mut self, position: RiseRTCCameraPosition, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        if self.sandbox_owns_active_call(cx) {
            return RtcGuideSandboxHooks::switch_practice_camera(position, cx);
        }
        if self.sandbox_owns_active_room(cx) {
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| actions.switch_camera(position, cx))
    }

    pub fn select_audio_input(&mut self, id: Option<String>, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        if self.sandbox_owns_active_room(cx) {
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| actions.select_audio_input(id, cx))
    }

    pub fn select_audio_output(&mut self, id: Option<String>, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        if self.sandbox_owns_active_room(cx) {
            return Task::ready(Ok(()));
        }
        self.actions.update(cx, |actions, cx| actions.select_audio_output(id, cx))
    }

    pub fn start_direct_call(
        &mut self,
        callee_id: &str,
        chat_id: &str,
        media_mode: RiseRTCCallMediaMode,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        let normalized = Self::normalized(callee_id, rtc_key::CALLEE_USER_ID)
            .and_then(|callee_id| Ok((callee_id, Self::normalized(chat_id, rtc_key::CHAT_ID)?)));
        let (callee_id, chat_id) = match normalized {
            Ok(ids) => ids,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| {
            actions.start_direct_call(callee_id, chat_id, media_mode, None, cx)
        })
    }

    pub fn request_system_outgoing_call(
        &mut self,
        callee_id: &str,
        chat_id: &str,
        display_name: String,
        avatar_url: Option<String>,
        has_video: bool,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Uuid>> {
        if let Some(practice) = RtcGuideSandboxHooks::start_practice_call(callee_id, cx) {
            return Task::ready(Ok(practice));
        }
        let normalized = Self::normalized(callee_id, rtc_key::CALLEE_USER_ID)
            .and_then(|callee_id| Ok((callee_id, Self::normalized(chat_id, rtc_key::CHAT_ID)?)));
        let (callee_id, chat_id) = match normalized {
            Ok(ids) => ids,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| {
            actions.request_system_outgoing_call_action(callee_id, chat_id, display_name, avatar_url, has_video, cx)
        })
    }

    pub fn request_system_call_end_call_uuid(&mut self, call_uuid: Uuid, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        self.actions.update(cx, |actions, cx| actions.request_system_call_end_action_call_uuid(call_uuid, cx))
    }

    pub fn request_system_call_end_server_call_id(
        &mut self,
        server_call_id: &str,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<bool>> {
        if RtcGuideSandboxHooks::owns_call(Some(server_call_id), cx) {
            return Task::ready(Ok(false));
        }
        let server_call_id = match Self::normalized(server_call_id, rtc_key::CALL_ID) {
            Ok(server_call_id) => server_call_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| {
            actions.request_system_call_end_action_server_call_id(server_call_id, cx)
        })
    }

    pub fn request_system_call_answer(&mut self, server_call_id: &str, cx: &mut Context<Self>) -> Task<anyhow::Result<bool>> {
        if RtcGuideSandboxHooks::owns_call(Some(server_call_id), cx) {
            return Task::ready(Ok(false));
        }
        let server_call_id = match Self::normalized(server_call_id, rtc_key::CALL_ID) {
            Ok(server_call_id) => server_call_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        let answer = self.actions.update(cx, |actions, cx| {
            actions.request_system_call_answer_action(server_call_id, cx)
        });
        cx.spawn(async move |_: WeakEntity<Self>, _: &mut AsyncApp| Ok(answer.await))
    }

    pub fn request_system_call_muted_call_uuid(
        &mut self,
        is_muted: bool,
        call_uuid: Uuid,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        self.actions.update(cx, |actions, cx| {
            actions.request_system_call_muted_action_call_uuid(is_muted, call_uuid, cx)
        })
    }

    pub fn request_system_call_muted_server_call_id(
        &mut self,
        is_muted: bool,
        server_call_id: &str,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<bool>> {
        if RtcGuideSandboxHooks::owns_call(Some(server_call_id), cx) {
            return Task::ready(Ok(false));
        }
        let server_call_id = match Self::normalized(server_call_id, rtc_key::CALL_ID) {
            Ok(server_call_id) => server_call_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| {
            actions.request_system_call_muted_action_server_call_id(is_muted, server_call_id, cx)
        })
    }

    pub fn answer_call(
        &mut self,
        call_id: &str,
        display_name: Option<String>,
        installation_id: &str,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCAnswerOutcome>> {
        let normalized = Self::normalized(call_id, rtc_key::CALL_ID).and_then(|call_id| {
            Ok((call_id, Self::normalized(installation_id, rtc_key::INSTALLATION_ID)?))
        });
        let (call_id, installation_id) = match normalized {
            Ok(ids) => ids,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| {
            actions.answer_call(call_id, display_name, installation_id, cx)
        })
    }

    pub fn rejoin_call(
        &mut self,
        call_id: &str,
        installation_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        if RtcGuideSandboxHooks::owns_call(Some(call_id), cx)
            && let Some(practice) = RtcGuideSandboxHooks::practice_call(call_id, cx)
        {
            return Task::ready(Ok(practice));
        }
        let call_id = match Self::normalized(call_id, rtc_key::CALL_ID) {
            Ok(call_id) => call_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| actions.rejoin_call(call_id, installation_id, cx))
    }

    pub fn decline_call(
        &mut self,
        call_id: &str,
        reason: Option<String>,
        idempotency_key: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        if RtcGuideSandboxHooks::owns_call(Some(call_id), cx)
            && let Some(practice) = RtcGuideSandboxHooks::end_practice_call(cx)
        {
            return Task::ready(Ok(practice));
        }
        let call_id = match Self::normalized(call_id, rtc_key::CALL_ID) {
            Ok(call_id) => call_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| {
            actions.decline_call(call_id, reason, idempotency_key, cx)
        })
    }

    pub fn cancel_call(
        &mut self,
        call_id: &str,
        reason: Option<String>,
        idempotency_key: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        if RtcGuideSandboxHooks::owns_call(Some(call_id), cx)
            && let Some(practice) = RtcGuideSandboxHooks::end_practice_call(cx)
        {
            return Task::ready(Ok(practice));
        }
        let call_id = match Self::normalized(call_id, rtc_key::CALL_ID) {
            Ok(call_id) => call_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| {
            actions.cancel_call(call_id, reason, idempotency_key, cx)
        })
    }

    pub fn end_call(
        &mut self,
        call_id: &str,
        reason: Option<String>,
        idempotency_key: Option<String>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        if RtcGuideSandboxHooks::owns_call(Some(call_id), cx)
            && let Some(practice) = RtcGuideSandboxHooks::end_practice_call(cx)
        {
            return Task::ready(Ok(practice));
        }
        let call_id = match Self::normalized(call_id, rtc_key::CALL_ID) {
            Ok(call_id) => call_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| {
            actions.end_call(call_id, reason, idempotency_key, cx)
        })
    }

    pub fn deliver_hangup_when_online(
        &mut self,
        action: RiseRTCHangupAction,
        call_id: &str,
        reason: String,
        idempotency_key: String,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let call_id = match Self::normalized(call_id, rtc_key::CALL_ID) {
            Ok(call_id) => call_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| {
            actions.deliver_hangup_when_online(action, call_id, reason, idempotency_key, cx)
        })
    }

    pub fn set_call_recording(
        &mut self,
        call_id: &str,
        on: bool,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<RiseRTCDirectCall>> {
        if RtcGuideSandboxHooks::owns_call(Some(call_id), cx)
            && let Some(practice) = RtcGuideSandboxHooks::practice_call(call_id, cx)
        {
            return Task::ready(Ok(practice));
        }
        let call_id = match Self::normalized(call_id, rtc_key::CALL_ID) {
            Ok(call_id) => call_id,
            Err(error) => return Task::ready(Err(error.into())),
        };
        self.actions.update(cx, |actions, cx| actions.set_call_recording(call_id, on, cx))
    }

    fn normalized(value: &str, field: &str) -> Result<String, RiseRTCStateError> {
        let value = trimming_whitespaces_and_newlines(value);
        if value.is_empty() {
            return Err(RiseRTCStateError::Validation(field.to_owned()));
        }
        Ok(value.to_owned())
    }
}
