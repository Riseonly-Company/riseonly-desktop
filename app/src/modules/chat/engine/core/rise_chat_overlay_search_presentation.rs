use std::collections::HashMap;

use rise_canon::RiseLimits;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationAuxiliaryLoadState, RiseConversationAuxiliaryPageSnapshot,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_actions::post_wire_types::{GetFeedPostItemDTO, GetFeedPostItemDTOExt};
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;
use crate::modules::search::stores::search_actions::search_wire_types::{SearchResultItemDTO, SearchResultItemDTOExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiseChatSearchRecentDestination {
    Chat,
    Channel,
    User,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RiseChatSearchRecentItem {
    pub id: String,
    pub destination: RiseChatSearchRecentDestination,
    pub title: String,
    pub subtitle: String,
    #[serde(rename = "logoURL")]
    pub logo_url: String,
    #[serde(rename = "chatID")]
    pub chat_id: Option<String>,
    #[serde(rename = "userID")]
    pub user_id: Option<String>,
    #[serde(rename = "userTag")]
    pub user_tag: Option<String>,
    #[serde(rename = "isOnline")]
    pub is_online: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiseChatSearchRecentsPayload {
    pub items: Vec<RiseChatSearchRecentItem>,
}

pub enum RiseChatSearchResultsPolicy {}

impl RiseChatSearchResultsPolicy {
    pub fn collapsed_global_limit() -> i64 {
        RiseLimits::client_value("rise_chat_search_results_policy_collapsed_global_limit")
    }

    pub fn expanded_global_limit() -> i64 {
        RiseLimits::client_value("rise_chat_search_results_policy_expanded_global_limit")
    }

    pub fn recent_limit() -> i64 {
        RiseLimits::client_value("rise_chat_search_results_policy_recent_limit")
    }

    pub fn global_limit(local_result_count: i64, is_expanded: bool) -> i64 {
        if local_result_count == 0 {
            return Self::expanded_global_limit();
        }
        if is_expanded { Self::expanded_global_limit() } else { Self::collapsed_global_limit() }
    }

    pub fn inserting_recent(
        item: RiseChatSearchRecentItem,
        current: &[RiseChatSearchRecentItem],
    ) -> Vec<RiseChatSearchRecentItem> {
        let peer = if item.destination == RiseChatSearchRecentDestination::Channel {
            None
        } else {
            Self::peer_id(&item).map(str::to_owned)
        };
        let mut updated: Vec<RiseChatSearchRecentItem> = Vec::with_capacity(current.len() + 1);
        let item_id = item.id.clone();
        updated.push(item);
        updated.extend(
            current
                .iter()
                .filter(|other| {
                    if other.id == item_id {
                        return false;
                    }
                    match &peer {
                        Some(peer) if other.destination != RiseChatSearchRecentDestination::Channel => {
                            Self::peer_id(other) != Some(peer.as_str())
                        }
                        _ => true,
                    }
                })
                .cloned(),
        );
        updated.truncate(usize::try_from(Self::recent_limit()).unwrap_or(0));
        updated
    }

    fn peer_id(item: &RiseChatSearchRecentItem) -> Option<&str> {
        let trimmed = trimming_whitespaces_and_newlines(item.user_id.as_deref().unwrap_or(""));
        if trimmed.is_empty() { None } else { Some(trimmed) }
    }
}

pub enum RiseSearchRequestGenerationPolicy {}

impl RiseSearchRequestGenerationPolicy {
    pub fn accepts<Scope: PartialEq>(
        active_scope: Option<&Scope>,
        active_generation: Option<u64>,
        expected_scope: &Scope,
        expected_generation: u64,
        is_cancelled: bool,
        is_shutdown: bool,
    ) -> bool {
        !is_cancelled
            && !is_shutdown
            && active_scope == Some(expected_scope)
            && active_generation == Some(expected_generation)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseChatOverlaySearchScope {
    pub query: String,
}

impl RiseChatOverlaySearchScope {
    pub fn new(query: &str) -> Option<Self> {
        let normalized =
            trimming_characters(trimming_whitespaces_and_newlines(query), |character| character == '@').to_lowercase();
        if normalized.is_empty() {
            return None;
        }
        Some(Self { query: normalized })
    }
}

#[derive(Clone, Debug)]
pub struct RiseChatOverlaySearchPage<Item> {
    pub items: Vec<Item>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub load_state: RiseConversationAuxiliaryLoadState,
    pub error: Option<String>,
}

impl<Item> RiseChatOverlaySearchPage<Item> {
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            cursor: None,
            has_more: false,
            load_state: RiseConversationAuxiliaryLoadState::Idle,
            error: None,
        }
    }

    pub fn status(&self) -> SaiWsSliceStatus {
        match self.load_state {
            RiseConversationAuxiliaryLoadState::Idle => SaiWsSliceStatus::Idle,
            RiseConversationAuxiliaryLoadState::Loading => {
                if self.items.is_empty() { SaiWsSliceStatus::Pending } else { SaiWsSliceStatus::Fulfilled }
            }
            RiseConversationAuxiliaryLoadState::Loaded => SaiWsSliceStatus::Fulfilled,
            RiseConversationAuxiliaryLoadState::Failed => {
                if self.items.is_empty() { SaiWsSliceStatus::Rejected } else { SaiWsSliceStatus::Fulfilled }
            }
        }
    }

    pub fn did_load(&self) -> bool {
        self.load_state == RiseConversationAuxiliaryLoadState::Loaded
            || self.load_state == RiseConversationAuxiliaryLoadState::Failed
    }
}

#[derive(Clone)]
pub struct RiseChatOverlaySearchPresentationState {
    pub entities: RiseChatOverlaySearchPage<SearchEntityRow>,
    pub chats: RiseChatOverlaySearchPage<ChatInfo>,
    pub posts: RiseChatOverlaySearchPage<GetPostFeedResponse>,
    pub revision: u64,
}

impl RiseChatOverlaySearchPresentationState {
    pub fn empty() -> Self {
        Self {
            entities: RiseChatOverlaySearchPage::empty(),
            chats: RiseChatOverlaySearchPage::empty(),
            posts: RiseChatOverlaySearchPage::empty(),
            revision: 0,
        }
    }
}

#[derive(Clone)]
pub struct RiseChatOverlayEntitySearchSnapshot {
    pub account_id: String,
    pub scope: RiseChatOverlaySearchScope,
    pub page: RiseChatOverlaySearchPage<SearchEntityRow>,
}

#[derive(Clone)]
pub struct RiseChatOverlayChatSearchSnapshot {
    pub account_id: String,
    pub scope: RiseChatOverlaySearchScope,
    pub page: RiseChatOverlaySearchPage<ChatInfo>,
}

#[derive(Clone)]
pub struct RiseChatOverlayPostSearchSnapshot {
    pub account_id: String,
    pub scope: RiseChatOverlaySearchScope,
    pub page: RiseChatOverlaySearchPage<GetPostFeedResponse>,
}

struct RiseChatOverlaySearchPresentationPreparerCacheEntry<Item> {
    revision: i64,
    items: Vec<Item>,
}

#[derive(Default)]
struct RiseChatOverlaySearchPresentationPreparerState {
    entity_cache: HashMap<RiseChatOverlaySearchScope, RiseChatOverlaySearchPresentationPreparerCacheEntry<SearchEntityRow>>,
    chat_cache: HashMap<RiseChatOverlaySearchScope, RiseChatOverlaySearchPresentationPreparerCacheEntry<ChatInfo>>,
    post_cache: HashMap<RiseChatOverlaySearchScope, RiseChatOverlaySearchPresentationPreparerCacheEntry<GetPostFeedResponse>>,
    order: Vec<RiseChatOverlaySearchScope>,
}

impl RiseChatOverlaySearchPresentationPreparerState {
    const LIMIT: usize = 32;

    fn touch(&mut self, scope: &RiseChatOverlaySearchScope) {
        self.order.retain(|other| other != scope);
        self.order.push(scope.clone());
        while self.order.len() > Self::LIMIT {
            let removed = self.order.remove(0);
            self.entity_cache.remove(&removed);
            self.chat_cache.remove(&removed);
            self.post_cache.remove(&removed);
        }
    }
}

pub struct RiseChatOverlaySearchPresentationPreparer {
    state: RiseIsolated<RiseChatOverlaySearchPresentationPreparerState>,
}

impl Default for RiseChatOverlaySearchPresentationPreparer {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseChatOverlaySearchPresentationPreparer {
    pub fn new() -> Self {
        Self { state: RiseIsolated::new(RiseChatOverlaySearchPresentationPreparerState::default()) }
    }

    pub async fn prepare_entities(
        &self,
        account_id: &str,
        scope: &RiseChatOverlaySearchScope,
        page: &RiseConversationAuxiliaryPageSnapshot<SearchResultItemDTO>,
    ) -> RiseChatOverlayEntitySearchSnapshot {
        let items = self.state.with(|state| {
            let items = match state.entity_cache.get(scope) {
                Some(cached) if cached.revision == page.revision => cached.items.clone(),
                _ => {
                    let items: Vec<SearchEntityRow> = page.items.iter().filter_map(|item| item.as_entity_row()).collect();
                    state.entity_cache.insert(
                        scope.clone(),
                        RiseChatOverlaySearchPresentationPreparerCacheEntry {
                            revision: page.revision,
                            items: items.clone(),
                        },
                    );
                    items
                }
            };
            state.touch(scope);
            items
        });
        RiseChatOverlayEntitySearchSnapshot {
            account_id: account_id.to_owned(),
            scope: scope.clone(),
            page: Self::page(items, page),
        }
    }

    pub async fn prepare_chats(
        &self,
        account_id: &str,
        scope: &RiseChatOverlaySearchScope,
        page: &RiseConversationAuxiliaryPageSnapshot<RiseStoredChat>,
    ) -> RiseChatOverlayChatSearchSnapshot {
        let items = self.state.with(|state| {
            let items = match state.chat_cache.get(scope) {
                Some(cached) if cached.revision == page.revision => cached.items.clone(),
                _ => {
                    let items: Vec<ChatInfo> = page.items.iter().map(RiseStoredChat::to_chat_info).collect();
                    state.chat_cache.insert(
                        scope.clone(),
                        RiseChatOverlaySearchPresentationPreparerCacheEntry {
                            revision: page.revision,
                            items: items.clone(),
                        },
                    );
                    items
                }
            };
            state.touch(scope);
            items
        });
        RiseChatOverlayChatSearchSnapshot {
            account_id: account_id.to_owned(),
            scope: scope.clone(),
            page: Self::page(items, page),
        }
    }

    pub async fn prepare_posts(
        &self,
        account_id: &str,
        scope: &RiseChatOverlaySearchScope,
        page: &RiseConversationAuxiliaryPageSnapshot<GetFeedPostItemDTO>,
    ) -> RiseChatOverlayPostSearchSnapshot {
        let items = self.state.with(|state| {
            let items = match state.post_cache.get(scope) {
                Some(cached) if cached.revision == page.revision => cached.items.clone(),
                _ => {
                    let items: Vec<GetPostFeedResponse> =
                        page.items.iter().filter_map(|item| item.as_feed_response()).collect();
                    state.post_cache.insert(
                        scope.clone(),
                        RiseChatOverlaySearchPresentationPreparerCacheEntry {
                            revision: page.revision,
                            items: items.clone(),
                        },
                    );
                    items
                }
            };
            state.touch(scope);
            items
        });
        RiseChatOverlayPostSearchSnapshot {
            account_id: account_id.to_owned(),
            scope: scope.clone(),
            page: Self::page(items, page),
        }
    }

    pub async fn reset(&self) {
        self.state.with(|state| *state = RiseChatOverlaySearchPresentationPreparerState::default());
    }

    fn page<Source, Item>(
        items: Vec<Item>,
        page: &RiseConversationAuxiliaryPageSnapshot<Source>,
    ) -> RiseChatOverlaySearchPage<Item> {
        RiseChatOverlaySearchPage {
            items,
            cursor: page.cursor.clone(),
            has_more: page.has_more,
            load_state: page.load_state,
            error: page.error.clone(),
        }
    }
}

#[derive(Default)]
pub struct RiseChatOverlaySearchPresentationBox {
    pub value: Option<RiseChatOverlaySearchPresentationState>,
}
