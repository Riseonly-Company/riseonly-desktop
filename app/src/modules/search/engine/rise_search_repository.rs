use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use rise_canon::{RiseTimingCanon, rise_request};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreViewKey, RiseStoreWriteBatch,
    RiseStoredValue, RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine};
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostCacheSource, RisePostPagePresentation, RisePostPageStatus,
};
use crate::modules::search::engine::rise_search_debug_trace::RiseSearchDebugTrace;
use crate::modules::search::engine::rise_search_engine_models::{
    RiseGlobalSearchCategory, RiseGlobalSearchPagePresentation, RiseGlobalSearchResponse, RiseGlobalSearchResponseExt,
    RiseGlobalSearchRow, RiseGlobalSearchScope, RiseSearchPresentationSnapshot, RiseSearchStoredEntity,
};
use crate::modules::search::engine::rise_search_rpc::{RiseSearchEndpoints, RiseSearchRPC};
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;
use crate::modules::search::stores::search_actions::search_wire_types::SearchResultItemDTOExt;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseSearchRepositoryError {
    #[error("staleAccount")]
    StaleAccount,
    #[error("missingActiveSession")]
    MissingActiveSession,
    #[error("server: {0}")]
    Server(String),
}

/// why a search load was started
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseSearchRepositoryLoadIntent {
    Activate,
    Refresh,
    LoadMore,
}

/// ticket handed out for one search load
#[derive(Clone, Debug)]
pub struct RiseSearchRepositoryLoadTicket {
    pub scope: RiseGlobalSearchScope,
    pub slot: String,
    pub intent: RiseSearchRepositoryLoadIntent,
    pub generation: Option<u64>,
    pub cursor: Option<String>,
    pub snapshot: RiseSearchPresentationSnapshot,
}

/// one persisted page of search results
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseSearchRepositoryPersistentPage {
    scope_key: String,
    rows: Vec<RiseSearchStoredEntity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
    has_more: bool,
    updated_at_milliseconds: i64,
    revision: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseSearchRepositoryScopeSlot {
    scope_key: String,
    access: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseSearchRepositoryScopeIndex {
    slots: HashMap<String, RiseSearchRepositoryScopeSlot>,
    access_clock: u64,
}

static NAMESPACE: LazyLock<RiseStoreNamespace> = LazyLock::new(|| RiseStoreNamespace::new("search.resources.v1"));
const INDEX_KEY: &str = "global-search-scope-slots.v1";

#[derive(Default)]
struct RiseSearchRepositoryState {
    pages_by_scope_key: HashMap<String, RiseGlobalSearchPagePresentation>,
    session_pages_by_scope_key: HashMap<String, RiseSearchRepositoryPersistentPage>,
    scope_index: RiseSearchRepositoryScopeIndex,
    scope_index_loaded: bool,
    generations: HashMap<String, u64>,
    in_flight_scopes: HashSet<String>,
    presentation_revision: i64,
    store_version: i64,
    is_shutdown: bool,
}

pub struct RiseSearchRepository {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    engine: Arc<RiseEngine>,
    rpc: RiseSearchRPC,
    refresh_interval_milliseconds: i64,
    state: RiseIsolated<RiseSearchRepositoryState>,
}

impl RiseSearchRepository {
    const MAXIMUM_SCOPE_COUNT: usize = 24;
    const MAXIMUM_ROWS: usize = 100;

    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Self {
        Self {
            account_id,
            account_epoch,
            session_id,
            rpc: RiseSearchRPC::new(engine.clone()),
            engine,
            refresh_interval_milliseconds: RiseTimingCanon::freshness_ms("global_search_page"),
            state: RiseIsolated::new(RiseSearchRepositoryState::default()),
        }
    }

    pub async fn activate(&self) -> RiseSearchPresentationSnapshot {
        self.state.with(|state| self.make_snapshot(state))
    }

    pub async fn begin_post_load(
        &self,
        scope: &RiseGlobalSearchScope,
        intent: RiseSearchRepositoryLoadIntent,
    ) -> RiseSearchPresentationSnapshot {
        let key = scope.key();
        self.state.with(|state| {
            let current = state.pages_by_scope_key.remove(&key).unwrap_or(RiseGlobalSearchPagePresentation::IDLE);
            state.pages_by_scope_key.insert(key, Self::loading_page(current, intent));
            self.make_snapshot(state)
        })
    }

    pub async fn finish_post_load(
        &self,
        scope: &RiseGlobalSearchScope,
        page: RisePostPagePresentation,
    ) -> RiseSearchPresentationSnapshot {
        let key = scope.key();
        self.state.with(|state| {
            state.pages_by_scope_key.insert(
                key,
                RiseGlobalSearchPagePresentation {
                    items: page.items.into_iter().map(RiseGlobalSearchRow::Post).collect(),
                    status: page.status,
                    error_message: page.error_message,
                    has_more: page.has_more,
                    is_refreshing: page.is_refreshing,
                    is_loading_more: page.is_loading_more,
                    cache_source: page.cache_source,
                    revision: page.revision,
                },
            );
            self.make_snapshot(state)
        })
    }

    pub async fn begin_load(
        &self,
        scope: &RiseGlobalSearchScope,
        intent: RiseSearchRepositoryLoadIntent,
    ) -> RiseSearchRepositoryLoadTicket {
        if self.state.with(|state| state.is_shutdown) {
            return self.idle_ticket(scope, String::new(), intent);
        }
        self.load_index_if_needed().await;
        let key = scope.key();
        let slot = self.state.with(|state| Self::allocate_slot(state, &key));
        let cached = self.load_page(scope, &slot).await;

        let refresh_interval = self.refresh_interval_milliseconds;
        let started = self.state.with(|state| {
            let current = state.pages_by_scope_key.get(&key).cloned().unwrap_or(cached);
            state.pages_by_scope_key.insert(key.clone(), current.clone());

            if state.in_flight_scopes.contains(&key) {
                return None;
            }
            if intent == RiseSearchRepositoryLoadIntent::LoadMore && (!current.has_more || current.items.is_empty()) {
                return None;
            }
            if intent == RiseSearchRepositoryLoadIntent::Activate
                && current.status == RisePostPageStatus::Loaded
                && Self::now_milliseconds() - current.revision < refresh_interval
            {
                return None;
            }

            let generation = state.generations.get(&key).copied().unwrap_or(0).wrapping_add(1);
            state.generations.insert(key.clone(), generation);
            state.in_flight_scopes.insert(key.clone());
            let known_cursor = if intent == RiseSearchRepositoryLoadIntent::LoadMore {
                state.session_pages_by_scope_key.get(&key).map(|session| session.cursor.clone())
            } else {
                Some(None)
            };
            Some((current, generation, known_cursor))
        });
        let Some((current, generation, known_cursor)) = started else {
            return self.idle_ticket(scope, slot, intent);
        };

        let cursor = match known_cursor {
            Some(cursor) => cursor,
            None => self.load_persistent_page(&slot, &key).await.and_then(|page| page.cursor),
        };
        let snapshot = self.state.with(|state| {
            state.pages_by_scope_key.insert(key.clone(), Self::loading_page(current, intent));
            self.make_snapshot(state)
        });
        RiseSearchRepositoryLoadTicket {
            scope: scope.clone(),
            slot,
            intent,
            generation: Some(generation),
            cursor,
            snapshot,
        }
    }

    pub async fn finish_load(&self, ticket: &RiseSearchRepositoryLoadTicket) -> RiseSearchPresentationSnapshot {
        let key = ticket.scope.key();
        let Some(generation) = ticket.generation else {
            return self.state.with(|state| self.make_snapshot(state));
        };
        if !self.is_current_generation(&key, generation) {
            return self.state.with(|state| self.make_snapshot(state));
        }

        let result = self.perform_load(ticket, &key, generation).await;
        self.state.with(|state| {
            state.in_flight_scopes.remove(&key);
            if let Err(error) = &result
                && !error.is::<CancellationError>()
            {
                let current =
                    state.pages_by_scope_key.remove(&key).unwrap_or(RiseGlobalSearchPagePresentation::IDLE);
                let is_final_refusal = CanonErrors::failure(error)
                    .is_some_and(|failure| !CanonErrors::is_retryable(failure.code.as_deref()));
                let status = if current.items.is_empty() && !is_final_refusal {
                    RisePostPageStatus::Failed
                } else {
                    RisePostPageStatus::Loaded
                };
                state.pages_by_scope_key.insert(
                    key.clone(),
                    RiseGlobalSearchPagePresentation {
                        items: current.items,
                        status,
                        error_message: Some(CanonErrors::load_failure_text(error)),
                        has_more: current.has_more,
                        is_refreshing: false,
                        is_loading_more: false,
                        cache_source: current.cache_source,
                        revision: current.revision,
                    },
                );
            }
            self.make_snapshot(state)
        })
    }

    pub async fn shutdown(&self) {
        self.state.with(|state| {
            state.is_shutdown = true;
            for generation in state.generations.values_mut() {
                *generation = generation.wrapping_add(1);
            }
            state.in_flight_scopes.clear();
            state.pages_by_scope_key.clear();
            state.session_pages_by_scope_key.clear();
        });
    }

    async fn perform_load(
        &self,
        ticket: &RiseSearchRepositoryLoadTicket,
        key: &str,
        generation: u64,
    ) -> anyhow::Result<()> {
        let request = rise_request::search::GlobalSearch {
            category: Some(ticket.scope.category.wire().to_owned()),
            limit: Some(20),
            query: Some(ticket.scope.normalized_query()),
            relative_id: ticket.cursor.clone(),
            ..rise_request::search::GlobalSearch::default()
        };
        let response: RiseGlobalSearchResponse =
            self.rpc.call(RiseSearchEndpoints::global_search(), &request).await?;
        if let Some(error) = response.error().filter(|error| !error.is_empty()) {
            return Err(RiseSearchRepositoryError::Server(error).into());
        }
        if !self.is_current_generation(key, generation) || !self.is_current_account() {
            return Err(CancellationError.into());
        }
        self.commit(&response, ticket, key).await
    }

    async fn commit(
        &self,
        response: &RiseGlobalSearchResponse,
        ticket: &RiseSearchRepositoryLoadTicket,
        key: &str,
    ) -> anyhow::Result<()> {
        let mut next_rows = Self::rows(response, ticket.scope.category);
        if ticket.intent == RiseSearchRepositoryLoadIntent::LoadMore {
            let session_rows =
                self.state.with(|state| state.session_pages_by_scope_key.get(key).map(|page| page.rows.clone()));
            let earlier_rows = match session_rows {
                Some(rows) => Some(rows),
                None => self.load_persistent_page(&ticket.slot, key).await.map(|page| page.rows),
            };
            if let Some(mut rows) = earlier_rows {
                rows.append(&mut next_rows);
                next_rows = rows;
            }
        }
        let mut seen = HashSet::new();
        next_rows.retain(|row| seen.insert(row.id.clone()));

        let (page, index_data, version, now) = self.state.with(|state| {
            Self::advance_store_version(state);
            let now = Self::now_milliseconds();
            let page = RiseSearchRepositoryPersistentPage {
                scope_key: key.to_owned(),
                rows: next_rows,
                cursor: response.next_cursor(),
                has_more: response.has_more(),
                updated_at_milliseconds: now,
                revision: state.store_version,
            };
            state.session_pages_by_scope_key.insert(key.to_owned(), page.clone());
            serde_json::to_vec(&state.scope_index).map(|index_data| (page, index_data, state.store_version, now))
        })?;

        let mut mutations = Vec::with_capacity(2);
        let mut invalidated_views = HashSet::new();
        if page.rows.len() <= Self::MAXIMUM_ROWS {
            mutations.push(Self::set_value(&ticket.slot, serde_json::to_vec(&page)?, version, now));
            invalidated_views.insert(RiseStoreViewKey::value(&NAMESPACE, &ticket.slot));
        }
        mutations.push(Self::set_value(INDEX_KEY, index_data, version, now));
        invalidated_views.insert(RiseStoreViewKey::value(&NAMESPACE, INDEX_KEY));
        self.engine.write(RiseStoreWriteBatch::new(mutations, invalidated_views)).await?;

        let presentation = Self::presentation(&page, ticket.scope.category, RisePostCacheSource::Network);
        self.state.with(|state| {
            state.pages_by_scope_key.insert(key.to_owned(), presentation);
        });
        Ok(())
    }

    fn set_value(key: &str, data: Vec<u8>, version: i64, now: i64) -> RiseStoreMutation {
        RiseStoreMutation::SetValue {
            namespace: NAMESPACE.clone(),
            key: key.to_owned(),
            value: RiseStoredValue::new(data, version, now),
            precondition: RiseStoreVersionPrecondition::Any,
            policy: RiseStoredValueWritePolicy::ServerAuthoritative,
        }
    }

    fn rows(response: &RiseGlobalSearchResponse, category: RiseGlobalSearchCategory) -> Vec<RiseSearchStoredEntity> {
        match response {
            RiseGlobalSearchResponse::CategorizedProfiles(page) => {
                if category != RiseGlobalSearchCategory::Profiles {
                    return Vec::new();
                }
                page.items
                    .iter()
                    .filter_map(|item| {
                        let row = item.as_entity_row()?;
                        if row.is_channel() || row.entity_type.to_uppercase() == "GROUP" || row.user_id.is_none() {
                            return None;
                        }
                        Some(RiseSearchStoredEntity::new(&row))
                    })
                    .collect()
            }
            RiseGlobalSearchResponse::CategorizedChats(page) => {
                if category != RiseGlobalSearchCategory::Channels && category != RiseGlobalSearchCategory::Groups {
                    return Vec::new();
                }
                page.items
                    .iter()
                    .filter_map(|item| {
                        let chat = item.chat.as_ref()?;
                        if chat.id.is_empty() {
                            return None;
                        }
                        let tag = [chat.tag.as_deref(), Some(chat.username.as_str())]
                            .into_iter()
                            .flatten()
                            .map(trimming_whitespaces_and_newlines)
                            .find(|tag| !tag.is_empty())
                            .unwrap_or_default();
                        let is_groups = category == RiseGlobalSearchCategory::Groups;
                        Some(RiseSearchStoredEntity::new(&SearchEntityRow {
                            id: chat.id.clone(),
                            entity_type: if is_groups { "GROUP" } else { "CHANNEL" }.to_owned(),
                            display_name: chat.title.clone(),
                            tag_line: if tag.is_empty() { String::new() } else { format!("@{tag}") },
                            logo_url: chat.logo_url.clone(),
                            user_id: None,
                            user_tag: None,
                            chat_id: Some(chat.id.clone()),
                            is_premium: chat.is_verified,
                            is_official: false,
                            member_count: Some(chat.member_count),
                            is_public: Some(chat.is_public),
                            ..SearchEntityRow::default()
                        }))
                    })
                    .collect()
            }
            RiseGlobalSearchResponse::CategorizedPosts(_) | RiseGlobalSearchResponse::MixedResults(_) => Vec::new(),
        }
    }

    async fn load_page(&self, scope: &RiseGlobalSearchScope, slot: &str) -> RiseGlobalSearchPagePresentation {
        let key = scope.key();
        if let Some(memory) = self.state.with(|state| state.pages_by_scope_key.get(&key).cloned()) {
            RiseSearchDebugTrace::cache(&key, RisePostCacheSource::Memory, memory.revision);
            return memory;
        }
        let Some(page) = self.load_persistent_page(slot, &key).await else {
            RiseSearchDebugTrace::cache(&key, RisePostCacheSource::Miss, 0);
            return RiseGlobalSearchPagePresentation::IDLE;
        };
        RiseSearchDebugTrace::cache(&key, RisePostCacheSource::PersistentStore, page.revision);
        Self::presentation(&page, scope.category, RisePostCacheSource::PersistentStore)
    }

    async fn load_persistent_page(&self, slot: &str, scope_key: &str) -> Option<RiseSearchRepositoryPersistentPage> {
        let stored = self.engine.value(NAMESPACE.clone(), slot.to_owned()).await.ok().flatten()?;
        let page = serde_json::from_slice::<RiseSearchRepositoryPersistentPage>(&stored.data).ok()?;
        if page.scope_key != scope_key {
            return None;
        }
        self.state.with(|state| state.store_version = state.store_version.max(stored.version));
        Some(page)
    }

    fn presentation(
        page: &RiseSearchRepositoryPersistentPage,
        category: RiseGlobalSearchCategory,
        source: RisePostCacheSource,
    ) -> RiseGlobalSearchPagePresentation {
        let items = page
            .rows
            .iter()
            .map(|stored| match category {
                RiseGlobalSearchCategory::Profiles => RiseGlobalSearchRow::Profile(stored.row()),
                RiseGlobalSearchCategory::Channels => RiseGlobalSearchRow::Channel(stored.row()),
                RiseGlobalSearchCategory::Groups => RiseGlobalSearchRow::Group(stored.row()),
                RiseGlobalSearchCategory::Publications | RiseGlobalSearchCategory::Threads => {
                    RiseGlobalSearchRow::Profile(stored.row())
                }
            })
            .collect();
        RiseGlobalSearchPagePresentation {
            items,
            status: RisePostPageStatus::Loaded,
            error_message: None,
            has_more: page.has_more,
            is_refreshing: false,
            is_loading_more: false,
            cache_source: source,
            revision: page.updated_at_milliseconds,
        }
    }

    async fn load_index_if_needed(&self) {
        let already_loaded = self.state.with(|state| std::mem::replace(&mut state.scope_index_loaded, true));
        if already_loaded {
            return;
        }
        let Some(stored) = self.engine.value(NAMESPACE.clone(), INDEX_KEY.to_owned()).await.ok().flatten() else {
            return;
        };
        let Ok(decoded) = serde_json::from_slice::<RiseSearchRepositoryScopeIndex>(&stored.data) else {
            return;
        };
        self.state.with(|state| {
            state.scope_index = decoded;
            state.store_version = state.store_version.max(stored.version);
        });
    }

    fn allocate_slot(state: &mut RiseSearchRepositoryState, scope_key: &str) -> String {
        state.scope_index.access_clock = state.scope_index.access_clock.wrapping_add(1);
        let access_clock = state.scope_index.access_clock;
        let matching =
            state.scope_index.slots.iter().find(|(_, slot)| slot.scope_key == scope_key).map(|(key, _)| key.clone());
        if let Some(matching) = matching {
            state.scope_index.slots.insert(
                matching.clone(),
                RiseSearchRepositoryScopeSlot { scope_key: scope_key.to_owned(), access: access_clock },
            );
            return matching;
        }
        let first_slot = || "global-search-slot-0".to_owned();
        let slot = if state.scope_index.slots.len() < Self::MAXIMUM_SCOPE_COUNT {
            (0..Self::MAXIMUM_SCOPE_COUNT)
                .map(|index| format!("global-search-slot-{index}"))
                .find(|slot| !state.scope_index.slots.contains_key(slot))
                .unwrap_or_else(first_slot)
        } else {
            let evicted = state
                .scope_index
                .slots
                .iter()
                .min_by_key(|(_, slot)| slot.access)
                .map(|(key, slot)| (key.clone(), slot.scope_key.clone()));
            match evicted {
                Some((slot, evicted_scope_key)) => {
                    state.pages_by_scope_key.remove(&evicted_scope_key);
                    state.session_pages_by_scope_key.remove(&evicted_scope_key);
                    slot
                }
                None => first_slot(),
            }
        };
        state.scope_index.slots.insert(
            slot.clone(),
            RiseSearchRepositoryScopeSlot { scope_key: scope_key.to_owned(), access: access_clock },
        );
        slot
    }

    /// ticket for a load that never starts, screen stays as is and nothing goes out
    fn idle_ticket(
        &self,
        scope: &RiseGlobalSearchScope,
        slot: String,
        intent: RiseSearchRepositoryLoadIntent,
    ) -> RiseSearchRepositoryLoadTicket {
        RiseSearchRepositoryLoadTicket {
            scope: scope.clone(),
            slot,
            intent,
            generation: None,
            cursor: None,
            snapshot: self.state.with(|state| self.make_snapshot(state)),
        }
    }

    fn loading_page(
        current: RiseGlobalSearchPagePresentation,
        intent: RiseSearchRepositoryLoadIntent,
    ) -> RiseGlobalSearchPagePresentation {
        let has_items = !current.items.is_empty();
        let status = if has_items { RisePostPageStatus::Loaded } else { RisePostPageStatus::Loading };
        let is_load_more = intent == RiseSearchRepositoryLoadIntent::LoadMore;
        RiseGlobalSearchPagePresentation {
            items: current.items,
            status,
            error_message: None,
            has_more: current.has_more,
            is_refreshing: !is_load_more && has_items,
            is_loading_more: is_load_more,
            cache_source: current.cache_source,
            revision: current.revision,
        }
    }

    fn make_snapshot(&self, state: &mut RiseSearchRepositoryState) -> RiseSearchPresentationSnapshot {
        state.presentation_revision = (state.presentation_revision + 1).max(Self::now_milliseconds());
        RiseSearchPresentationSnapshot {
            account_id: self.account_id.clone(),
            account_epoch: self.account_epoch.clone(),
            session_id: self.session_id,
            revision: state.presentation_revision,
            pages_by_scope_key: state.pages_by_scope_key.clone(),
        }
    }

    fn advance_store_version(state: &mut RiseSearchRepositoryState) {
        state.store_version = (state.store_version + 1).max(Self::now_milliseconds());
    }

    fn is_current_generation(&self, key: &str, generation: u64) -> bool {
        self.state.with(|state| !state.is_shutdown && state.generations.get(key) == Some(&generation))
    }

    fn is_current_account(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(Some(&self.account_id), Some(&self.account_epoch))
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}
