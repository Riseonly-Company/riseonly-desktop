use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use gpui::AsyncApp;
use rise_canon::{RiseLimits, RiseTimingCanon, rise_request};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{RiseEngine, RiseWireError, RiseWireMethodDescriptor};
use rise_i18n::date_format::iso8601_date_formatter::ISO8601DateFormatter;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingPageExt};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::comment::engine::core::rise_comment_resource_repository::{
    RiseCommentResourceRepository, RiseCommentResourceSnapshot,
};
use crate::modules::comment::engine::core::rise_comment_rpc::{RiseCommentEndpoints, RiseCommentRPC};
use crate::modules::comment::engine::rise_comment_engine_models::{
    RiseCommentCacheSource, RiseCommentCreateDraft, RiseCommentCreateOutcome, RiseCommentError,
    RiseCommentListSnapshot, RiseCommentLoadState, RiseCommentScope, RiseCommentUserEntryContext,
};
use crate::modules::comment::engine::rise_comment_policies::{
    RiseCommentReactionSession, RiseCommentReactionSessionResolution, RiseCommentReactionSessionStep,
    RiseCommentReactionState, RiseCommentReplyPlacement,
};
use crate::modules::comment::engine::rise_comment_presentation::RiseCommentPresentation;
use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentEntities, CommentMedia, CommentReaction, CommentReactionCommand, CommentReactionEndpoint,
    CommentSortType, GetCommentsResponse, RepliesSortType, UserCommentEntry,
};
use crate::modules::comment::stores::comment_actions::comment_wire_types::{
    GetUserCommentsGatewayDTOExt, RiseCommentWireRow,
};
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

static REFRESH_INTERVAL_MS: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::freshness_ms("comments_list"));
static POST_PAGE_SIZE: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::page("comments_page").page_size_items());
static REPLIES_PAGE_SIZE: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::page("comment_replies_page").page_size_items());
static USER_PAGE_SIZE: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::page("user_comments_page").page_size_items());
static MEMORY_LISTS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::client_value("comment_lists_memory")).unwrap_or(0));
static MEMORY_ITEMS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::client_value("comment_list_memory_items")).unwrap_or(0));
static REPLY_PLACEMENTS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::client_value("comment_reply_placements_memory")).unwrap_or(0));

struct RiseCommentRepositoryListState {
    ids: Vec<String>,
    has_more: bool,
    relative_id: Option<String>,
    load_state: RiseCommentLoadState,
    error_text: Option<String>,
    is_loading_more: bool,
    is_refreshing: bool,
    is_private: bool,
    cache_source: RiseCommentCacheSource,
    last_refreshed_at_ms: i64,
    request_generation: u64,
    access: u64,
    did_read_store: bool,
    user_context: HashMap<String, RiseCommentUserEntryContext>,
    tail_pinned: HashSet<String>,
}

impl Default for RiseCommentRepositoryListState {
    fn default() -> Self {
        Self {
            ids: Vec::new(),
            has_more: false,
            relative_id: None,
            load_state: RiseCommentLoadState::Idle,
            error_text: None,
            is_loading_more: false,
            is_refreshing: false,
            is_private: false,
            cache_source: RiseCommentCacheSource::Miss,
            last_refreshed_at_ms: 0,
            request_generation: 0,
            access: 0,
            did_read_store: false,
            user_context: HashMap::new(),
            tail_pinned: HashSet::new(),
        }
    }
}

struct RiseCommentRepositoryPendingCreate {
    draft: RiseCommentCreateDraft,
    scope: RiseCommentScope,
    at_start: bool,
    is_sent: bool,
}

struct RiseCommentRepositoryPendingDelete {
    comment: GetCommentsResponse,
    positions: HashMap<RiseCommentScope, usize>,
    user_context: HashMap<RiseCommentScope, RiseCommentUserEntryContext>,
}

struct RiseCommentRepositoryPage {
    comments: Vec<GetCommentsResponse>,
    user_context: HashMap<String, RiseCommentUserEntryContext>,
    parents: Vec<GetCommentsResponse>,
    has_more: bool,
    relative_id: Option<String>,
    is_private: bool,
}

struct RiseCommentRepositoryState {
    entities: HashMap<String, GetCommentsResponse>,
    lists: HashMap<RiseCommentScope, RiseCommentRepositoryListState>,
    reaction_sessions: HashMap<String, RiseCommentReactionSession>,
    reaction_clock: i64,
    pending_creates: HashMap<String, RiseCommentRepositoryPendingCreate>,
    pending_deletes: HashMap<String, RiseCommentRepositoryPendingDelete>,
    reply_placement: RiseCommentReplyPlacement,
    revision: u64,
    access_clock: u64,
    is_shutdown: bool,
}

pub struct RiseCommentRepository {
    engine: Arc<RiseEngine>,
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    rpc: RiseCommentRPC,
    resource: Arc<RiseCommentResourceRepository>,
    state: RiseIsolated<RiseCommentRepositoryState>,
}

impl RiseCommentRepository {
    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Self {
        Self {
            rpc: RiseCommentRPC::new(engine.clone()),
            resource: Arc::new(RiseCommentResourceRepository::new(engine.clone(), &account_id)),
            engine,
            account_id,
            account_epoch,
            session_id,
            state: RiseIsolated::new(RiseCommentRepositoryState {
                entities: HashMap::new(),
                lists: HashMap::new(),
                reaction_sessions: HashMap::new(),
                reaction_clock: 0,
                pending_creates: HashMap::new(),
                pending_deletes: HashMap::new(),
                reply_placement: RiseCommentReplyPlacement::new(*REPLY_PLACEMENTS),
                revision: 0,
                access_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    /// returns the focused comment id only when the read was a focused one
    pub async fn open(
        &self,
        scope: &RiseCommentScope,
        force: bool,
        focus_comment_id: Option<&str>,
    ) -> Option<String> {
        let focus_id = focus_comment_id.and_then(|id| id.parse::<i64>().ok());
        let should_read_store = self.state.with(|s| {
            if s.is_shutdown || self.account_id.is_empty() {
                return None;
            }
            self.touch(s, scope);
            if focus_id.is_some() {
                return Some(false);
            }
            let Some(state) = s.lists.get_mut(scope) else {
                return Some(false);
            };
            if state.did_read_store {
                return Some(false);
            }
            state.did_read_store = true;
            Some(true)
        })?;

        if should_read_store && let Some(cached) = self.resource.cached(scope).await {
            self.state.with(|s| {
                if s.is_shutdown {
                    return;
                }
                let Some(state) = s.lists.get(scope) else {
                    return;
                };
                if state.ids.is_empty() && state.load_state != RiseCommentLoadState::Loaded {
                    self.seed(s, scope, cached);
                }
            });
        }

        let can_call = if focus_id.is_none() {
            self.engine.can_call(&Self::page_descriptor(scope)).await.ok()
        } else {
            Some(true)
        };
        let offline = if can_call == Some(false) { Self::is_socket_dropped().await } else { false };

        let generation = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            let now = Self::now_milliseconds();
            let state = s.lists.get_mut(scope)?;
            if focus_id.is_none()
                && !force
                && !state.is_private
                && state.load_state == RiseCommentLoadState::Loaded
                && state.last_refreshed_at_ms > 0
                && now - state.last_refreshed_at_ms < *REFRESH_INTERVAL_MS
            {
                return None;
            }

            if offline {
                if state.ids.is_empty() {
                    state.load_state = RiseCommentLoadState::Failed;
                }
                state.is_refreshing = false;
                state.error_text = Some(RiseWireError::NotConnected.to_string());
                self.publish(s, &Self::single(scope), Vec::new());
                return None;
            }

            state.request_generation = state.request_generation.wrapping_add(1);
            let generation = state.request_generation;
            if state.ids.is_empty() {
                state.load_state = RiseCommentLoadState::Loading;
            }
            state.is_refreshing = true;
            state.error_text = None;
            self.publish(s, &Self::single(scope), Vec::new());
            Some(generation)
        })?;

        if !Self::is_current_account(&self.account_id, &self.account_epoch) || self.is_shutdown() {
            return None;
        }

        match self.fetch(scope, None, focus_id).await {
            Ok(page) => self.state.with(|s| {
                if s.is_shutdown || s.lists.get(scope)?.request_generation != generation {
                    return None;
                }
                let RiseCommentRepositoryPage { comments, user_context, parents, has_more, relative_id, is_private } =
                    page;
                for parent in parents {
                    Self::upsert(s, parent);
                }
                let fresh_ids: Vec<String> = comments.iter().map(|comment| comment.id.clone()).collect();
                for comment in comments {
                    Self::upsert(s, comment);
                }
                let fresh: HashSet<String> = fresh_ids.iter().cloned().collect();
                let state = s.lists.get_mut(scope)?;
                let temps: Vec<String> =
                    state.ids.iter().filter(|id| s.pending_creates.contains_key(*id)).cloned().collect();
                let tail: Vec<String> = if focus_id.is_none() && has_more {
                    Self::rows_below_head(&state.ids, &fresh, scope)
                        .into_iter()
                        .filter(|id| !s.pending_creates.contains_key(id))
                        .collect()
                } else {
                    Vec::new()
                };
                let mut ids = fresh_ids.clone();
                ids.extend(tail.iter().cloned());
                let tail_set: HashSet<&String> = tail.iter().collect();
                state.tail_pinned.retain(|id| tail_set.contains(id));
                for temp_id in temps {
                    if s.pending_creates.get(&temp_id).is_some_and(|pending| pending.at_start) {
                        ids.insert(0, temp_id);
                    } else {
                        ids.push(temp_id);
                    }
                }
                state.ids = ids;
                state.user_context.retain(|id, _| tail_set.contains(id));
                state.user_context.extend(user_context);
                if tail.is_empty() {
                    state.has_more = has_more;
                    state.relative_id = relative_id;
                } else {
                    state.has_more = true;
                }
                state.is_private = is_private;
                state.load_state = RiseCommentLoadState::Loaded;
                state.is_refreshing = false;
                state.error_text = None;
                state.cache_source = RiseCommentCacheSource::Network;
                tracing::warn!(target: "riseonly::X1903", "head committed scope={scope:?} ids={}", state.ids.len());
                state.last_refreshed_at_ms = if focus_id.is_none() { Self::now_milliseconds() } else { 0 };
                Self::collect_garbage(s);
                self.publish(s, &Self::single(scope), Vec::new());
                if focus_id.is_none() {
                    self.persist(s, scope);
                    return None;
                }
                fresh_ids.first().cloned()
            }),
            Err(error) => {
                let text = error.to_string();
                self.state.with(|s| {
                    if s.is_shutdown {
                        return;
                    }
                    let Some(state) = s.lists.get_mut(scope) else {
                        return;
                    };
                    if state.request_generation != generation {
                        return;
                    }
                    if state.ids.is_empty() {
                        state.load_state = RiseCommentLoadState::Failed;
                    }
                    state.is_refreshing = false;
                    state.error_text = Some(text);
                    self.publish(s, &Self::single(scope), Vec::new());
                });
                None
            }
        }
    }

    pub async fn load_more(&self, scope: &RiseCommentScope) {
        let can_start = self.state.with(|s| {
            !s.is_shutdown
                && s.lists.get(scope).is_some_and(|current| {
                    current.has_more && !current.is_loading_more && current.load_state == RiseCommentLoadState::Loaded
                })
        });
        if !can_start {
            return;
        }
        if self.engine.can_call(&Self::page_descriptor(scope)).await.ok() == Some(false) {
            self.state.with(|s| {
                if s.is_shutdown {
                    return;
                }
                let Some(state) = s.lists.get_mut(scope) else {
                    return;
                };
                if state.is_loading_more || state.error_text.is_some() {
                    return;
                }
                state.error_text = Some(RiseWireError::NotConnected.to_string());
                self.publish(s, &Self::single(scope), Vec::new());
            });
            return;
        }

        let started = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            let state = s.lists.get(scope)?;
            if !state.has_more || state.is_loading_more || state.load_state != RiseCommentLoadState::Loaded {
                return None;
            }
            let last_server_id =
                state.ids.iter().rev().find(|id| !s.pending_creates.contains_key(*id)).cloned();
            let relative_id = state.relative_id.clone().or(last_server_id).filter(|id| !id.is_empty())?;

            self.touch(s, scope);
            let state = s.lists.get_mut(scope)?;
            state.is_loading_more = true;
            state.error_text = None;
            let generation = state.request_generation;
            self.publish(s, &Self::single(scope), Vec::new());
            Some((relative_id, generation))
        });
        let Some((relative_id, generation)) = started else {
            return;
        };

        match self.fetch(scope, Some(&relative_id), None).await {
            Ok(page) => self.state.with(|s| {
                if s.is_shutdown {
                    return;
                }
                let Some(state) = s.lists.get_mut(scope) else {
                    return;
                };
                if state.request_generation != generation {
                    state.is_loading_more = false;
                    return;
                }
                let RiseCommentRepositoryPage { comments, user_context, parents, has_more, relative_id, .. } = page;
                let reached: HashSet<String> =
                    comments.iter().filter(|c| state.tail_pinned.contains(&c.id)).map(|c| c.id.clone()).collect();
                if !reached.is_empty() {
                    state.ids.retain(|id| !reached.contains(id));
                    state.tail_pinned.retain(|id| !reached.contains(id));
                }
                let known: HashSet<String> = state.ids.iter().cloned().collect();
                for parent in parents {
                    Self::upsert(s, parent);
                }
                let mut appended: Vec<String> = Vec::new();
                for comment in comments {
                    if !known.contains(&comment.id) {
                        appended.push(comment.id.clone());
                    }
                    Self::upsert(s, comment);
                }
                let Some(state) = s.lists.get_mut(scope) else {
                    return;
                };
                let trailing_own = state
                    .ids
                    .iter()
                    .rev()
                    .take_while(|id| {
                        s.pending_creates.get(*id).is_some_and(|pending| !pending.at_start)
                            || state.tail_pinned.contains(*id)
                    })
                    .count();
                let trailing = state.ids.split_off(state.ids.len() - trailing_own);
                let next_relative_id = relative_id.or_else(|| appended.last().cloned());
                let reached_end = appended.is_empty();
                state.ids.extend(appended);
                state.ids.extend(trailing);
                state.user_context.extend(user_context);
                if next_relative_id.is_some() {
                    state.relative_id = next_relative_id;
                }
                state.has_more = has_more && !reached_end;
                state.is_loading_more = false;
                self.publish(s, &Self::single(scope), Vec::new());
            }),
            Err(error) => {
                let text = error.to_string();
                self.state.with(|s| {
                    if s.is_shutdown {
                        return;
                    }
                    let Some(state) = s.lists.get_mut(scope) else {
                        return;
                    };
                    state.is_loading_more = false;
                    state.error_text = Some(text);
                    self.publish(s, &Self::single(scope), Vec::new());
                });
            }
        }
    }

    /// reconnect, only lists on screen right now are worth a read
    pub async fn refresh(&self, scopes: &[RiseCommentScope]) {
        let mut post_ids: HashSet<String> = HashSet::new();
        for scope in scopes {
            let should_read = self.state.with(|s| {
                s.lists.get(scope).is_some_and(|state| {
                    state.load_state == RiseCommentLoadState::Loaded || state.load_state == RiseCommentLoadState::Failed
                })
            });
            if !should_read {
                continue;
            }
            self.open(scope, true, None).await;
            if let RiseCommentScope::Post { post_id, .. } = scope {
                post_ids.insert(post_id.clone());
            }
        }
        for post_id in post_ids {
            self.refresh_post_comments_count(post_id).await;
        }
    }

    async fn refresh_post_comments_count(&self, post_id: String) {
        if self.is_shutdown() || !post_id.parse::<i64>().is_ok_and(|value| value > 0) {
            return;
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    let refresh = cx.update(|cx| {
                        PostActionsStore::shared(cx)
                            .update(cx, |store, cx| store.refresh_post_detail_action(&post_id, cx))
                    });
                    refresh.await;
                    cx.update(|cx| {
                        PostServicesStore::shared(cx)
                            .update(cx, |store, cx| store.sync_selected_post_comment_state(&post_id, cx));
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    pub fn trim_unobserved(&self, scope: &RiseCommentScope) {
        self.state.with(|s| {
            if s.is_shutdown {
                return;
            }
            let Some(state) = s.lists.get_mut(scope) else {
                return;
            };
            if state.ids.len() <= *MEMORY_ITEMS
                || state.is_loading_more
                || state.is_refreshing
                || state.ids.iter().any(|id| s.pending_creates.contains_key(id))
            {
                return;
            }
            let pinned = std::mem::take(&mut state.tail_pinned);
            let kept: Vec<String> =
                state.ids.iter().filter(|id| !pinned.contains(*id)).take(*MEMORY_ITEMS).cloned().collect();
            state.ids = kept;
            let kept_set: HashSet<&String> = state.ids.iter().collect();
            state.user_context.retain(|id, _| kept_set.contains(id));
            state.relative_id = state.ids.last().cloned();
            state.has_more = true;
            Self::collect_garbage(s);
            self.publish(s, &Self::single(scope), Vec::new());
        });
    }

    /// draws the temporary row
    /// with send_now the request follows at once, a photo waits for complete_create after its upload
    pub async fn create(&self, draft: RiseCommentCreateDraft, send_now: bool) -> RiseCommentCreateOutcome {
        let parent_id = draft.parent_id.clone().filter(|parent_id| !parent_id.is_empty());
        let (scope, at_start) = match &parent_id {
            Some(parent_id) => (
                RiseCommentScope::Replies { root_id: parent_id.clone(), sort: draft.replies_sort },
                draft.replies_sort != RepliesSortType::Old,
            ),
            None => (
                RiseCommentScope::Post { post_id: draft.post_id.clone(), sort: draft.list_sort },
                draft.list_sort != CommentSortType::Old,
            ),
        };
        let temp_id = draft.temp_id.clone();
        let post_id = draft.post_id.clone();

        let changed = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            self.touch(s, &scope);

            let (entities, custom_emojis) = CommentEntities::normalized(draft.entities.as_deref());
            let now = ISO8601DateFormatter::new().string_from(&Date::now());
            let temp = GetCommentsResponse {
                id: draft.temp_id.clone(),
                content: draft.content.clone(),
                original_content: draft.content.clone(),
                created_at: now.clone(),
                updated_at: now,
                post_id: draft.post_id.clone(),
                author_id: draft.author.id.clone().unwrap_or_default(),
                parent_id: draft.parent_id.clone(),
                addressed_to_name: draft.reply_target_name.clone(),
                addressed_to_tag: draft.reply_target_tag.clone(),
                author: draft.author.clone(),
                media: draft.media.clone(),
                entities,
                custom_emojis,
                is_temp: true,
                ..GetCommentsResponse::default()
            };
            s.entities.insert(temp_id.clone(), temp);
            let target = draft
                .reply_target_id
                .as_deref()
                .map(trimming_whitespaces_and_newlines)
                .unwrap_or_default()
                .to_owned();
            s.pending_creates.insert(
                temp_id.clone(),
                RiseCommentRepositoryPendingCreate { draft, scope: scope.clone(), at_start, is_sent: false },
            );
            if let Some(state) = s.lists.get_mut(&scope)
                && !state.ids.contains(&temp_id)
            {
                if at_start {
                    state.ids.insert(0, temp_id.clone());
                } else {
                    state.ids.push(temp_id.clone());
                }
                if state.load_state == RiseCommentLoadState::Idle || state.load_state == RiseCommentLoadState::Failed {
                    state.load_state = RiseCommentLoadState::Loaded;
                }
            }

            let mut changed = Self::single(&scope);
            if let RiseCommentScope::Replies { root_id, .. } = &scope {
                let target_id = if target.is_empty() { root_id.as_str() } else { target.as_str() };
                s.reply_placement.remember(&temp_id, target_id);
                changed.extend(self.adjust_replies_count(s, root_id, 1));
            }
            Some(changed)
        });
        let Some(changed) = changed else {
            return RiseCommentCreateOutcome::Withdrawn;
        };
        if parent_id.is_none() {
            Self::adjust_post_comments_count(&post_id, 1).await;
        }
        self.state.with(|s| self.publish(s, &changed, Vec::new()));

        if !send_now {
            return RiseCommentCreateOutcome::Staged;
        }
        self.send(&temp_id).await
    }

    pub async fn complete_create(&self, temp_id: &str, media: CommentMedia) -> RiseCommentCreateOutcome {
        let is_pending = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            let Some(pending) = s.pending_creates.get_mut(temp_id) else {
                return false;
            };
            pending.draft.media = Some(media.clone());
            if let Some(entity) = s.entities.get_mut(temp_id) {
                entity.media = Some(media);
                let changed = Self::scopes_containing(s, temp_id);
                self.publish(s, &changed, Vec::new());
            }
            true
        });
        if !is_pending {
            return RiseCommentCreateOutcome::Withdrawn;
        }
        self.send(temp_id).await
    }

    pub async fn discard_create(&self, temp_id: &str) {
        let can_discard = self.state.with(|s| s.pending_creates.get(temp_id).is_some_and(|pending| !pending.is_sent));
        if can_discard {
            self.withdraw(temp_id).await;
        }
    }

    async fn send(&self, temp_id: &str) -> RiseCommentCreateOutcome {
        let draft = self.state.with(|s| {
            let pending = s.pending_creates.get_mut(temp_id)?;
            if pending.is_sent {
                return None;
            }
            pending.is_sent = true;
            Some(pending.draft.clone())
        });
        let Some(draft) = draft else {
            return RiseCommentCreateOutcome::Withdrawn;
        };

        let response = if Self::is_current_account(&self.account_id, &self.account_epoch) {
            self.rpc
                .create_comment(&rise_request::comment::CreateComment {
                    addressed_to_name: draft.reply_target_name.clone(),
                    addressed_to_tag: draft.reply_target_tag.clone(),
                    client_request_id: Some(temp_id.to_owned()),
                    content: Some(draft.content.clone()),
                    entities: draft.entities.clone(),
                    media: draft.media.as_ref().map(Into::into),
                    original_content: Some(draft.content.clone()),
                    parent_id: draft.parent_id.as_deref().and_then(|id| id.parse::<i64>().ok()),
                    post_id: draft.post_id.parse::<i64>().ok(),
                })
                .await
        } else {
            Err(RiseCommentError::StaleAccount.into())
        };

        let failure = match response {
            Ok(dto) => {
                if self.is_shutdown() {
                    return RiseCommentCreateOutcome::Withdrawn;
                }
                match dto.comment.as_ref().and_then(RiseCommentWireRow::as_comment) {
                    Some(real) => {
                        let server_id = real.id.clone();
                        self.state.with(|s| self.confirm(s, temp_id, real));
                        return RiseCommentCreateOutcome::Confirmed { temp_id: temp_id.to_owned(), server_id };
                    }
                    None => anyhow::Error::from(RiseCommentError::MissingCreateAcknowledgement),
                }
            }
            Err(error) => error,
        };
        if self.is_shutdown() {
            return RiseCommentCreateOutcome::Withdrawn;
        }
        match self.withdraw(temp_id).await {
            Some(failed) => RiseCommentCreateOutcome::Failed { draft: failed, refusal: CanonErrors::failure(&failure) },
            None => RiseCommentCreateOutcome::Withdrawn,
        }
    }

    fn confirm(&self, s: &mut RiseCommentRepositoryState, temp_id: &str, real: GetCommentsResponse) {
        let Some(pending) = s.pending_creates.remove(temp_id) else {
            return;
        };
        s.entities.remove(temp_id);
        let real_id = real.id.clone();
        Self::upsert(s, real);
        s.reply_placement.rename(temp_id, &real_id);

        let mut changed: HashSet<RiseCommentScope> = HashSet::new();
        for (scope, state) in s.lists.iter_mut() {
            let Some(index) = state.ids.iter().position(|id| id == temp_id) else {
                continue;
            };
            let paged_in = state.ids.contains(&real_id);
            state.ids.retain(|id| *id != real_id);
            let slot = index.min(state.ids.len());
            if slot < state.ids.len() && state.ids[slot] == temp_id {
                state.ids[slot] = real_id.clone();
            } else {
                state.ids.retain(|id| id != temp_id);
                let at = slot.min(state.ids.len());
                state.ids.insert(at, real_id.clone());
            }
            if !pending.at_start && !paged_in && state.has_more {
                state.tail_pinned.insert(real_id.clone());
            }
            changed.insert(scope.clone());
        }
        if changed.is_empty()
            && let Some(state) = s.lists.get_mut(&pending.scope)
        {
            if pending.at_start {
                state.ids.insert(0, real_id);
            } else {
                state.ids.push(real_id.clone());
                if state.has_more {
                    state.tail_pinned.insert(real_id);
                }
            }
            changed.insert(pending.scope.clone());
        }
        self.publish(s, &changed, Vec::new());
        self.persist(s, &pending.scope);
    }

    async fn withdraw(&self, temp_id: &str) -> Option<RiseCommentCreateDraft> {
        let (pending, changed) = self.state.with(|s| {
            let pending = s.pending_creates.remove(temp_id)?;
            s.entities.remove(temp_id);
            s.reply_placement.forget(temp_id);
            let mut changed = Self::remove_from_lists(s, temp_id);
            if let RiseCommentScope::Replies { root_id, .. } = &pending.scope {
                changed.extend(self.adjust_replies_count(s, root_id, -1));
            }
            Some((pending, changed))
        })?;
        if !matches!(pending.scope, RiseCommentScope::Replies { .. }) {
            Self::adjust_post_comments_count(&pending.draft.post_id, -1).await;
        }
        self.state.with(|s| self.publish(s, &changed, Vec::new()));
        Some(pending.draft)
    }

    pub async fn delete(&self, comment_id: &str) {
        let started = self.state.with(|s| {
            if s.is_shutdown || s.pending_deletes.contains_key(comment_id) {
                return None;
            }
            let comment = s.entities.get(comment_id).filter(|comment| !comment.is_temp)?.clone();

            let mut positions: HashMap<RiseCommentScope, usize> = HashMap::new();
            let mut contexts: HashMap<RiseCommentScope, RiseCommentUserEntryContext> = HashMap::new();
            for (scope, state) in &s.lists {
                let Some(index) = state.ids.iter().position(|id| id == comment_id) else {
                    continue;
                };
                positions.insert(scope.clone(), index);
                if let Some(context) = state.user_context.get(comment_id) {
                    contexts.insert(scope.clone(), context.clone());
                }
            }
            if positions.is_empty() {
                return None;
            }
            let scopes: Vec<RiseCommentScope> = positions.keys().cloned().collect();
            s.pending_deletes.insert(
                comment_id.to_owned(),
                RiseCommentRepositoryPendingDelete { comment: comment.clone(), positions, user_context: contexts },
            );
            s.reaction_sessions.remove(comment_id);
            let changed = Self::remove_from_lists(s, comment_id);
            Some((comment, scopes, changed))
        });
        let Some((comment, scopes, mut changed)) = started else {
            return;
        };
        changed.extend(self.adjust_counts(&comment, -1).await);
        self.state.with(|s| self.publish(s, &changed, Vec::new()));

        let result = self
            .rpc
            .delete_comment(&rise_request::comment::DeleteComment { comment_id: comment_id.parse::<i64>().ok() })
            .await;
        match result {
            Ok(_) => self.state.with(|s| {
                if s.is_shutdown {
                    return;
                }
                s.pending_deletes.remove(comment_id);
                s.entities.remove(comment_id);
                Self::collect_garbage(s);
                for scope in &scopes {
                    self.persist(s, scope);
                }
            }),
            Err(_) => {
                let restored = self.state.with(|s| {
                    if s.is_shutdown {
                        return None;
                    }
                    let pending = s.pending_deletes.remove(comment_id)?;
                    s.entities.insert(comment_id.to_owned(), pending.comment.clone());
                    let mut restored: HashSet<RiseCommentScope> = HashSet::new();
                    for (scope, index) in &pending.positions {
                        let Some(state) = s.lists.get_mut(scope) else {
                            continue;
                        };
                        if state.ids.iter().any(|id| id == comment_id) {
                            continue;
                        }
                        let at = (*index).min(state.ids.len());
                        state.ids.insert(at, comment_id.to_owned());
                        if let Some(context) = pending.user_context.get(scope) {
                            state.user_context.insert(comment_id.to_owned(), context.clone());
                        }
                        restored.insert(scope.clone());
                    }
                    Some((pending.comment, restored))
                });
                let Some((comment, mut restored)) = restored else {
                    return;
                };
                restored.extend(self.adjust_counts(&comment, 1).await);
                self.state.with(|s| self.publish(s, &restored, Vec::new()));
            }
        }
    }

    pub fn set_desired_reaction(&self, comment_id: &str, desired: CommentReaction) {
        self.state.with(|s| {
            if s.is_shutdown {
                return;
            }
            let Some(comment) = s.entities.get(comment_id) else {
                return;
            };
            if comment.is_temp || comment.is_moderation_disabled() {
                return;
            }
            let reaction = comment.reaction();
            let likes_count = comment.likes_count;
            let dislikes_count = comment.dislikes_count;
            s.reaction_clock = s.reaction_clock.wrapping_add(1);
            let generation = s.reaction_clock;
            let session = s.reaction_sessions.entry(comment_id.to_owned()).or_insert_with(|| {
                RiseCommentReactionSession {
                    base: RiseCommentReactionState { reaction, likes_count, dislikes_count },
                    desired: reaction,
                    generation: 0,
                    inflight_generation: None,
                }
            });
            session.desired = desired;
            session.generation = generation;
            let derived = session.derived();
            self.apply(s, &derived, comment_id);
        });
    }

    pub async fn flush_reaction(&self, comment_id: &str) {
        loop {
            let command = self.state.with(|s| {
                if s.is_shutdown {
                    return None;
                }
                let step = s.reaction_sessions.get_mut(comment_id)?.next_step(comment_id);
                match step {
                    RiseCommentReactionSessionStep::Wait => None,
                    RiseCommentReactionSessionStep::Settle(state) => {
                        s.reaction_sessions.remove(comment_id);
                        self.apply(s, &state, comment_id);
                        None
                    }
                    RiseCommentReactionSessionStep::Send(command) => Some(command),
                }
            });
            let Some(command) = command else {
                return;
            };
            if !self.perform(&command).await {
                return;
            }
        }
    }

    /// whether the session goes on with a newer tap
    async fn perform(&self, command: &CommentReactionCommand) -> bool {
        let acknowledged = self.send_reaction(command).await;
        self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            let Some(session) = s.reaction_sessions.get_mut(&command.comment_id) else {
                return false;
            };
            let resolution = match acknowledged {
                Ok(state) => session.acknowledge(command.generation, state),
                Err(_) => session.fail(command.generation),
            };
            match resolution {
                RiseCommentReactionSessionResolution::Ignored => false,
                RiseCommentReactionSessionResolution::Finished(state) => {
                    s.reaction_sessions.remove(&command.comment_id);
                    self.apply(s, &state, &command.comment_id);
                    false
                }
                RiseCommentReactionSessionResolution::Continues => {
                    let derived = session.derived();
                    self.apply(s, &derived, &command.comment_id);
                    true
                }
            }
        })
    }

    async fn send_reaction(&self, command: &CommentReactionCommand) -> anyhow::Result<RiseCommentReactionState> {
        let comment_id = command.comment_id.parse::<i64>().ok();
        let (liked, disliked, likes, dislikes) = match command.endpoint {
            CommentReactionEndpoint::Like => {
                let dto = self
                    .rpc
                    .like_comment(&rise_request::comment::LikeComment {
                        comment_id,
                        set_liked: Some(command.enabled),
                    })
                    .await?;
                (dto.is_liked, dto.is_disliked, dto.likes_count, dto.dislikes_count)
            }
            CommentReactionEndpoint::Dislike => {
                let dto = self
                    .rpc
                    .dislike_comment(&rise_request::comment::DislikeComment {
                        comment_id,
                        set_disliked: Some(command.enabled),
                    })
                    .await?;
                (dto.is_liked, dto.is_disliked, dto.likes_count, dto.dislikes_count)
            }
        };
        if liked && disliked {
            return Err(RiseCommentError::InvalidReactionAcknowledgement.into());
        }
        let reaction = if liked {
            CommentReaction::Like
        } else if disliked {
            CommentReaction::Dislike
        } else {
            CommentReaction::None
        };
        Ok(RiseCommentReactionState { reaction, likes_count: likes.max(0), dislikes_count: dislikes.max(0) })
    }

    fn apply(&self, s: &mut RiseCommentRepositoryState, state: &RiseCommentReactionState, comment_id: &str) {
        let Some(current) = s.entities.get(comment_id) else {
            return;
        };
        let mut comment = current.clone();
        comment.user_liked = state.reaction == CommentReaction::Like;
        comment.user_disliked = state.reaction == CommentReaction::Dislike;
        comment.likes_count = state.likes_count.max(0);
        comment.dislikes_count = state.dislikes_count.max(0);
        if comment == *current {
            return;
        }
        s.entities.insert(comment_id.to_owned(), comment.clone());
        let changed = Self::scopes_containing(s, comment_id);
        self.publish(s, &changed, Vec::new());
        self.notify_changed(vec![comment]);
    }

    pub async fn apply_remote_delete(&self, comment_id: &str, post_id: &str, parent_id: Option<&str>) {
        let removal = self.state.with(|s| {
            if s.is_shutdown || comment_id.is_empty() {
                return None;
            }
            s.pending_deletes.remove(comment_id);
            s.reaction_sessions.remove(comment_id);
            let removed_from = Self::remove_from_lists(s, comment_id);
            let entity = s.entities.remove(comment_id);
            if removed_from.is_empty() {
                return None;
            }
            let mut probe = entity
                .unwrap_or_else(|| GetCommentsResponse { id: comment_id.to_owned(), ..GetCommentsResponse::default() });
            if probe.post_id.is_empty() {
                probe.post_id = post_id.to_owned();
            }
            if probe.parent_id.is_none() {
                probe.parent_id = parent_id.map(str::to_owned);
            }
            Some((removed_from, probe))
        });
        let Some((removed_from, probe)) = removal else {
            return;
        };
        let mut changed = removed_from.clone();
        changed.extend(self.adjust_counts(&probe, -1).await);
        self.state.with(|s| {
            Self::collect_garbage(s);
            self.publish(s, &changed, Vec::new());
            for scope in &removed_from {
                self.persist(s, scope);
            }
        });
    }

    pub async fn apply_remote_create(&self, incoming: GetCommentsResponse) {
        let changed = self.state.with(|s| {
            if s.is_shutdown
                || incoming.id.is_empty()
                || s.entities.contains_key(&incoming.id)
                || s.pending_deletes.contains_key(&incoming.id)
            {
                return None;
            }
            if incoming.author_id == self.account_id && !s.pending_creates.is_empty() {
                return None;
            }
            let root_id = incoming.parent_id.as_deref().filter(|parent_id| !parent_id.is_empty());
            let mut changed: HashSet<RiseCommentScope> = HashSet::new();
            for (scope, state) in s.lists.iter_mut() {
                let at_start = match scope {
                    RiseCommentScope::Post { post_id, sort } => {
                        if root_id.is_some() || *post_id != incoming.post_id {
                            continue;
                        }
                        match sort {
                            CommentSortType::New | CommentSortType::Feed => true,
                            CommentSortType::Old => false,
                            CommentSortType::My => continue,
                        }
                    }
                    RiseCommentScope::Replies { root_id: list_root_id, sort } => {
                        if root_id != Some(list_root_id.as_str()) {
                            continue;
                        }
                        match sort {
                            RepliesSortType::New | RepliesSortType::Popular => true,
                            RepliesSortType::Old => false,
                            RepliesSortType::My => continue,
                        }
                    }
                    RiseCommentScope::User { .. } => continue,
                };
                if state.ids.contains(&incoming.id) {
                    continue;
                }
                if at_start {
                    state.ids.insert(0, incoming.id.clone());
                } else {
                    if state.has_more {
                        continue;
                    }
                    state.ids.push(incoming.id.clone());
                }
                changed.insert(scope.clone());
            }
            if changed.is_empty() {
                return None;
            }
            Self::upsert(s, incoming.clone());
            Some(changed)
        });
        let Some(mut changed) = changed else {
            return;
        };
        changed.extend(self.adjust_counts(&incoming, 1).await);
        self.state.with(|s| {
            self.publish(s, &changed, Vec::new());
            for scope in &changed {
                self.persist(s, scope);
            }
        });
    }

    pub fn apply_remote_update(
        &self,
        comment_id: &str,
        content: Option<String>,
        original_content: Option<String>,
        updated_at: Option<String>,
        raw_entities: Option<Option<String>>,
    ) {
        self.state.with(|s| {
            if s.is_shutdown {
                return;
            }
            let Some(current) = s.entities.get(comment_id) else {
                return;
            };
            let mut comment = current.clone();
            if let Some(content) = content {
                comment.content = content;
            }
            if let Some(original_content) = original_content {
                comment.original_content = original_content;
            }
            if let Some(updated_at) = updated_at {
                comment.updated_at = updated_at;
            }
            if let Some(raw_entities) = raw_entities {
                let (entities, custom_emojis) = CommentEntities::normalized(raw_entities.as_deref());
                comment.entities = entities;
                comment.custom_emojis = custom_emojis;
            }
            if comment == *current {
                return;
            }
            s.entities.insert(comment_id.to_owned(), comment.clone());
            let changed = Self::scopes_containing(s, comment_id);
            self.publish(s, &changed, Vec::new());
            self.notify_changed(vec![comment]);
            for scope in &changed {
                self.persist(s, scope);
            }
        });
    }

    pub fn apply_moderation_status(&self, comment_id: &str, status: &str) {
        self.state.with(|s| {
            if s.is_shutdown {
                return;
            }
            let Some(comment) = s.entities.get_mut(comment_id) else {
                return;
            };
            if comment.moderation_status == status {
                return;
            }
            comment.moderation_status = status.to_owned();
            let comment = comment.clone();
            let changed = Self::scopes_containing(s, comment_id);
            self.publish(s, &changed, Vec::new());
            self.notify_changed(vec![comment]);
            for scope in &changed {
                self.persist(s, scope);
            }
        });
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.entities.clear();
            s.lists.clear();
            s.reaction_sessions.clear();
            s.pending_creates.clear();
            s.pending_deletes.clear();
        });
        self.resource.shutdown();
    }

    fn page_descriptor(scope: &RiseCommentScope) -> RiseWireMethodDescriptor {
        match scope {
            RiseCommentScope::Post { .. } => RiseCommentEndpoints::get_comments().descriptor(),
            RiseCommentScope::Replies { .. } => RiseCommentEndpoints::get_comment_replies().descriptor(),
            RiseCommentScope::User { .. } => RiseCommentEndpoints::get_user_comments().descriptor(),
        }
    }

    async fn fetch(
        &self,
        scope: &RiseCommentScope,
        relative_id: Option<&str>,
        focus_id: Option<i64>,
    ) -> anyhow::Result<RiseCommentRepositoryPage> {
        let wire_relative_id = relative_id.and_then(|id| id.parse::<i64>().ok());
        match scope {
            RiseCommentScope::Post { post_id, sort } => {
                let dto = self
                    .rpc
                    .get_comments(&rise_request::comment::GetComments {
                        focus_comment_id: focus_id,
                        limit: Some(*POST_PAGE_SIZE),
                        post_id: post_id.parse::<i64>().ok(),
                        relative_id: wire_relative_id,
                        sort: Some(sort.raw_value().to_owned()),
                        up: Some(false),
                        ..rise_request::comment::GetComments::default()
                    })
                    .await
                    .inspect_err(|error| {
                        tracing::warn!(target: "riseonly::X1903", "post page failed post={post_id} error={error:#}");
                    })?;
                let comments: Vec<GetCommentsResponse> =
                    dto.items.iter().filter_map(RiseCommentWireRow::as_comment).collect();
                tracing::warn!(
                    target: "riseonly::X1903",
                    "post page post={post_id} relative={wire_relative_id:?} focus={focus_id:?} raw={} mapped={} deleted_authors={} has_more={}",
                    dto.items.len(),
                    comments.len(),
                    comments.iter().filter(|comment| comment.author.is_deleted).count(),
                    dto.has_more
                );
                let relative_id = dto
                    .relative_id
                    .map(|value| value.to_string())
                    .or_else(|| comments.last().map(|comment| comment.id.clone()));
                Ok(RiseCommentRepositoryPage {
                    comments,
                    user_context: HashMap::new(),
                    parents: Vec::new(),
                    has_more: dto.has_more,
                    relative_id,
                    is_private: false,
                })
            }
            RiseCommentScope::Replies { root_id, sort } => {
                let dto = self
                    .rpc
                    .get_comment_replies(&rise_request::comment::GetCommentReplies {
                        comment_id: root_id.parse::<i64>().ok(),
                        limit: Some(*REPLIES_PAGE_SIZE),
                        relative_id: wire_relative_id,
                        sort: Some(sort.raw_value().to_owned()),
                        up: Some(false),
                    })
                    .await?;
                let comments: Vec<GetCommentsResponse> =
                    dto.items.iter().filter_map(RiseCommentWireRow::as_comment).collect();
                let relative_id = dto
                    .relative_id
                    .map(|value| value.to_string())
                    .or_else(|| comments.last().map(|comment| comment.id.clone()));
                Ok(RiseCommentRepositoryPage {
                    comments,
                    user_context: HashMap::new(),
                    parents: Vec::new(),
                    has_more: dto.has_more || dto.is_have_more,
                    relative_id,
                    is_private: false,
                })
            }
            RiseCommentScope::User { user_id } => {
                let dto = self
                    .rpc
                    .get_user_comments(&rise_request::comment::GetUserComments {
                        limit: Some(*USER_PAGE_SIZE),
                        relative_id: wire_relative_id,
                        target_user_id: Some(user_id.clone()),
                    })
                    .await?;
                let entries = dto.as_user_comment_entries();
                let relative_id = dto
                    .relative_id
                    .map(|value| value.to_string())
                    .or_else(|| entries.last().map(|entry| entry.comment.id.clone()));
                let mut comments = Vec::with_capacity(entries.len());
                let mut parents = Vec::new();
                let mut user_context = HashMap::with_capacity(entries.len());
                for entry in entries {
                    user_context.insert(
                        entry.comment.id.clone(),
                        RiseCommentUserEntryContext {
                            post: entry.post,
                            post_card: entry.post_card,
                            parent_id: entry.parent.as_ref().map(|parent| parent.id.clone()),
                        },
                    );
                    comments.push(entry.comment);
                    parents.extend(entry.parent);
                }
                Ok(RiseCommentRepositoryPage {
                    comments,
                    user_context,
                    parents,
                    has_more: dto.has_more,
                    relative_id,
                    is_private: dto.is_private,
                })
            }
        }
    }

    /// when a server copy meets an open reaction session the server state becomes its base
    /// and the row keeps showing the desired reaction
    fn upsert(s: &mut RiseCommentRepositoryState, incoming: GetCommentsResponse) {
        if s.pending_deletes.contains_key(&incoming.id) {
            return;
        }
        let mut comment = incoming;
        if let Some(session) = s.reaction_sessions.get_mut(&comment.id) {
            session.base = RiseCommentReactionState {
                reaction: comment.reaction(),
                likes_count: comment.likes_count,
                dislikes_count: comment.dislikes_count,
            };
            let derived = session.derived();
            comment.user_liked = derived.reaction == CommentReaction::Like;
            comment.user_disliked = derived.reaction == CommentReaction::Dislike;
            comment.likes_count = derived.likes_count;
            comment.dislikes_count = derived.dislikes_count;
        }
        s.entities.insert(comment.id.clone(), comment);
    }

    fn rows_below_head(ids: &[String], fresh: &HashSet<String>, scope: &RiseCommentScope) -> Vec<String> {
        let is_time_ordered = match scope {
            RiseCommentScope::Post { sort, .. } => *sort != CommentSortType::My,
            RiseCommentScope::Replies { sort, .. } => *sort == RepliesSortType::New || *sort == RepliesSortType::Old,
            RiseCommentScope::User { .. } => true,
        };
        if !is_time_ordered {
            return ids.iter().filter(|id| !fresh.contains(*id)).cloned().collect();
        }
        let Some(last_fresh) = ids.iter().rposition(|id| fresh.contains(id)) else {
            return Vec::new();
        };
        ids[(last_fresh + 1)..].iter().filter(|id| !fresh.contains(*id)).cloned().collect()
    }

    fn seed(&self, s: &mut RiseCommentRepositoryState, scope: &RiseCommentScope, cached: RiseCommentResourceSnapshot) {
        let mut ids: Vec<String> = Vec::with_capacity(cached.records.len());
        for record in &cached.records {
            if s.pending_deletes.contains_key(&record.id) {
                continue;
            }
            s.entities.entry(record.id.clone()).or_insert_with(|| record.as_comment());
            ids.push(record.id.clone());
        }
        if ids.is_empty() {
            return;
        }
        let Some(state) = s.lists.get_mut(scope) else {
            return;
        };
        state.relative_id = ids.last().cloned();
        state.ids = ids;
        state.has_more = cached.has_more;
        state.load_state = RiseCommentLoadState::Loaded;
        state.cache_source = cached.source;
        self.publish(s, &Self::single(scope), Vec::new());
    }

    fn touch(&self, s: &mut RiseCommentRepositoryState, scope: &RiseCommentScope) {
        s.access_clock = s.access_clock.wrapping_add(1);
        let access = s.access_clock;
        let state = s.lists.entry(scope.clone()).or_default();
        if state.cache_source == RiseCommentCacheSource::Miss && !state.ids.is_empty() {
            state.cache_source = RiseCommentCacheSource::Memory;
        }
        state.access = access;
        self.evict_if_needed(s, scope);
    }

    fn evict_if_needed(&self, s: &mut RiseCommentRepositoryState, protecting: &RiseCommentScope) {
        if s.lists.len() <= *MEMORY_LISTS {
            return;
        }
        let busy: HashSet<&RiseCommentScope> = s.pending_creates.values().map(|pending| &pending.scope).collect();
        let mut candidates: Vec<(RiseCommentScope, u64)> = s
            .lists
            .iter()
            .filter(|(key, _)| *key != protecting && !busy.contains(*key))
            .map(|(key, state)| (key.clone(), state.access))
            .collect();
        candidates.sort_by_key(|(_, access)| *access);
        let mut evicted: Vec<RiseCommentScope> = Vec::new();
        for (key, _) in candidates {
            if s.lists.len() <= *MEMORY_LISTS {
                break;
            }
            s.lists.remove(&key);
            evicted.push(key);
        }
        if evicted.is_empty() {
            return;
        }
        Self::collect_garbage(s);
        self.publish(s, &HashSet::new(), evicted);
    }

    /// drops entities that no list or pending write or profile context points at anymore
    fn collect_garbage(s: &mut RiseCommentRepositoryState) {
        let mut live: HashSet<String> = s.pending_creates.keys().cloned().collect();
        live.extend(s.pending_deletes.keys().cloned());
        for state in s.lists.values() {
            live.extend(state.ids.iter().cloned());
            for context in state.user_context.values() {
                if let Some(parent_id) = &context.parent_id {
                    live.insert(parent_id.clone());
                }
            }
        }
        for scope in s.lists.keys() {
            if let RiseCommentScope::Replies { root_id, .. } = scope {
                live.insert(root_id.clone());
            }
        }
        s.entities.retain(|id, _| live.contains(id));
        s.reaction_sessions.retain(|id, _| live.contains(id));
    }

    fn scopes_containing(s: &RiseCommentRepositoryState, comment_id: &str) -> HashSet<RiseCommentScope> {
        let mut result: HashSet<RiseCommentScope> = HashSet::new();
        for (scope, state) in &s.lists {
            if state.ids.iter().any(|id| id == comment_id)
                || state.user_context.values().any(|context| context.parent_id.as_deref() == Some(comment_id))
            {
                result.insert(scope.clone());
            }
        }
        result
    }

    fn remove_from_lists(s: &mut RiseCommentRepositoryState, comment_id: &str) -> HashSet<RiseCommentScope> {
        let mut changed: HashSet<RiseCommentScope> = HashSet::new();
        for (scope, state) in s.lists.iter_mut() {
            if !state.ids.iter().any(|id| id == comment_id) {
                continue;
            }
            state.ids.retain(|id| id != comment_id);
            state.user_context.remove(comment_id);
            state.tail_pinned.remove(comment_id);
            changed.insert(scope.clone());
        }
        changed
    }

    fn adjust_replies_count(
        &self,
        s: &mut RiseCommentRepositoryState,
        root_id: &str,
        delta: i64,
    ) -> HashSet<RiseCommentScope> {
        let Some(root) = s.entities.get_mut(root_id) else {
            return HashSet::new();
        };
        root.replies_count = (root.replies_count + delta).max(0);
        self.notify_changed(vec![root.clone()]);
        Self::scopes_containing(s, root_id)
    }

    async fn adjust_counts(&self, comment: &GetCommentsResponse, delta: i64) -> HashSet<RiseCommentScope> {
        if let Some(root_id) = comment.parent_id.as_deref().filter(|root_id| !root_id.is_empty()) {
            return self.state.with(|s| self.adjust_replies_count(s, root_id, delta));
        }
        if !comment.post_id.is_empty() {
            Self::adjust_post_comments_count(&comment.post_id, delta).await;
        }
        HashSet::new()
    }

    /// post counter belongs to the post domain so this is an intent to its facade not a write
    /// into its data
    async fn adjust_post_comments_count(post_id: &str, delta: i64) {
        if post_id.is_empty() {
            return;
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let post_id = post_id.to_owned();
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        PostServicesStore::shared(cx)
                            .update(cx, |store, cx| store.adjust_comments_count(&post_id, delta, cx));
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    /// the open thread keeps a copy of its root for the header and follows the entity table
    /// through this hop
    fn notify_changed(&self, comments: Vec<GetCommentsResponse>) {
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        main.post(move |cx| {
            if !Self::is_current_account(&account_id, &account_epoch) {
                return;
            }
            CommentInteractionsStore::shared(cx).update(cx, |store, cx| store.comments_did_change(&comments, cx));
        });
    }

    fn snapshot(s: &RiseCommentRepositoryState, scope: &RiseCommentScope) -> Option<RiseCommentListSnapshot> {
        let state = s.lists.get(scope)?;
        let arranged;
        let ids: &[String] = if let RiseCommentScope::Replies { root_id, sort: RepliesSortType::Old } = scope {
            arranged = s.reply_placement.arranged(&state.ids, root_id);
            &arranged
        } else {
            &state.ids
        };
        let items: Vec<GetCommentsResponse> = ids.iter().filter_map(|id| s.entities.get(id).cloned()).collect();
        let user_entries: Vec<UserCommentEntry> = if matches!(scope, RiseCommentScope::User { .. }) {
            items
                .iter()
                .map(|comment| {
                    let context = state.user_context.get(&comment.id);
                    UserCommentEntry {
                        comment: comment.clone(),
                        post: context.and_then(|context| context.post.clone()),
                        post_card: context.and_then(|context| context.post_card.clone()),
                        parent: context
                            .and_then(|context| context.parent_id.as_ref())
                            .and_then(|parent_id| s.entities.get(parent_id).cloned()),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        Some(RiseCommentListSnapshot {
            scope: scope.clone(),
            items,
            user_entries,
            has_more: state.has_more,
            is_loading_more: state.is_loading_more,
            is_refreshing: state.is_refreshing,
            is_private: state.is_private,
            load_state: state.load_state,
            error_text: state.error_text.clone(),
            cache_source: state.cache_source,
            revision: s.revision,
        })
    }

    /// snapshots are built on the repository executor and handed to the main thread in one commit
    /// commits may land out of order so the presentation refuses an older revision
    fn publish(
        &self,
        s: &mut RiseCommentRepositoryState,
        scopes: &HashSet<RiseCommentScope>,
        evicted: Vec<RiseCommentScope>,
    ) {
        if s.is_shutdown || (scopes.is_empty() && evicted.is_empty()) {
            return;
        }
        s.revision = s.revision.wrapping_add(1);
        let snapshots: Vec<RiseCommentListSnapshot> =
            scopes.iter().filter_map(|scope| Self::snapshot(s, scope)).collect();
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        main.post(move |cx| {
            RiseCommentPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(snapshots, evicted, &account_id, &account_epoch, session_id, cx);
            });
        });
    }

    fn persist(&self, s: &RiseCommentRepositoryState, scope: &RiseCommentScope) {
        if !scope.is_persisted() {
            return;
        }
        let Some(state) = s.lists.get(scope) else {
            return;
        };
        let comments: Vec<GetCommentsResponse> = state
            .ids
            .iter()
            .filter(|id| !state.tail_pinned.contains(*id))
            .filter_map(|id| s.entities.get(id).cloned())
            .collect();
        let has_more = state.has_more;
        let resource = self.resource.clone();
        let scope = scope.clone();
        RiseAccountRuntime::current().spawn(async move {
            resource.store(&scope, &comments, has_more).await;
        });
    }

    fn single(scope: &RiseCommentScope) -> HashSet<RiseCommentScope> {
        HashSet::from([scope.clone()])
    }

    fn is_shutdown(&self) -> bool {
        self.state.with(|s| s.is_shutdown)
    }

    fn is_current_account(account_id: &str, account_epoch: &str) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(Some(account_id), Some(account_epoch))
    }

    async fn is_socket_dropped() -> bool {
        let Some(main) = RiseEngineMainHop::installed() else {
            return false;
        };
        main.run(|cx: AsyncApp| {
            async move {
                Ok(cx.update(|cx| {
                    RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.is_socket_dropped(cx))
                }))
            }
            .boxed_local()
        })
        .await
        .unwrap_or(false)
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
