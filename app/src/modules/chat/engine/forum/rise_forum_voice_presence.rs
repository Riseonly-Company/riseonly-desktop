use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Weak};
use std::time::Duration;

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity, Task};
use parking_lot::Mutex;
use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use tokio::task::JoinHandle;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::chat_engine_hooks::chat_engine_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatTopic, VoiceChannelParticipant, VoiceChannelState,
};
use crate::modules::rtc::engine::rise_rtc_engine_models::{RiseRTCParticipant, RiseRTCParticipantRole};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseForumVoicePresenceSnapshot {
    pub account_id: Option<String>,
    pub generation: u64,
    pub channels: HashMap<String, HashMap<String, VoiceChannelState>>,
}

impl RiseForumVoicePresenceSnapshot {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn nearest_hand_deadline_ms(&self, after_now_ms: i64) -> Option<i64> {
        self.channels
            .values()
            .flat_map(|topics| topics.values())
            .flat_map(|state| state.participants.iter())
            .filter(|participant| participant.hand_raised)
            .filter_map(|participant| participant.hand_expires_at_ms)
            .filter(|deadline| *deadline > after_now_ms)
            .min()
    }

    /// one clock for every surface that ages a hand badge so room and topic list agree
    pub fn now_ms_for_display() -> i64 {
        RiseClock::now().ms()
    }

    pub fn state(&self, chat_id: &str, topic_id: &str) -> Option<VoiceChannelState> {
        self.channels.get(chat_id)?.get(topic_id).cloned()
    }
}

const MAX_CHATS: usize = 8;
const MAX_CHANNELS_PER_CHAT: usize = 128;

static MAX_PARTICIPANTS_PER_CHANNEL: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("voice_preview_participants")).unwrap_or(0));

static EXPANDED_CEILING: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("max_page_size_rtc"));

const ROSTER_REPAIR_DELAY: Duration = Duration::from_millis(300);
const ROSTER_REPAIR_WINDOW: Duration = Duration::from_secs(5);

static SELF_CLAIM_WINDOW: LazyLock<Duration> = LazyLock::new(|| {
    Duration::from_millis(u64::try_from(RiseTimingCanon::heartbeat("rtc_participant").effective_window_ms()).unwrap_or(0))
});

fn roster_count(participants: &[VoiceChannelParticipant]) -> i64 {
    i64::try_from(participants.len()).unwrap_or(i64::MAX)
}

fn channel_key(chat_id: &str, topic_id: &str) -> String {
    format!("{chat_id}\u{1F}{topic_id}")
}

fn vacant_state(
    stored: Option<&VoiceChannelState>,
    answered_at_ms: i64,
    is_optimistically_self_entered: bool,
) -> Option<VoiceChannelState> {
    if answered_at_ms <= 0 || is_optimistically_self_entered {
        return None;
    }
    if stored.is_some_and(|stored| stored.participant_count <= 0 && stored.participants.is_empty()) {
        return None;
    }
    Some(VoiceChannelState {
        room_id: stored.map(|stored| stored.room_id.clone()).unwrap_or_default(),
        participant_count: 0,
        user_limit: stored.map_or(0, |stored| stored.user_limit),
        participants: Vec::new(),
        has_more: false,
        generated_at_ms: answered_at_ms,
        session_started_at_ms: 0,
        visible_limit: stored.map_or(VoiceChannelState::COLLAPSED_ROWS, |stored| stored.visible_limit),
    })
}

fn seating(participant: VoiceChannelParticipant, state: &VoiceChannelState, cap: usize) -> VoiceChannelState {
    let mut participants = state.participants.clone();
    participants.push(participant);
    participants.sort_by_key(|row| row.order_sequence);
    participants.truncate(cap);
    let held = roster_count(&participants);
    let count = (state.participant_count + 1).max(held);
    let mut seated = state.clone();
    seated.participants = participants;
    seated.participant_count = count;
    seated.has_more = count > held;
    seated
}

fn keeping_expanded_tail(stored: &VoiceChannelState, listing: &VoiceChannelState, limit: i64) -> VoiceChannelState {
    let listed: HashSet<&str> = listing.participants.iter().map(|row| row.user_id.as_str()).collect();
    let last_order = listing.participants.last().map_or(i64::MIN, |row| row.order_sequence);
    let tail: Vec<VoiceChannelParticipant> = stored
        .participants
        .iter()
        .filter(|row| !listed.contains(row.user_id.as_str()) && row.order_sequence > last_order)
        .cloned()
        .collect();
    if tail.is_empty() {
        return listing.clone();
    }
    let mut merged = listing.clone();
    merged.participants.extend(tail);
    merged.participants.truncate(usize::try_from(limit.min(listing.participant_count.max(0))).unwrap_or(0));
    merged.has_more = merged.participant_count > roster_count(&merged.participants);
    merged
}

fn participant_from(row: &RiseRTCParticipant, known: Option<&VoiceChannelParticipant>) -> VoiceChannelParticipant {
    let profile = row.profile.as_ref();
    let avatar = profile.map(|profile| profile.avatar_url.clone()).unwrap_or_default();
    VoiceChannelParticipant {
        user_id: row.user_id.clone(),
        name: profile
            .map(|profile| profile.name.clone())
            .or_else(|| known.map(|known| known.name.clone()))
            .unwrap_or_default(),
        tag: profile
            .map(|profile| profile.tag.clone())
            .or_else(|| known.map(|known| known.tag.clone()))
            .unwrap_or_default(),
        avatar_url: if avatar.is_empty() { known.map(|known| known.avatar_url.clone()).unwrap_or_default() } else { avatar },
        is_premium: profile.map(|profile| profile.is_premium).or_else(|| known.map(|known| known.is_premium)).unwrap_or(false),
        is_official: profile
            .map(|profile| profile.is_official)
            .or_else(|| known.map(|known| known.is_official))
            .unwrap_or(false),
        role_name: known.map(|known| known.role_name.clone()).unwrap_or_default(),
        role_color: known.map_or(0, |known| known.role_color),
        microphone_enabled: row.microphone_enabled,
        camera_enabled: row.camera_enabled,
        screen_share_enabled: row.screen_share_enabled,
        deafened: row.deafened,
        server_muted: row.server_muted,
        server_deafened: row.server_deafened,
        connected: row.connected,
        joined_at_ms: row.joined_at_ms,
        order_sequence: row.order_sequence,
        role: row.role,
        hand_raised: row.hand_raised,
        hand_expires_at_ms: if row.hand_raised { known.and_then(|known| known.hand_expires_at_ms) } else { None },
    }
}

/// request to expand the roster of a forum voice channel
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseForumVoicePresenceRepositoryExpandedRosterRequest {
    pub account_id: String,
    pub generation: u64,
    pub room_id: String,
    pub limit: i64,
    pub after_order: i64,
    pub cursor: Option<String>,
    pub page_limit: i64,
}

/// a refill a delta made due, started once the state lock is released
enum RiseForumVoiceRepair {
    Roster(String),
    Expanded { chat_id: String, topic_id: String },
}

#[derive(Default)]
struct RiseForumVoicePresenceState {
    account_id: Option<String>,
    generation: u64,
    channels: HashMap<String, HashMap<String, VoiceChannelState>>,
    chat_order: Vec<String>,
    self_claims: HashMap<String, Duration>,
    roster_repairs: HashMap<String, JoinHandle<()>>,
    roster_repairs_pending: HashSet<String>,
    expanded_limits: HashMap<String, i64>,
    expanded_repairs: HashMap<String, JoinHandle<()>>,
    repairs_due: Vec<RiseForumVoiceRepair>,
}

impl RiseForumVoicePresenceState {
    fn snapshot(&self) -> RiseForumVoicePresenceSnapshot {
        let mut published = self.channels.clone();
        for (key, limit) in &self.expanded_limits {
            let mut parts = key.splitn(2, '\u{1F}');
            let (Some(chat_id), Some(topic_id)) = (parts.next(), parts.next()) else {
                continue;
            };
            if let Some(state) = published.get_mut(chat_id).and_then(|topics| topics.get_mut(topic_id)) {
                state.visible_limit = *limit;
            }
        }
        RiseForumVoicePresenceSnapshot { account_id: self.account_id.clone(), generation: self.generation, channels: published }
    }

    fn clear(&mut self) {
        for (_, repair) in self.roster_repairs.drain() {
            repair.abort();
        }
        self.roster_repairs_pending = HashSet::new();
        self.clear_expansions();
        self.repairs_due = Vec::new();
        self.channels = HashMap::new();
        self.chat_order = Vec::new();
        self.self_claims = HashMap::new();
    }

    fn clear_expansions(&mut self) {
        for (_, repair) in self.expanded_repairs.drain() {
            repair.abort();
        }
        self.expanded_limits = HashMap::new();
    }

    /// rows the overlay holds for a channel, server preview or more once expanded
    fn roster_cap(&self, key: &str) -> usize {
        let expanded = self.expanded_limits.get(key).map_or(0, |limit| usize::try_from(*limit).unwrap_or(0));
        (*MAX_PARTICIPANTS_PER_CHANNEL).max(expanded)
    }

    fn activate(&mut self, account_id: &str) -> Option<RiseForumVoicePresenceSnapshot> {
        if self.account_id.as_deref() == Some(account_id) {
            return None;
        }
        self.account_id = Some(account_id.to_owned());
        self.generation = self.generation.wrapping_add(1);
        self.clear();
        Some(self.snapshot())
    }

    fn deactivate(&mut self) -> RiseForumVoicePresenceSnapshot {
        self.account_id = None;
        self.generation = self.generation.wrapping_add(1);
        self.clear();
        self.snapshot()
    }

    fn has_live_self_claim(&mut self, key: &str) -> bool {
        let Some(claimed_at) = self.self_claims.get(key) else {
            return false;
        };
        if RiseClock::monotonic().saturating_sub(*claimed_at) <= *SELF_CLAIM_WINDOW {
            return true;
        }
        self.self_claims.remove(key);
        false
    }

    fn adopt(&mut self, chat_id: &str, topics: &[ChatTopic]) -> Option<RiseForumVoicePresenceSnapshot> {
        let account_id = self.account_id.clone()?;
        if chat_id.is_empty() {
            return None;
        }
        let mut bucket = self.channels.get(chat_id).cloned().unwrap_or_default();
        let mut changed = false;
        let answered_at_ms = topics
            .iter()
            .filter(|topic| topic.is_rtc_channel())
            .filter_map(|topic| topic.voice_state.as_ref())
            .fold(0_i64, |stamp, state| stamp.max(state.generated_at_ms));
        for topic in topics.iter().filter(|topic| topic.is_rtc_channel()) {
            let stored = bucket.get(&topic.id);
            let key = channel_key(chat_id, &topic.id);
            let self_claimed = self.has_live_self_claim(&key);
            let Some(fetched) =
                topic.voice_state.clone().or_else(|| vacant_state(stored, answered_at_ms, self_claimed))
            else {
                continue;
            };
            if stored.is_some_and(|stored| stored.generated_at_ms >= fetched.generated_at_ms) {
                continue;
            }
            let mut adopted = fetched;
            if let Some(limit) = self.expanded_limits.get(&key).copied()
                && let Some(stored) = stored
            {
                adopted = keeping_expanded_tail(stored, &adopted, limit);
            }
            if self_claimed
                && !adopted.participants.iter().any(|row| row.user_id == account_id)
                && let Some(claimed) =
                    stored.and_then(|stored| stored.participants.iter().find(|row| row.user_id == account_id))
            {
                adopted = seating(claimed.clone(), &adopted, self.roster_cap(&key));
            }
            adopted.session_started_at_ms = VoiceChannelState::resolve_session_start(
                stored,
                &adopted.room_id,
                &adopted.participants,
                adopted.participant_count,
            );
            bucket.insert(topic.id.clone(), adopted);
            changed = true;
        }
        if !changed {
            return None;
        }
        self.channels.insert(chat_id.to_owned(), bucket);
        self.touch(chat_id);
        self.evict_if_needed();
        Some(self.snapshot())
    }

    fn projected_count(&self, chat_id: &str, topic_id: &str, user_id: &str, joined: bool) -> i64 {
        let state = self.channels.get(chat_id).and_then(|topics| topics.get(topic_id));
        let current = state.map_or(0, |state| state.participant_count);
        let already_listed = state.is_some_and(|state| state.participants.iter().any(|row| row.user_id == user_id));
        if joined {
            return if already_listed { current } else { current + 1 };
        }
        if already_listed { (current - 1).max(0) } else { current }
    }

    fn apply_optimistic_self(
        &mut self,
        chat_id: &str,
        topic_id: &str,
        joined: bool,
        participant: Option<VoiceChannelParticipant>,
    ) -> Option<RiseForumVoicePresenceSnapshot> {
        let account_id = self.account_id.clone()?;
        if chat_id.is_empty() || topic_id.is_empty() {
            return None;
        }
        if participant.as_ref().is_some_and(|participant| participant.user_id != account_id) {
            return None;
        }

        let key = channel_key(chat_id, topic_id);
        if joined {
            self.self_claims.insert(key, RiseClock::monotonic());
        } else {
            self.self_claims.remove(&key);
        }

        let event = RiseForumVoiceUpdate {
            chat_id: chat_id.to_owned(),
            topic_id: topic_id.to_owned(),
            scope: RiseForumVoiceUpdateScope::Channel,
            room_id: String::new(),
            kind: if joined { RiseForumVoiceUpdateKind::Joined } else { RiseForumVoiceUpdateKind::Left },
            participant_count: self.projected_count(chat_id, topic_id, &account_id, joined),
            user_id: account_id,
            participant: if joined { participant } else { None },
            occurred_at_ms: self
                .channels
                .get(chat_id)
                .and_then(|topics| topics.get(topic_id))
                .map_or(0, |state| state.generated_at_ms),
        };
        self.write_delta(&event)
    }

    fn apply(&mut self, event: &RiseForumVoiceUpdate) -> Option<RiseForumVoicePresenceSnapshot> {
        let account_id = self.account_id.clone()?;
        if event.chat_id.is_empty() || event.topic_id.is_empty() {
            return None;
        }

        if event.user_id == account_id && event.kind != RiseForumVoiceUpdateKind::Media {
            self.self_claims.remove(&channel_key(&event.chat_id, &event.topic_id));
        }

        self.write_delta(event)
    }

    fn write_delta(&mut self, event: &RiseForumVoiceUpdate) -> Option<RiseForumVoicePresenceSnapshot> {
        let bucket = self.channels.get(&event.chat_id);
        let stored = bucket.and_then(|topics| topics.get(&event.topic_id));
        if stored.is_none() && bucket.is_some_and(|topics| topics.len() >= MAX_CHANNELS_PER_CHAT) {
            return None;
        }
        let state = stored.cloned().unwrap_or_else(|| VoiceChannelState {
            room_id: event.room_id.clone(),
            participant_count: 0,
            user_limit: 0,
            participants: Vec::new(),
            has_more: false,
            generated_at_ms: 0,
            session_started_at_ms: 0,
            visible_limit: VoiceChannelState::COLLAPSED_ROWS,
        });

        let is_current = event.occurred_at_ms >= state.generated_at_ms;
        if event.kind != RiseForumVoiceUpdateKind::Left && !is_current {
            return None;
        }

        let mut participants = state.participants.clone();
        let mut resolved_count = event.participant_count;
        match event.kind {
            RiseForumVoiceUpdateKind::Joined => {
                let mut participant = event.participant.clone()?;
                if participant.avatar_url.is_empty()
                    && let Some(known) = bucket.and_then(|topics| {
                        topics
                            .values()
                            .flat_map(|state| state.participants.iter())
                            .find(|row| row.user_id == participant.user_id && !row.avatar_url.is_empty())
                    })
                {
                    participant.avatar_url = known.avatar_url.clone();
                }
                if let Some(index) = participants.iter().position(|row| row.user_id == participant.user_id) {
                    participants[index] = participant;
                } else if roster_count(&participants) >= state.participant_count {
                    participants.push(participant);
                    participants.sort_by_key(|row| row.order_sequence);
                }
            }
            RiseForumVoiceUpdateKind::Media => {
                let patch = event.participant.as_ref()?;
                let updated = participants.iter_mut().find(|row| row.user_id == patch.user_id)?;
                updated.microphone_enabled = patch.microphone_enabled;
                updated.camera_enabled = patch.camera_enabled;
                updated.screen_share_enabled = patch.screen_share_enabled;
                updated.deafened = patch.deafened;
                updated.server_muted = patch.server_muted;
                updated.server_deafened = patch.server_deafened;
                updated.connected = patch.connected;
                if patch.role != RiseRTCParticipantRole::Unspecified {
                    updated.role = patch.role;
                }
                updated.hand_raised = patch.hand_raised;
                updated.hand_expires_at_ms = patch.hand_expires_at_ms;
            }
            RiseForumVoiceUpdateKind::Left => {
                let before = roster_count(&participants);
                participants.retain(|row| row.user_id != event.user_id);
                if is_current {
                    if roster_count(&participants) > event.participant_count {
                        participants.truncate(usize::try_from(event.participant_count.max(0)).unwrap_or(0));
                    }
                } else {
                    resolved_count = (state.participant_count - (before - roster_count(&participants))).max(0);
                }
            }
        }

        let key = channel_key(&event.chat_id, &event.topic_id);
        participants.truncate(self.roster_cap(&key));
        let visible = participants;
        let held = roster_count(&visible);
        let count = resolved_count.max(held);
        let room_id = if event.room_id.is_empty() { state.room_id.clone() } else { event.room_id.clone() };
        let session_started_at_ms =
            VoiceChannelState::resolve_session_start(Some(&state), &room_id, &visible, count);
        self.channels.entry(event.chat_id.clone()).or_default().insert(
            event.topic_id.clone(),
            VoiceChannelState {
                room_id,
                participant_count: count,
                user_limit: state.user_limit,
                participants: visible,
                has_more: count > held,
                generated_at_ms: event.occurred_at_ms.max(state.generated_at_ms),
                session_started_at_ms,
                visible_limit: state.visible_limit,
            },
        );
        self.touch(&event.chat_id);
        self.evict_if_needed();
        if let Some(limit) = self.expanded_limits.get(&key).copied() {
            if held < count.min(limit) {
                self.repairs_due.push(RiseForumVoiceRepair::Expanded {
                    chat_id: event.chat_id.clone(),
                    topic_id: event.topic_id.clone(),
                });
            }
        } else if held < count.min(VoiceChannelState::COLLAPSED_ROWS) {
            self.repairs_due.push(RiseForumVoiceRepair::Roster(event.chat_id.clone()));
        }
        Some(self.snapshot())
    }

    fn expand(&mut self, chat_id: &str, topic_id: &str) -> (bool, Option<RiseForumVoicePresenceSnapshot>) {
        if self.account_id.is_none() {
            return (false, None);
        }
        let Some((held, total)) = self
            .channels
            .get(chat_id)
            .and_then(|topics| topics.get(topic_id))
            .map(|state| (roster_count(&state.participants), state.participant_count))
        else {
            return (false, None);
        };
        let key = channel_key(chat_id, topic_id);
        let current = self.expanded_limits.get(&key).copied().unwrap_or(VoiceChannelState::COLLAPSED_ROWS);
        let next = (current + VoiceChannelState::EXPAND_STEP).min(*EXPANDED_CEILING);
        if next <= current {
            return (false, None);
        }
        self.expanded_limits.insert(key, next);
        self.touch(chat_id);
        (held < total.min(next), Some(self.snapshot()))
    }

    fn collapse(&mut self, chat_id: &str, topic_id: &str) -> Option<RiseForumVoicePresenceSnapshot> {
        let key = channel_key(chat_id, topic_id);
        self.expanded_limits.remove(&key)?;
        if let Some(repair) = self.expanded_repairs.remove(&key) {
            repair.abort();
        }
        let cap = *MAX_PARTICIPANTS_PER_CHANNEL;
        if let Some(state) = self.channels.get_mut(chat_id).and_then(|topics| topics.get_mut(topic_id))
            && state.participants.len() > cap
        {
            state.participants.truncate(cap);
            state.has_more = state.participant_count > roster_count(&state.participants);
        }
        Some(self.snapshot())
    }

    fn expanded_roster_request(
        &mut self,
        chat_id: &str,
        topic_id: &str,
    ) -> (Option<RiseForumVoicePresenceRepositoryExpandedRosterRequest>, Option<RiseForumVoicePresenceSnapshot>) {
        let key = channel_key(chat_id, topic_id);
        let (Some(account_id), Some(limit)) = (self.account_id.clone(), self.expanded_limits.get(&key).copied()) else {
            return (None, None);
        };
        let Some(state) = self.channels.get(chat_id).and_then(|topics| topics.get(topic_id)) else {
            return (None, None);
        };
        if state.room_id.is_empty() {
            return (None, self.fail_expanded_roster(chat_id, topic_id));
        }
        let after_order = state.participants.last().map_or(0, |row| row.order_sequence);
        let request = RiseForumVoicePresenceRepositoryExpandedRosterRequest {
            account_id,
            generation: self.generation,
            room_id: state.room_id.clone(),
            limit,
            after_order,
            cursor: (after_order > 0).then(|| after_order.to_string()),
            page_limit: (limit - roster_count(&state.participants)).max(1),
        };
        (Some(request), None)
    }

    fn adopt_expanded_roster(
        &mut self,
        chat_id: &str,
        topic_id: &str,
        request: &RiseForumVoicePresenceRepositoryExpandedRosterRequest,
        page: &[RiseRTCParticipant],
        has_more: bool,
    ) -> Option<RiseForumVoicePresenceSnapshot> {
        let key = channel_key(chat_id, topic_id);
        if self.account_id.as_deref() != Some(request.account_id.as_str())
            || self.generation != request.generation
            || self.expanded_limits.get(&key).copied() != Some(request.limit)
        {
            return None;
        }
        let mut state = self.channels.get(chat_id)?.get(topic_id)?.clone();
        if state.room_id != request.room_id {
            return None;
        }
        if state.participants.last().map_or(0, |row| row.order_sequence) != request.after_order {
            self.repairs_due.push(RiseForumVoiceRepair::Expanded {
                chat_id: chat_id.to_owned(),
                topic_id: topic_id.to_owned(),
            });
            return None;
        }
        let held: HashSet<&str> = state.participants.iter().map(|row| row.user_id.as_str()).collect();
        let fresh: Vec<VoiceChannelParticipant> = page
            .iter()
            .filter(|row| !held.contains(row.user_id.as_str()))
            .take(usize::try_from(request.page_limit).unwrap_or(0))
            .map(|row| participant_from(row, None))
            .collect();
        let mut rows = state.participants.clone();
        rows.extend(fresh);
        let row_count = roster_count(&rows);
        let count = if has_more { state.participant_count.max(row_count) } else { row_count };
        rows.truncate(usize::try_from(request.limit).unwrap_or(0));
        state.participants = rows;
        state.participant_count = count;
        state.has_more = count > row_count;
        state.session_started_at_ms =
            VoiceChannelState::resolve_session_start(Some(&state), &state.room_id, &state.participants, count);
        self.channels.entry(chat_id.to_owned()).or_default().insert(topic_id.to_owned(), state);
        Some(self.snapshot())
    }

    fn fail_expanded_roster(&mut self, chat_id: &str, topic_id: &str) -> Option<RiseForumVoicePresenceSnapshot> {
        let key = channel_key(chat_id, topic_id);
        let limit = self.expanded_limits.get(&key).copied()?;
        let held_rows = roster_count(&self.channels.get(chat_id)?.get(topic_id)?.participants);
        let held = VoiceChannelState::COLLAPSED_ROWS.max(limit.min(held_rows));
        if held >= limit {
            return None;
        }
        if held <= VoiceChannelState::COLLAPSED_ROWS {
            self.expanded_limits.remove(&key);
        } else {
            self.expanded_limits.insert(key, held);
        }
        Some(self.snapshot())
    }

    fn finish_expanded_repair(&mut self, key: &str, generation: u64) -> bool {
        if self.generation != generation {
            return false;
        }
        self.expanded_repairs.remove(key);
        self.expanded_limits.contains_key(key)
    }

    fn preview_is_short(&self, chat_id: &str) -> bool {
        let Some(bucket) = self.channels.get(chat_id) else {
            return false;
        };
        bucket.iter().any(|(topic_id, state)| {
            !self.expanded_limits.contains_key(&channel_key(chat_id, topic_id))
                && roster_count(&state.participants) < state.participant_count.min(VoiceChannelState::COLLAPSED_ROWS)
        })
    }

    fn touch(&mut self, chat_id: &str) {
        self.chat_order.retain(|id| id != chat_id);
        self.chat_order.push(chat_id.to_owned());
    }

    fn evict_if_needed(&mut self) {
        while self.chat_order.len() > MAX_CHATS {
            let evicted = self.chat_order.remove(0);
            self.channels.remove(&evicted);
            let prefix = channel_key(&evicted, "");
            let expanded: Vec<String> = self.expanded_limits.keys().filter(|key| key.starts_with(&prefix)).cloned().collect();
            for key in expanded {
                self.expanded_limits.remove(&key);
                if let Some(repair) = self.expanded_repairs.remove(&key) {
                    repair.abort();
                }
            }
        }
    }
}

/// live occupancy overlay for the forum topic list
/// holds only what changes between fetches so a mute toggle patches one row
pub struct RiseForumVoicePresenceRepository {
    state: Arc<Mutex<RiseForumVoicePresenceState>>,
}

impl Default for RiseForumVoicePresenceRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseForumVoicePresenceRepository {
    pub fn new() -> Self {
        Self { state: Arc::new(Mutex::new(RiseForumVoicePresenceState::default())) }
    }

    pub async fn activate(&self, account_id: &str) {
        let Some(snapshot) = self.state.lock().activate(account_id) else {
            return;
        };
        Self::publish(snapshot).await;
    }

    pub async fn deactivate(&self) {
        let snapshot = self.state.lock().deactivate();
        Self::publish(snapshot).await;
    }

    /// seeds the overlay from a listing so an early patch has a base
    /// each channel keeps newer of fetch and delta by generated_at_ms
    /// no roster means empty if another channel has voice_state
    pub async fn adopt(&self, chat_id: &str, topics: &[ChatTopic]) {
        let Some(snapshot) = self.state.lock().adopt(chat_id, topics) else {
            return;
        };
        Self::publish(snapshot).await;
    }

    /// tap on N more, shows EXPAND_STEP more rows
    /// returns true if the held roster is short so the caller asks the rtc listing
    /// missing rows are placeholders
    pub async fn expand(&self, chat_id: &str, topic_id: &str) -> bool {
        let (is_short, snapshot) = self.state.lock().expand(chat_id, topic_id);
        let Some(snapshot) = snapshot else {
            return false;
        };
        Self::publish(snapshot).await;
        is_short
    }

    /// show less, back to collapsed rows trimmed to the server preview in join order
    pub async fn collapse(&self, chat_id: &str, topic_id: &str) {
        let Some(snapshot) = self.state.lock().collapse(chat_id, topic_id) else {
            return;
        };
        Self::publish(snapshot).await;
    }

    pub async fn expanded_roster_request(
        &self,
        chat_id: &str,
        topic_id: &str,
    ) -> Option<RiseForumVoicePresenceRepositoryExpandedRosterRequest> {
        let (request, failed) = self.state.lock().expanded_roster_request(chat_id, topic_id);
        if let Some(snapshot) = failed {
            Self::publish(snapshot).await;
        }
        request
    }

    pub async fn adopt_expanded_roster(
        &self,
        chat_id: &str,
        topic_id: &str,
        request: &RiseForumVoicePresenceRepositoryExpandedRosterRequest,
        participants: &[RiseRTCParticipant],
        has_more: bool,
    ) {
        let snapshot = self.state.lock().adopt_expanded_roster(chat_id, topic_id, request, participants, has_more);
        if let Some(snapshot) = snapshot {
            Self::publish(snapshot).await;
        }
        self.start_due_repairs();
    }

    /// the listing failed so the expansion shrinks to the rows held
    /// the more button stays for another tap
    pub async fn fail_expanded_roster(&self, chat_id: &str, topic_id: &str) {
        let Some(snapshot) = self.state.lock().fail_expanded_roster(chat_id, topic_id) else {
            return;
        };
        Self::publish(snapshot).await;
    }

    fn start_due_repairs(&self) {
        let mut state = self.state.lock();
        for repair in std::mem::take(&mut state.repairs_due) {
            match repair {
                RiseForumVoiceRepair::Roster(chat_id) => Self::schedule_roster_repair(&self.state, &mut state, chat_id),
                RiseForumVoiceRepair::Expanded { chat_id, topic_id } => {
                    Self::schedule_expanded_repair(&self.state, &mut state, chat_id, topic_id);
                }
            }
        }
    }

    fn schedule_expanded_repair(
        shared: &Arc<Mutex<RiseForumVoicePresenceState>>,
        state: &mut RiseForumVoicePresenceState,
        chat_id: String,
        topic_id: String,
    ) {
        let key = channel_key(&chat_id, &topic_id);
        if state.expanded_repairs.contains_key(&key) {
            return;
        }
        let generation = state.generation;
        let weak: Weak<Mutex<RiseForumVoicePresenceState>> = Arc::downgrade(shared);
        let task_key = key.clone();
        let repair = RiseAccountRuntime::current().spawn(async move {
            tokio::time::sleep(ROSTER_REPAIR_DELAY).await;
            let Some(shared) = weak.upgrade() else {
                return;
            };
            if !shared.lock().finish_expanded_repair(&task_key, generation) {
                return;
            }
            Self::on_main(move |cx| {
                if let Some(hooks) = chat_engine_hooks() {
                    (hooks.load_forum_voice_roster_action)(&chat_id, &topic_id, cx);
                }
            })
            .await;
        });
        state.expanded_repairs.insert(key, repair);
    }

    /// a delta can only remove a listed member, who moves up into the preview only the server knows
    /// one forced listing per chat per window refills it
    /// the window stops a churning room from fetching on every left
    fn schedule_roster_repair(
        shared: &Arc<Mutex<RiseForumVoicePresenceState>>,
        state: &mut RiseForumVoicePresenceState,
        chat_id: String,
    ) {
        if state.roster_repairs.contains_key(&chat_id) {
            state.roster_repairs_pending.insert(chat_id);
            return;
        }
        let generation = state.generation;
        let weak: Weak<Mutex<RiseForumVoicePresenceState>> = Arc::downgrade(shared);
        let task_chat_id = chat_id.clone();
        let repair = RiseAccountRuntime::current().spawn(async move {
            tokio::time::sleep(ROSTER_REPAIR_DELAY).await;
            let Some(shared) = weak.upgrade() else {
                return;
            };
            if shared.lock().generation != generation {
                return;
            }
            let listing_chat_id = task_chat_id.clone();
            Self::on_main(move |cx| {
                if let Some(hooks) = chat_engine_hooks() {
                    (hooks.get_forum_structure_action)(&listing_chat_id, true, None, None, cx);
                }
            })
            .await;
            tokio::time::sleep(ROSTER_REPAIR_WINDOW).await;
            Self::finish_roster_repair(&shared, task_chat_id, generation);
        });
        state.roster_repairs.insert(chat_id, repair);
    }

    fn finish_roster_repair(shared: &Arc<Mutex<RiseForumVoicePresenceState>>, chat_id: String, generation: u64) {
        let mut state = shared.lock();
        if state.generation != generation {
            return;
        }
        state.roster_repairs.remove(&chat_id);
        if !state.roster_repairs_pending.remove(&chat_id) || !state.preview_is_short(&chat_id) {
            return;
        }
        Self::schedule_roster_repair(shared, &mut state, chat_id);
    }

    async fn on_main(work: impl FnOnce(&mut App) + Send + 'static) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(work);
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    /// puts this account in or out of a channel before the server says
    /// reconciled by user id so doing it twice changes nothing, leave comes from many paths
    pub async fn apply_optimistic_self(
        &self,
        chat_id: &str,
        topic_id: &str,
        joined: bool,
        participant: Option<VoiceChannelParticipant>,
    ) {
        let Some(snapshot) = self.state.lock().apply_optimistic_self(chat_id, topic_id, joined, participant) else {
            return;
        };
        Self::publish(snapshot).await;
        self.start_due_repairs();
    }

    pub async fn apply(&self, event: RiseForumVoiceUpdate) {
        let Some(snapshot) = self.state.lock().apply(&event) else {
            return;
        };
        Self::publish(snapshot).await;
        self.start_due_repairs();
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> RiseForumVoicePresenceSnapshot {
        self.state.lock().snapshot()
    }

    async fn publish(snapshot: RiseForumVoicePresenceSnapshot) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(move |cx| {
                        RiseForumVoicePresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.publish(snapshot, cx);
                        });
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseForumVoiceUpdateKind {
    Joined,
    Left,
    Media,
}

impl RiseForumVoiceUpdateKind {
    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        let kind = rise_chat::VoiceUpdateKind::ALL.into_iter().find(|kind| kind.wire() == raw_value)?;
        Some(match kind {
            rise_chat::VoiceUpdateKind::Joined => Self::Joined,
            rise_chat::VoiceUpdateKind::Left => Self::Left,
            rise_chat::VoiceUpdateKind::Media => Self::Media,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseForumVoiceUpdateScope {
    Channel,
    Group,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseForumVoiceUpdate {
    pub chat_id: String,
    pub topic_id: String,
    pub scope: RiseForumVoiceUpdateScope,
    pub room_id: String,
    pub kind: RiseForumVoiceUpdateKind,
    pub user_id: String,
    pub participant_count: i64,
    pub participant: Option<VoiceChannelParticipant>,
    pub occurred_at_ms: i64,
}

pub struct RiseForumVoicePresentation {
    snapshot: RiseForumVoicePresenceSnapshot,
    hand_expiry_revision: i64,
    hand_expiry_task: Option<Task<()>>,
    hand_expiry_deadline_ms: Option<i64>,
}

impl RiseForumVoicePresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {
            snapshot: RiseForumVoicePresenceSnapshot::empty(),
            hand_expiry_revision: 0,
            hand_expiry_task: None,
            hand_expiry_deadline_ms: None,
        })
    }

    pub fn snapshot(&self) -> &RiseForumVoicePresenceSnapshot {
        &self.snapshot
    }

    pub fn hand_expiry_revision(&self) -> i64 {
        self.hand_expiry_revision
    }

    pub fn publish(&mut self, value: RiseForumVoicePresenceSnapshot, cx: &mut Context<Self>) {
        if self.snapshot == value {
            return;
        }
        self.snapshot = value;
        self.schedule_hand_expiry(cx);
        cx.notify();
    }

    /// one timer for all on screen channels armed on the earliest deadline
    /// not per hand and never a per second tick, that is idle burn
    fn schedule_hand_expiry(&mut self, cx: &mut Context<Self>) {
        let now_ms = Self::now_ms();
        let next = self.snapshot.nearest_hand_deadline_ms(now_ms);
        if self.hand_expiry_deadline_ms == next {
            return;
        }
        self.hand_expiry_deadline_ms = next;
        self.hand_expiry_task = None;
        let Some(next) = next else {
            return;
        };
        let window = RiseTimingCanon::server_window("rtc_raised_hand_ttl").window_ms;
        let remaining = (next - now_ms).max(0).min(window);
        self.hand_expiry_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(u64::try_from(remaining).unwrap_or(0))).await;
            let _ = this.update(cx, |presentation, cx| {
                presentation.hand_expiry_deadline_ms = None;
                presentation.hand_expiry_revision = presentation.hand_expiry_revision.wrapping_add(1);
                presentation.schedule_hand_expiry(cx);
                cx.notify();
            });
        }));
    }

    pub fn now_ms() -> i64 {
        RiseClock::now().ms()
    }

    pub fn state(&self, chat_id: &str, topic_id: &str) -> Option<VoiceChannelState> {
        self.snapshot.state(chat_id, topic_id)
    }
}
