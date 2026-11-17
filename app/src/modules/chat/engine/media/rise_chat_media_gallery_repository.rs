use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use rise_canon::{rise_request, rise_response};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::RiseEngine;
use serde::{Deserialize, Serialize};

use crate::modules::chat::engine::core::rise_conversation_resource_keys::RiseConversationResourceKeys;
use crate::modules::chat::engine::core::rise_conversation_resource_repository::RiseConversationResourceRepository;
use crate::modules::chat::engine::core::rise_conversation_rpc::{RiseConversationEndpoints, RiseConversationRPC};
use crate::modules::chat::engine::core::rise_conversation_store_layout::RiseConversationStoreLayout;
use crate::modules::chat::engine::media::rise_chat_media_gallery_policy::RiseChatMediaGalleryPolicy;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageContentType, MessageMediaKind};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    GetMediaMessagesGatewayDTO, MediaItemWireDTO, MessageMediaInfoWireDTO,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseChatMediaGalleryEdge {
    Older,
    Newer,
}

/// everything the viewer needs to draw a row the chat screen doesnt hold
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiseChatMediaGalleryPayload {
    pub sender_id: String,
    pub sender_name: String,
    pub sender_logo: Option<String>,
    pub caption: String,
    pub content_type: Option<String>,
    pub has_protected_content: bool,
    pub total_media_in_message: i64,
    pub media: MessageMediaInfoWireDTO,
}

/// one row of the window
/// payload is none for a row the chat message list already holds, only identity and order are seeded
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiseChatMediaGalleryItem {
    pub id: String,
    pub message_id: String,
    pub media_index: i64,
    pub created_at: f64,
    pub payload: Option<RiseChatMediaGalleryPayload>,
}

/// the window as the ui sees it, oldest first
/// same order as the message list it was opened from
#[derive(Clone, Debug)]
pub struct RiseChatMediaGalleryWindow {
    pub session_id: Uuid,
    pub chat_id: String,
    pub topic_id: Option<String>,
    pub items: Vec<RiseChatMediaGalleryItem>,
    pub has_more_older: bool,
    pub has_more_newer: bool,
    pub is_loading_older: bool,
    pub is_loading_newer: bool,
    pub positions: HashMap<String, i64>,
    pub total: Option<i64>,
    pub revision: i64,
}

/// identity of a media feed row, message id then index inside the message
/// same string the server uses as id and takes back as relative_id so seeded and fed rows collapse
pub enum RiseChatMediaGalleryIdentity {}

impl RiseChatMediaGalleryIdentity {
    pub fn make(message_id: &str, media_index: i64) -> String {
        format!("{message_id}:{}", media_index.max(0))
    }
}

/// one turn of the index walk
#[derive(Clone, Debug)]
pub enum RiseChatMediaGalleryWalkStep {
    Advanced(RiseChatMediaGalleryWindow),
    Finished,
}

/// version on every stored value, checked before it is trusted
/// index and rows are two values, oversized values are refused not truncated
/// bump it when meaning changes, v1 and v2 had opposite order lol
enum RiseChatMediaGalleryStoreVersion {}

impl RiseChatMediaGalleryStoreVersion {
    const CURRENT: i64 = 3;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RiseChatMediaGalleryStoredIndex {
    version: Option<i64>,
    identities: Vec<String>,
    reached_oldest: bool,
}

impl RiseChatMediaGalleryStoredIndex {
    fn is_current(&self) -> bool {
        self.version == Some(RiseChatMediaGalleryStoreVersion::CURRENT)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RiseChatMediaGalleryStoredWindow {
    version: Option<i64>,
    items: Vec<RiseChatMediaGalleryItem>,
    older_cursor: Option<String>,
    newer_cursor: Option<String>,
    has_more_older: bool,
    has_more_newer: bool,
}

impl RiseChatMediaGalleryStoredWindow {
    fn is_current(&self) -> bool {
        self.version == Some(RiseChatMediaGalleryStoreVersion::CURRENT)
    }
}

struct FetchedPage {
    items: Vec<RiseChatMediaGalleryItem>,
    has_more: bool,
    next_cursor: Option<String>,
    dropped_boundary: bool,
}

struct StoredState {
    index: Option<RiseChatMediaGalleryStoredIndex>,
    window: Option<RiseChatMediaGalleryStoredWindow>,
    is_fresh: bool,
}

struct RiseChatMediaGalleryRepositoryState {
    session_id: Option<Uuid>,
    chat_id: String,
    topic_id: Option<String>,
    items: Vec<RiseChatMediaGalleryItem>,
    window_membership: HashSet<String>,
    has_more_older: bool,
    has_more_newer: bool,
    older_cursor: Option<String>,
    newer_cursor: Option<String>,
    loading_edges: HashSet<RiseChatMediaGalleryEdge>,

    index: Vec<String>,
    index_positions: HashMap<String, usize>,
    index_has_head: bool,
    index_reached_oldest: bool,
    index_cursor: Option<String>,
    is_walking: bool,
    index_overflowed: bool,

    generation: u64,
    revision: i64,
    is_dirty: bool,
}

impl RiseChatMediaGalleryRepositoryState {
    fn new() -> Self {
        Self {
            session_id: None,
            chat_id: String::new(),
            topic_id: None,
            items: Vec::new(),
            window_membership: HashSet::new(),
            has_more_older: true,
            has_more_newer: true,
            older_cursor: None,
            newer_cursor: None,
            loading_edges: HashSet::new(),
            index: Vec::new(),
            index_positions: HashMap::new(),
            index_has_head: false,
            index_reached_oldest: false,
            index_cursor: None,
            is_walking: false,
            index_overflowed: false,
            generation: 0,
            revision: 0,
            is_dirty: false,
        }
    }

    fn clear_session(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.session_id = None;
        self.chat_id = String::new();
        self.topic_id = None;
        self.items = Vec::new();
        self.window_membership = HashSet::new();
        self.index = Vec::new();
        self.index_positions = HashMap::new();
        self.index_has_head = false;
        self.index_cursor = None;
        self.is_walking = false;
        self.loading_edges.clear();
        self.older_cursor = None;
        self.newer_cursor = None;
        self.is_dirty = false;
    }

    fn adopt_head(&mut self, identities: Vec<String>) {
        if identities.is_empty() {
            return;
        }
        if self.index.is_empty() {
            self.index = identities;
            self.rebuild_index_positions();
            return;
        }
        let Some(overlap) = identities.iter().position(|identity| self.index_positions.contains_key(identity)) else {
            self.index = identities;
            self.index_reached_oldest = false;
            self.index_cursor = None;
            self.rebuild_index_positions();
            return;
        };
        if overlap == 0 {
            return;
        }
        drop(self.index.splice(0..0, identities.into_iter().take(overlap)));
        self.rebuild_index_positions();
    }

    /// a window row newer than every indexed row came after the index top was read
    /// so the walk re reads the newest page and adopt_head joins it
    fn distrust_head_below_window(&mut self) {
        if !self.index_has_head {
            return;
        }
        let Some(last_indexed) = self.items.iter().rposition(|item| self.index_positions.contains_key(&item.id)) else {
            return;
        };
        if last_indexed + 1 >= self.items.len() {
            return;
        }
        self.index_has_head = false;
    }

    fn rebuild_index_positions(&mut self) {
        self.index_positions.clear();
        self.index_positions.reserve(self.index.len());
        for (position, identity) in self.index.iter().enumerate() {
            self.index_positions.insert(identity.clone(), position);
        }
    }

    fn snapshot(&self) -> RiseChatMediaGalleryWindow {
        let mut positions: HashMap<String, i64> = HashMap::new();
        let mut total: Option<i64> = None;
        if !self.index_overflowed && self.index_has_head && self.index_reached_oldest {
            let count = self.index.len();
            total = Some(count as i64);
            positions.reserve(self.items.len());
            let mut last_indexed: Option<i64> = None;
            let mut is_monotone = true;
            for item in &self.items {
                let Some(from_newest) = self.index_positions.get(&item.id) else {
                    continue;
                };
                let position = (count - from_newest) as i64;
                if last_indexed.is_some_and(|last| position <= last) {
                    is_monotone = false;
                    break;
                }
                last_indexed = Some(position);
                positions.insert(item.id.clone(), position);
            }
            if !is_monotone {
                positions = HashMap::new();
                total = None;
            }
        }
        RiseChatMediaGalleryWindow {
            session_id: self.session_id.unwrap_or_else(Uuid::new_v4),
            chat_id: self.chat_id.clone(),
            topic_id: self.topic_id.clone(),
            items: self.items.clone(),
            has_more_older: self.has_more_older,
            has_more_newer: self.has_more_newer,
            is_loading_older: self.loading_edges.contains(&RiseChatMediaGalleryEdge::Older),
            is_loading_newer: self.loading_edges.contains(&RiseChatMediaGalleryEdge::Newer),
            positions,
            total,
            revision: self.revision,
        }
    }

    fn window_has_more(&self, edge: RiseChatMediaGalleryEdge) -> bool {
        match edge {
            RiseChatMediaGalleryEdge::Older => self.has_more_older,
            RiseChatMediaGalleryEdge::Newer => self.has_more_newer,
        }
    }

    fn set_window_has_more(&mut self, value: bool, edge: RiseChatMediaGalleryEdge) {
        match edge {
            RiseChatMediaGalleryEdge::Older => self.has_more_older = value,
            RiseChatMediaGalleryEdge::Newer => self.has_more_newer = value,
        }
    }

    fn window_cursor(&self, edge: RiseChatMediaGalleryEdge) -> Option<String> {
        match edge {
            RiseChatMediaGalleryEdge::Older => self.older_cursor.clone(),
            RiseChatMediaGalleryEdge::Newer => self.newer_cursor.clone(),
        }
    }

    fn set_window_cursor(&mut self, value: Option<String>, edge: RiseChatMediaGalleryEdge) {
        match edge {
            RiseChatMediaGalleryEdge::Older => self.older_cursor = value,
            RiseChatMediaGalleryEdge::Newer => self.newer_cursor = value,
        }
    }

    /// cuts the end the reader walks away from
    /// dropped end keeps there is more and its cursor goes to the new boundary so turning back refetches
    fn trim(&mut self, edge: RiseChatMediaGalleryEdge, focus_id: Option<&str>) {
        let limit = RiseChatMediaGalleryPolicy::window_limit().max(0) as usize;
        let count = self.items.len();
        if count <= limit {
            return;
        }
        let focus_index = focus_id
            .and_then(|id| self.items.iter().position(|item| item.id == id))
            .unwrap_or(if edge == RiseChatMediaGalleryEdge::Older { 0 } else { count - 1 });
        let retention = RiseChatMediaGalleryPolicy::prefetch_lookahead().max(0) as usize;
        let (lower, upper) = match edge {
            RiseChatMediaGalleryEdge::Newer => {
                let lower = focus_index.saturating_sub(retention).min(count - limit);
                (lower, count.min(lower + limit))
            }
            RiseChatMediaGalleryEdge::Older => {
                let upper = count.min(focus_index + retention + 1).max(limit);
                (upper.saturating_sub(limit), upper)
            }
        };
        let snapped = RiseChatMediaGalleryRepository::message_aligned_bounds(&self.items, lower, upper, focus_index);
        let (lower, upper) = (snapped.start, snapped.end);
        if lower == 0 && upper >= count {
            return;
        }
        if lower > 0 {
            self.has_more_older = true;
            self.older_cursor = Some(self.items[lower].id.clone());
        }
        if upper < count {
            self.has_more_newer = true;
            self.newer_cursor = Some(self.items[upper - 1].id.clone());
        }
        self.items.truncate(upper);
        self.items.drain(..lower);
        self.window_membership = self.items.iter().map(|item| item.id.clone()).collect();
    }
}

struct RiseChatMediaGalleryLoadingGuard<'repository> {
    repository: &'repository RiseChatMediaGalleryRepository,
    edge: RiseChatMediaGalleryEdge,
    expected_generation: u64,
}

impl Drop for RiseChatMediaGalleryLoadingGuard<'_> {
    fn drop(&mut self) {
        self.repository.state.with(|state| {
            if state.generation == self.expected_generation {
                state.loading_edges.remove(&self.edge);
            }
        });
    }
}

struct RiseChatMediaGalleryWalkingGuard<'repository> {
    repository: &'repository RiseChatMediaGalleryRepository,
    expected_generation: u64,
}

impl Drop for RiseChatMediaGalleryWalkingGuard<'_> {
    fn drop(&mut self) {
        self.repository.state.with(|state| {
            if state.generation == self.expected_generation {
                state.is_walking = false;
            }
        });
    }
}

enum RiseChatMediaGalleryExtendTurn {
    Stale,
    Retry,
    Stop,
    Grew,
}

enum RiseChatMediaGalleryWalkTurn {
    Stale,
    Head { persist: bool },
    Retry,
    Stop,
}

/// paging window behind the chat media viewer and its index
/// window is rows around the opened item, index is all media ids on disk
pub struct RiseChatMediaGalleryRepository {
    account_id: String,
    rpc: RiseConversationRPC,
    resources: RiseConversationResourceRepository,
    state: RiseIsolated<RiseChatMediaGalleryRepositoryState>,
}

impl RiseChatMediaGalleryRepository {
    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Arc<Self> {
        assert!(!account_id.is_empty());
        assert!(engine.account.account_id == account_id);
        let rpc = RiseConversationRPC::new(engine.clone());
        let resources = RiseConversationResourceRepository::with_memory_limits(engine, 8, 4 * 1_024 * 1_024);
        Arc::new(Self {
            account_id,
            rpc,
            resources,
            state: RiseIsolated::new(RiseChatMediaGalleryRepositoryState::new()),
        })
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// installs what is already known and answers before any request
    /// that means screen rows plus disk rows and index
    /// both ends of a fresh window start as there may be more
    pub async fn open(
        &self,
        session_id: Uuid,
        chat_id: &str,
        topic_id: Option<&str>,
        seed: Vec<RiseChatMediaGalleryItem>,
        anchor_id: &str,
    ) -> RiseChatMediaGalleryWindow {
        let expected_generation = self.state.with(|state| {
            state.generation = state.generation.wrapping_add(1);
            state.revision = state.revision.wrapping_add(1);
            state.session_id = Some(session_id);
            state.chat_id = chat_id.to_owned();
            state.topic_id = topic_id.map(str::to_owned);
            state.loading_edges.clear();
            state.is_dirty = false;
            state.index = Vec::new();
            state.index_positions = HashMap::new();
            state.index_has_head = false;
            state.index_reached_oldest = false;
            state.index_cursor = None;
            state.is_walking = false;
            state.index_overflowed = false;
            state.generation
        });

        let stored = self.load_stored(chat_id, topic_id).await;

        self.state.with(|state| {
            if state.generation != expected_generation {
                return state.snapshot();
            }
            let mut region = Self::deduplicated(seed);
            let mut stored_has_more_older = true;
            let mut stored_has_more_newer = true;
            if let Some(stored_window) = stored.window.as_ref()
                && let Some(merged) = Self::merged(&region, &stored_window.items)
            {
                region = merged;
                stored_has_more_older = stored_window.has_more_older
                    || region.first().map(|item| &item.id) != stored_window.items.first().map(|item| &item.id);
                stored_has_more_newer = stored_window.has_more_newer
                    || region.last().map(|item| &item.id) != stored_window.items.last().map(|item| &item.id);
            }

            let anchor_index = region.iter().position(|item| item.id == anchor_id).unwrap_or(region.len() / 2);
            let bounds = Self::window_bounds(
                &region,
                anchor_index,
                RiseChatMediaGalleryPolicy::window_limit().max(0) as usize,
            );
            state.items = region[bounds.clone()].to_vec();
            state.window_membership = state.items.iter().map(|item| item.id.clone()).collect();
            state.has_more_older = bounds.start > 0 || stored_has_more_older;
            state.has_more_newer = bounds.end < region.len() || stored_has_more_newer || !stored.is_fresh;
            state.older_cursor = state.items.first().map(|item| item.id.clone());
            state.newer_cursor = state.items.last().map(|item| item.id.clone());

            if let Some(stored_index) = stored.index
                && !stored_index.identities.is_empty()
            {
                state.index = stored_index.identities;
                state.rebuild_index_positions();
                state.index_reached_oldest = stored_index.reached_oldest;
                state.index_overflowed = state.index.len() as i64 > RiseChatMediaGalleryPolicy::index_limit();
                state.index_has_head = stored.is_fresh;
            }
            state.distrust_head_below_window();
            state.snapshot()
        })
    }

    pub async fn close(&self, session_id: Uuid) {
        if self.state.with(|state| state.session_id) != Some(session_id) {
            return;
        }
        self.persist_if_needed().await;
        self.state.with(RiseChatMediaGalleryRepositoryState::clear_session);
    }

    pub async fn clear_memory(&self) {
        self.state.with(RiseChatMediaGalleryRepositoryState::clear_session);
    }

    pub fn current_window(&self, session_id: Uuid) -> Option<RiseChatMediaGalleryWindow> {
        self.state.with(|state| {
            if state.session_id != Some(session_id) {
                return None;
            }
            Some(state.snapshot())
        })
    }

    pub fn drop_messages(
        &self,
        message_ids: &HashSet<String>,
        expected_session_id: Option<Uuid>,
        expected_chat_id: Option<&str>,
    ) -> Option<RiseChatMediaGalleryWindow> {
        self.state.with(|state| {
            if state.session_id.is_none()
                || message_ids.is_empty()
                || expected_session_id.is_some_and(|expected| state.session_id != Some(expected))
                || expected_chat_id.is_some_and(|expected| state.chat_id != expected)
            {
                return None;
            }
            let prefix: Vec<String> = message_ids.iter().map(|message_id| format!("{message_id}:")).collect();
            let dropped_rows: HashSet<String> = state
                .items
                .iter()
                .filter(|row| {
                    message_ids.contains(&row.message_id) || prefix.iter().any(|prefix| row.id.starts_with(prefix.as_str()))
                })
                .map(|row| row.id.clone())
                .collect();
            let remaining: Vec<RiseChatMediaGalleryItem> =
                state.items.iter().filter(|row| !dropped_rows.contains(&row.id)).cloned().collect();
            let remaining_index: Vec<String> = state
                .index
                .iter()
                .filter(|identity| {
                    !dropped_rows.contains(identity.as_str())
                        && !prefix.iter().any(|prefix| identity.starts_with(prefix.as_str()))
                })
                .cloned()
                .collect();
            if remaining.len() == state.items.len() && remaining_index.len() == state.index.len() {
                return None;
            }
            state.generation = state.generation.wrapping_add(1);
            state.loading_edges.clear();
            state.is_walking = false;
            state.items = remaining;
            state.window_membership = state.items.iter().map(|item| item.id.clone()).collect();
            state.index = remaining_index;
            state.rebuild_index_positions();
            if state.older_cursor.as_ref().is_none_or(|cursor| !state.window_membership.contains(cursor)) {
                state.older_cursor = state.items.first().map(|item| item.id.clone());
            }
            if state.newer_cursor.as_ref().is_none_or(|cursor| !state.window_membership.contains(cursor)) {
                state.newer_cursor = state.items.last().map(|item| item.id.clone());
            }
            state.is_dirty = true;
            state.revision = state.revision.wrapping_add(1);
            Some(state.snapshot())
        })
    }

    /// one page toward the edge, merged into the window
    /// other end is cut back to the ceiling
    /// returns none when nothing to do so the caller can tell
    pub async fn extend(
        &self,
        session_id: Uuid,
        edge: RiseChatMediaGalleryEdge,
        focus_id: Option<&str>,
        user_id: &str,
    ) -> anyhow::Result<Option<RiseChatMediaGalleryWindow>> {
        let expected_generation = self.state.with(|state| {
            if state.session_id != Some(session_id)
                || state.chat_id.is_empty()
                || user_id.is_empty()
                || !state.window_has_more(edge)
                || state.loading_edges.contains(&edge)
            {
                return None;
            }
            state.loading_edges.insert(edge);
            Some(state.generation)
        });
        let Some(expected_generation) = expected_generation else {
            return Ok(None);
        };
        let _loading = RiseChatMediaGalleryLoadingGuard { repository: self, edge, expected_generation };

        let chain = RiseChatMediaGalleryPolicy::empty_page_chain().max(1);
        let mut attempts = 0;
        let mut did_grow = false;
        while attempts < chain {
            attempts += 1;
            let cursor = self.state.with(|state| state.window_cursor(edge));
            let page = self.fetch(edge, cursor.as_deref(), user_id).await?;
            let turn = self.state.with(|state| {
                if state.session_id != Some(session_id) || state.generation != expected_generation {
                    return RiseChatMediaGalleryExtendTurn::Stale;
                }
                let FetchedPage { items, has_more, next_cursor, dropped_boundary } = page;
                let fresh: Vec<RiseChatMediaGalleryItem> =
                    items.into_iter().filter(|item| !state.window_membership.contains(&item.id)).collect();
                if fresh.is_empty() {
                    let next = next_cursor.filter(|next| has_more && !dropped_boundary && cursor.as_ref() != Some(next));
                    return match next {
                        Some(next) => {
                            state.set_window_cursor(Some(next), edge);
                            RiseChatMediaGalleryExtendTurn::Retry
                        }
                        None => {
                            state.set_window_has_more(false, edge);
                            RiseChatMediaGalleryExtendTurn::Stop
                        }
                    };
                }

                state.window_membership.extend(fresh.iter().map(|item| item.id.clone()));
                match edge {
                    RiseChatMediaGalleryEdge::Older => drop(state.items.splice(0..0, fresh)),
                    RiseChatMediaGalleryEdge::Newer => state.items.extend(fresh),
                }
                let edge_item = match edge {
                    RiseChatMediaGalleryEdge::Older => state.items.first(),
                    RiseChatMediaGalleryEdge::Newer => state.items.last(),
                };
                let edge_item_id = edge_item.map(|item| item.id.clone());
                let resolved = (if dropped_boundary { None } else { next_cursor }).or(edge_item_id);
                let advanced = resolved.is_some() && resolved != cursor;
                state.set_window_has_more(has_more && advanced, edge);
                if resolved.is_some() {
                    state.set_window_cursor(resolved, edge);
                }
                RiseChatMediaGalleryExtendTurn::Grew
            });
            match turn {
                RiseChatMediaGalleryExtendTurn::Stale => return Ok(None),
                RiseChatMediaGalleryExtendTurn::Retry => continue,
                RiseChatMediaGalleryExtendTurn::Stop => break,
                RiseChatMediaGalleryExtendTurn::Grew => {
                    did_grow = true;
                    break;
                }
            }
        }

        Ok(self.state.with(|state| {
            if state.session_id != Some(session_id) || state.generation != expected_generation {
                return None;
            }
            state.loading_edges.remove(&edge);
            if did_grow {
                state.trim(edge, focus_id);
                state.distrust_head_below_window();
                state.is_dirty = true;
            }
            state.revision = state.revision.wrapping_add(1);
            Some(state.snapshot())
        }))
    }

    pub async fn walk_index(&self, session_id: Uuid, user_id: &str) -> anyhow::Result<RiseChatMediaGalleryWalkStep> {
        let expected_generation = self.state.with(|state| {
            if state.session_id != Some(session_id)
                || state.chat_id.is_empty()
                || user_id.is_empty()
                || state.index_overflowed
                || (state.index_has_head && state.index_reached_oldest)
                || state.is_walking
            {
                return None;
            }
            state.is_walking = true;
            Some(state.generation)
        });
        let Some(expected_generation) = expected_generation else {
            return Ok(RiseChatMediaGalleryWalkStep::Finished);
        };
        let _walking = RiseChatMediaGalleryWalkingGuard { repository: self, expected_generation };

        let chain = RiseChatMediaGalleryPolicy::empty_page_chain().max(1);
        let mut attempts = 0;
        while attempts < chain {
            attempts += 1;
            let cursor = self.state.with(|state| {
                if state.index_has_head { state.index_cursor.clone().or_else(|| state.index.last().cloned()) } else { None }
            });
            let page = self.fetch(RiseChatMediaGalleryEdge::Older, cursor.as_deref(), user_id).await?;
            let turn = self.state.with(|state| {
                if state.session_id != Some(session_id) || state.generation != expected_generation {
                    return RiseChatMediaGalleryWalkTurn::Stale;
                }
                let FetchedPage { items, has_more, next_cursor, dropped_boundary } = page;
                let identities: Vec<String> = items.into_iter().rev().map(|item| item.id).collect();

                if !state.index_has_head {
                    state.adopt_head(identities);
                    state.index_has_head = true;
                    if !has_more {
                        state.index_reached_oldest = true;
                    }
                    state.is_dirty = true;
                    state.revision = state.revision.wrapping_add(1);
                    return RiseChatMediaGalleryWalkTurn::Head { persist: state.index_reached_oldest };
                }

                let fresh: Vec<String> =
                    identities.into_iter().filter(|identity| !state.index_positions.contains_key(identity)).collect();
                if fresh.is_empty() {
                    let next = next_cursor.filter(|next| has_more && !dropped_boundary && cursor.as_ref() != Some(next));
                    return match next {
                        Some(next) => {
                            state.index_cursor = Some(next);
                            RiseChatMediaGalleryWalkTurn::Retry
                        }
                        None => {
                            state.index_reached_oldest = true;
                            RiseChatMediaGalleryWalkTurn::Stop
                        }
                    };
                }

                for identity in fresh {
                    state.index_positions.insert(identity.clone(), state.index.len());
                    state.index.push(identity);
                }
                state.index_cursor = None;
                if state.index.len() as i64 > RiseChatMediaGalleryPolicy::index_limit() {
                    state.index_overflowed = true;
                }
                if !has_more {
                    state.index_reached_oldest = true;
                }
                RiseChatMediaGalleryWalkTurn::Stop
            });
            match turn {
                RiseChatMediaGalleryWalkTurn::Stale => return Ok(RiseChatMediaGalleryWalkStep::Finished),
                RiseChatMediaGalleryWalkTurn::Head { persist } => {
                    if persist {
                        self.persist_if_needed().await;
                    }
                    return Ok(RiseChatMediaGalleryWalkStep::Advanced(self.state.with(|state| state.snapshot())));
                }
                RiseChatMediaGalleryWalkTurn::Retry => continue,
                RiseChatMediaGalleryWalkTurn::Stop => break,
            }
        }

        let should_persist = self.state.with(|state| {
            if state.session_id != Some(session_id) || state.generation != expected_generation {
                return None;
            }
            state.is_walking = false;
            state.is_dirty = true;
            state.revision = state.revision.wrapping_add(1);
            Some(state.index_reached_oldest || state.index_overflowed)
        });
        let Some(should_persist) = should_persist else {
            return Ok(RiseChatMediaGalleryWalkStep::Finished);
        };
        if should_persist {
            self.persist_if_needed().await;
        }
        Ok(RiseChatMediaGalleryWalkStep::Advanced(self.state.with(|state| state.snapshot())))
    }

    async fn fetch(
        &self,
        edge: RiseChatMediaGalleryEdge,
        cursor: Option<&str>,
        _user_id: &str,
    ) -> anyhow::Result<FetchedPage> {
        let (chat_id, topic_id) = self.state.with(|state| (state.chat_id.clone(), state.topic_id.clone()));
        let canon: rise_response::message::GetMediaMessages = self
            .rpc
            .call(
                RiseConversationEndpoints::get_media_messages(),
                rise_request::message::GetMediaMessages {
                    chat_id,
                    limit: Some(RiseChatMediaGalleryPolicy::page_limit()),
                    relative_id: cursor.map(str::to_owned),
                    topic_id,
                    up: Some(edge == RiseChatMediaGalleryEdge::Older),
                },
            )
            .await?;
        let response = GetMediaMessagesGatewayDTO::from_canon(&canon);
        let has_more = response.is_have_more.unwrap_or(false);
        let page = Self::chronological(response.media_items.as_deref().unwrap_or_default());
        let page_count = page.len();
        let usable = Self::complete_messages(page, edge, has_more);
        let dropped_boundary = usable.len() != page_count;
        Ok(FetchedPage {
            items: usable,
            has_more,
            next_cursor: Self::normalized(response.next_cursor.as_deref()),
            dropped_boundary,
        })
    }

    async fn load_stored(&self, chat_id: &str, topic_id: Option<&str>) -> StoredState {
        let index_key = RiseConversationResourceKeys::media_gallery_index(chat_id, topic_id);
        let window_key = RiseConversationResourceKeys::media_gallery_window(chat_id, topic_id);
        let stored_index = self.resources.cached::<RiseChatMediaGalleryStoredIndex>(&index_key).await.ok();
        let stored_window = self.resources.cached::<RiseChatMediaGalleryStoredWindow>(&window_key).await.ok();
        let revision = stored_index.as_ref().map_or(0, |snapshot| snapshot.revision);
        let age_ms = (Self::now_milliseconds() - revision) as f64;
        let is_fresh = age_ms >= 0.0 && age_ms <= RiseChatMediaGalleryPolicy::index_freshness() * 1_000.0;
        StoredState {
            index: stored_index.and_then(|snapshot| snapshot.value).filter(RiseChatMediaGalleryStoredIndex::is_current),
            window: stored_window.and_then(|snapshot| snapshot.value).filter(RiseChatMediaGalleryStoredWindow::is_current),
            is_fresh,
        }
    }

    async fn persist_if_needed(&self) {
        let stored = self.state.with(|state| {
            if !state.is_dirty || state.chat_id.is_empty() {
                return None;
            }
            state.is_dirty = false;
            let index_limit = RiseChatMediaGalleryPolicy::index_limit().max(0) as usize;
            let stored_index = RiseChatMediaGalleryStoredIndex {
                version: Some(RiseChatMediaGalleryStoreVersion::CURRENT),
                identities: if state.index_overflowed {
                    Vec::new()
                } else {
                    state.index.iter().take(index_limit).cloned().collect()
                },
                reached_oldest: if state.index_overflowed { false } else { state.index_reached_oldest },
            };
            let stored_window = RiseChatMediaGalleryStoredWindow {
                version: Some(RiseChatMediaGalleryStoreVersion::CURRENT),
                items: Self::budgeted(&state.items, RiseChatMediaGalleryPolicy::window_bytes()),
                older_cursor: state.older_cursor.clone(),
                newer_cursor: state.newer_cursor.clone(),
                has_more_older: state.has_more_older,
                has_more_newer: state.has_more_newer,
            };
            let index_key = RiseConversationResourceKeys::media_gallery_index(&state.chat_id, state.topic_id.as_deref());
            let window_key = RiseConversationResourceKeys::media_gallery_window(&state.chat_id, state.topic_id.as_deref());
            Some((stored_index, index_key, stored_window, window_key))
        });
        let Some((stored_index, index_key, stored_window, window_key)) = stored else {
            return;
        };
        let _ = self.resources.store(&stored_index, &index_key).await;
        let _ = self.resources.store(&stored_window, &window_key).await;
    }

    fn budgeted(items: &[RiseChatMediaGalleryItem], bytes: i64) -> Vec<RiseChatMediaGalleryItem> {
        if items.is_empty() || bytes <= 0 {
            return Vec::new();
        }
        let bytes = bytes as usize;
        let middle = items.len() / 2;
        let mut lower = middle;
        let mut upper = middle;
        let mut spent = Self::estimated_size(&items[middle]);
        if spent > bytes {
            return Vec::new();
        }
        while lower > 0 || upper + 1 < items.len() {
            let mut did_grow = false;
            if lower > 0 {
                let candidate = Self::estimated_size(&items[lower - 1]);
                if spent + candidate <= bytes {
                    spent += candidate;
                    lower -= 1;
                    did_grow = true;
                }
            }
            if upper + 1 < items.len() {
                let candidate = Self::estimated_size(&items[upper + 1]);
                if spent + candidate <= bytes {
                    spent += candidate;
                    upper += 1;
                    did_grow = true;
                }
            }
            if !did_grow {
                break;
            }
        }
        items[lower..=upper].to_vec()
    }

    fn estimated_size(item: &RiseChatMediaGalleryItem) -> usize {
        let mut size = 96;
        size += item.id.len();
        size += item.message_id.len();
        let Some(payload) = item.payload.as_ref() else {
            return size;
        };
        size += 256;
        size += payload.caption.len();
        size += payload.sender_id.len();
        size += payload.sender_name.len();
        size += Self::length(payload.sender_logo.as_deref());
        size += Self::length(payload.content_type.as_deref());
        size += Self::length(payload.media.file_url.as_deref());
        size += Self::length(payload.media.thumbnail_url.as_deref());
        size += Self::length(payload.media.file_id.as_deref());
        size += Self::length(payload.media.file_name.as_deref());
        size += Self::length(payload.media.waveform.as_deref());
        size
    }

    fn length(value: Option<&str>) -> usize {
        value.map_or(0, str::len)
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }

    /// newest first pages into viewer order
    /// attachments of one message stay in grid order so only the runs are reversed
    fn chronological(page: &[MediaItemWireDTO]) -> Vec<RiseChatMediaGalleryItem> {
        let mut runs: Vec<Vec<RiseChatMediaGalleryItem>> = Vec::new();
        let mut current: Vec<RiseChatMediaGalleryItem> = Vec::new();
        let mut current_message_id: Option<String> = None;
        for wire in page {
            let Some(item) = Self::make_item(wire) else {
                continue;
            };
            if current_message_id.as_deref() != Some(item.message_id.as_str()) {
                if !current.is_empty() {
                    runs.push(std::mem::take(&mut current));
                }
                current_message_id = Some(item.message_id.clone());
            }
            current.push(item);
        }
        if !current.is_empty() {
            runs.push(current);
        }
        runs.into_iter().rev().flatten().collect()
    }

    fn complete_messages(
        page: Vec<RiseChatMediaGalleryItem>,
        edge: RiseChatMediaGalleryEdge,
        truncated: bool,
    ) -> Vec<RiseChatMediaGalleryItem> {
        if !truncated || page.is_empty() {
            return page;
        }
        let boundary = match edge {
            RiseChatMediaGalleryEdge::Older => page.first(),
            RiseChatMediaGalleryEdge::Newer => page.last(),
        };
        let Some(boundary_message_id) = boundary.map(|item| item.message_id.clone()) else {
            return page;
        };
        if !page.iter().any(|item| item.message_id != boundary_message_id) {
            return page;
        }
        page.into_iter().filter(|item| item.message_id != boundary_message_id).collect()
    }

    fn make_item(wire: &MediaItemWireDTO) -> Option<RiseChatMediaGalleryItem> {
        let media = wire.media_info.as_ref()?;
        let normalized_message_id = Self::normalized(wire.message_id.as_deref());
        let normalized_content_type = Self::normalized(wire.content_type.as_deref());
        let media_index = wire.media_index.unwrap_or(0).max(0);
        let kind = media
            .to_media_item(
                media_index,
                normalized_message_id.as_deref().unwrap_or_default(),
                normalized_content_type.as_deref().map(MessageContentType::from_raw_value),
            )
            .kind;
        if !matches!(kind, MessageMediaKind::Image | MessageMediaKind::Video) {
            return None;
        }
        let message_id = normalized_message_id.or_else(|| {
            Self::normalized(wire.id.as_deref())
                .map(|id| id.split(':').find(|part| !part.is_empty()).map(str::to_owned).unwrap_or_else(|| id.clone()))
        })?;
        let sender = wire.sender.as_ref();
        Some(RiseChatMediaGalleryItem {
            id: RiseChatMediaGalleryIdentity::make(&message_id, media_index),
            message_id,
            media_index,
            created_at: RiseConversationStoreLayout::normalized_timestamp(wire.created_at.or(wire.timestamp).unwrap_or(0.0)),
            payload: Some(RiseChatMediaGalleryPayload {
                sender_id: wire
                    .sender_id
                    .clone()
                    .or_else(|| sender.and_then(|sender| sender.id.clone()))
                    .unwrap_or_default(),
                sender_name: wire
                    .sender_name
                    .clone()
                    .or_else(|| sender.and_then(|sender| sender.name.clone()))
                    .unwrap_or_default(),
                sender_logo: Self::normalized(
                    sender.and_then(|sender| sender.more.as_ref()).and_then(|more| more.logo.as_deref()),
                ),
                caption: wire.caption.clone().unwrap_or_default(),
                content_type: normalized_content_type,
                has_protected_content: wire.has_protected_content.unwrap_or(false),
                total_media_in_message: wire.total_media_in_message.unwrap_or(1).max(1),
                media: media.clone(),
            }),
        })
    }

    fn deduplicated(seed: Vec<RiseChatMediaGalleryItem>) -> Vec<RiseChatMediaGalleryItem> {
        let mut seen: HashSet<String> = HashSet::with_capacity(seed.len());
        let mut result: Vec<RiseChatMediaGalleryItem> = Vec::with_capacity(seed.len());
        for item in seed {
            if !seen.insert(item.id.clone()) {
                continue;
            }
            result.push(item);
        }
        result
    }

    /// splices a stored region into the seeded one only where they share a row
    /// no proof means the join is a guess and could leave an invisible gap
    /// seeded region wins every shared row
    fn merged(
        region: &[RiseChatMediaGalleryItem],
        stored: &[RiseChatMediaGalleryItem],
    ) -> Option<Vec<RiseChatMediaGalleryItem>> {
        if region.is_empty() || stored.is_empty() {
            return None;
        }
        let region_ids: HashSet<&str> = region.iter().map(|item| item.id.as_str()).collect();
        let first_shared = stored.iter().position(|item| region_ids.contains(item.id.as_str()))?;
        let last_shared = stored.iter().rposition(|item| region_ids.contains(item.id.as_str())).unwrap_or(first_shared);
        let mut result: Vec<RiseChatMediaGalleryItem> = stored[..first_shared].to_vec();
        result.extend_from_slice(region);
        result.extend_from_slice(&stored[(last_shared + 1)..]);
        Some(Self::deduplicated(result))
    }

    /// slice of limit width around anchor, keeps the anchor in the middle
    /// spends the whole budget when one side is short
    fn window_bounds(items: &[RiseChatMediaGalleryItem], anchor: usize, limit: usize) -> Range<usize> {
        if items.is_empty() {
            return 0..0;
        }
        if items.len() <= limit || limit == 0 {
            return 0..items.len();
        }
        let lower = anchor.saturating_sub(limit / 2);
        let upper = items.len().min(lower + limit);
        let lower = upper.saturating_sub(limit);
        Self::message_aligned_bounds(items, lower, upper, anchor)
    }

    /// pulls slice edges inwards until none splits a message
    /// the rail is of the open message so a split album shows a wrong count
    /// if the anchor would be lost the raw edge stays
    fn message_aligned_bounds(
        items: &[RiseChatMediaGalleryItem],
        lower: usize,
        upper: usize,
        anchor: usize,
    ) -> Range<usize> {
        if lower >= upper || upper > items.len() {
            return lower..upper;
        }
        let mut aligned_lower = lower;
        if aligned_lower > 0 {
            let message_id = &items[aligned_lower].message_id;
            if &items[aligned_lower - 1].message_id == message_id {
                while aligned_lower < upper && &items[aligned_lower].message_id == message_id {
                    aligned_lower += 1;
                }
            }
        }
        let mut aligned_upper = upper;
        if aligned_upper < items.len() {
            let message_id = &items[aligned_upper - 1].message_id;
            if &items[aligned_upper].message_id == message_id {
                while aligned_upper > aligned_lower && &items[aligned_upper - 1].message_id == message_id {
                    aligned_upper -= 1;
                }
            }
        }
        if aligned_lower >= aligned_upper || anchor < aligned_lower || anchor >= aligned_upper {
            return lower..upper;
        }
        aligned_lower..aligned_upper
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        if value.is_empty() { None } else { Some(value.to_string()) }
    }
}
