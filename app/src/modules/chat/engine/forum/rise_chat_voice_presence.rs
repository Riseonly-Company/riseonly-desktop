use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity};
use parking_lot::Mutex;
use rise_canon::RiseLimits;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::forum::rise_forum_voice_presence::{
    RiseForumVoiceUpdate, RiseForumVoiceUpdateKind, RiseForumVoiceUpdateScope,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{VoiceChannelParticipant, VoiceChannelState};

const MAX_CHATS: usize = 128;

static MAX_GROUP_PARTICIPANTS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("voice_preview_participants")).unwrap_or(0));

fn roster_count(participants: &[VoiceChannelParticipant]) -> i64 {
    i64::try_from(participants.len()).unwrap_or(i64::MAX)
}

#[derive(Default)]
struct RiseChatVoicePresenceState {
    account_id: Option<String>,
    generation: u64,
    occupied: HashMap<String, HashSet<String>>,
    order: Vec<String>,
    observed: HashSet<String>,
    group_channels: HashMap<String, VoiceChannelState>,
    self_rooms: HashMap<String, HashSet<String>>,
}

impl RiseChatVoicePresenceState {
    fn clear(&mut self) {
        self.occupied = HashMap::new();
        self.order = Vec::new();
        self.observed = HashSet::new();
        self.group_channels = HashMap::new();
        self.self_rooms = HashMap::new();
    }

    fn snapshot(&self) -> RiseChatVoicePresenceSnapshot {
        RiseChatVoicePresenceSnapshot {
            account_id: self.account_id.clone(),
            generation: self.generation,
            live_chat_ids: self.occupied.keys().cloned().collect(),
            observed_chat_ids: self.observed.clone(),
            group_channels: self.group_channels.clone(),
        }
    }

    fn activate(&mut self, account_id: &str) -> Option<RiseChatVoicePresenceSnapshot> {
        if self.account_id.as_deref() == Some(account_id) {
            return None;
        }
        self.account_id = Some(account_id.to_owned());
        self.generation = self.generation.wrapping_add(1);
        self.clear();
        Some(self.snapshot())
    }

    fn deactivate(&mut self) -> RiseChatVoicePresenceSnapshot {
        self.account_id = None;
        self.generation = self.generation.wrapping_add(1);
        self.clear();
        self.snapshot()
    }

    fn adopt_group(&mut self, chat_id: &str, state: Option<VoiceChannelState>) -> Option<RiseChatVoicePresenceSnapshot> {
        if self.account_id.is_none() || chat_id.is_empty() {
            return None;
        }
        let Some(state) = state else {
            let had_roster = self.group_channels.remove(chat_id).is_some();
            let had_rooms = self.occupied.remove(chat_id).is_some();
            let is_new = self.observed.insert(chat_id.to_owned());
            self.touch(chat_id);
            self.evict_if_needed();
            if !(had_roster || had_rooms || is_new) {
                return None;
            }
            return Some(self.snapshot());
        };
        let stored = self.group_channels.get(chat_id);
        tracing::warn!(target: "riseonly::X1906", chat = %chat_id, fetched_at = state.generated_at_ms, stored_at = stored.map_or(-1, |stored| stored.generated_at_ms), count = state.participant_count, "voice group adopt");
        if stored.is_some_and(|stored| stored.generated_at_ms >= state.generated_at_ms) {
            return None;
        }
        let mut adopted = state;
        adopted.session_started_at_ms = VoiceChannelState::resolve_session_start(
            stored,
            &adopted.room_id,
            &adopted.participants,
            adopted.participant_count,
        );
        let room_id = adopted.room_id.clone();
        let participant_count = adopted.participant_count;
        self.group_channels.insert(chat_id.to_owned(), adopted);
        self.observed.insert(chat_id.to_owned());
        self.touch(chat_id);
        if participant_count > 0 && !room_id.is_empty() {
            self.occupied.entry(chat_id.to_owned()).or_default().insert(room_id);
            self.touch(chat_id);
            self.evict_if_needed();
        }
        Some(self.snapshot())
    }

    fn drop_delta_answers_after_reconnect(&mut self) -> Option<RiseChatVoicePresenceSnapshot> {
        self.account_id.as_ref()?;
        let stale_rooms: Vec<String> =
            self.occupied.keys().filter(|chat_id| !self.self_rooms.contains_key(*chat_id)).cloned().collect();
        let stale_observed: Vec<String> =
            self.observed.iter().filter(|chat_id| !self.self_rooms.contains_key(*chat_id)).cloned().collect();
        if stale_rooms.is_empty() && stale_observed.is_empty() {
            return None;
        }
        for chat_id in &stale_rooms {
            self.occupied.remove(chat_id);
        }
        for chat_id in &stale_observed {
            self.observed.remove(chat_id);
        }
        Some(self.snapshot())
    }

    fn forget(&mut self, chat_id: &str) -> Option<RiseChatVoicePresenceSnapshot> {
        if chat_id.is_empty() {
            return None;
        }
        if self.self_rooms.get(chat_id).is_some_and(|rooms| !rooms.is_empty()) {
            return None;
        }
        let had_opinion = self.observed.remove(chat_id);
        let had_rooms = self.occupied.remove(chat_id).is_some();
        let had_roster = self.group_channels.remove(chat_id).is_some();
        self.order.retain(|id| id != chat_id);
        if !(had_opinion || had_rooms || had_roster) {
            return None;
        }
        Some(self.snapshot())
    }

    fn apply_optimistic_self(
        &mut self,
        chat_id: &str,
        room_id: &str,
        is_group_room: bool,
        joined: bool,
        participant: Option<VoiceChannelParticipant>,
    ) -> Option<RiseChatVoicePresenceSnapshot> {
        let account_id = self.account_id.clone()?;
        if chat_id.is_empty() || room_id.is_empty() {
            return None;
        }
        if participant.as_ref().is_some_and(|participant| participant.user_id != account_id) {
            return None;
        }

        let mut rooms = self.occupied.get(chat_id).cloned().unwrap_or_default();
        let was_seated = self.self_rooms.get(chat_id).is_some_and(|mine| mine.contains(room_id));
        let mut mine = self.self_rooms.get(chat_id).cloned().unwrap_or_default();
        if joined {
            rooms.insert(room_id.to_owned());
            mine.insert(room_id.to_owned());
        } else {
            rooms.remove(room_id);
            mine.remove(room_id);
        }
        if mine.is_empty() {
            self.self_rooms.remove(chat_id);
        } else {
            self.self_rooms.insert(chat_id.to_owned(), mine);
        }
        self.observed.insert(chat_id.to_owned());

        if is_group_room {
            let stored = self.group_channels.get(chat_id).cloned().unwrap_or_else(|| VoiceChannelState {
                room_id: room_id.to_owned(),
                participant_count: 0,
                user_limit: 0,
                participants: Vec::new(),
                has_more: false,
                generated_at_ms: 0,
                session_started_at_ms: 0,
                visible_limit: VoiceChannelState::COLLAPSED_ROWS,
            });
            let mut participants = stored.participants.clone();
            let was_listed = participants.iter().any(|row| row.user_id == account_id);
            if joined && let Some(participant) = participant {
                if !was_listed {
                    participants.push(participant);
                }
            } else {
                participants.retain(|row| row.user_id != account_id);
            }
            let leaves_seat = was_listed || was_seated;
            let held = roster_count(&participants);
            let count = if joined {
                stored.participant_count.max(held)
            } else if leaves_seat {
                (stored.participant_count - 1).min(held).max(0)
            } else {
                held.max(stored.participant_count)
            };
            let session_started_at_ms =
                VoiceChannelState::resolve_session_start(Some(&stored), room_id, &participants, count);
            if !joined && count > 0 {
                rooms.insert(room_id.to_owned());
                tracing::warn!(target: "riseonly::X1867", chat_id, room_id, count, "self left, group room still live");
            }
            if count <= 0 {
                self.group_channels.remove(chat_id);
            } else {
                self.group_channels.insert(
                    chat_id.to_owned(),
                    VoiceChannelState {
                        room_id: room_id.to_owned(),
                        participant_count: count,
                        user_limit: stored.user_limit,
                        has_more: count > held,
                        participants,
                        generated_at_ms: stored.generated_at_ms,
                        session_started_at_ms,
                        visible_limit: stored.visible_limit,
                    },
                );
            }
        }

        if rooms.is_empty() {
            self.occupied.remove(chat_id);
        } else {
            self.occupied.insert(chat_id.to_owned(), rooms);
        }

        self.touch(chat_id);
        self.evict_if_needed();
        Some(self.snapshot())
    }

    fn apply(&mut self, event: &RiseForumVoiceUpdate) -> Option<RiseChatVoicePresenceSnapshot> {
        let account_id = self.account_id.clone()?;
        if event.chat_id.is_empty() || event.room_id.is_empty() {
            return None;
        }

        if event.kind == RiseForumVoiceUpdateKind::Left && event.user_id == account_id {
            let mut mine = self.self_rooms.get(&event.chat_id).cloned().unwrap_or_default();
            mine.remove(&event.room_id);
            if mine.is_empty() {
                self.self_rooms.remove(&event.chat_id);
            } else {
                self.self_rooms.insert(event.chat_id.clone(), mine);
            }
        }

        let before = self.occupied.get(&event.chat_id).cloned().unwrap_or_default();
        let mut rooms = before.clone();
        let was_observed = self.observed.contains(&event.chat_id);
        self.observed.insert(event.chat_id.clone());
        if event.kind == RiseForumVoiceUpdateKind::Left && event.participant_count <= 0 {
            rooms.remove(&event.room_id);
        } else if event.participant_count > 0 {
            rooms.insert(event.room_id.clone());
        }
        if event.scope == RiseForumVoiceUpdateScope::Group {
            let stored_at = self.group_channels.get(&event.chat_id).map_or(-1, |state| state.generated_at_ms);
            self.apply_group_roster(event);
            let after = self.group_channels.get(&event.chat_id).map_or(-1, |state| state.participant_count);
            tracing::warn!(target: "riseonly::X1906", chat = %event.chat_id, kind = ?event.kind, occurred = event.occurred_at_ms, stored_at, after, "voice group roster applied");
        }
        if rooms == before && event.scope != RiseForumVoiceUpdateScope::Group && was_observed {
            return None;
        }

        if rooms.is_empty() {
            self.occupied.remove(&event.chat_id);
        } else {
            self.occupied.insert(event.chat_id.clone(), rooms);
        }
        self.touch(&event.chat_id);
        self.evict_if_needed();
        Some(self.snapshot())
    }

    fn apply_group_roster(&mut self, event: &RiseForumVoiceUpdate) {
        let state = self.group_channels.get(&event.chat_id).cloned().unwrap_or_else(|| VoiceChannelState {
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
            return;
        }
        let mut participants = state.participants.clone();
        let mut resolved_count = event.participant_count;
        match event.kind {
            RiseForumVoiceUpdateKind::Joined => {
                let Some(mut participant) = event.participant.clone() else {
                    return;
                };
                if let Some(index) = participants.iter().position(|row| row.user_id == participant.user_id) {
                    if participant.avatar_url.is_empty() {
                        participant.avatar_url = participants[index].avatar_url.clone();
                    }
                    participants[index] = participant;
                } else {
                    participants.push(participant);
                    participants.sort_by_key(|row| row.order_sequence);
                }
            }
            RiseForumVoiceUpdateKind::Media => {
                let Some(patch) = event.participant.as_ref() else {
                    return;
                };
                let Some(updated) = participants.iter_mut().find(|row| row.user_id == patch.user_id) else {
                    return;
                };
                updated.microphone_enabled = patch.microphone_enabled;
                updated.camera_enabled = patch.camera_enabled;
                updated.screen_share_enabled = patch.screen_share_enabled;
                updated.deafened = patch.deafened;
                updated.server_muted = patch.server_muted;
                updated.server_deafened = patch.server_deafened;
                updated.connected = patch.connected;
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
        participants.truncate(*MAX_GROUP_PARTICIPANTS);
        let visible = participants;
        let held = roster_count(&visible);
        let count = resolved_count.max(held);
        let room_id = if event.room_id.is_empty() { state.room_id.clone() } else { event.room_id.clone() };
        let session_started_at_ms =
            VoiceChannelState::resolve_session_start(Some(&state), &room_id, &visible, count);
        if count <= 0 {
            self.group_channels.remove(&event.chat_id);
        } else {
            self.group_channels.insert(
                event.chat_id.clone(),
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
        }
    }

    fn touch(&mut self, chat_id: &str) {
        self.order.retain(|id| id != chat_id);
        self.order.push(chat_id.to_owned());
    }

    fn evict_if_needed(&mut self) {
        while self.order.len() > MAX_CHATS {
            let Some(index) = self.order.iter().position(|id| !self.self_rooms.contains_key(id)) else {
                return;
            };
            let evicted = self.order.remove(index);
            self.occupied.remove(&evicted);
            self.observed.remove(&evicted);
            self.group_channels.remove(&evicted);
        }
    }
}

/// somebody is in voice in this chat, one boolean per chat
/// separate from forum presence repo, full rosters and capped at eight chats
/// badge goes dark only when every room is empty
pub struct RiseChatVoicePresenceRepository {
    state: Mutex<RiseChatVoicePresenceState>,
}

impl Default for RiseChatVoicePresenceRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseChatVoicePresenceRepository {
    pub fn new() -> Self {
        Self { state: Mutex::new(RiseChatVoicePresenceState::default()) }
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

    pub async fn adopt_group(&self, chat_id: &str, state: Option<VoiceChannelState>) {
        let Some(snapshot) = self.state.lock().adopt_group(chat_id, state) else {
            return;
        };
        Self::publish(snapshot).await;
    }

    pub async fn drop_delta_answers_after_reconnect(&self) {
        let Some(snapshot) = self.state.lock().drop_delta_answers_after_reconnect() else {
            return;
        };
        Self::publish(snapshot).await;
    }

    /// chat subscription ended so the overlay cant answer for it anymore
    /// else nobody would outlive the unsubscribe and override a fetch
    pub async fn forget(&self, chat_id: &str) {
        let Some(snapshot) = self.state.lock().forget(chat_id) else {
            return;
        };
        Self::publish(snapshot).await;
    }

    /// own join or leave of a voice room, applied before the reply
    /// later joined or left merges by room and user id, nothing doubles
    /// device clock not used, a fast one mutes later deltas
    pub async fn apply_optimistic_self(
        &self,
        chat_id: &str,
        room_id: &str,
        is_group_room: bool,
        joined: bool,
        participant: Option<VoiceChannelParticipant>,
    ) {
        let Some(snapshot) =
            self.state.lock().apply_optimistic_self(chat_id, room_id, is_group_room, joined, participant)
        else {
            return;
        };
        Self::publish(snapshot).await;
    }

    pub async fn apply(&self, event: RiseForumVoiceUpdate) {
        let Some(snapshot) = self.state.lock().apply(&event) else {
            return;
        };
        Self::publish(snapshot).await;
    }

    async fn publish(snapshot: RiseChatVoicePresenceSnapshot) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(move |cx| {
                        RiseChatVoicePresentation::shared(cx).update(cx, |presentation, cx| {
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseChatVoicePresenceSnapshot {
    pub account_id: Option<String>,
    pub generation: u64,
    pub live_chat_ids: HashSet<String>,
    pub observed_chat_ids: HashSet<String>,
    pub group_channels: HashMap<String, VoiceChannelState>,
}

impl RiseChatVoicePresenceSnapshot {
    pub fn empty() -> Self {
        Self::default()
    }
}

pub struct RiseChatVoicePresentation {
    snapshot: RiseChatVoicePresenceSnapshot,
}

impl RiseChatVoicePresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { snapshot: RiseChatVoicePresenceSnapshot::empty() })
    }

    pub fn snapshot(&self) -> &RiseChatVoicePresenceSnapshot {
        &self.snapshot
    }

    pub fn publish(&mut self, value: RiseChatVoicePresenceSnapshot, cx: &mut Context<Self>) {
        if self.snapshot == value {
            return;
        }
        self.snapshot = value;
        cx.notify();
    }

    /// none means no opinion, the socket isnt subscribed so the row shows what the last fetch said
    pub fn is_live(&self, chat_id: &str) -> Option<bool> {
        if self.snapshot.live_chat_ids.contains(chat_id) {
            return Some(true);
        }
        self.snapshot.observed_chat_ids.contains(chat_id).then_some(false)
    }

    pub fn group_state(&self, chat_id: &str) -> Option<VoiceChannelState> {
        self.snapshot.group_channels.get(chat_id).cloned()
    }
}
