use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use rise_canon::{RiseLimits, rise_reply, rise_request, rise_response, rise_view};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::NSRange;
use rise_engine::RiseEngine;

pub use crate::modules::chat::components::chat_search::chat_search_overlay_store::ChatSearchMessageKind;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationAuxiliaryLoadState, RiseConversationAuxiliaryRepositoryError,
};
use crate::modules::chat::engine::core::rise_conversation_rpc::{RiseConversationEndpoints, RiseConversationRPC};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    GetMessagesListGatewayDTO, MessageSearchChatWireDTO, MessageSearchMatchWireDTO, MessageSearchRangeWireDTO,
    MessageWireDTO,
};

const SEARCH_BACKEND_UNAVAILABLE_MARKER: &str = "elasticsearch_unavailable";

/// chat kind filter of the global message search
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatSearchChatKindFilter {
    All,
    Direct,
    Group,
    Channel,
}

impl ChatSearchChatKindFilter {
    pub const ALL_CASES: [Self; 4] = [Self::All, Self::Direct, Self::Group, Self::Channel];

    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Direct => "direct",
            Self::Group => "group",
            Self::Channel => "channel",
        }
    }

    /// wire value of chat_type, absent for all which is the server default
    pub const fn wire_value(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            _ => Some(self.raw_value()),
        }
    }

    pub const fn title_key(self) -> &'static str {
        match self {
            Self::All => "chat_search_filter_all_chats",
            Self::Direct => "chat_search_filter_direct",
            Self::Group => "chat_search_filter_groups",
            Self::Channel => "chat_search_filter_channels",
        }
    }
}

/// one message search, a normalized query inside one chat
/// or across every chat of one kind when chat_id is none
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseMessageSearchScope {
    pub chat_id: Option<String>,
    pub query: String,
    pub chat_kind: ChatSearchChatKindFilter,
}

impl RiseMessageSearchScope {
    pub fn new(chat_id: &str, query: &str) -> Option<Self> {
        let chat = trimming_whitespaces_and_newlines(chat_id);
        let normalized = trimming_whitespaces_and_newlines(query).to_lowercase();
        if chat.is_empty() || normalized.is_empty() {
            return None;
        }
        Some(Self { chat_id: Some(chat.to_owned()), query: normalized, chat_kind: ChatSearchChatKindFilter::All })
    }

    pub fn global_query(query: &str, chat_kind: ChatSearchChatKindFilter) -> Option<Self> {
        let normalized = trimming_whitespaces_and_newlines(query).to_lowercase();
        if normalized.is_empty() {
            return None;
        }
        Some(Self { chat_id: None, query: normalized, chat_kind })
    }

    pub fn is_global(&self) -> bool {
        self.chat_id.is_none()
    }

    /// one live session per surface, a chat or the global section per kind
    /// a new query on the same surface supersedes the old
    pub fn session_key(&self) -> String {
        match &self.chat_id {
            Some(chat_id) => chat_id.clone(),
            None => format!("global:{}", self.chat_kind.raw_value()),
        }
    }
}

/// a search hit as the server described it, the message plus matched words in utf16 units
#[derive(Clone, Debug, PartialEq)]
pub struct RiseMessageSearchHit {
    pub id: String,
    pub message: GetMessageMessage,
    pub highlight_ranges: Vec<NSRange>,
    pub snippet: String,
    pub snippet_ranges: Vec<NSRange>,
    pub chat: Option<ChatInfo>,
}

#[derive(Clone, Debug)]
pub struct RiseMessageSearchPresentationSnapshot {
    pub account_id: String,
    pub scope: RiseMessageSearchScope,
    pub hits: Vec<RiseMessageSearchHit>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub has_newer: bool,
    pub total_count: Option<i64>,
    pub load_state: RiseConversationAuxiliaryLoadState,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RiseMessageSearchPresentationState {
    pub hits: Vec<RiseMessageSearchHit>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub has_newer: bool,
    pub total_count: Option<i64>,
    pub load_state: RiseConversationAuxiliaryLoadState,
    pub error: Option<String>,
    pub revision: u64,
}

impl RiseMessageSearchPresentationState {
    pub fn empty() -> Self {
        Self {
            hits: Vec::new(),
            cursor: None,
            has_more: false,
            has_newer: false,
            total_count: None,
            load_state: RiseConversationAuxiliaryLoadState::Idle,
            error: None,
            revision: 0,
        }
    }

    pub fn did_load(&self) -> bool {
        matches!(self.load_state, RiseConversationAuxiliaryLoadState::Loaded | RiseConversationAuxiliaryLoadState::Failed)
    }

    pub fn is_loading(&self) -> bool {
        self.load_state == RiseConversationAuxiliaryLoadState::Loading
    }
}

#[derive(Clone, Debug, Default)]
pub struct RiseMessageSearchPresentationBox {
    pub value: Option<RiseMessageSearchPresentationState>,
}

static PAGE_LIMIT: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("chat_message_search_page_limit"));
static GLOBAL_PAGE_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("chat_search_overlay_store_messages_limit"));
static GLOBAL_WINDOW_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("chat_search_overlay_store_messages_window_limit"));

pub enum ChatMessageSearchPolicy {}

impl ChatMessageSearchPolicy {
    pub const PREFETCH_DISTANCE: i64 = 3;

    pub fn page_limit() -> i64 {
        *PAGE_LIMIT
    }

    pub fn global_page_limit() -> i64 {
        *GLOBAL_PAGE_LIMIT
    }

    pub fn global_window_limit() -> i64 {
        *GLOBAL_WINDOW_LIMIT
    }

    pub fn should_load_more(current_index: i64, loaded_count: i64, has_more: bool, is_loading: bool) -> bool {
        has_more && !is_loading && loaded_count > 0 && current_index >= loaded_count - 1 - Self::PREFETCH_DISTANCE
    }

    pub fn should_load_newer(visible_index: i64, has_newer: bool, is_loading: bool) -> bool {
        has_newer && !is_loading && visible_index <= Self::PREFETCH_DISTANCE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseMessageSearchError {
    #[error("A newer query replaced this message search")]
    StaleResponse,
}

struct Session {
    hits: Vec<RiseMessageSearchHit>,
    cursor: Option<String>,
    has_more: bool,
    has_newer: bool,
    top_anchor_id: Option<String>,
    total_count: Option<i64>,
    generation: u64,
    is_loading: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    First,
    Older,
    Newer,
}

#[derive(Default)]
struct RiseMessageSearchRepositoryState {
    sessions: HashMap<RiseMessageSearchScope, Session>,
    order: Vec<RiseMessageSearchScope>,
}

struct RiseMessageSearchFetchGuard<'a> {
    repository: &'a RiseMessageSearchRepository,
    scope: &'a RiseMessageSearchScope,
    direction: Direction,
    generation: u64,
    is_armed: bool,
}

impl Drop for RiseMessageSearchFetchGuard<'_> {
    fn drop(&mut self) {
        if !self.is_armed {
            return;
        }
        let mut state = self.repository.state.lock();
        let state = &mut *state;
        let Some(session) = state.sessions.get_mut(self.scope).filter(|session| session.generation == self.generation)
        else {
            return;
        };
        if self.direction == Direction::First {
            state.sessions.remove(self.scope);
            state.order.retain(|scope| scope != self.scope);
        } else {
            session.is_loading = false;
        }
    }
}

/// memory only search sessions, one per scope
/// hits newest first as pages arrive, a new query replaces the session
/// the global section keeps a bounded window
pub struct RiseMessageSearchRepository {
    rpc: RiseConversationRPC,
    account_id: String,
    state: Mutex<RiseMessageSearchRepositoryState>,
}

impl RiseMessageSearchRepository {
    const SESSION_LIMIT: usize = 8;

    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Arc<Self> {
        Arc::new(Self {
            rpc: RiseConversationRPC::new(engine),
            account_id,
            state: Mutex::new(RiseMessageSearchRepositoryState::default()),
        })
    }

    pub async fn cached(&self, scope: &RiseMessageSearchScope) -> Option<RiseMessageSearchPresentationSnapshot> {
        let state = self.state.lock();
        let session = state.sessions.get(scope)?;
        Some(self.snapshot(scope, session, RiseConversationAuxiliaryLoadState::Loaded, None))
    }

    /// first page, any page still in flight for the scope is superseded
    pub async fn search(
        &self,
        scope: &RiseMessageSearchScope,
        user_id: &str,
    ) -> anyhow::Result<RiseMessageSearchPresentationSnapshot> {
        let generation = {
            let mut state = self.state.lock();
            let generation = state.sessions.get(scope).map_or(0, |session| session.generation).wrapping_add(1);
            state.sessions.insert(
                scope.clone(),
                Session {
                    hits: Vec::new(),
                    cursor: None,
                    has_more: false,
                    has_newer: false,
                    top_anchor_id: None,
                    total_count: None,
                    generation,
                    is_loading: true,
                },
            );
            Self::touch(&mut state, scope);
            generation
        };
        self.fetch(scope, user_id, Direction::First, generation).await
    }

    /// next page toward older hits, no op when nothing is left or a page is in flight
    pub async fn load_more(
        &self,
        scope: &RiseMessageSearchScope,
        user_id: &str,
    ) -> anyhow::Result<Option<RiseMessageSearchPresentationSnapshot>> {
        let generation = {
            let mut state = self.state.lock();
            let Some(session) = state.sessions.get_mut(scope) else {
                return Ok(None);
            };
            if !session.has_more || session.is_loading || session.cursor.is_none() {
                return Ok(None);
            }
            session.is_loading = true;
            session.generation
        };
        self.fetch(scope, user_id, Direction::Older, generation).await.map(Some)
    }

    pub async fn load_newer(
        &self,
        scope: &RiseMessageSearchScope,
        user_id: &str,
    ) -> anyhow::Result<Option<RiseMessageSearchPresentationSnapshot>> {
        let generation = {
            let mut state = self.state.lock();
            let Some(session) = state.sessions.get_mut(scope) else {
                return Ok(None);
            };
            if !session.has_newer || session.is_loading || session.hits.is_empty() {
                return Ok(None);
            }
            session.is_loading = true;
            session.generation
        };
        self.fetch(scope, user_id, Direction::Newer, generation).await.map(Some)
    }

    pub async fn clear(&self, scope: &RiseMessageSearchScope) {
        let mut state = self.state.lock();
        state.sessions.remove(scope);
        state.order.retain(|known| known != scope);
    }

    pub async fn clear_all(&self) {
        let mut state = self.state.lock();
        state.sessions = HashMap::new();
        state.order = Vec::new();
    }

    async fn fetch(
        &self,
        scope: &RiseMessageSearchScope,
        _user_id: &str,
        direction: Direction,
        generation: u64,
    ) -> anyhow::Result<RiseMessageSearchPresentationSnapshot> {
        let mut guard = RiseMessageSearchFetchGuard { repository: self, scope, direction, generation, is_armed: true };
        let relative_id = {
            let state = self.state.lock();
            let current = state.sessions.get(scope);
            match direction {
                Direction::First => None,
                Direction::Older => current.and_then(|session| session.cursor.clone()),
                Direction::Newer => current.and_then(|session| session.hits.first().map(|hit| hit.id.clone())),
            }
        };
        let limit = if scope.is_global() {
            ChatMessageSearchPolicy::global_page_limit()
        } else {
            ChatMessageSearchPolicy::page_limit()
        };
        let response = if let Some(chat_id) = &scope.chat_id {
            let found: rise_view::message::Found = self
                .rpc
                .call(
                    RiseConversationEndpoints::search_chat_messages(),
                    rise_request::message::SearchMessages {
                        chat_id: chat_id.clone(),
                        content_type: None,
                        limit: Some(limit),
                        query: scope.query.clone(),
                        relative_id: relative_id.clone(),
                        up: Some(direction == Direction::Newer),
                    },
                )
                .await?;
            GetMessagesListGatewayDTO::from_canon(&found)?
        } else {
            let found: rise_response::message::SearchMessagesGlobal = self
                .rpc
                .call(
                    RiseConversationEndpoints::search_messages(),
                    rise_request::message::SearchMessagesGlobal {
                        chat_type: scope.chat_kind.wire_value().map(str::to_owned),
                        content_type: None,
                        limit: Some(limit),
                        query: scope.query.clone(),
                        relative_id: relative_id.clone(),
                        up: Some(direction == Direction::Newer),
                    },
                )
                .await?;
            let response = GetMessagesListGatewayDTO::from_canon_found_everywhere_with_chats(&found)?;
            if response.error.as_deref() == Some(SEARCH_BACKEND_UNAVAILABLE_MARKER) {
                return Err(RiseConversationAuxiliaryRepositoryError::SearchBackendUnavailable.into());
            }
            response
        };
        let incoming = Self::hits(&response);
        let server_has_more = response.is_have_more.unwrap_or(false);
        let snapshot = {
            let mut state = self.state.lock();
            let Some(session) = state.sessions.get_mut(scope).filter(|session| session.generation == generation) else {
                return Err(RiseMessageSearchError::StaleResponse.into());
            };
            match direction {
                Direction::First | Direction::Older => {
                    let first_incoming_id = incoming.first().map(|hit| hit.id.clone());
                    let last_incoming_id = incoming.last().map(|hit| hit.id.clone());
                    let is_incoming_empty = incoming.is_empty();
                    let mut seen: HashSet<String> = session.hits.iter().map(|hit| hit.id.clone()).collect();
                    for hit in incoming {
                        if seen.insert(hit.id.clone()) {
                            session.hits.push(hit);
                        }
                    }
                    if direction == Direction::First {
                        session.top_anchor_id = first_incoming_id;
                        session.has_newer = false;
                    }
                    let cursor = Self::normalized(response.relative_id.as_deref()).or(last_incoming_id);
                    let cursor_advanced = cursor.is_some() && cursor != Self::normalized(relative_id.as_deref());
                    session.cursor = cursor;
                    session.has_more = server_has_more && !is_incoming_empty && cursor_advanced;
                }
                Direction::Newer => {
                    let existing: HashSet<&str> = session.hits.iter().map(|hit| hit.id.as_str()).collect();
                    let fresh: Vec<RiseMessageSearchHit> =
                        incoming.into_iter().filter(|hit| !existing.contains(hit.id.as_str())).collect();
                    let is_fresh_empty = fresh.is_empty();
                    session.hits.splice(0..0, fresh);
                    let reached_top = is_fresh_empty
                        || !server_has_more
                        || (session.top_anchor_id.is_some()
                            && session.hits.first().map(|hit| &hit.id) == session.top_anchor_id.as_ref());
                    session.has_newer = !reached_top;
                }
            }
            if scope.is_global() {
                Self::trim_window(session, direction);
            }
            if let Some(total) = response.total_count {
                session.total_count = Some(total);
            }
            session.is_loading = false;
            self.snapshot(scope, session, RiseConversationAuxiliaryLoadState::Loaded, None)
        };
        guard.is_armed = false;
        Ok(snapshot)
    }

    fn trim_window(session: &mut Session, direction: Direction) {
        let window = ChatMessageSearchPolicy::global_window_limit();
        let overflow = session.hits.len() as i64 - window;
        if window <= 0 || overflow <= 0 {
            return;
        }
        match direction {
            Direction::First | Direction::Older => {
                session.hits.drain(..overflow as usize);
                session.has_newer = true;
            }
            Direction::Newer => {
                session.hits.truncate(window as usize);
                session.has_more = true;
                session.cursor = session.hits.last().map(|hit| hit.id.clone());
            }
        }
    }

    fn snapshot(
        &self,
        scope: &RiseMessageSearchScope,
        session: &Session,
        load_state: RiseConversationAuxiliaryLoadState,
        error: Option<String>,
    ) -> RiseMessageSearchPresentationSnapshot {
        RiseMessageSearchPresentationSnapshot {
            account_id: self.account_id.clone(),
            scope: scope.clone(),
            hits: session.hits.clone(),
            cursor: session.cursor.clone(),
            has_more: session.has_more,
            has_newer: session.has_newer,
            total_count: session.total_count,
            load_state,
            error,
        }
    }

    fn touch(state: &mut RiseMessageSearchRepositoryState, scope: &RiseMessageSearchScope) {
        state.order.retain(|known| known != scope);
        state.order.push(scope.clone());
        while state.order.len() > Self::SESSION_LIMIT {
            let oldest = state.order.remove(0);
            state.sessions.remove(&oldest);
        }
    }

    pub fn hits(response: &GetMessagesListGatewayDTO) -> Vec<RiseMessageSearchHit> {
        let empty_wires: &[MessageWireDTO] = &[];
        let wires = response.messages.as_deref().unwrap_or(empty_wires);
        let mut matches_by_id: HashMap<String, &MessageSearchMatchWireDTO> = HashMap::new();
        for found_match in response.matches.as_deref().unwrap_or_default() {
            if let Some(id) = Self::normalized(found_match.message_id.as_deref()) {
                matches_by_id.insert(id, found_match);
            }
        }
        let mut chats: HashMap<String, ChatInfo> = HashMap::new();
        for wire in response.chats.as_deref().unwrap_or_default() {
            if let Some(chat) = Self::search_chat(wire) {
                chats.insert(chat.id.clone(), chat);
            }
        }
        let mut hits = Vec::with_capacity(wires.len());
        for wire in wires {
            let message = wire.to_message();
            let server_id = Self::normalized(wire.server_id.as_deref())
                .or_else(|| Self::normalized(wire.id.as_deref()))
                .unwrap_or_else(|| message.sai_server_id());
            if server_id.is_empty() {
                continue;
            }
            let found_match = matches_by_id.get(&server_id).or_else(|| matches_by_id.get(&message.id)).copied();
            let has_json_payload = message.poll.is_some() || message.location.is_some();
            let text_snippet = if has_json_payload { None } else { found_match };
            hits.push(RiseMessageSearchHit {
                highlight_ranges: Self::ranges(found_match.and_then(|found| found.ranges.as_deref())),
                snippet: if has_json_payload {
                    String::new()
                } else {
                    text_snippet.and_then(|found| found.snippet.clone()).unwrap_or_else(|| message.text.clone())
                },
                snippet_ranges: Self::ranges(text_snippet.and_then(|found| found.snippet_ranges.as_deref())),
                chat: chats.get(&message.chat_id).cloned(),
                id: server_id,
                message,
            });
        }
        hits
    }

    fn search_chat(wire: &MessageSearchChatWireDTO) -> Option<ChatInfo> {
        let id = Self::normalized(wire.id.as_deref())?;
        let chat_type = wire.r#type.as_deref().and_then(ChatType::matching)?;
        if !chat_type.is_community() {
            return None;
        }
        Some(ChatInfo {
            id,
            title: wire.title.clone().unwrap_or_default(),
            r#type: chat_type,
            logo_url: Self::normalized(wire.logo_url.as_deref()),
            username: Self::normalized(wire.username.as_deref()),
            is_verified: wire.is_verified.unwrap_or(false),
            is_public: wire.is_public.unwrap_or(false),
            ..ChatInfo::default()
        })
    }

    fn ranges(wires: Option<&[MessageSearchRangeWireDTO]>) -> Vec<NSRange> {
        wires
            .unwrap_or_default()
            .iter()
            .filter(|range| range.end > range.start && range.start >= 0)
            .map(|range| NSRange::new(range.start as usize, (range.end - range.start) as usize))
            .collect()
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        if value.is_empty() { None } else { Some(value.to_owned()) }
    }
}

impl GetMessagesListGatewayDTO {
    pub fn from_canon_found_everywhere_with_chats(
        found: &rise_reply::message::FoundEverywhereWithChats,
    ) -> anyhow::Result<Self> {
        let messages =
            found.messages.iter().map(MessageWireDTO::from_canon).collect::<anyhow::Result<Vec<MessageWireDTO>>>()?;
        Ok(Self {
            success: None,
            error: found.error.clone(),
            messages: Some(messages),
            is_have_more: found.is_have_more,
            relative_id: found.relative_id.clone(),
            matches: Some(found.matches.iter().map(MessageSearchMatchWireDTO::from_canon).collect()),
            total_count: found.total_count,
            chats: found.chats.as_ref().map(|chats| chats.iter().map(MessageSearchChatWireDTO::from_canon).collect()),
        })
    }
}

impl MessageSearchChatWireDTO {
    pub fn from_canon(chat: &rise_view::message::HitChat) -> Self {
        Self {
            id: Some(chat.id.clone()),
            title: Some(chat.title.clone()),
            r#type: Some(chat.r#type.clone()),
            logo_url: Some(chat.logo_url.clone()),
            username: Some(chat.username.clone()),
            is_public: Some(chat.is_public),
            is_verified: Some(chat.is_verified),
        }
    }
}
