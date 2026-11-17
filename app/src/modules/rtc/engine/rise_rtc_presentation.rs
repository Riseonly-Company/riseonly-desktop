use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_canon::{RiseGuideSignal, RiseTimingCanon};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::modules::rtc::engine::core::rise_rtc_store_layout::{
    RiseRTCCallRepositorySnapshot, RiseRTCRoomRepositorySnapshot, RiseRTCStoredCall,
};
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCallState, RiseRTCConnectionState, RiseRTCDirectCall, RiseRTCIncomingCall, RiseRTCParticipant,
    RiseRTCParticipantRole, RiseRTCRaisedHand, RiseRTCReaction, RiseRTCRoomConfig, RiseRTCRoomEviction,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCRoomPresentation {
    pub config: RiseRTCRoomConfig,
    pub self_participant: Option<RiseRTCParticipant>,
    pub participants: Vec<RiseRTCParticipant>,
    pub raised_hands: Vec<RiseRTCRaisedHand>,
    pub participant_count: i64,
    pub speaker_count: i64,
    pub has_more_participants: bool,
    pub next_participant_cursor: Option<String>,
    pub generated_at_ms: i64,
    pub profiles_complete: bool,
    pub profile_error: Option<String>,
    pub is_refreshing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCPresentationSnapshot {
    pub account_id: Option<String>,
    pub generation: u64,
    pub connection_state: RiseRTCConnectionState,
    pub active_room: Option<RiseRTCRoomPresentation>,
    pub recent_calls: Vec<RiseRTCDirectCall>,
    pub incoming_call: Option<RiseRTCIncomingCall>,
    pub reactions: Vec<RiseRTCReaction>,
    pub last_error_code: Option<String>,
}

impl RiseRTCPresentationSnapshot {
    pub const EMPTY: RiseRTCPresentationSnapshot = RiseRTCPresentationSnapshot {
        account_id: None,
        generation: 0,
        connection_state: RiseRTCConnectionState::Idle,
        active_room: None,
        recent_calls: Vec::new(),
        incoming_call: None,
        reactions: Vec::new(),
        last_error_code: None,
    };
}

const RAISED_HAND_TTL: &str = "rtc_raised_hand_ttl";

static HAND_WINDOW_MS: LazyLock<i64> =
    LazyLock::new(|| i64::try_from(RiseTimingCanon::server_window(RAISED_HAND_TTL).window_ms).unwrap_or(i64::MAX));

/// main thread projection, tokens
/// connection ids and session identities are not in this type at all
pub struct RiseRTCPresentation {
    snapshot: RiseRTCPresentationSnapshot,

    current_active_room: Option<RiseRTCRoomPresentation>,
    current_recent_calls: Vec<RiseRTCDirectCall>,
    current_incoming_call: Option<RiseRTCIncomingCall>,
    current_reactions: Vec<RiseRTCReaction>,
    current_connection_state: RiseRTCConnectionState,
    current_last_error_code: Option<String>,
    current_account_id: Option<String>,
    verification_emojis_by_call: HashMap<String, Vec<String>>,
    current_room_eviction: Option<RiseRTCRoomEviction>,

    account_id: Option<String>,
    generation: u64,
    connection_state: RiseRTCConnectionState,
    active_room_snapshot: Option<RiseRTCRoomRepositorySnapshot>,
    active_room_id: Option<String>,
    calls: Vec<RiseRTCStoredCall>,
    reactions: VecDeque<RiseRTCReaction>,
    reaction_ids: HashSet<String>,
    is_refreshing: bool,
    last_error_code: Option<String>,

    hand_expiry_task: Option<Task<()>>,
    hand_expiry_deadline_ms: Option<i64>,
}

impl RiseRTCPresentation {
    const REACTION_LIMIT: usize = 128;
    const CALL_LIMIT: usize = 128;
    const PARTICIPANT_LIMIT: usize = 512;
    const RAISED_HAND_LIMIT: usize = 256;
    const VERIFICATION_CALL_LIMIT: usize = 32;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseRTCPresentationSnapshot::EMPTY,
            current_active_room: None,
            current_recent_calls: Vec::new(),
            current_incoming_call: None,
            current_reactions: Vec::new(),
            current_connection_state: RiseRTCConnectionState::Idle,
            current_last_error_code: None,
            current_account_id: None,
            verification_emojis_by_call: HashMap::new(),
            current_room_eviction: None,
            account_id: None,
            generation: 0,
            connection_state: RiseRTCConnectionState::Idle,
            active_room_snapshot: None,
            active_room_id: None,
            calls: Vec::new(),
            reactions: VecDeque::new(),
            reaction_ids: HashSet::new(),
            is_refreshing: false,
            last_error_code: None,
            hand_expiry_task: None,
            hand_expiry_deadline_ms: None,
        }
    }

    pub fn snapshot(&self) -> &RiseRTCPresentationSnapshot {
        &self.snapshot
    }

    pub fn current_active_room(&self) -> Option<&RiseRTCRoomPresentation> {
        self.current_active_room.as_ref()
    }

    pub fn current_recent_calls(&self) -> &[RiseRTCDirectCall] {
        &self.current_recent_calls
    }

    pub fn current_incoming_call(&self) -> Option<&RiseRTCIncomingCall> {
        self.current_incoming_call.as_ref()
    }

    pub fn current_reactions(&self) -> &[RiseRTCReaction] {
        &self.current_reactions
    }

    pub fn current_connection_state(&self) -> RiseRTCConnectionState {
        self.current_connection_state
    }

    pub fn current_last_error_code(&self) -> Option<&str> {
        self.current_last_error_code.as_deref()
    }

    pub fn current_account_id(&self) -> Option<&str> {
        self.current_account_id.as_deref()
    }

    pub fn verification_emojis_by_call(&self) -> &HashMap<String, Vec<String>> {
        &self.verification_emojis_by_call
    }

    pub fn current_room_eviction(&self) -> Option<&RiseRTCRoomEviction> {
        self.current_room_eviction.as_ref()
    }

    pub fn is_active(&self) -> bool {
        self.account_id.is_some()
    }

    pub fn activate(&mut self, account_id: &str, cx: &mut Context<Self>) {
        let account_id = trimming_whitespaces_and_newlines(account_id);
        if account_id.is_empty() {
            self.deactivate(cx);
            return;
        }
        if self.account_id.as_deref() == Some(account_id) {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.account_id = Some(account_id.to_owned());
        self.connection_state = RiseRTCConnectionState::Idle;
        self.active_room_snapshot = None;
        self.active_room_id = None;
        self.verification_emojis_by_call = HashMap::new();
        self.current_room_eviction = None;
        self.calls = Vec::new();
        self.reactions = VecDeque::new();
        self.reaction_ids = HashSet::new();
        self.last_error_code = None;
        self.rebuild(cx);
        cx.notify();
    }

    pub fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.account_id = None;
        self.connection_state = RiseRTCConnectionState::Idle;
        self.active_room_snapshot = None;
        self.active_room_id = None;
        self.verification_emojis_by_call = HashMap::new();
        self.current_room_eviction = None;
        self.calls = Vec::new();
        self.reactions = VecDeque::new();
        self.reaction_ids = HashSet::new();
        self.is_refreshing = false;
        self.last_error_code = None;
        self.rebuild(cx);
        cx.notify();
    }

    pub fn select_room(&mut self, room_id: Option<String>, cx: &mut Context<Self>) {
        let keeps_snapshot = self
            .active_room_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.room.as_ref().map(|room| &room.room_id) == room_id.as_ref());
        if !keeps_snapshot {
            self.active_room_snapshot = None;
        }
        self.active_room_id = room_id;
        self.reactions.clear();
        self.reaction_ids.clear();
        self.rebuild(cx);
    }

    pub fn publish_room(&mut self, value: RiseRTCRoomRepositorySnapshot, cx: &mut Context<Self>) {
        if Some(&value.account_id) != self.account_id.as_ref()
            || value.room.as_ref().map(|room| &room.room_id) != self.active_room_id.as_ref()
        {
            return;
        }
        self.active_room_snapshot = Some(value);
        self.rebuild(cx);
    }

    pub fn publish_calls(&mut self, value: RiseRTCCallRepositorySnapshot, cx: &mut Context<Self>) {
        if Some(&value.account_id) != self.account_id.as_ref() {
            return;
        }
        self.calls = value.calls.into_iter().take(Self::CALL_LIMIT).collect();
        self.rebuild(cx);
    }

    pub fn publish_verification_emojis(&mut self, verification_emojis: Vec<String>, call_id: &str, cx: &mut Context<Self>) {
        if self.verification_emojis_by_call.get(call_id) == Some(&verification_emojis) {
            return;
        }
        if self.verification_emojis_by_call.len() > Self::VERIFICATION_CALL_LIMIT {
            self.verification_emojis_by_call.clear();
        }
        self.verification_emojis_by_call.insert(call_id.to_owned(), verification_emojis);
        cx.notify();
    }

    pub fn publish_eviction(&mut self, eviction: RiseRTCRoomEviction, cx: &mut Context<Self>) {
        if !self.is_active() {
            return;
        }
        self.current_room_eviction = Some(eviction);
        cx.notify();
    }

    pub fn acknowledge_eviction(&mut self, cx: &mut Context<Self>) {
        if self.current_room_eviction.is_none() {
            return;
        }
        self.current_room_eviction = None;
        cx.notify();
    }

    pub fn publish_connection_state(&mut self, connection_state: RiseRTCConnectionState, cx: &mut Context<Self>) {
        self.connection_state = connection_state;
        if connection_state != RiseRTCConnectionState::Failed {
            self.last_error_code = None;
        }
        self.rebuild(cx);
    }

    pub fn publish_reaction(&mut self, reaction: RiseRTCReaction, cx: &mut Context<Self>) {
        if Some(&reaction.room_id) != self.active_room_id.as_ref() || !self.reaction_ids.insert(reaction.event_id.clone()) {
            return;
        }
        self.reactions.push_back(reaction);
        while self.reactions.len() > Self::REACTION_LIMIT {
            if let Some(dropped) = self.reactions.pop_front() {
                self.reaction_ids.remove(&dropped.event_id);
            }
        }
        self.rebuild(cx);
    }

    pub fn publish_refreshing(&mut self, refreshing: bool, cx: &mut Context<Self>) {
        self.is_refreshing = refreshing;
        self.rebuild(cx);
    }

    pub fn publish_error_code(&mut self, error_code: Option<String>, cx: &mut Context<Self>) {
        if error_code.is_some() {
            self.connection_state = RiseRTCConnectionState::Failed;
        }
        self.last_error_code = error_code;
        self.rebuild(cx);
    }

    pub fn publish_nonfatal_error_code(&mut self, nonfatal_error_code: String, cx: &mut Context<Self>) {
        self.last_error_code = Some(nonfatal_error_code);
        self.rebuild(cx);
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let room_presentation = self.room_presentation(cx);

        GuideFacts::set(
            RiseGuideSignal::StageHandRaised,
            room_presentation.as_ref().is_some_and(|room| !room.raised_hands.is_empty()),
            cx,
        );

        let recent_calls: Vec<RiseRTCDirectCall> = self.calls.iter().map(|stored| stored.call.clone()).collect();
        let incoming = self
            .calls
            .iter()
            .find(|stored| {
                stored.call.state == RiseRTCCallState::Ringing
                    && Some(&stored.call.callee_user_id) == self.account_id.as_ref()
            })
            .map(|stored| RiseRTCIncomingCall {
                call: stored.call.clone(),
                caller: stored.caller.clone(),
                event_id: stored.event_id.clone(),
                occurred_at_ms: stored.occurred_at_ms,
            });
        let reactions: Vec<RiseRTCReaction> = self.reactions.iter().cloned().collect();

        let mut changed = false;
        changed |= assign_if_changed(&mut self.current_active_room, &room_presentation);
        changed |= assign_if_changed(&mut self.current_recent_calls, &recent_calls);
        changed |= assign_if_changed(&mut self.current_incoming_call, &incoming);
        changed |= assign_if_changed(&mut self.current_reactions, &reactions);
        changed |= assign_if_changed(&mut self.current_connection_state, &self.connection_state);
        changed |= assign_if_changed(&mut self.current_last_error_code, &self.last_error_code);
        changed |= assign_if_changed(&mut self.current_account_id, &self.account_id);

        let next = RiseRTCPresentationSnapshot {
            account_id: self.account_id.clone(),
            generation: self.generation,
            connection_state: self.connection_state,
            active_room: room_presentation,
            recent_calls,
            incoming_call: incoming,
            reactions,
            last_error_code: self.last_error_code.clone(),
        };
        if next != self.snapshot {
            self.snapshot = next;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    fn room_presentation(&mut self, cx: &mut Context<Self>) -> Option<RiseRTCRoomPresentation> {
        let room_snapshot = self.active_room_snapshot.as_ref();
        let room = room_snapshot.and_then(|snapshot| snapshot.room.as_ref());
        let (Some(room_snapshot), Some(room)) = (room_snapshot, room) else {
            self.schedule_hand_expiry(None, 0, cx);
            return None;
        };
        if Some(&room.room_id) != self.active_room_id.as_ref() {
            self.schedule_hand_expiry(None, 0, cx);
            return None;
        }
        let participants: Vec<RiseRTCParticipant> =
            room_snapshot.participants.iter().take(Self::PARTICIPANT_LIMIT).cloned().collect();
        let now_ms = Self::now_ms();
        let hands: Vec<RiseRTCRaisedHand> = room_snapshot
            .raised_hands
            .iter()
            .take(Self::RAISED_HAND_LIMIT)
            .filter(|hand| !hand.has_expired(now_ms))
            .cloned()
            .collect();
        let meta = room_snapshot.meta.as_ref();
        let presentation = RiseRTCRoomPresentation {
            config: room.clone(),
            self_participant: participants
                .iter()
                .find(|participant| Some(&participant.user_id) == self.account_id.as_ref())
                .cloned(),
            participant_count: meta.map_or(participants.len() as i64, |meta| meta.participant_count),
            speaker_count: meta.map_or_else(
                || {
                    participants
                        .iter()
                        .filter(|participant| {
                            participant.role == RiseRTCParticipantRole::Speaker
                                || participant.role == RiseRTCParticipantRole::Moderator
                        })
                        .count() as i64
                },
                |meta| meta.speaker_count,
            ),
            has_more_participants: meta.is_some_and(|meta| meta.has_more_participants),
            next_participant_cursor: meta.and_then(|meta| meta.next_participant_cursor.clone()),
            generated_at_ms: meta.map_or(room.updated_at_ms, |meta| meta.generated_at_ms),
            profiles_complete: meta.is_some_and(|meta| meta.profiles_complete),
            profile_error: meta.and_then(|meta| meta.profile_error.clone()),
            is_refreshing: self.is_refreshing,
            participants,
            raised_hands: hands,
        };
        let next_deadline_ms = presentation.raised_hands.iter().filter_map(|hand| hand.expires_at_ms).min();
        self.schedule_hand_expiry(next_deadline_ms, now_ms, cx);
        Some(presentation)
    }

    /// republishes the room when the nearest queued hand falls due
    /// so the badge goes away on time with no traffic
    /// server drops the hand lazily and both sides agree since both read expires_at
    fn schedule_hand_expiry(&mut self, next_deadline_ms: Option<i64>, now_ms: i64, cx: &mut Context<Self>) {
        if self.hand_expiry_deadline_ms == next_deadline_ms {
            return;
        }
        self.hand_expiry_deadline_ms = next_deadline_ms;
        self.hand_expiry_task = None;
        let Some(next_deadline_ms) = next_deadline_ms else {
            return;
        };
        let remaining = next_deadline_ms.saturating_sub(now_ms).max(0).min(*HAND_WINDOW_MS);
        let expected_generation = self.generation;
        let wait = Duration::from_millis(u64::try_from(remaining).unwrap_or(0));
        self.hand_expiry_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != expected_generation {
                    return;
                }
                this.hand_expiry_deadline_ms = None;
                this.rebuild(cx);
            });
        }));
    }

    fn now_ms() -> i64 {
        RiseClock::now().ms()
    }
}

fn assign_if_changed<T: Clone + PartialEq>(slot: &mut T, value: &T) -> bool {
    if slot == value {
        return false;
    }
    *slot = value.clone();
    true
}
