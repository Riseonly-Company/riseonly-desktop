use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use gpui::{App, AsyncApp};
use rise_canon::{RiseTimingCanon, rise_key, rise_request};
use rise_core::rise_isolated::RiseIsolated;
use rise_engine::{RiseEngine, RiseJSONValue};

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::chats::rise_chat_repository_models::RiseChatRepositoryError;
use crate::modules::chat::engine::core::rise_conversation_rpc::{RiseConversationEndpoints, RiseConversationRPC};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_unread_marks_wire_types::{
    ClearUnreadMarksGatewayDTO, GetUnreadMarksGatewayDTO, MarkMessageAsMentionedReadGatewayDTO, MarkReactionsAsReadGatewayDTO,
};

const RECENTLY_CONSUMED_LIMIT: usize = 512;
const REALTIME_CHANGE_ADDED: &str = "added";
const REALTIME_CHANGE_READ: &str = "read";
const REALTIME_CHANGE_REMOVED: &str = "removed";

static PAGE_LIMIT: LazyLock<i64> = LazyLock::new(|| {
    let page = RiseTimingCanon::page("unread_marks");
    page.client_page_size_items.unwrap_or(page.server_maximum_items)
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseUnreadMarkKind {
    Mention,
    Reaction,
    Reply,
}

impl RiseUnreadMarkKind {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Mention => "mention",
            Self::Reaction => "reaction",
            Self::Reply => "reply",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        match raw_value {
            "mention" => Some(Self::Mention),
            "reaction" => Some(Self::Reaction),
            "reply" => Some(Self::Reply),
            _ => None,
        }
    }
}

/// kind of a hand made unread flag on a whole chat
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseChatUnreadFlagKind {
    Chat,
}

impl RiseChatUnreadFlagKind {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Chat => "chat",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        (raw_value == Self::Chat.raw_value()).then_some(Self::Chat)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseUnreadReactionMark {
    pub message_id: String,
    pub count: i64,
    pub created_at: f64,
}

/// unread mentions and replies and reactions of one chat or topic, oldest first
/// id lists are a window the server caps
/// the counts are always complete
#[derive(Clone, Debug, PartialEq)]
pub struct RiseUnreadMarksSnapshot {
    pub scope: RiseMessageScope,
    pub mention_message_ids: Vec<String>,
    pub mentions_count: i64,
    pub reply_message_ids: Vec<String>,
    pub replies_count: i64,
    pub reaction_marks: Vec<RiseUnreadReactionMark>,
    pub reactions_count: i64,
    pub is_loaded: bool,
    pub revision: u64,
}

impl RiseUnreadMarksSnapshot {
    pub fn empty(scope: RiseMessageScope, loaded: bool) -> Self {
        Self {
            scope,
            mention_message_ids: Vec::new(),
            mentions_count: 0,
            reply_message_ids: Vec::new(),
            replies_count: 0,
            reaction_marks: Vec::new(),
            reactions_count: 0,
            is_loaded: loaded,
            revision: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.mentions_count == 0 && self.replies_count == 0 && self.reactions_count == 0
    }
}

/// a message on screen with the per row flags the wire carries
/// the ledger decides if it is still unread, its id lists beat the flags
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseUnreadMarkCandidate {
    pub message_id: String,
    pub is_mentioned_you: bool,
    pub has_unread_reactions: bool,
    pub is_replied_you: bool,
}

type RiseUnreadMarkCurrentCandidates = Arc<dyn Fn(&mut App) -> Vec<RiseUnreadMarkCandidate> + Send + Sync>;

#[derive(Clone)]
pub struct RiseUnreadMarkViewport {
    pub entered_candidates: Vec<RiseUnreadMarkCandidate>,
    current_candidates: RiseUnreadMarkCurrentCandidates,
}

impl RiseUnreadMarkViewport {
    pub fn new(
        entered_candidates: Vec<RiseUnreadMarkCandidate>,
        current_candidates: impl Fn(&mut App) -> Vec<RiseUnreadMarkCandidate> + Send + Sync + 'static,
    ) -> Self {
        Self { entered_candidates, current_candidates: Arc::new(current_candidates) }
    }

    pub async fn current_candidates(&self) -> Vec<RiseUnreadMarkCandidate> {
        let Some(main) = RiseEngineMainHop::installed() else {
            return Vec::new();
        };
        let current_candidates = self.current_candidates.clone();
        main.run(move |cx: AsyncApp| async move { Ok(cx.update(|cx| current_candidates(cx))) }.boxed_local())
            .await
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseUnreadMarksConsumption {
    pub scope: RiseMessageScope,
    pub mention_ids: Vec<String>,
    pub reply_ids: Vec<String>,
    pub reaction_ids: Vec<String>,
    pub reactions_cleared: i64,
}

impl RiseUnreadMarksConsumption {
    pub fn is_empty(&self) -> bool {
        self.mention_ids.is_empty() && self.reply_ids.is_empty() && self.reaction_ids.is_empty()
    }

    /// one mark_message_as_mentioned_read clears both the mention and the reply of a row
    pub fn mentioned_read_ids(&self) -> Vec<String> {
        let mentions: HashSet<&String> = self.mention_ids.iter().collect();
        self.mention_ids.iter().chain(self.reply_ids.iter().filter(|id| !mentions.contains(id))).cloned().collect()
    }
}

#[derive(Clone, Debug)]
pub struct RiseUnreadMarksRealtimeChange {
    pub chat_id: String,
    pub topic_id: Option<String>,
    pub kind: RiseUnreadMarkKind,
    pub change: String,
    pub message_ids: Vec<String>,
    pub delta: i64,
    pub timestamp: f64,
}

impl RiseUnreadMarksRealtimeChange {
    pub fn new(root: &BTreeMap<String, RiseJSONValue>) -> Option<Self> {
        let chat_id = RiseMessageWireCodec::string(root, &[rise_key::message::CHAT_ID]).filter(|chat_id| !chat_id.is_empty())?;
        let kind_raw = RiseMessageWireCodec::string(root, &[rise_key::events::KIND])?;
        let kind = RiseUnreadMarkKind::from_raw_value(&kind_raw)?;
        let change = RiseMessageWireCodec::string(root, &[rise_key::events::CHANGE])?;
        let delta = RiseMessageWireCodec::double(root, &[rise_key::events::DELTA])?;
        Some(Self {
            chat_id,
            topic_id: RiseMessageWireCodec::string(root, &[rise_key::message::TOPIC_ID]),
            kind,
            change,
            message_ids: RiseMessageWireCodec::strings(root, &[rise_key::message::MESSAGE_IDS]),
            delta: delta as i64,
            timestamp: RiseMessageWireCodec::double(root, &[rise_key::events::TIMESTAMP]).unwrap_or(0.0),
        })
    }
}

#[derive(Clone, Debug)]
pub struct RiseUnreadMarksRealtimeOutcome {
    pub applies_to_chat_row: bool,
    pub snapshots: Vec<RiseUnreadMarksSnapshot>,
    pub consumption: Option<RiseUnreadMarksConsumption>,
}

#[derive(Clone, Debug)]
pub struct RiseUnreadMarksConsumed {
    pub consumption: RiseUnreadMarksConsumption,
    pub snapshots: Vec<RiseUnreadMarksSnapshot>,
}

#[derive(Clone, Debug)]
pub struct RiseUnreadMarksCleared {
    pub removed: i64,
    pub message_ids: Vec<String>,
    pub snapshots: Vec<RiseUnreadMarksSnapshot>,
}

/// a consumption this device made
/// until the server confirms it a ledger page may still list the mark
#[derive(Clone, Copy)]
struct ConsumedMark {
    consumed_at: u64,
    confirmed_at: Option<u64>,
}

struct LoadingState {
    id: u64,
    changed_by_realtime: bool,
}

#[derive(Default)]
struct RiseUnreadMarksRepositoryState {
    snapshots: HashMap<RiseMessageScope, RiseUnreadMarksSnapshot>,
    loading: HashMap<RiseMessageScope, LoadingState>,
    assumed_empty: HashSet<RiseMessageScope>,
    revision_clock: u64,
    recently_consumed_mentions: HashMap<String, ConsumedMark>,
    recently_consumed_replies: HashMap<String, ConsumedMark>,
    recently_consumed_reactions: HashMap<String, ConsumedMark>,
    consumption_clock: u64,
    loading_clock: u64,
    seen_rows: HashMap<String, u64>,
    seen_clock: u64,
}

impl RiseUnreadMarksRepositoryState {
    fn store(&mut self, snapshot: RiseUnreadMarksSnapshot) -> RiseUnreadMarksSnapshot {
        self.revision_clock = self.revision_clock.wrapping_add(1);
        let mut next = snapshot;
        next.revision = self.revision_clock;
        self.snapshots.insert(next.scope.clone(), next.clone());
        next
    }

    fn remember_consumed(&mut self, id: &str, kind: RiseUnreadMarkKind) {
        self.consumption_clock = self.consumption_clock.wrapping_add(1);
        let mark = ConsumedMark { consumed_at: self.consumption_clock, confirmed_at: None };
        let ledger = match kind {
            RiseUnreadMarkKind::Mention => &mut self.recently_consumed_mentions,
            RiseUnreadMarkKind::Reply => &mut self.recently_consumed_replies,
            RiseUnreadMarkKind::Reaction => &mut self.recently_consumed_reactions,
        };
        ledger.insert(id.to_owned(), mark);
        Self::trim(ledger);
    }

    fn mark_confirmed(&mut self, mention_ids: &[String], reply_ids: &[String], reaction_ids: &[String]) {
        if mention_ids.is_empty() && reply_ids.is_empty() && reaction_ids.is_empty() {
            return;
        }
        self.consumption_clock = self.consumption_clock.wrapping_add(1);
        for id in mention_ids {
            if let Some(mark) = self.recently_consumed_mentions.get_mut(id) {
                mark.confirmed_at = Some(self.consumption_clock);
            }
        }
        for id in reply_ids {
            if let Some(mark) = self.recently_consumed_replies.get_mut(id) {
                mark.confirmed_at = Some(self.consumption_clock);
            }
        }
        for id in reaction_ids {
            if let Some(mark) = self.recently_consumed_reactions.get_mut(id) {
                mark.confirmed_at = Some(self.consumption_clock);
            }
        }
    }

    fn trim(ledger: &mut HashMap<String, ConsumedMark>) {
        if ledger.len() <= RECENTLY_CONSUMED_LIMIT {
            return;
        }
        let overflow = ledger.len() - RECENTLY_CONSUMED_LIMIT;
        let mut by_age: Vec<(String, u64)> = ledger.iter().map(|(id, mark)| (id.clone(), mark.consumed_at)).collect();
        by_age.sort_by_key(|(_, consumed_at)| *consumed_at);
        for (id, _) in by_age.into_iter().take(overflow) {
            ledger.remove(&id);
        }
    }

    /// removes the ids from every loaded ledger of the chat and moves the counts
    /// the scope explicitly zeroed ends at zero whatever its window held
    fn apply_local_removal(
        &mut self,
        chat_id: &str,
        mention_ids: &HashSet<String>,
        reply_ids: &HashSet<String>,
        reaction_ids: &HashSet<String>,
        mentions_delta: i64,
        replies_delta: i64,
        reactions_delta: i64,
        zeroing: Option<(&RiseMessageScope, RiseUnreadMarkKind)>,
    ) -> Vec<RiseUnreadMarksSnapshot> {
        let mut updated: Vec<RiseUnreadMarksSnapshot> = Vec::new();
        let chat_snapshots: Vec<RiseUnreadMarksSnapshot> =
            self.snapshots.iter().filter(|(scope, _)| scope.chat_id == chat_id).map(|(_, snapshot)| snapshot.clone()).collect();
        for snapshot in chat_snapshots {
            let scope = snapshot.scope.clone();
            let mut next = snapshot;
            let mention_hits = next.mention_message_ids.iter().filter(|id| mention_ids.contains(*id)).count() as i64;
            let reply_hits = next.reply_message_ids.iter().filter(|id| reply_ids.contains(*id)).count() as i64;
            let reaction_hits: i64 = next
                .reaction_marks
                .iter()
                .filter(|mark| reaction_ids.contains(&mark.message_id))
                .map(|mark| mark.count)
                .sum();
            let is_zeroed_scope = zeroing.is_some_and(|(zeroed, _)| *zeroed == scope);
            let touches_scope = mention_hits > 0
                || reply_hits > 0
                || reaction_hits > 0
                || is_zeroed_scope
                || (scope.topic_id.is_none() && (mentions_delta != 0 || replies_delta != 0 || reactions_delta != 0));
            if !touches_scope {
                continue;
            }
            next.mention_message_ids.retain(|id| !mention_ids.contains(id));
            next.reply_message_ids.retain(|id| !reply_ids.contains(id));
            next.reaction_marks.retain(|mark| !reaction_ids.contains(&mark.message_id));
            let zeroed_kind = zeroing.filter(|_| is_zeroed_scope).map(|(_, kind)| kind);
            if zeroed_kind == Some(RiseUnreadMarkKind::Mention) {
                next.mentions_count = 0;
            } else if mention_hits > 0 || scope.topic_id.is_none() {
                let delta = if mention_hits > 0 { -mention_hits } else { mentions_delta };
                next.mentions_count = (next.mentions_count + delta).max(0);
            }
            if zeroed_kind == Some(RiseUnreadMarkKind::Reply) {
                next.replies_count = 0;
            } else if reply_hits > 0 || scope.topic_id.is_none() {
                let delta = if reply_hits > 0 { -reply_hits } else { replies_delta };
                next.replies_count = (next.replies_count + delta).max(0);
            }
            if zeroed_kind == Some(RiseUnreadMarkKind::Reaction) {
                next.reactions_count = 0;
            } else if reaction_hits > 0 || scope.topic_id.is_none() {
                let delta = if reaction_hits > 0 { -reaction_hits } else { reactions_delta };
                next.reactions_count = (next.reactions_count + delta).max(0);
            }
            updated.push(self.store(next));
        }
        updated
    }

    fn note_seen(&mut self, message_ids: &[String]) {
        for id in message_ids.iter().filter(|id| !id.is_empty()) {
            self.seen_clock = self.seen_clock.wrapping_add(1);
            self.seen_rows.insert(id.clone(), self.seen_clock);
        }
        if self.seen_rows.len() <= RECENTLY_CONSUMED_LIMIT {
            return;
        }
        let overflow = self.seen_rows.len() - RECENTLY_CONSUMED_LIMIT;
        let mut by_age: Vec<(String, u64)> = self.seen_rows.iter().map(|(id, clock)| (id.clone(), *clock)).collect();
        by_age.sort_by_key(|(_, clock)| *clock);
        for (id, _) in by_age.into_iter().take(overflow) {
            self.seen_rows.remove(&id);
        }
    }

    fn consume_seen_replies(
        &mut self,
        ids: Vec<String>,
        scopes: &[RiseMessageScope],
        updated: Vec<RiseUnreadMarksSnapshot>,
    ) -> RiseUnreadMarksRealtimeOutcome {
        let candidates: Vec<RiseUnreadMarkCandidate> = ids
            .iter()
            .map(|id| RiseUnreadMarkCandidate {
                message_id: id.clone(),
                is_mentioned_you: false,
                has_unread_reactions: false,
                is_replied_you: false,
            })
            .collect();
        if let Some(scope) = scopes.iter().find(|scope| self.snapshots.contains_key(*scope)).cloned()
            && let Some(consumed) = self.consume(&scope, &candidates)
        {
            let replacements: HashMap<RiseMessageScope, RiseUnreadMarksSnapshot> =
                consumed.snapshots.into_iter().map(|snapshot| (snapshot.scope.clone(), snapshot)).collect();
            let snapshots =
                updated.into_iter().map(|snapshot| replacements.get(&snapshot.scope).cloned().unwrap_or(snapshot)).collect();
            return RiseUnreadMarksRealtimeOutcome {
                applies_to_chat_row: true,
                snapshots,
                consumption: Some(consumed.consumption),
            };
        }
        let no_ledger = scopes.iter().all(|scope| !self.snapshots.contains_key(scope));
        let Some(chat_scope) = scopes.last().filter(|_| no_ledger).cloned() else {
            return RiseUnreadMarksRealtimeOutcome { applies_to_chat_row: true, snapshots: updated, consumption: None };
        };
        for id in &ids {
            self.remember_consumed(id, RiseUnreadMarkKind::Reply);
        }
        RiseUnreadMarksRealtimeOutcome {
            applies_to_chat_row: true,
            snapshots: updated,
            consumption: Some(RiseUnreadMarksConsumption {
                scope: chat_scope,
                mention_ids: Vec::new(),
                reply_ids: ids,
                reaction_ids: Vec::new(),
                reactions_cleared: 0,
            }),
        }
    }

    fn consume(&mut self, scope: &RiseMessageScope, candidates: &[RiseUnreadMarkCandidate]) -> Option<RiseUnreadMarksConsumed> {
        if candidates.is_empty() {
            return None;
        }
        let current = self.snapshots.get(scope)?;
        let known_mentions: HashSet<&String> = current.mention_message_ids.iter().collect();
        let known_replies: HashSet<&String> = current.reply_message_ids.iter().collect();
        let mut known_reactions: HashMap<&String, i64> = HashMap::new();
        for mark in &current.reaction_marks {
            known_reactions.entry(&mark.message_id).or_insert(mark.count);
        }
        let mut mention_ids: Vec<String> = Vec::new();
        let mut reply_ids: Vec<String> = Vec::new();
        let mut reaction_ids: Vec<String> = Vec::new();
        let mut reactions_cleared = 0_i64;
        let mut mentions_beyond_window = (current.mentions_count - current.mention_message_ids.len() as i64).max(0);
        let mut replies_beyond_window = (current.replies_count - current.reply_message_ids.len() as i64).max(0);
        let mut reactions_beyond_window =
            (current.reactions_count - current.reaction_marks.iter().map(|mark| mark.count).sum::<i64>()).max(0);
        for candidate in candidates {
            let id = &candidate.message_id;
            if known_mentions.contains(id) {
                mention_ids.push(id.clone());
            } else if candidate.is_mentioned_you
                && !self.recently_consumed_mentions.contains_key(id)
                && mentions_beyond_window > 0
            {
                mention_ids.push(id.clone());
                mentions_beyond_window -= 1;
            }
            if known_replies.contains(id) {
                reply_ids.push(id.clone());
            } else if candidate.is_replied_you
                && !self.recently_consumed_replies.contains_key(id)
                && replies_beyond_window > 0
            {
                reply_ids.push(id.clone());
                replies_beyond_window -= 1;
            }
            if let Some(count) = known_reactions.get(id) {
                reaction_ids.push(id.clone());
                reactions_cleared += *count;
            } else if candidate.has_unread_reactions
                && !self.recently_consumed_reactions.contains_key(id)
                && reactions_beyond_window > 0
            {
                reaction_ids.push(id.clone());
                reactions_cleared += 1;
                reactions_beyond_window -= 1;
            }
        }
        if mention_ids.is_empty() && reply_ids.is_empty() && reaction_ids.is_empty() {
            return None;
        }
        for id in &mention_ids {
            self.remember_consumed(id, RiseUnreadMarkKind::Mention);
        }
        for id in &reply_ids {
            self.remember_consumed(id, RiseUnreadMarkKind::Reply);
        }
        for id in &reaction_ids {
            self.remember_consumed(id, RiseUnreadMarkKind::Reaction);
        }
        let snapshots = self.apply_local_removal(
            &scope.chat_id,
            &mention_ids.iter().cloned().collect(),
            &reply_ids.iter().cloned().collect(),
            &reaction_ids.iter().cloned().collect(),
            -(mention_ids.len() as i64),
            -(reply_ids.len() as i64),
            -reactions_cleared,
            None,
        );
        Some(RiseUnreadMarksConsumed {
            consumption: RiseUnreadMarksConsumption {
                scope: scope.clone(),
                mention_ids,
                reply_ids,
                reaction_ids,
                reactions_cleared,
            },
            snapshots,
        })
    }
}

struct RiseUnreadMarksLoadingGuard<'a> {
    state: &'a RiseIsolated<RiseUnreadMarksRepositoryState>,
    scope: &'a RiseMessageScope,
    load_id: u64,
}

impl Drop for RiseUnreadMarksLoadingGuard<'_> {
    fn drop(&mut self) {
        self.state.with(|state| {
            if state.loading.get(self.scope).is_some_and(|loading| loading.id == self.load_id) {
                state.loading.remove(self.scope);
            }
        });
    }
}

/// per scope unread mark ledger, memory only
/// the chat row carries the real counts across launches
pub struct RiseUnreadMarksRepository {
    rpc: RiseConversationRPC,
    _account_id: String,
    state: RiseIsolated<RiseUnreadMarksRepositoryState>,
}

impl RiseUnreadMarksRepository {
    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Arc<Self> {
        assert!(!account_id.is_empty());
        Arc::new(Self {
            rpc: RiseConversationRPC::new(engine),
            _account_id: account_id,
            state: RiseIsolated::new(RiseUnreadMarksRepositoryState::default()),
        })
    }

    pub async fn snapshot(&self, scope: &RiseMessageScope) -> Option<RiseUnreadMarksSnapshot> {
        self.state.with(|state| state.snapshots.get(scope).cloned())
    }

    pub async fn is_loaded(&self, scope: &RiseMessageScope) -> bool {
        self.state.with(|state| state.snapshots.get(scope).is_some_and(|snapshot| snapshot.is_loaded))
    }

    pub async fn is_loading(&self, scope: &RiseMessageScope) -> bool {
        self.state.with(|state| state.loading.contains_key(scope))
    }

    pub async fn is_assumed_empty(&self, scope: &RiseMessageScope) -> bool {
        self.state.with(|state| state.assumed_empty.contains(scope))
    }

    pub async fn release(&self, scope: &RiseMessageScope) {
        self.state.with(|state| {
            state.snapshots.remove(scope);
            state.loading.remove(scope);
            state.assumed_empty.remove(scope);
        });
    }

    pub async fn release_all(&self) {
        self.state.with(|state| {
            state.snapshots.clear();
            state.loading.clear();
            state.assumed_empty.clear();
            state.seen_rows.clear();
            state.recently_consumed_mentions.clear();
            state.recently_consumed_replies.clear();
            state.recently_consumed_reactions.clear();
        });
    }

    pub async fn note_seen(&self, message_ids: &[String]) {
        self.state.with(|state| state.note_seen(message_ids));
    }

    pub async fn install_empty(&self, scope: &RiseMessageScope) -> RiseUnreadMarksSnapshot {
        self.state.with(|state| {
            if let Some(existing) = state.snapshots.get(scope).filter(|existing| existing.is_loaded) {
                return existing.clone();
            }
            state.assumed_empty.insert(scope.clone());
            state.store(RiseUnreadMarksSnapshot::empty(scope.clone(), true))
        })
    }

    pub async fn load(&self, scope: &RiseMessageScope) -> anyhow::Result<Option<RiseUnreadMarksSnapshot>> {
        let load_id = self.state.with(|state| {
            if state.loading.contains_key(scope) {
                return None;
            }
            state.loading_clock = state.loading_clock.wrapping_add(1);
            let load_id = state.loading_clock;
            state.loading.insert(scope.clone(), LoadingState { id: load_id, changed_by_realtime: false });
            Some(load_id)
        });
        let Some(load_id) = load_id else {
            return Ok(None);
        };
        let _guard = RiseUnreadMarksLoadingGuard { state: &self.state, scope, load_id };
        if let Some(snapshot) = self.load_page(scope, load_id).await? {
            return Ok(Some(snapshot));
        }
        let should_refetch = self.state.with(|state| match state.loading.get_mut(scope) {
            Some(loading) if loading.id == load_id && loading.changed_by_realtime => {
                loading.changed_by_realtime = false;
                true
            }
            _ => false,
        });
        if !should_refetch {
            return Ok(None);
        }
        if let Some(snapshot) = self.load_page(scope, load_id).await? {
            return Ok(Some(snapshot));
        }
        Ok(self.state.with(|state| {
            if state.loading.get(scope).is_none_or(|loading| loading.id != load_id) {
                return None;
            }
            state.snapshots.get(scope).filter(|snapshot| snapshot.is_loaded).cloned()
        }))
    }

    async fn load_page(&self, scope: &RiseMessageScope, load_id: u64) -> anyhow::Result<Option<RiseUnreadMarksSnapshot>> {
        let requested_at = self.state.with(|state| state.consumption_clock);
        let response: GetUnreadMarksGatewayDTO = self
            .rpc
            .call(
                RiseConversationEndpoints::get_unread_marks(),
                rise_request::message::GetUnreadMarks {
                    chat_id: scope.chat_id.clone(),
                    limit: Some(*PAGE_LIMIT),
                    topic_id: scope.topic_id.clone(),
                },
            )
            .await?;
        Ok(self.state.with(|state| {
            match state.loading.get(scope) {
                Some(loading) if loading.id == load_id && !loading.changed_by_realtime => {}
                _ => return None,
            }
            let page_mentions = response.mentions.as_ref().map(|mentions| mentions.message_ids.clone()).unwrap_or_default();
            let page_mentions_count = page_mentions.len() as i64;
            let mentions: Vec<String> = page_mentions
                .into_iter()
                .filter(|id| !Self::page_may_predate(state.recently_consumed_mentions.get(id), requested_at))
                .collect();
            let page_replies = response.replies.as_ref().map(|replies| replies.message_ids.clone()).unwrap_or_default();
            let page_replies_count = page_replies.len() as i64;
            let replies: Vec<String> = page_replies
                .into_iter()
                .filter(|id| !Self::page_may_predate(state.recently_consumed_replies.get(id), requested_at))
                .collect();
            let page_reactions: Vec<RiseUnreadReactionMark> = response
                .reactions
                .as_ref()
                .map(|reactions| reactions.items.as_slice())
                .unwrap_or_default()
                .iter()
                .filter(|item| !item.message_id.is_empty())
                .map(|item| RiseUnreadReactionMark {
                    message_id: item.message_id.clone(),
                    count: item.count.max(1),
                    created_at: item.created_at as f64,
                })
                .collect();
            let (stale, reactions): (Vec<RiseUnreadReactionMark>, Vec<RiseUnreadReactionMark>) = page_reactions
                .into_iter()
                .partition(|mark| Self::page_may_predate(state.recently_consumed_reactions.get(&mark.message_id), requested_at));
            let stale_reactions: i64 = stale.iter().map(|mark| mark.count).sum();
            let stale_mentions = page_mentions_count - mentions.len() as i64;
            let stale_replies = page_replies_count - replies.len() as i64;
            let replies_total = response.replies.as_ref().map_or(replies.len() as i64, |replies| replies.count);
            let mentions_total = response.mentions.as_ref().map_or(mentions.len() as i64, |mentions| mentions.count);
            let reactions_total: i64 = response
                .reactions
                .as_ref()
                .map_or_else(|| reactions.iter().map(|mark| mark.count).sum(), |reactions| reactions.count);
            let snapshot = RiseUnreadMarksSnapshot {
                scope: scope.clone(),
                mention_message_ids: mentions,
                mentions_count: (mentions_total - stale_mentions).max(0),
                reply_message_ids: replies,
                replies_count: (replies_total - stale_replies).max(0),
                reaction_marks: reactions,
                reactions_count: (reactions_total - stale_reactions).max(0),
                is_loaded: true,
                revision: 0,
            };
            state.assumed_empty.remove(scope);
            Some(state.store(snapshot))
        }))
    }

    /// a mark consumed here shows in a page only as a leftover
    /// unconfirmed or confirmed after the page was requested, else it is a new mark
    fn page_may_predate(consumed: Option<&ConsumedMark>, requested_at: u64) -> bool {
        let Some(consumed) = consumed else {
            return false;
        };
        let Some(confirmed_at) = consumed.confirmed_at else {
            return true;
        };
        confirmed_at > requested_at
    }

    pub async fn next_mention(&self, scope: &RiseMessageScope) -> Option<String> {
        self.state.with(|state| state.snapshots.get(scope).and_then(|snapshot| snapshot.mention_message_ids.first().cloned()))
    }

    pub async fn next_reply(&self, scope: &RiseMessageScope) -> Option<String> {
        self.state.with(|state| state.snapshots.get(scope).and_then(|snapshot| snapshot.reply_message_ids.first().cloned()))
    }

    pub async fn next_reaction(&self, scope: &RiseMessageScope) -> Option<String> {
        self.state
            .with(|state| state.snapshots.get(scope).and_then(|snapshot| snapshot.reaction_marks.first().map(|mark| mark.message_id.clone())))
    }

    /// optimistic half of rows on screen
    /// drops them from every loaded ledger of the chat and returns what the server must be told
    pub async fn consume(
        &self,
        scope: &RiseMessageScope,
        candidates: impl AsRef<[RiseUnreadMarkCandidate]>,
    ) -> Option<RiseUnreadMarksConsumed> {
        self.state.with(|state| state.consume(scope, candidates.as_ref()))
    }

    pub async fn invalidate_consumption(&self, consumption: RiseUnreadMarksConsumption) {
        self.state.with(|state| {
            for id in &consumption.mention_ids {
                state.recently_consumed_mentions.remove(id);
            }
            for id in &consumption.reply_ids {
                state.recently_consumed_replies.remove(id);
            }
            for id in &consumption.reaction_ids {
                state.recently_consumed_reactions.remove(id);
            }
        });
    }

    pub async fn confirm_consumption(&self, consumption: &RiseUnreadMarksConsumption) -> anyhow::Result<()> {
        let mentioned_read_ids = consumption.mentioned_read_ids();
        if !mentioned_read_ids.is_empty() {
            let response: MarkMessageAsMentionedReadGatewayDTO = self
                .rpc
                .call(
                    RiseConversationEndpoints::mark_message_as_mentioned_read(),
                    rise_request::message::MarkMessageAsMentionedRead { message_ids: Some(mentioned_read_ids) },
                )
                .await?;
            if !response.success {
                return Err(RiseChatRepositoryError::Rejected(response.error).into());
            }
        }
        if !consumption.reaction_ids.is_empty() {
            let response: MarkReactionsAsReadGatewayDTO = self
                .rpc
                .call(
                    RiseConversationEndpoints::mark_reactions_as_read(),
                    rise_request::message::MarkReactionsAsRead { message_ids: Some(consumption.reaction_ids.clone()) },
                )
                .await?;
            if !response.success {
                return Err(RiseChatRepositoryError::Rejected(response.error).into());
            }
        }
        self.state.with(|state| {
            state.mark_confirmed(&consumption.mention_ids, &consumption.reply_ids, &consumption.reaction_ids)
        });
        Ok(())
    }

    pub async fn clear_all(&self, scope: &RiseMessageScope, kind: RiseUnreadMarkKind) -> Option<RiseUnreadMarksCleared> {
        self.state.with(|state| {
            let current = state.snapshots.get(scope)?.clone();
            match kind {
                RiseUnreadMarkKind::Mention => {
                    if current.mentions_count <= 0 {
                        return None;
                    }
                    for id in &current.mention_message_ids {
                        state.remember_consumed(id, RiseUnreadMarkKind::Mention);
                    }
                    let snapshots = state.apply_local_removal(
                        &scope.chat_id,
                        &current.mention_message_ids.iter().cloned().collect(),
                        &HashSet::new(),
                        &HashSet::new(),
                        -current.mentions_count,
                        0,
                        0,
                        Some((scope, kind)),
                    );
                    Some(RiseUnreadMarksCleared {
                        removed: current.mentions_count,
                        message_ids: current.mention_message_ids,
                        snapshots,
                    })
                }
                RiseUnreadMarkKind::Reply => {
                    if current.replies_count <= 0 {
                        return None;
                    }
                    for id in &current.reply_message_ids {
                        state.remember_consumed(id, RiseUnreadMarkKind::Reply);
                    }
                    let snapshots = state.apply_local_removal(
                        &scope.chat_id,
                        &HashSet::new(),
                        &current.reply_message_ids.iter().cloned().collect(),
                        &HashSet::new(),
                        0,
                        -current.replies_count,
                        0,
                        Some((scope, kind)),
                    );
                    Some(RiseUnreadMarksCleared {
                        removed: current.replies_count,
                        message_ids: current.reply_message_ids,
                        snapshots,
                    })
                }
                RiseUnreadMarkKind::Reaction => {
                    if current.reactions_count <= 0 {
                        return None;
                    }
                    for mark in &current.reaction_marks {
                        state.remember_consumed(&mark.message_id, RiseUnreadMarkKind::Reaction);
                    }
                    let message_ids: Vec<String> = current.reaction_marks.iter().map(|mark| mark.message_id.clone()).collect();
                    let snapshots = state.apply_local_removal(
                        &scope.chat_id,
                        &HashSet::new(),
                        &HashSet::new(),
                        &message_ids.iter().cloned().collect(),
                        0,
                        0,
                        -current.reactions_count,
                        Some((scope, kind)),
                    );
                    Some(RiseUnreadMarksCleared { removed: current.reactions_count, message_ids, snapshots })
                }
            }
        })
    }

    pub async fn confirm_clear_all(
        &self,
        scope: &RiseMessageScope,
        kind: RiseUnreadMarkKind,
        message_ids: Vec<String>,
    ) -> anyhow::Result<()> {
        let response: ClearUnreadMarksGatewayDTO = self
            .rpc
            .call(
                RiseConversationEndpoints::clear_unread_marks(),
                rise_request::message::ClearUnreadMarks {
                    chat_id: scope.chat_id.clone(),
                    mentions: Some(kind == RiseUnreadMarkKind::Mention),
                    reactions: Some(kind == RiseUnreadMarkKind::Reaction),
                    replies: Some(kind == RiseUnreadMarkKind::Reply),
                    topic_id: scope.topic_id.clone(),
                },
            )
            .await?;
        if !response.success {
            return Err(RiseChatRepositoryError::Rejected(response.error).into());
        }
        self.state.with(|state| match kind {
            RiseUnreadMarkKind::Mention => state.mark_confirmed(&message_ids, &[], &[]),
            RiseUnreadMarkKind::Reply => state.mark_confirmed(&[], &message_ids, &[]),
            RiseUnreadMarkKind::Reaction => state.mark_confirmed(&[], &[], &message_ids),
        });
        Ok(())
    }

    pub async fn drop_all(&self, chat_id: &str) -> Vec<RiseUnreadMarksSnapshot> {
        self.state.with(|state| {
            let matching: Vec<RiseUnreadMarksSnapshot> = state
                .snapshots
                .iter()
                .filter(|(scope, snapshot)| scope.chat_id == chat_id && !snapshot.is_empty())
                .map(|(_, snapshot)| snapshot.clone())
                .collect();
            let mut updated: Vec<RiseUnreadMarksSnapshot> = Vec::new();
            for snapshot in matching {
                let mut next = snapshot;
                for id in &next.mention_message_ids {
                    state.remember_consumed(id, RiseUnreadMarkKind::Mention);
                }
                for id in &next.reply_message_ids {
                    state.remember_consumed(id, RiseUnreadMarkKind::Reply);
                }
                for mark in &next.reaction_marks {
                    state.remember_consumed(&mark.message_id, RiseUnreadMarkKind::Reaction);
                }
                let reaction_ids: Vec<String> = next.reaction_marks.iter().map(|mark| mark.message_id.clone()).collect();
                state.mark_confirmed(&next.mention_message_ids, &next.reply_message_ids, &reaction_ids);
                next.mention_message_ids = Vec::new();
                next.mentions_count = 0;
                next.reply_message_ids = Vec::new();
                next.replies_count = 0;
                next.reaction_marks = Vec::new();
                next.reactions_count = 0;
                updated.push(state.store(next));
            }
            updated
        })
    }

    pub async fn apply_realtime(
        &self,
        change: &RiseUnreadMarksRealtimeChange,
        visible_scope: Option<RiseMessageScope>,
        visible_candidates: Vec<RiseUnreadMarkCandidate>,
    ) -> RiseUnreadMarksRealtimeOutcome {
        self.state.with(|state| {
            let consumed_ledger = match change.kind {
                RiseUnreadMarkKind::Mention => &state.recently_consumed_mentions,
                RiseUnreadMarkKind::Reply => &state.recently_consumed_replies,
                RiseUnreadMarkKind::Reaction => &state.recently_consumed_reactions,
            };
            if change.change == REALTIME_CHANGE_READ
                && !change.message_ids.is_empty()
                && change.message_ids.iter().any(|id| consumed_ledger.contains_key(id))
            {
                return RiseUnreadMarksRealtimeOutcome { applies_to_chat_row: false, snapshots: Vec::new(), consumption: None };
            }
            if let Some(visible_scope) = visible_scope.as_ref()
                && !state.snapshots.contains_key(visible_scope)
                && visible_candidates.iter().any(|candidate| change.message_ids.contains(&candidate.message_id))
            {
                state.snapshots.insert(visible_scope.clone(), RiseUnreadMarksSnapshot::empty(visible_scope.clone(), false));
            }
            let mut updated: Vec<RiseUnreadMarksSnapshot> = Vec::new();
            let topic_scope =
                RiseMessageScope::new(change.chat_id.clone(), Some(change.topic_id.clone().unwrap_or_default()), None);
            let chat_scope = RiseMessageScope::new(change.chat_id.clone(), None, None);
            let scopes = [topic_scope, chat_scope];
            for scope in &scopes {
                if let Some(loading) = state.loading.get_mut(scope) {
                    loading.changed_by_realtime = true;
                }
            }
            for scope in &scopes {
                let Some(mut snapshot) = state.snapshots.get(scope).cloned() else {
                    continue;
                };
                let ids: HashSet<&String> = change.message_ids.iter().collect();
                match (change.kind, change.change.as_str()) {
                    (RiseUnreadMarkKind::Mention, REALTIME_CHANGE_ADDED) => {
                        for id in &change.message_ids {
                            if !snapshot.mention_message_ids.contains(id) {
                                snapshot.mention_message_ids.push(id.clone());
                            }
                        }
                        snapshot.mentions_count = (snapshot.mentions_count + change.delta.max(0)).max(0);
                    }
                    (RiseUnreadMarkKind::Mention, _) => {
                        snapshot.mention_message_ids.retain(|id| !ids.contains(id));
                        snapshot.mentions_count = (snapshot.mentions_count + change.delta.min(0)).max(0);
                    }
                    (RiseUnreadMarkKind::Reply, REALTIME_CHANGE_ADDED) => {
                        for id in &change.message_ids {
                            if !snapshot.reply_message_ids.contains(id) {
                                snapshot.reply_message_ids.push(id.clone());
                            }
                        }
                        snapshot.replies_count = (snapshot.replies_count + change.delta.max(0)).max(0);
                    }
                    (RiseUnreadMarkKind::Reply, _) => {
                        snapshot.reply_message_ids.retain(|id| !ids.contains(id));
                        snapshot.replies_count = (snapshot.replies_count + change.delta.min(0)).max(0);
                    }
                    (RiseUnreadMarkKind::Reaction, REALTIME_CHANGE_ADDED) => {
                        for id in &change.message_ids {
                            if let Some(mark) = snapshot.reaction_marks.iter_mut().find(|mark| mark.message_id == *id) {
                                mark.count += change.delta.max(1);
                            } else {
                                snapshot.reaction_marks.push(RiseUnreadReactionMark {
                                    message_id: id.clone(),
                                    count: change.delta.max(1),
                                    created_at: change.timestamp,
                                });
                            }
                        }
                        snapshot.reactions_count = (snapshot.reactions_count + change.delta.max(0)).max(0);
                    }
                    (RiseUnreadMarkKind::Reaction, _) => {
                        let only_index = (change.change == REALTIME_CHANGE_REMOVED && ids.len() == 1)
                            .then(|| ids.iter().next().copied())
                            .flatten()
                            .and_then(|id| snapshot.reaction_marks.iter().position(|mark| mark.message_id == *id));
                        if let Some(index) = only_index {
                            snapshot.reaction_marks[index].count += change.delta.min(0);
                            if snapshot.reaction_marks[index].count <= 0 {
                                snapshot.reaction_marks.remove(index);
                            }
                        } else {
                            snapshot.reaction_marks.retain(|mark| !ids.contains(&mark.message_id));
                        }
                        snapshot.reactions_count = (snapshot.reactions_count + change.delta.min(0)).max(0);
                    }
                }
                updated.push(state.store(snapshot));
            }
            if change.change == REALTIME_CHANGE_ADDED
                && let Some(visible_scope) = visible_scope.as_ref()
            {
                let changed_ids: HashSet<&String> = change.message_ids.iter().collect();
                let candidates: Vec<RiseUnreadMarkCandidate> = visible_candidates
                    .iter()
                    .filter(|candidate| changed_ids.contains(&candidate.message_id))
                    .map(|candidate| RiseUnreadMarkCandidate {
                        message_id: candidate.message_id.clone(),
                        is_mentioned_you: false,
                        has_unread_reactions: false,
                        is_replied_you: false,
                    })
                    .collect();
                if let Some(consumed) = state.consume(visible_scope, &candidates) {
                    let replacements: HashMap<RiseMessageScope, RiseUnreadMarksSnapshot> =
                        consumed.snapshots.into_iter().map(|snapshot| (snapshot.scope.clone(), snapshot)).collect();
                    let snapshots = updated
                        .into_iter()
                        .map(|snapshot| replacements.get(&snapshot.scope).cloned().unwrap_or(snapshot))
                        .collect();
                    return RiseUnreadMarksRealtimeOutcome {
                        applies_to_chat_row: true,
                        snapshots,
                        consumption: Some(consumed.consumption),
                    };
                }
            }
            if change.kind == RiseUnreadMarkKind::Reply && change.change == REALTIME_CHANGE_ADDED {
                let seen: Vec<String> = change
                    .message_ids
                    .iter()
                    .filter(|id| state.seen_rows.contains_key(*id) && !state.recently_consumed_replies.contains_key(*id))
                    .cloned()
                    .collect();
                if !seen.is_empty() {
                    return state.consume_seen_replies(seen, &scopes, updated);
                }
            }
            RiseUnreadMarksRealtimeOutcome { applies_to_chat_row: true, snapshots: updated, consumption: None }
        })
    }
}
