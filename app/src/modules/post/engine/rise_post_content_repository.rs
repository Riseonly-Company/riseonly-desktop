use rise_engine::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, LazyLock, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::channel::oneshot;
use gpui::{App, AsyncApp};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_canon::RiseEvent;
use rise_canon::rise_request::post::AcceptCoauthorInvite as AcceptCoauthorInviteRequest;
use rise_canon::rise_request::post::CreatePost as CreatePostRequest;
use rise_canon::rise_request::post::DeclineCoauthorInvite as DeclineCoauthorInviteRequest;
use rise_canon::rise_request::post::GetPostCollaborators as GetPostCollaboratorsRequest;
use rise_canon::rise_request::post::InviteCoauthors as InviteCoauthorsRequest;
use rise_canon::rise_request::post::RemoveCoauthor as RemoveCoauthorRequest;
use rise_canon::rise_request::post::RemovePostTag as RemovePostTagRequest;
use rise_canon::rise_request::post::SetCoauthorGridVisibility as SetCoauthorGridVisibilityRequest;
use rise_canon::rise_request::post::SetPostTags as SetPostTagsRequest;
use rise_canon::rise_request::post::set_post_tags::TaggedUsersElement as SetPostTagsTaggedUsersElement;
use rise_canon::rise_request::post::create_post::TaggedUsersElement as CreatePostTaggedUsersElement;
use rise_canon::rise_request::post::CreateRepost as CreateRepostRequest;
use rise_canon::rise_request::post::DeletePost as DeletePostRequest;
use rise_canon::rise_request::post::DeleteRepost as DeleteRepostRequest;
use rise_canon::rise_request::post::UpdatePost as UpdatePostRequest;
use rise_canon::rise_request::post::GetFeed as GetFeedRequest;
use rise_canon::rise_request::post::GetPost as GetPostRequest;
use rise_canon::rise_request::post::GetShortsFeed as GetShortsFeedRequest;
use rise_canon::rise_request::post::GetUserMentions as GetUserMentionsRequest;
use rise_canon::rise_request::post::GetUserPosts as GetUserPostsRequest;
use rise_canon::rise_request::post::GetUserReposts as GetUserRepostsRequest;
use rise_canon::rise_request::post::SearchPosts as SearchPostsRequest;
use rise_canon::rise_request::post::SearchPostsByHashtag as SearchPostsByHashtagRequest;
use rise_canon::rise_request::post::SetPostFavorite as SetPostFavoriteRequest;
use rise_canon::rise_request::post::SetPostLike as SetPostLikeRequest;
use rise_canon::rise_request::post::GetPostSparks as GetPostSparksRequest;
use rise_canon::rise_request::post::SendPostSparks as SendPostSparksRequest;
use rise_canon::rise_key::post as post_key;
use rise_canon::rise_request::post::UpdateRepost as UpdateRepostRequest;
use rise_canon::rise_request::post::update_post::TaggedUsersElement as UpdatePostTaggedUsersElement;
use rise_canon::rise_response::post::CreatePost as CreatePostResponse;
use rise_canon::rise_response::post::CreateRepost as CreateRepostResponse;
use rise_canon::rise_response::post::DeletePost as DeletePostResponse;
use rise_canon::rise_response::post::DeleteRepost as DeleteRepostResponse;
use rise_canon::rise_response::post::GetPostCollaborators as GetPostCollaboratorsResponse;
use rise_canon::rise_response::post::UpdatePost as UpdatePostResponse;
use rise_canon::rise_response::post::UpdateRepost as UpdateRepostResponse;
use rise_canon::rise_response::post::GetFeed as GetFeedResponse;
use rise_canon::rise_response::post::GetPost as GetPostResponse;
use rise_canon::rise_response::post::GetShortsFeed as GetShortsFeedResponse;
use rise_canon::rise_response::post::GetUserMentions as GetUserMentionsResponse;
use rise_canon::rise_response::post::GetUserPosts as GetUserPostsResponse;
use rise_canon::rise_response::post::GetUserReposts as GetUserRepostsResponse;
use rise_canon::rise_response::post::SearchPosts as SearchPostsResponse;
use rise_canon::rise_response::post::SearchPostsByHashtag as SearchPostsByHashtagResponse;
use rise_canon::rise_response::post::SetPostFavorite as SetPostFavoriteResponse;
use rise_canon::rise_response::post::SetPostLike as SetPostLikeResponse;
use rise_canon::rise_response::post::GetPostSparks as GetPostSparksResponse;
use rise_canon::rise_response::post::SendPostSparks as SendPostSparksResponse;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine, RiseJSONValue, RiseWirePushEnvelope, RiseWireRemoteError};
use rise_catalog::{PostMediaStatus, PostModerationStatus};
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RiseEntityViewKey, RiseEntityViewLoadState, RiseEntityViewMembership,
    RiseEntityViewRetention, RiseEntityViewState, RiseEntityWindow, RiseEntityWindowQuery, RiseEntityWritePolicy, RisePositionKey, RiseStoreEntity,
};
use rise_engine::rise_store::rise_store_models::{
    RiseStoreCommit, RiseStoreMutation, RiseStoreNamespace, RiseStoreObservation, RiseStoreSnapshotRequest,
    RiseStoreVersionPrecondition, RiseStoreViewKey, RiseStoreWriteBatch, RiseStoredValue, RiseStoredValueWritePolicy,
};
use rise_engine::rise_sync::RiseSyncPushHandler;
use rise_errors::{ErrorCategory, ErrorCode};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::post::engine::rise_post_debug_trace::RisePostDebugTrace;
use crate::modules::post::engine::rise_post_engine_models::{
    PostCollaboratorsSnapshot, RiseProtoPostCollaboratorRowExt, RisePostCollaborationResponse, RisePostContentKind, RisePostCacheSource, RisePostContentKindResolve, RisePostContentPresentationSnapshot, RisePostCreateDraft,
    RisePostEditMediaItem, RisePostFeedKind, RisePostPageStatus, RisePostMediaEdit, RisePostPageCursor, RisePostPagePresentation,
    RisePostPageScope, RisePostPlacement, RisePostProfileKind, RisePostRepostAcknowledgement,
    RisePostResolvedMediaEdit, RisePostSearchKind,
    RisePostStoredEntity, RisePostUpdateDraft, RisePostAuthorEvent, RisePostCollaboratorsChangedEvent,
    RisePostCreatedEvent, RisePostEngagementAcknowledgement, RisePostLinkPreviewEvent, RisePostMediaReadyEvent,
};
use crate::modules::post::engine::rise_post_repository::{RisePostMutationError, RisePostRepositoryError};
use crate::modules::post::engine::rise_post_rpc::{RisePostEndpoints, RisePostRPC};
use crate::modules::post::engine::rise_post_upload_pipeline::{RisePostUploadPipeline, RisePostUploadProgress};
use crate::modules::post::shared::post_field_rules::PostFieldRules;
use crate::modules::post::shared::post_media_tag_remap::PostMediaTagRemap;
use crate::modules::post::stores::post_actions::post_types::{
    GetPostFeedResponse, PostCollaboratorKind, PostCollaboratorRejection, PostCollaboratorRow, PostCollaboratorStatus,
    PostCollaboratorUser, PostTagInputDraft, PostTaggedUser, PostViewerCollaboration,
};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::post::stores::post_actions::post_wire_types::{
    GetFeedPostItemDTO, GetFeedPostItemDTOExt, PostLinkPreviewDTOExt, RisePostWirePage, WireID,
};
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::sparks::engine::rise_sparks_engine_models::{RiseStoredSparksSendResult, RiseStoredSparksTop};

/// server refusal of a like or favourite delivered after the optimistic state was rolled back
/// the flag is the state the post was restored to
pub type RisePostEngagementRefusalHandler = Arc<dyn Fn(RiseFailure, bool, &mut App) + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RisePostContentRepositoryEngagementKind {
    Like,
    Favorite,
}

impl RisePostContentRepositoryEngagementKind {
    fn raw_value(self) -> &'static str {
        match self {
            RisePostContentRepositoryEngagementKind::Like => "like",
            RisePostContentRepositoryEngagementKind::Favorite => "favorite",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RisePostContentRepositoryLoadIntent {
    Activate,
    Refresh,
    LoadMore,
}

pub struct RisePostContentLoadGuard<'a> {
    repository: &'a RisePostContentRepository,
    ticket: &'a RisePostContentRepositoryLoadTicket,
    on_abandon: Option<RisePostContentAbandonHandler>,
}

pub type RisePostContentAbandonHandler = Box<dyn FnOnce(RisePostContentPresentationSnapshot) + Send>;

impl RisePostContentLoadGuard<'_> {
    /// load settled through finish_load or was never started
    pub fn disarm(mut self) {
        self.on_abandon = None;
    }
}

impl Drop for RisePostContentLoadGuard<'_> {
    fn drop(&mut self) {
        let Some(on_abandon) = self.on_abandon.take() else {
            return;
        };
        tracing::warn!(target: "riseonly::post_list_pagination", "load abandoned scope {}", self.ticket.scope.key());
        if let Some(snapshot) = self.repository.abandon_load(self.ticket) {
            on_abandon(snapshot);
        }
    }
}

#[derive(Clone)]
pub struct RisePostContentRepositoryLoadTicket {
    pub scope: RisePostPageScope,
    pub view: String,
    pub intent: RisePostContentRepositoryLoadIntent,
    pub generation: Option<u64>,
    pub seed_version: i64,
    pub cursor: Option<RisePostPageCursor>,
    pub snapshot: RisePostContentPresentationSnapshot,
    pub previous: Option<RisePostPagePresentation>,
    pub processing_post_ids: Vec<String>,
}

#[derive(Clone)]
pub struct RisePostContentRepositoryCreateTicket {
    pub request_id: String,
    pub temp_id: String,
    pub draft: RisePostCreateDraft,
    pub post: GetPostFeedResponse,
    pub snapshot: RisePostContentPresentationSnapshot,
}

#[derive(Clone)]
pub struct RisePostContentRepositoryUpdateTicket {
    pub draft: RisePostUpdateDraft,
    pub local_id: String,
    pub request_id: String,
    pub previous: RisePostStoredEntity,
    pub optimistic: RisePostStoredEntity,
    pub optimistic_subject: RisePostContentRepositoryMembershipSubject,
    pub removed_memberships: HashMap<String, RiseEntityViewMembership>,
    pub added_views: Vec<String>,
    pub refresh_scopes: Vec<RisePostPageScope>,
    pub touches_collaborators: bool,
    pub previous_collaborators: Option<RisePostContentRepositoryCollaboratorsEntry>,
    pub snapshot: RisePostContentPresentationSnapshot,
}

#[derive(Clone)]
pub struct RisePostContentRepositoryDeleteTicket {
    pub post_id: i64,
    pub local_id: Option<String>,
    pub memberships: HashMap<String, RiseEntityViewMembership>,
    pub snapshot: RisePostContentPresentationSnapshot,
}

#[derive(Clone)]
pub struct RisePostContentRepositoryRepostTicket {
    pub root_id: i64,
    pub root_local_id: String,
    pub previous_root: RisePostStoredEntity,
    pub desired: bool,
    pub content: Option<String>,
    pub request_id: String,
    pub generation: u64,
    pub repost_tab_views: Vec<String>,
    pub repost_tab_memberships: HashMap<String, RiseEntityViewMembership>,
    pub snapshot: RisePostContentPresentationSnapshot,
}

/// removal of another accounts repost by a viewer acting as author
/// row leaves that accounts reposts tab first, server confirms
/// a refusal puts it back
#[derive(Clone)]
pub struct RisePostContentRepositoryForeignRepostTicket {
    pub root_id: i64,
    pub root_local_id: String,
    pub owner_id: String,
    pub request_id: String,
    pub generation: u64,
    pub memberships: HashMap<String, RiseEntityViewMembership>,
    pub snapshot: RisePostContentPresentationSnapshot,
}

#[derive(Clone)]
pub struct RisePostContentRepositoryOpinionTicket {
    pub post_id: i64,
    pub content: String,
    pub entities: Option<String>,
    pub generation: u64,
    pub previous_entities: Vec<RiseStoreEntity>,
    pub snapshot: RisePostContentPresentationSnapshot,
}

/// one coauthor or tag operation
/// all nine share a ticket since the rollback is the same
#[derive(Clone)]
pub enum RisePostContentRepositoryCollaborationOperation {
    AcceptInvite,
    DeclineInvite,
    Leave,
    SetGridVisibility(bool),
    RemoveOwnTag,
    Invite(Vec<PostCollaboratorUser>),
    RevokeCoauthor(String),
    SetTags(Vec<PostTagInputDraft>),
    RemoveTag(String),
}

#[derive(Clone, Default)]
pub struct RisePostContentRepositoryCollaboratorsEntry {
    pub items: Vec<PostCollaboratorRow>,
    pub has_more: bool,
    pub is_loading: bool,
    pub loaded_once: bool,
    pub relative_id: Option<i64>,
    pub kind: Option<PostCollaboratorKind>,
    pub status: Option<PostCollaboratorStatus>,
    pub generation: u64,
    pub access: u64,
}

#[derive(Clone)]
pub struct RisePostContentRepositoryCollaborationTicket {
    pub post_id: i64,
    pub local_id: Option<String>,
    pub operation: RisePostContentRepositoryCollaborationOperation,
    pub previous: Option<RisePostStoredEntity>,
    pub removed_memberships: HashMap<String, RiseEntityViewMembership>,
    pub added_views: Vec<String>,
    pub previous_collaborators: Option<RisePostContentRepositoryCollaboratorsEntry>,
    pub generation: u64,
    pub snapshot: RisePostContentPresentationSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RisePostContentRepositoryScopeSlot {
    scope_key: String,
    access: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile_kind: Option<String>,
}

/// what decides which lists a post belongs to
#[derive(Clone, Debug, PartialEq)]
pub struct RisePostContentRepositoryMembershipSubject {
    pub content_kind: Option<RisePostContentKind>,
    pub author_id: String,
    pub placement: RisePostPlacement,
}

impl RisePostContentRepositoryMembershipSubject {
    pub fn new(
        content_kind: Option<RisePostContentKind>,
        author_id: String,
        placement: RisePostPlacement,
    ) -> Self {
        Self { content_kind, author_id, placement }
    }

    pub fn from_post(post: &GetPostFeedResponse) -> Self {
        Self {
            content_kind: RisePostContentKind::from_wire(&post.content_kind),
            author_id: post.author_id.clone(),
            placement: post.placement(),
        }
    }

    pub fn lists_in_publications(&self) -> bool {
        self.content_kind != Some(RisePostContentKind::Publication)
            || !self.placement.is_riff
            || self.placement.show_in_publications
    }

    pub fn lists_in_riffs(&self) -> bool {
        self.content_kind == Some(RisePostContentKind::Publication) && self.placement.is_riff
    }
}

#[derive(Default)]
struct RisePostContentRepositoryPlacementMoves {
    removed_memberships: HashMap<String, RiseEntityViewMembership>,
    removed_views: Vec<String>,
    added_views: Vec<String>,
    refresh_scopes: Vec<RisePostPageScope>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RisePostContentRepositoryScopeMembership {
    Accepts,
    Rejects,
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RisePostContentRepositoryScopeIndex {
    slots: HashMap<String, RisePostContentRepositoryScopeSlot>,
    access_clock: u64,
}

struct RisePostContentRepositoryPendingCreateMembership {
    memberships_by_view: HashMap<String, RiseEntityViewMembership>,
    server_id: Option<String>,
    settled_at_milliseconds: Option<i64>,
}

#[derive(Default)]
struct RisePostContentRepositoryState {
    pages_by_scope_key: HashMap<String, RisePostPagePresentation>,
    scopes_by_key: HashMap<String, RisePostPageScope>,
    scope_index: RisePostContentRepositoryScopeIndex,
    scope_index_loaded: bool,
    scope_index_dirty: bool,
    evicted_views: HashSet<String>,
    generations: HashMap<String, u64>,
    in_flight_scopes: HashSet<String>,
    in_flight_started_at: HashMap<String, i64>,
    presentation_revision: i64,
    store_version: i64,
    entity_mutation_versions: HashMap<String, i64>,
    forced_refresh_scope_keys: HashSet<String>,
    network_loaded_scope_keys: HashSet<String>,
    deleted_post_versions: HashMap<String, i64>,
    create_progress_by_request_id: HashMap<String, (f64, i64)>,
    pending_create_memberships: HashMap<String, RisePostContentRepositoryPendingCreateMembership>,
    session_create_local_ids: HashSet<String>,
    discarding_create_local_ids: HashSet<String>,
    server_accepted_create_request_ids: HashSet<String>,
    in_flight_update_post_ids: HashMap<String, String>,
    engagement_generations: HashMap<String, u64>,
    engagement_tasks: HashMap<String, JoinHandle<()>>,
    engagement_refusal_handlers: HashMap<String, RisePostEngagementRefusalHandler>,
    repost_generations: HashMap<String, u64>,
    repost_desired_states: HashMap<String, bool>,
    repost_queues: HashMap<String, Vec<u64>>,
    repost_turn_waiters: HashMap<String, HashMap<u64, oneshot::Sender<()>>>,
    opinion_generations: HashMap<String, u64>,
    collaboration_generations: HashMap<String, u64>,
    in_flight_collaboration_keys: HashSet<String>,
    collaborators_by_post_id: HashMap<String, RisePostContentRepositoryCollaboratorsEntry>,
    collaborators_access_clock: u64,
    in_flight_repost_keys: HashSet<String>,
    in_flight_opinion_keys: HashSet<String>,
    mutation_generation_clock: u64,
    is_shutdown: bool,
}

static ENTITY_NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("post.entities.v2"));
static MAINTENANCE_FAILURE: LazyLock<RiseFailure> = LazyLock::new(|| {
    RiseFailure::new(Some(ErrorCode::GwMaintenance.wire().to_owned()), String::new(), HashMap::new())
});
static RESOURCE_NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("post.resources.v2"));
const SCOPE_INDEX_KEY: &str = "content-scope-slots.v1";

const PENDING_CREATE_PROTECTION_MILLISECONDS: i64 = 60_000;
const STALE_IN_FLIGHT_MILLISECONDS: i64 = 60_000;
const ENGAGEMENT_COALESCE_MILLISECONDS: u64 = 500;

pub struct RisePostContentRepository {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    this: Weak<Self>,
    engine: Arc<RiseEngine>,
    rpc: RisePostRPC,
    refresh_interval_milliseconds: i64,
    state: RiseIsolated<RisePostContentRepositoryState>,
}

struct RisePostContentRepositoryRepostTurnRelease<'a> {
    repository: &'a RisePostContentRepository,
    root_key: String,
    generation: u64,
}

impl Drop for RisePostContentRepositoryRepostTurnRelease<'_> {
    fn drop(&mut self) {
        self.repository.state.with(|s| {
            RisePostContentRepository::complete_repost_turn(s, &self.root_key, self.generation);
        });
    }
}

impl RisePostContentRepository {
    const MAXIMUM_SCOPE_COUNT: usize = 32;
    const MAXIMUM_VIEW_ITEMS: usize = 240;
    const MAXIMUM_COLLABORATOR_POSTS: usize = 32;
    const MAXIMUM_COLLABORATOR_ITEMS: usize = 200;
    const COLLABORATORS_PAGE_LIMIT: i64 = 30;
    const PAGE_LIMIT: i64 = 20;
    const FEED_SORT: &'static str = "feed";
    const PROFILE_SORT: &'static str = "new";

    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            this: this.clone(),
            rpc: RisePostRPC::new(engine.clone()),
            engine,
            account_id,
            account_epoch,
            session_id,
            refresh_interval_milliseconds: rise_canon::RiseTimingCanon::freshness_ms("post_feed_page"),
            state: RiseIsolated::new(RisePostContentRepositoryState::default()),
        })
    }

    /// same as the stored window
    /// when it was half of it pages past the 120th post were stored but never shown so a profile
    /// with 129 threads stopped at 120
    fn maximum_presentation_items(&self) -> usize {
        Self::MAXIMUM_VIEW_ITEMS
    }

    /// feeds and shorts keep the newest pages and drop the oldest from the top
    /// so a full window still loads further
    fn slides_window(scope: &RisePostPageScope) -> bool {
        match scope {
            RisePostPageScope::Feed(_) | RisePostPageScope::Shorts => true,
            RisePostPageScope::Profile { .. } | RisePostPageScope::Search { .. } | RisePostPageScope::Detail { .. } => {
                false
            }
        }
    }

    pub async fn activate(&self) -> RisePostContentPresentationSnapshot {
        if self.state.with(|s| s.is_shutdown) {
            return self.state.with(|s| self.make_snapshot(s));
        }
        self.load_scope_index_if_needed().await;
        let scope = RisePostPageScope::Feed(RisePostFeedKind::All);
        let view = self.state.with(|s| Self::allocate_view(s, &scope));
        let cached = self.load_page(&scope, &view, None).await;
        self.state.with(|s| {
            if s.is_shutdown {
                return self.make_snapshot(s);
            }
            let key = scope.key();
            s.scopes_by_key.insert(key.clone(), scope);
            s.pages_by_scope_key.entry(key).or_insert(cached);
            self.make_snapshot(s)
        })
    }

    pub async fn begin_create(
        &self,
        draft: RisePostCreateDraft,
    ) -> anyhow::Result<RisePostContentRepositoryCreateTicket> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        Self::validate_create(&draft)?;
        self.load_scope_index_if_needed().await;
        let request_id = Uuid::new_v4().uuid_string().to_lowercase();
        let temp_id = format!("temp_post_{request_id}");
        let now = Self::now_milliseconds();
        let local_preview_url = draft
            .local_video
            .as_ref()
            .map(|video| video.url.absolute_string().to_owned())
            .or_else(|| draft.local_uploads.first().map(|upload| upload.url.absolute_string().to_owned()));
        let has_local_media = !draft.local_uploads.is_empty() || draft.local_video.is_some();
        let placement = draft.placement();
        let mut post = GetPostFeedResponse {
            id: temp_id.clone(),
            title: draft.title.clone(),
            content: draft.content.clone(),
            original_content: draft.content.clone(),
            hashtags: draft.hashtags.clone(),
            author_id: self.account_id.clone(),
            images: draft.local_uploads.iter().map(|upload| upload.url.absolute_string().to_owned()).collect(),
            author: draft.author.clone(),
            video: draft.local_video.as_ref().map(|video| video.url.absolute_string().to_owned()),
            is_short: draft.is_short(),
            is_image_reel: draft.is_image_reel(),
            is_riff: placement.is_riff,
            show_in_publications: placement.show_in_publications,
            video_position: draft.resolved_video_position(),
            achievement_id: draft.achievement_id,
            map_journey_id: draft.map_journey_id.clone(),
            track_id: draft.track_id.clone(),
            track: draft.track.clone(),
            track_start_ms: draft.track_start_milliseconds,
            track_end_ms: draft.track_end_milliseconds,
            track_plate: draft.track_plate.as_ref().map(|plate| plate.as_dto()),
            pending_coauthors_count: draft.coauthor_ids.len() as i64,
            tagged_users: draft
                .tagged_users
                .iter()
                .map(|tag| PostTaggedUser {
                    placements: tag.placements.clone(),
                    user: Some(tag.user.clone().unwrap_or_else(|| PostCollaboratorUser {
                        id: tag.user_id.clone(),
                        ..PostCollaboratorUser::default()
                    })),
                })
                .collect(),
            tagged_users_count: draft.tagged_users.len() as i64,
            content_kind: draft.content_kind.wire().to_owned(),
            created_at_ms: Some(now),
            updated_at_ms: Some(now),
            presentation_id: Some(temp_id.clone()),
            is_temp: true,
            is_uploading: has_local_media,
            is_compressing: has_local_media,
            local_preview_url,
            ..GetPostFeedResponse::default()
        };
        post.can_comment = draft.can_comment;
        post.count_visibility = draft.count_visibility.clone();
        post.audio_baked = draft.audio_pre_baked;
        let data = serde_json::to_vec(&RisePostStoredEntity { post: post.clone() })?;
        let subject = RisePostContentRepositoryMembershipSubject::new(
            Some(draft.content_kind),
            self.account_id.clone(),
            placement,
        );
        let mutations = self.state.with(|s| {
            Self::advance_store_version(s);
            let store_version = s.store_version;
            let mut mutations = vec![
                RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(temp_id.clone(), data, store_version, now),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                },
                RiseStoreMutation::SetEntityAlias {
                    namespace: ENTITY_NAMESPACE.clone(),
                    alias: RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), temp_id.clone()),
                    local_id: temp_id.clone(),
                },
                RiseStoreMutation::SetEntityAlias {
                    namespace: ENTITY_NAMESPACE.clone(),
                    alias: RiseEntityAlias::new(RiseEntityAliasKind::client_id(), request_id.clone()),
                    local_id: temp_id.clone(),
                },
            ];
            let mut inserted_memberships: HashMap<String, RiseEntityViewMembership> = HashMap::new();
            for (view, slot) in &s.scope_index.slots {
                if !Self::can_prepend_created_post(s, slot)
                    || Self::membership_of_slot(s, slot, &subject) != RisePostContentRepositoryScopeMembership::Accepts
                {
                    continue;
                }
                let position = RisePositionKey::make(|builder| {
                    builder.append_signed_int64(-store_version);
                    builder.append_string(&temp_id);
                });
                let membership = RiseEntityViewMembership::new(temp_id.clone(), position, store_version);
                inserted_memberships.insert(view.clone(), membership.clone());
                mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                    namespace: ENTITY_NAMESPACE.clone(),
                    view: view.clone(),
                    membership,
                });
            }
            s.pending_create_memberships.insert(
                temp_id.clone(),
                RisePostContentRepositoryPendingCreateMembership {
                    memberships_by_view: inserted_memberships,
                    server_id: None,
                    settled_at_milliseconds: None,
                },
            );
            s.session_create_local_ids.insert(temp_id.clone());
            mutations
        });
        if let Err(error) = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await {
            self.state.with(|s| {
                s.session_create_local_ids.remove(&temp_id);
                s.pending_create_memberships.remove(&temp_id);
            });
            return Err(error);
        }
        let snapshot = self.refresh_active_pages_from_store().await;
        Ok(RisePostContentRepositoryCreateTicket { request_id, temp_id, draft, post, snapshot })
    }

    pub async fn finish_create(
        &self,
        ticket: &RisePostContentRepositoryCreateTicket,
        on_progress: RisePostUploadProgress,
    ) -> anyhow::Result<(GetPostFeedResponse, Vec<PostCollaboratorRejection>, RisePostContentPresentationSnapshot)> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(CancellationError.into());
        }
        let uploads = RisePostUploadPipeline {
            account_id: self.account_id.clone(),
            account_epoch: self.account_epoch.clone(),
            session_id: self.session_id,
        };
        let draft = &ticket.draft;
        let image_progress_scale = if draft.local_video.is_none() { 1.0 } else { 0.4 };
        let video_progress_start = if draft.local_uploads.is_empty() { 0.0 } else { 0.4 };
        let image_progress: RisePostUploadProgress = {
            let on_progress = on_progress.clone();
            Arc::new(move |progress| on_progress(progress * image_progress_scale))
        };
        let image_urls = uploads.upload(&draft.local_uploads, &ticket.request_id, image_progress).await?;
        let video_upload: Option<(String, Option<String>)> = match &draft.local_video {
            Some(local_video) => {
                let video_progress: RisePostUploadProgress = {
                    let on_progress = on_progress.clone();
                    Arc::new(move |progress| {
                        let remaining = 1.0 - video_progress_start;
                        on_progress(video_progress_start + progress * remaining)
                    })
                };
                Some(uploads.upload_video(local_video, &ticket.request_id, video_progress).await?)
            }
            None => None,
        };
        on_progress(1.0).await;
        let visibility = draft.count_visibility;
        let placement = draft.placement();
        let (video, video_thumbnail) = match video_upload {
            Some((video, thumbnail)) => (Some(video), thumbnail),
            None => (None, None),
        };
        let response: CreatePostResponse = self
            .rpc
            .attested_call(
                RisePostEndpoints::create_post(),
                &CreatePostRequest {
                    achievement_id: draft.achievement_id,
                    allow_download: Some(visibility.allows_download),
                    allow_download_without_watermark: Some(
                        visibility.allows_download && visibility.allows_download_without_watermark,
                    ),
                    allow_sparks: Some(visibility.allows_sparks),
                    audio_pre_baked: draft.audio_pre_baked.then_some(true),
                    can_comment: Some(draft.can_comment),
                    client_request_id: Some(ticket.request_id.clone()),
                    coauthor_ids: (!draft.coauthor_ids.is_empty()).then(|| draft.coauthor_ids.clone()),
                    content: Some(draft.content.clone()),
                    content_kind: Some(draft.content_kind.wire().to_owned()),
                    hashtags: Some(draft.hashtags.clone()),
                    images: Some(image_urls),
                    is_image_reel: Some(draft.is_image_reel()),
                    is_riff: Some(placement.is_riff),
                    is_short: Some(draft.is_short()),
                    link_areas: None,
                    map_journey_id: draft.map_journey_id.clone(),
                    map_journey_show_heat: Some(draft.map_journey_show_heat),
                    original_content: Some(draft.content.clone()),
                    show_comment_dislikes: Some(visibility.comment_dislikes),
                    show_comments_count: Some(visibility.comments),
                    show_in_publications: Some(placement.show_in_publications),
                    show_likes_count: Some(visibility.likes),
                    show_reposts_count: Some(visibility.reposts),
                    show_shares_count: Some(visibility.shares),
                    show_sparks: Some(visibility.sparks),
                    tagged_users: (!draft.tagged_users.is_empty())
                        .then(|| draft.tagged_users.iter().map(CreatePostTaggedUsersElement::from).collect()),
                    tags: Some(Vec::new()),
                    title: Some(draft.title.clone()),
                    track_end_ms: draft.track_end_milliseconds,
                    track_id: draft.track_id.clone(),
                    track_plate: draft.track_plate.as_ref().map(|plate| plate.create_wire()),
                    track_start_ms: draft.track_start_milliseconds,
                    video,
                    video_position: draft.resolved_video_position(),
                    video_thumbnail,
                },
            )
            .await?;
        self.state.with(|s| s.server_accepted_create_request_ids.insert(ticket.request_id.clone()));
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(CancellationError.into());
        }
        let Some(mut post) = response.post.as_ref().and_then(|dto| dto.as_feed_response()) else {
            return Err(RisePostRepositoryError::MissingActiveSession.into());
        };
        post.content_kind = draft.content_kind.wire().to_owned();
        post.map_journey_id = post.map_journey_id.or_else(|| draft.map_journey_id.clone());
        post.achievement_id = post.achievement_id.or(draft.achievement_id);
        post.track_plate = post.track_plate.or_else(|| draft.track_plate.as_ref().map(|plate| plate.as_dto()));
        post.is_temp = false;
        post.is_uploading = false;
        post.is_compressing = false;
        post.upload_progress = 100.0;
        post.compression_progress = 100.0;
        post.local_preview_url = None;
        post.presentation_id = Some(ticket.temp_id.clone());

        let now = Self::now_milliseconds();
        let data = serde_json::to_vec(&RisePostStoredEntity { post: post.clone() })?;
        let mutations = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &post.id);
            let store_version = s.store_version;
            let mut mutations = vec![
                RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(
                        ticket.temp_id.clone(),
                        data,
                        store_version,
                        post.updated_at_ms.unwrap_or(now),
                    ),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                },
                RiseStoreMutation::SetEntityAlias {
                    namespace: ENTITY_NAMESPACE.clone(),
                    alias: RiseEntityAlias::new(RiseEntityAliasKind::server_id(), post.id.clone()),
                    local_id: ticket.temp_id.clone(),
                },
                RiseStoreMutation::SetEntityAlias {
                    namespace: ENTITY_NAMESPACE.clone(),
                    alias: RiseEntityAlias::new(RiseEntityAliasKind::client_id(), ticket.request_id.clone()),
                    local_id: ticket.temp_id.clone(),
                },
            ];
            let mut pending = s.pending_create_memberships.remove(&ticket.temp_id).unwrap_or(
                RisePostContentRepositoryPendingCreateMembership {
                    memberships_by_view: HashMap::new(),
                    server_id: None,
                    settled_at_milliseconds: None,
                },
            );
            pending.server_id = Some(post.id.clone());
            pending.settled_at_milliseconds = Some(now);
            for (view, membership) in pending.memberships_by_view.iter_mut() {
                let restored =
                    RiseEntityViewMembership::new(ticket.temp_id.clone(), membership.position_key.clone(), store_version);
                *membership = restored.clone();
                mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                    namespace: ENTITY_NAMESPACE.clone(),
                    view: view.clone(),
                    membership: restored,
                });
            }
            s.pending_create_memberships.insert(ticket.temp_id.clone(), pending);
            mutations
        });
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        let created_subject = RisePostContentRepositoryMembershipSubject::from_post(&post);
        self.state.with(|s| {
            s.session_create_local_ids.remove(&ticket.temp_id);
            s.create_progress_by_request_id.remove(&ticket.request_id);
            s.server_accepted_create_request_ids.remove(&ticket.request_id);
            let forced: Vec<String> = s
                .scope_index
                .slots
                .values()
                .filter(|slot| {
                    Self::can_prepend_created_post(s, slot)
                        && Self::membership_of_slot(s, slot, &created_subject)
                            == RisePostContentRepositoryScopeMembership::Accepts
                })
                .map(|slot| slot.scope_key.clone())
                .collect();
            s.forced_refresh_scope_keys.extend(forced);
        });
        let snapshot = self.refresh_active_pages_from_store().await;
        Ok((post, response.rejected, snapshot))
    }

    pub async fn update_create_progress(
        &self,
        ticket: &RisePostContentRepositoryCreateTicket,
        raw_progress: f64,
    ) -> Option<(RisePostContentPresentationSnapshot, GetPostFeedResponse)> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return None;
        }
        let progress = raw_progress.clamp(0.0, 1.0);
        let now = Self::now_milliseconds();
        let throttled = self.state.with(|s| {
            s.create_progress_by_request_id
                .get(&ticket.request_id)
                .is_some_and(|last| progress < 1.0 && progress - last.0 < 0.03 && now - last.1 < 100)
        });
        if throttled {
            return None;
        }
        let entity = self
            .engine
            .entity_alias(
                ENTITY_NAMESPACE.clone(),
                RiseEntityAlias::new(RiseEntityAliasKind::client_id(), ticket.request_id.clone()),
            )
            .await
            .ok()
            .flatten()?;
        let mut stored = serde_json::from_slice::<RisePostStoredEntity>(&entity.data).ok()?;
        let accepted = self.state.with(|s| {
            if !stored.post.is_temp
                || !s.session_create_local_ids.contains(&ticket.temp_id)
                || s.server_accepted_create_request_ids.contains(&ticket.request_id)
            {
                return false;
            }
            s.create_progress_by_request_id.insert(ticket.request_id.clone(), (progress, now));
            true
        });
        if !accepted {
            return None;
        }
        let photo_count = ticket.draft.local_uploads.len();
        let has_video = ticket.draft.local_video.is_some();
        let units = (photo_count + usize::from(has_video)).max(1);
        let compression_end = if has_video && photo_count == 0 { 0.35 * 0.9 } else { 0.35 / units as f64 };
        stored.post.is_compressing = progress < compression_end;
        stored.post.compression_progress = (progress / compression_end * 100.0).min(100.0);
        stored.post.is_uploading = photo_count > 0 || has_video;
        stored.post.upload_progress = (progress * 100.0).min(100.0);
        let store_version = self.state.with(|s| {
            Self::advance_store_version(s);
            s.store_version
        });
        let data = serde_json::to_vec(&stored).unwrap_or_else(|_| entity.data.clone());
        let _ = self
            .engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(entity.local_id.clone(), data, store_version, now),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                }],
                HashSet::new(),
            ))
            .await;
        Some((self.refresh_active_pages_from_store().await, stored.post))
    }

    pub async fn rollback_create(
        &self,
        ticket: &RisePostContentRepositoryCreateTicket,
    ) -> RisePostContentPresentationSnapshot {
        if !self.state.with(|s| s.is_shutdown) {
            let reconciled = self
                .engine
                .entity_alias(
                    ENTITY_NAMESPACE.clone(),
                    RiseEntityAlias::new(RiseEntityAliasKind::client_id(), ticket.request_id.clone()),
                )
                .await
                .ok()
                .flatten();
            let settled = reconciled
                .and_then(|entity| serde_json::from_slice::<RisePostStoredEntity>(&entity.data).ok())
                .is_some_and(|stored| !stored.post.is_temp && stored.post.id != ticket.temp_id);
            if settled {
                self.state.with(|s| {
                    s.session_create_local_ids.remove(&ticket.temp_id);
                    s.create_progress_by_request_id.remove(&ticket.request_id);
                    s.server_accepted_create_request_ids.remove(&ticket.request_id);
                });
                return self.refresh_active_pages_from_store().await;
            }
            if !self.state.with(|s| s.server_accepted_create_request_ids.contains(&ticket.request_id)) {
                let _ = self
                    .engine
                    .write(RiseStoreWriteBatch::new(
                        vec![RiseStoreMutation::RemoveEntity {
                            namespace: ENTITY_NAMESPACE.clone(),
                            local_id: ticket.temp_id.clone(),
                        }],
                        HashSet::new(),
                    ))
                    .await;
            }
        }
        self.state.with(|s| {
            s.create_progress_by_request_id.remove(&ticket.request_id);
            if !s.server_accepted_create_request_ids.contains(&ticket.request_id) {
                s.session_create_local_ids.remove(&ticket.temp_id);
                s.pending_create_memberships.remove(&ticket.temp_id);
            }
            s.server_accepted_create_request_ids.remove(&ticket.request_id);
        });
        self.refresh_active_pages_from_store().await
    }

    pub async fn begin_update(
        &self,
        draft: RisePostUpdateDraft,
    ) -> anyhow::Result<RisePostContentRepositoryUpdateTicket> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        Self::validate_update(&draft)?;
        self.load_scope_index_if_needed().await;
        let entity = self.entity_by_server_id(&draft.post_id).await?;
        let local_id = entity.as_ref().map(|entity| entity.local_id.clone()).unwrap_or_else(|| draft.post_id.clone());
        let mut stored = entity
            .and_then(|entity| serde_json::from_slice::<RisePostStoredEntity>(&entity.data).ok())
            .unwrap_or_else(|| RisePostStoredEntity { post: draft.current_post.clone() });
        let previous = stored.clone();
        stored.post.title = draft.title.clone();
        stored.post.content = draft.content.clone();
        stored.post.original_content = draft.content.clone();
        stored.post.hashtags = draft.hashtags.clone();
        stored.post.can_comment = draft.can_comment;
        stored.post.count_visibility = draft.count_visibility;
        stored.post.track_id = draft.track_id.clone();
        stored.post.track = draft.track.clone();
        stored.post.track_start_ms = draft.track_start_milliseconds;
        stored.post.track_end_ms = draft.track_end_milliseconds;
        stored.post.track_plate = draft.track_plate.as_ref().map(|plate| plate.as_dto());
        stored.post.content_kind = draft.content_kind.wire().to_owned();
        if let Some(media) = &draft.media {
            media.apply(&mut stored.post);
        }
        if let Some(tags) = &draft.tagged_users {
            stored.post.tagged_users = tags
                .iter()
                .map(|tag| PostTaggedUser {
                    placements: tag.placements.clone(),
                    user: Some(tag.user.clone().unwrap_or_else(|| PostCollaboratorUser {
                        id: tag.user_id.clone(),
                        ..PostCollaboratorUser::default()
                    })),
                })
                .collect();
            stored.post.tagged_users_count = stored.post.tagged_users.len() as i64;
        } else if draft.media.is_some() {
            let carried = PostMediaTagRemap::remap(
                &previous.post.tagged_users,
                &previous.post.media_slide_keys(),
                &stored.post.media_slide_keys(),
            );
            stored.post.tagged_users_count =
                (carried.tagged_users.len() as i64).max(previous.post.tagged_users_count - carried.removed_tags);
            stored.post.tagged_users = carried.tagged_users;
        }
        if (draft.changes_placement() || draft.media.is_some()) && draft.content_kind == RisePostContentKind::Publication
        {
            let placement = draft.placement(&stored.post);
            Self::apply_placement(placement, &mut stored.post);
        }
        let optimistic_subject = RisePostContentRepositoryMembershipSubject::from_post(&stored.post);
        let moves = self
            .placement_moves(
                &local_id,
                &RisePostContentRepositoryMembershipSubject::from_post(&previous.post),
                &optimistic_subject,
            )
            .await;
        let data = serde_json::to_vec(&stored)?;
        let now = Self::now_milliseconds();
        let mutations = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &draft.post_id);
            let store_version = s.store_version;
            let mut mutations = vec![
                RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(local_id.clone(), data, store_version, now),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                },
                RiseStoreMutation::SetEntityAlias {
                    namespace: ENTITY_NAMESPACE.clone(),
                    alias: RiseEntityAlias::new(RiseEntityAliasKind::server_id(), draft.post_id.clone()),
                    local_id: local_id.clone(),
                },
            ];
            mutations.extend(Self::membership_mutations(
                &local_id,
                &moves.added_views,
                &moves.removed_views,
                store_version,
            ));
            Self::release_pending_create_memberships(s, &local_id, &moves.removed_views);
            mutations
        });
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        let request_id = Uuid::new_v4().uuid_string().to_lowercase();
        let touches_collaborators = draft.media.is_some() || draft.tagged_users.is_some();
        let previous_collaborators = self.state.with(|s| {
            s.in_flight_update_post_ids.insert(request_id.clone(), draft.post_id.clone());
            let previous_collaborators = s.collaborators_by_post_id.get(&draft.post_id).cloned();
            if touches_collaborators {
                self.apply_authoritative_collaborator_rows(s, &stored.post, &draft.post_id);
            }
            previous_collaborators
        });
        let snapshot = self.refresh_active_pages_from_store().await;
        Ok(RisePostContentRepositoryUpdateTicket {
            draft,
            local_id,
            request_id,
            previous,
            optimistic: stored,
            optimistic_subject,
            removed_memberships: moves.removed_memberships,
            added_views: moves.added_views,
            refresh_scopes: moves.refresh_scopes,
            touches_collaborators,
            previous_collaborators,
            snapshot,
        })
    }

    pub async fn finish_update(
        &self,
        ticket: &RisePostContentRepositoryUpdateTicket,
        on_progress: RisePostUploadProgress,
    ) -> anyhow::Result<(
        GetPostFeedResponse,
        Vec<PostCollaboratorRejection>,
        RisePostContentPresentationSnapshot,
        Vec<RisePostPageScope>,
    )> {
        let Ok(post_id) = ticket.draft.post_id.parse::<i64>() else {
            return Err(RisePostRepositoryError::StaleAccount.into());
        };
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(CancellationError.into());
        }
        let resolved_media = match &ticket.draft.media {
            Some(media) => {
                let resolved = self.upload_edited_media(media, &ticket.request_id, on_progress).await?;
                if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
                    return Err(CancellationError.into());
                }
                Some(resolved)
            }
            None => None,
        };
        let draft = &ticket.draft;
        let replacement_tags = draft.tagged_users.as_ref();
        let requested_placement = (draft.changes_placement()
            && draft.content_kind == RisePostContentKind::Publication)
            .then(|| draft.placement(&ticket.previous.post));
        let visibility = draft.count_visibility;
        let response: UpdatePostResponse = self
            .rpc
            .call(
                RisePostEndpoints::update_post(),
                &UpdatePostRequest {
                    allow_download: Some(visibility.allows_download),
                    allow_download_without_watermark: Some(
                        visibility.allows_download && visibility.allows_download_without_watermark,
                    ),
                    allow_sparks: Some(visibility.allows_sparks),
                    can_comment: Some(draft.can_comment),
                    clear_hashtags: Some(draft.hashtags.is_empty()),
                    clear_track: Some(draft.current_post.track_id.is_some() && draft.track_id.is_none()),
                    clear_track_plate: Some(draft.current_post.track_plate.is_some() && draft.track_plate.is_none()),
                    content: Some(draft.content.clone()),
                    content_kind: Some(draft.content_kind.wire().to_owned()),
                    hashtags: Some(draft.hashtags.clone()),
                    images: resolved_media.as_ref().map(|media| media.images.clone()),
                    is_riff: requested_placement.map(|placement| placement.is_riff),
                    link_areas: None,
                    original_content: Some(draft.content.clone()),
                    post_id: Some(post_id),
                    replace_media: resolved_media.as_ref().map(|_| true),
                    replace_tags: replacement_tags.map(|_| true),
                    show_comment_dislikes: Some(visibility.comment_dislikes),
                    show_comments_count: Some(visibility.comments),
                    show_in_publications: requested_placement.map(|placement| placement.show_in_publications),
                    show_likes_count: Some(visibility.likes),
                    show_reposts_count: Some(visibility.reposts),
                    show_shares_count: Some(visibility.shares),
                    show_sparks: Some(visibility.sparks),
                    tagged_users: replacement_tags
                        .map(|tags| tags.iter().map(UpdatePostTaggedUsersElement::from).collect()),
                    tags: None,
                    title: Some(draft.title.clone()),
                    track_end_ms: draft.track_end_milliseconds,
                    track_id: draft.track_id.clone(),
                    track_plate: draft.track_plate.as_ref().map(|plate| plate.update_wire()),
                    track_start_ms: draft.track_start_milliseconds,
                    video: resolved_media.as_ref().and_then(|media| media.video.clone()),
                    video_position: resolved_media.as_ref().and_then(|media| media.video_position),
                    video_thumbnail: resolved_media.as_ref().and_then(|media| media.video_thumbnail.clone()),
                },
            )
            .await?;
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(CancellationError.into());
        }
        let Some(dto) = response.post.as_ref() else {
            return Err(RisePostRepositoryError::MissingActiveSession.into());
        };
        let Some(mut post) = dto.as_feed_response() else {
            return Err(RisePostRepositoryError::MissingActiveSession.into());
        };
        if dto.tagged_users.is_empty() {
            post.tagged_users = ticket.optimistic.post.tagged_users.clone();
            post.tagged_users_count = ticket.optimistic.post.tagged_users_count;
        }
        if dto.coauthors.is_empty() {
            post.coauthors = ticket.optimistic.post.coauthors.clone();
            post.coauthors_count = ticket.optimistic.post.coauthors_count;
            post.pending_coauthors_count = ticket.optimistic.post.pending_coauthors_count;
            post.my_collaboration = ticket.optimistic.post.my_collaboration.clone();
        }
        post.content_kind = draft.content_kind.wire().to_owned();
        post.map_journey_id = post.map_journey_id.or_else(|| draft.current_post.map_journey_id.clone());
        post.achievement_id = post.achievement_id.or(draft.current_post.achievement_id);
        if let Some(plate) = &draft.track_plate {
            post.track_plate = post.track_plate.or_else(|| Some(plate.as_dto()));
        }
        post.presentation_id = Some(ticket.local_id.clone());
        let moves = self
            .placement_moves(
                &ticket.local_id,
                &ticket.optimistic_subject,
                &RisePostContentRepositoryMembershipSubject::from_post(&post),
            )
            .await;
        let data = serde_json::to_vec(&RisePostStoredEntity { post: post.clone() })?;
        let updated_at = post.updated_at_ms.unwrap_or_else(Self::now_milliseconds);
        let mutations = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &post.id);
            let store_version = s.store_version;
            let mut mutations = vec![
                RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(ticket.local_id.clone(), data, store_version, updated_at),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                },
                RiseStoreMutation::SetEntityAlias {
                    namespace: ENTITY_NAMESPACE.clone(),
                    alias: RiseEntityAlias::new(RiseEntityAliasKind::server_id(), post.id.clone()),
                    local_id: ticket.local_id.clone(),
                },
            ];
            mutations.extend(Self::membership_mutations(
                &ticket.local_id,
                &moves.added_views,
                &moves.removed_views,
                store_version,
            ));
            Self::release_pending_create_memberships(s, &ticket.local_id, &moves.removed_views);
            mutations
        });
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        let mut refresh_scopes = ticket.refresh_scopes.clone();
        for scope in moves.refresh_scopes {
            if !refresh_scopes.contains(&scope) {
                refresh_scopes.push(scope);
            }
        }
        self.state.with(|s| {
            if ticket.touches_collaborators {
                self.apply_authoritative_collaborator_rows(s, &post, &ticket.draft.post_id);
            }
            s.in_flight_update_post_ids.remove(&ticket.request_id);
        });
        let snapshot = self.refresh_active_pages_from_store().await;
        Ok((post, Vec::new(), snapshot, refresh_scopes))
    }

    async fn upload_edited_media(
        &self,
        media: &RisePostMediaEdit,
        request_id: &str,
        on_progress: RisePostUploadProgress,
    ) -> anyhow::Result<RisePostResolvedMediaEdit> {
        let local_images = media.local_images();
        let local_video = media.local_video();
        let uploads = RisePostUploadPipeline {
            account_id: self.account_id.clone(),
            account_epoch: self.account_epoch.clone(),
            session_id: self.session_id,
        };
        let image_progress_scale = if local_video.is_none() { 1.0 } else { 0.4 };
        let video_progress_start = if local_images.is_empty() { 0.0 } else { 0.4 };
        let image_progress: RisePostUploadProgress = {
            let on_progress = on_progress.clone();
            Arc::new(move |progress| on_progress(progress * image_progress_scale))
        };
        let uploaded_images = uploads.upload(&local_images, request_id, image_progress).await?;
        if uploaded_images.len() != local_images.len() {
            return Err(RisePostMutationError::Server("create_post_error_connection".to_owned()).into());
        }
        let uploaded_video: Option<(String, Option<String>)> = match &local_video {
            Some(local_video) => {
                let video_progress: RisePostUploadProgress = {
                    let on_progress = on_progress.clone();
                    Arc::new(move |progress| {
                        let remaining = 1.0 - video_progress_start;
                        on_progress(video_progress_start + progress * remaining)
                    })
                };
                Some(uploads.upload_video(local_video, request_id, video_progress).await?)
            }
            None => None,
        };
        if media.has_local_uploads() {
            on_progress(1.0).await;
        }

        let mut next_uploaded_image = uploaded_images.into_iter();
        let mut images: Vec<String> = Vec::with_capacity(media.items().len());
        let mut video: Option<String> = None;
        let mut video_thumbnail: Option<String> = None;
        let mut video_position: Option<i64> = None;
        for item in media.items() {
            match item {
                RisePostEditMediaItem::RemoteImage { url, .. } => images.push(url.clone()),
                RisePostEditMediaItem::LocalImage(_) => {
                    let Some(url) = next_uploaded_image.next() else {
                        return Err(RisePostMutationError::Server("create_post_error_connection".to_owned()).into());
                    };
                    images.push(url);
                }
                RisePostEditMediaItem::RemoteVideo { url, thumbnail, .. } => {
                    video = Some(url.clone());
                    video_thumbnail = thumbnail.clone();
                    video_position = Some(images.len() as i64);
                }
                RisePostEditMediaItem::LocalVideo { .. } => {
                    let Some((uploaded, thumbnail)) = &uploaded_video else {
                        return Err(RisePostMutationError::Server("create_post_error_connection".to_owned()).into());
                    };
                    video = Some(uploaded.clone());
                    video_thumbnail = thumbnail.clone();
                    video_position = Some(images.len() as i64);
                }
            }
        }
        Ok(RisePostResolvedMediaEdit { images, video, video_thumbnail, video_position })
    }

    fn validate_update(draft: &RisePostUpdateDraft) -> Result<(), RisePostMutationError> {
        let Some(media) = &draft.media else {
            return Ok(());
        };
        let has_track = draft
            .track_id
            .as_deref()
            .is_some_and(|track_id| !trimming_whitespaces_and_newlines(track_id).is_empty());
        let has_achievement = draft.current_post.achievement_id.unwrap_or(0) > 0;
        match draft.content_kind {
            RisePostContentKind::Thread => {
                if !media.items().is_empty() {
                    return Err(RisePostMutationError::InvalidDraft("thread_cannot_contain_media".to_owned()));
                }
            }
            RisePostContentKind::Publication => {
                if has_achievement && !media.items().is_empty() {
                    return Err(RisePostMutationError::InvalidDraft("achievement_cannot_contain_media".to_owned()));
                }
                if media.items().is_empty() && !has_track && !has_achievement {
                    return Err(RisePostMutationError::InvalidDraft("publication_requires_media".to_owned()));
                }
            }
        }
        Ok(())
    }

    pub async fn rollback_update(
        &self,
        ticket: &RisePostContentRepositoryUpdateTicket,
    ) -> RisePostContentPresentationSnapshot {
        let is_shutdown = self.state.with(|s| {
            s.in_flight_update_post_ids.remove(&ticket.request_id);
            if ticket.touches_collaborators
                && let Some(previous_collaborators) = &ticket.previous_collaborators
            {
                s.collaborators_by_post_id.insert(ticket.draft.post_id.clone(), previous_collaborators.clone());
            }
            s.is_shutdown
        });
        if !is_shutdown && let Ok(data) = serde_json::to_vec(&ticket.previous) {
            let now = Self::now_milliseconds();
            let mutations = self.state.with(|s| {
                Self::advance_store_version(s);
                let version = s.store_version;
                let mut mutations = vec![RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(ticket.local_id.clone(), data, version, now),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                }];
                for view in &ticket.added_views {
                    mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                        namespace: ENTITY_NAMESPACE.clone(),
                        view: view.clone(),
                        local_id: ticket.local_id.clone(),
                        if_version_at_most: None,
                    });
                }
                for (view, membership) in &ticket.removed_memberships {
                    mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                        namespace: ENTITY_NAMESPACE.clone(),
                        view: view.clone(),
                        membership: RiseEntityViewMembership::new(
                            membership.local_id.clone(),
                            membership.position_key.clone(),
                            version,
                        ),
                    });
                }
                mutations
            });
            let _ = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await;
        }
        self.refresh_active_pages_from_store().await
    }

    fn apply_placement(placement: RisePostPlacement, post: &mut GetPostFeedResponse) {
        let has_video = post.has_video_media();
        post.is_riff = placement.is_riff;
        post.show_in_publications = placement.show_in_publications;
        post.is_short = placement.legacy_is_short(has_video);
        post.is_image_reel = placement.legacy_is_image_reel(has_video, post.images.len() as i64);
    }

    /// lists a post enters or leaves when its placement changes
    /// a profile list gets the row at once plus a server refresh for its real position
    /// a ranked feed only loses the row
    async fn placement_moves(
        &self,
        local_id: &str,
        old: &RisePostContentRepositoryMembershipSubject,
        new: &RisePostContentRepositoryMembershipSubject,
    ) -> RisePostContentRepositoryPlacementMoves {
        use RisePostContentRepositoryScopeMembership::{Rejects, Unknown};
        let mut moves = RisePostContentRepositoryPlacementMoves::default();
        if old == new {
            return moves;
        }
        self.state.with(|s| {
            let mut forced: Vec<String> = Vec::new();
            for (view, slot) in &s.scope_index.slots {
                let before = Self::membership_of_slot(s, slot, old);
                let after = Self::membership_of_slot(s, slot, new);
                if before == after || before == Unknown || after == Unknown {
                    continue;
                }
                let is_profile = slot.family.as_deref() == Some("profile");
                if is_profile {
                    forced.push(slot.scope_key.clone());
                    if let Some(scope) = s.scopes_by_key.get(&slot.scope_key) {
                        moves.refresh_scopes.push(scope.clone());
                    }
                }
                if after != Rejects {
                    if is_profile {
                        moves.added_views.push(view.clone());
                    }
                    continue;
                }
                moves.removed_views.push(view.clone());
            }
            s.forced_refresh_scope_keys.extend(forced);
        });
        for view in &moves.removed_views {
            let Ok(window) = self
                .engine
                .entity_window(RiseEntityWindowQuery::new(
                    ENTITY_NAMESPACE.clone(),
                    view.clone(),
                    RiseEntityWindow::First { limit: Self::MAXIMUM_VIEW_ITEMS },
                ))
                .await
            else {
                continue;
            };
            let Some(item) = window.items.iter().find(|item| item.entity.local_id == local_id) else {
                continue;
            };
            moves.removed_memberships.insert(
                view.clone(),
                RiseEntityViewMembership::new(local_id.to_owned(), item.position_key.clone(), item.membership_version),
            );
        }
        moves
    }

    /// a list the post just left must not get the row back from the protection of its own
    /// unconfirmed create
    fn release_pending_create_memberships(s: &mut RisePostContentRepositoryState, local_id: &str, views: &[String]) {
        if views.is_empty() {
            return;
        }
        let Some(pending) = s.pending_create_memberships.get_mut(local_id) else {
            return;
        };
        for view in views {
            pending.memberships_by_view.remove(view);
        }
        if pending.memberships_by_view.is_empty() {
            s.pending_create_memberships.remove(local_id);
        }
    }

    /// server response is the source of truth for the open list too
    /// coauthors arrive in full and tags carry their own points
    fn apply_authoritative_collaborator_rows(
        &self,
        s: &mut RisePostContentRepositoryState,
        post: &GetPostFeedResponse,
        key: &str,
    ) {
        let Some(entry) = s.collaborators_by_post_id.get_mut(key) else {
            return;
        };
        let coauthor = PostCollaboratorKind::Coauthor.raw_value();
        let tag = PostCollaboratorKind::Tag.raw_value();
        let accepted = PostCollaboratorStatus::Accepted.raw_value();
        let accepted_ids: HashSet<&str> = post.coauthors.iter().map(|user| user.id.as_str()).collect();
        let mut rows: Vec<PostCollaboratorRow> = entry
            .items
            .iter()
            .filter(|row| {
                row.kind == coauthor
                    && row.status == PostCollaboratorStatus::Pending.raw_value()
                    && !accepted_ids.contains(row.user.id.as_str())
            })
            .cloned()
            .collect();
        rows.extend(post.coauthors.iter().map(|user| PostCollaboratorRow {
            user: user.clone(),
            kind: coauthor.to_owned(),
            status: accepted.to_owned(),
            show_in_grid: if user.id == self.account_id {
                post.my_collaboration.as_ref().map(|mine| mine.show_in_grid).unwrap_or(true)
            } else {
                true
            },
            placements: Vec::new(),
        }));
        rows.extend(post.tagged_users.iter().filter_map(|tagged| {
            let user = tagged.user.clone()?;
            Some(PostCollaboratorRow {
                user,
                kind: tag.to_owned(),
                status: accepted.to_owned(),
                show_in_grid: true,
                placements: tagged.placements.clone(),
            })
        }));
        let mut shown_at: HashMap<String, usize> = HashMap::new();
        for (offset, row) in entry.items.iter().enumerate() {
            shown_at.entry(row.id()).or_insert(offset);
        }
        let mut ordered: Vec<(usize, PostCollaboratorRow)> = rows.into_iter().enumerate().collect();
        ordered.sort_by_key(|(offset, row)| (shown_at.get(&row.id()).copied().unwrap_or(usize::MAX), *offset));
        entry.items = ordered
            .into_iter()
            .map(|(_, row)| row)
            .take(Self::MAXIMUM_COLLABORATOR_ITEMS)
            .collect();
    }

    fn membership_mutations(
        local_id: &str,
        added_views: &[String],
        removed_views: &[String],
        version: i64,
    ) -> Vec<RiseStoreMutation> {
        let mut mutations: Vec<RiseStoreMutation> = Vec::with_capacity(added_views.len() + removed_views.len());
        for view in removed_views {
            mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                namespace: ENTITY_NAMESPACE.clone(),
                view: view.clone(),
                local_id: local_id.to_owned(),
                if_version_at_most: None,
            });
        }
        for view in added_views {
            let position = RisePositionKey::make(|builder| {
                builder.append_signed_int64(-version);
                builder.append_string(local_id);
            });
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: ENTITY_NAMESPACE.clone(),
                view: view.clone(),
                membership: RiseEntityViewMembership::new(local_id.to_owned(), position, version),
            });
        }
        mutations
    }

    pub async fn adjust_comments_count(
        &self,
        post_id: &str,
        delta: i64,
    ) -> anyhow::Result<Option<RisePostContentPresentationSnapshot>> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await || post_id.parse::<i64>().is_err() {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        if delta == 0 {
            return Ok(None);
        }
        let Some(stored_entity) = self.entity_by_server_id(post_id).await? else {
            return Ok(None);
        };
        let Ok(mut stored) = serde_json::from_slice::<RisePostStoredEntity>(&stored_entity.data) else {
            return Ok(None);
        };
        let count = (stored.post.comments_count + delta).max(0);
        if count == stored.post.comments_count {
            return Ok(None);
        }
        stored.post.comments_count = count;
        let data = serde_json::to_vec(&stored)?;
        let now = Self::now_milliseconds();
        let mutations = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, post_id);
            vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(stored_entity.local_id.clone(), data, s.store_version, now),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }]
        });
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(Some(self.refresh_active_pages_from_store().await))
    }

    pub async fn set_author_relationship(
        &self,
        author_id: &str,
        is_subscribed: bool,
        is_subscriber: bool,
        is_friend: bool,
    ) -> anyhow::Result<Option<RisePostContentPresentationSnapshot>> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        if author_id.is_empty() {
            return Ok(None);
        }
        let post_ids: HashSet<String> = self.state.with(|s| {
            s.pages_by_scope_key
                .values()
                .flat_map(|page| page.items.iter())
                .filter(|item| item.author.id.as_deref() == Some(author_id) || item.author_id == author_id)
                .map(|item| item.id.clone())
                .collect()
        });
        let mut patched: Vec<(String, String, RisePostStoredEntity)> = Vec::new();
        for post_id in post_ids {
            let Some(stored_entity) = self.entity_by_server_id(&post_id).await? else {
                continue;
            };
            let Ok(mut stored) = serde_json::from_slice::<RisePostStoredEntity>(&stored_entity.data) else {
                continue;
            };
            let author = &stored.post.author;
            if author.is_subscribed == is_subscribed
                && author.is_subscriber == is_subscriber
                && author.is_friend == is_friend
            {
                continue;
            }
            stored.post.author.is_subscribed = is_subscribed;
            stored.post.author.is_subscriber = is_subscriber;
            stored.post.author.is_friend = is_friend;
            patched.push((stored_entity.local_id, post_id, stored));
        }
        if patched.is_empty() {
            return Ok(None);
        }
        let now = Self::now_milliseconds();
        let mut encoded: Vec<(String, String, Vec<u8>)> = Vec::with_capacity(patched.len());
        for (local_id, post_id, stored) in patched {
            encoded.push((local_id, post_id, serde_json::to_vec(&stored)?));
        }
        let mutations = self.state.with(|s| {
            Self::advance_store_version(s);
            let store_version = s.store_version;
            encoded
                .into_iter()
                .map(|(local_id, post_id, data)| {
                    Self::record_mutation_version(s, &post_id);
                    RiseStoreMutation::UpsertEntity {
                        namespace: ENTITY_NAMESPACE.clone(),
                        entity: RiseStoreEntity::new(local_id, data, store_version, now),
                        policy: RiseEntityWritePolicy::ServerAuthoritative,
                    }
                })
                .collect::<Vec<_>>()
        });
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(Some(self.refresh_active_pages_from_store().await))
    }

    /// live reposts tabs of one profile
    fn repost_tab_views(s: &RisePostContentRepositoryState, owner_id: &str) -> Vec<String> {
        s.scope_index
            .slots
            .iter()
            .filter(|(_, slot)| {
                matches!(
                    s.scopes_by_key.get(&slot.scope_key),
                    Some(RisePostPageScope::Profile { user_id, kind: RisePostProfileKind::Reposts, .. })
                        if user_id == owner_id
                )
            })
            .map(|(view, _)| view.clone())
            .collect()
    }

    /// positions of one row in the given views read before it leaves them so a rollback can put it back
    async fn current_memberships(&self, views: &[String], local_id: &str) -> HashMap<String, RiseEntityViewMembership> {
        let mut memberships: HashMap<String, RiseEntityViewMembership> = HashMap::new();
        for view in views {
            let Ok(window) = self
                .engine
                .entity_window(RiseEntityWindowQuery::new(
                    ENTITY_NAMESPACE.clone(),
                    view.clone(),
                    RiseEntityWindow::First { limit: Self::MAXIMUM_VIEW_ITEMS },
                ))
                .await
            else {
                continue;
            };
            let Some(item) = window.items.iter().find(|item| item.entity.local_id == local_id) else {
                continue;
            };
            memberships.insert(
                view.clone(),
                RiseEntityViewMembership::new(local_id.to_owned(), item.position_key.clone(), item.membership_version),
            );
        }
        memberships
    }

    pub async fn begin_repost(
        &self,
        post: &GetPostFeedResponse,
        desired: bool,
        content: Option<String>,
    ) -> anyhow::Result<RisePostContentRepositoryRepostTicket> {
        let root_id = post.id.parse::<i64>().ok();
        let Some(root_id) = root_id else {
            return Err(RisePostRepositoryError::StaleAccount.into());
        };
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        self.load_scope_index_if_needed().await;
        let root_id_string = root_id.to_string();
        let request_id = Uuid::new_v4().uuid_string().to_lowercase();
        let root_entity = self.entity_by_server_id(&root_id_string).await?;
        let root_local_id =
            root_entity.as_ref().map(|entity| entity.local_id.clone()).unwrap_or_else(|| root_id_string.clone());
        let mut root = root_entity
            .and_then(|entity| serde_json::from_slice::<RisePostStoredEntity>(&entity.data).ok())
            .unwrap_or_else(|| RisePostStoredEntity { post: post.clone() });
        root.post.id = root_id_string.clone();
        let previous_root = root.clone();
        if root.post.is_reposted != desired {
            root.post.repost_count = (root.post.repost_count + if desired { 1 } else { -1 }).max(0);
        }
        root.post.is_reposted = desired;
        root.post.my_repost_content = if desired { content.clone() } else { None };
        if !desired {
            root.post.my_repost_id = None;
        }

        let repost_tab_views = self.state.with(|s| Self::repost_tab_views(s, &self.account_id));
        let repost_tab_memberships = if desired {
            HashMap::new()
        } else {
            self.current_memberships(&repost_tab_views, &root_local_id).await
        };

        let data = serde_json::to_vec(&root)?;
        let now = Self::now_milliseconds();
        let (generation, mutations) = self.state.with(|s| {
            let generation = Self::next_mutation_generation(s);
            s.repost_generations.insert(root_id_string.clone(), generation);
            s.repost_desired_states.insert(root_id_string.clone(), desired);
            s.repost_queues.entry(root_id_string.clone()).or_default().push(generation);
            s.in_flight_repost_keys.insert(root_id_string.clone());
            Self::trim_mutation_generation_state(s);
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &root_id_string);
            let store_version = s.store_version;
            let mut mutations = vec![
                RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(root_local_id.clone(), data, store_version, now),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                },
                RiseStoreMutation::SetEntityAlias {
                    namespace: ENTITY_NAMESPACE.clone(),
                    alias: RiseEntityAlias::new(RiseEntityAliasKind::server_id(), root_id_string.clone()),
                    local_id: root_local_id.clone(),
                },
            ];
            if desired {
                mutations.extend(Self::membership_mutations(&root_local_id, &repost_tab_views, &[], store_version));
            } else {
                mutations.extend(Self::membership_mutations(&root_local_id, &[], &repost_tab_views, store_version));
            }
            (generation, mutations)
        });
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        let snapshot = self.refresh_active_pages_from_store().await;
        Ok(RisePostContentRepositoryRepostTicket {
            root_id,
            root_local_id,
            previous_root,
            desired,
            content,
            request_id,
            generation,
            repost_tab_views,
            repost_tab_memberships,
            snapshot,
        })
    }

    pub async fn finish_repost(
        &self,
        ticket: &RisePostContentRepositoryRepostTicket,
    ) -> anyhow::Result<RisePostContentPresentationSnapshot> {
        let root_key = ticket.root_id.to_string();
        self.wait_for_repost_turn(&root_key, ticket.generation).await;
        let client_session_id = self.session_id.uuid_string().to_lowercase();
        let client_sequence = i64::try_from(ticket.generation).unwrap_or(i64::MAX);
        let response: RisePostRepostAcknowledgement = if ticket.desired {
            let reply: CreateRepostResponse = self
                .rpc
                .call(
                    RisePostEndpoints::create_repost(),
                    &CreateRepostRequest {
                        client_request_id: Some(ticket.request_id.clone()),
                        client_sequence: Some(client_sequence),
                        client_session_id: Some(client_session_id.clone()),
                        content: ticket.content.clone(),
                        entities: None,
                        post_id: Some(ticket.root_id),
                    },
                )
                .await?;
            RisePostRepostAcknowledgement::from(&reply)
        } else {
            let reply: DeleteRepostResponse = self
                .rpc
                .call(
                    RisePostEndpoints::delete_repost(),
                    &DeleteRepostRequest {
                        client_request_id: Some(ticket.request_id.clone()),
                        client_sequence: Some(client_sequence),
                        client_session_id: Some(client_session_id.clone()),
                        post_id: Some(ticket.root_id),
                        repost_user_id: None,
                    },
                )
                .await?;
            RisePostRepostAcknowledgement::from(&reply)
        };
        if response.client_request_id.as_deref() != Some(ticket.request_id.as_str())
            || response.client_session_id.as_deref() != Some(client_session_id.as_str())
            || response.client_sequence != Some(client_sequence)
        {
            return Err(RisePostMutationError::Server("mutation_ack_mismatch".to_owned()).into());
        }
        if self.state.with(|s| s.is_shutdown)
            || !self.is_current_account().await
            || self.state.with(|s| s.repost_generations.get(&root_key).copied()) != Some(ticket.generation)
        {
            return Err(CancellationError.into());
        }
        let Some(root_entity) = self.entity_by_server_id(&root_key).await? else {
            return Err(CancellationError.into());
        };
        let Ok(mut root) = serde_json::from_slice::<RisePostStoredEntity>(&root_entity.data) else {
            return Err(CancellationError.into());
        };
        root.post.is_reposted = response.is_reposted;
        root.post.repost_count = response.repost_count.max(0);
        root.post.my_repost_id = response.my_repost_id.map(|id| id.to_string());
        root.post.my_repost_content =
            if ticket.desired { response.my_repost_content.or_else(|| ticket.content.clone()) } else { None };
        root.post.my_repost_entities = if ticket.desired { response.my_repost_entities } else { None };
        let data = serde_json::to_vec(&root)?;
        let now = Self::now_milliseconds();
        let mutations = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &root_key);
            vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(root_entity.local_id.clone(), data, s.store_version, now),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }]
        });
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        self.state.with(|s| {
            Self::complete_repost_turn(s, &root_key, ticket.generation);
            Self::trim_mutation_generation_state(s);
        });
        Ok(self.refresh_active_pages_from_store().await)
    }

    pub async fn begin_foreign_repost_removal(
        &self,
        root_id: i64,
        owner_id: &str,
    ) -> anyhow::Result<RisePostContentRepositoryForeignRepostTicket> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await || root_id <= 0 || owner_id.is_empty()
        {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        self.load_scope_index_if_needed().await;
        let root_key = root_id.to_string();
        let root_local_id = self
            .entity_by_server_id(&root_key)
            .await?
            .map(|entity| entity.local_id)
            .unwrap_or_else(|| root_key.clone());
        let views = self.state.with(|s| Self::repost_tab_views(s, owner_id));
        let memberships = self.current_memberships(&views, &root_local_id).await;
        let (generation, mutations) = self.state.with(|s| {
            let generation = Self::next_mutation_generation(s);
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &root_key);
            let mutations: Vec<RiseStoreMutation> = memberships
                .keys()
                .map(|view| RiseStoreMutation::RemoveEntityViewMembership {
                    namespace: ENTITY_NAMESPACE.clone(),
                    view: view.clone(),
                    local_id: root_local_id.clone(),
                    if_version_at_most: None,
                })
                .collect();
            (generation, mutations)
        });
        if !mutations.is_empty() {
            self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        }
        let snapshot = self.refresh_active_pages_from_store().await;
        Ok(RisePostContentRepositoryForeignRepostTicket {
            root_id,
            root_local_id,
            owner_id: owner_id.to_owned(),
            request_id: Uuid::new_v4().uuid_string().to_lowercase(),
            generation,
            memberships,
            snapshot,
        })
    }

    pub async fn finish_foreign_repost_removal(
        &self,
        ticket: &RisePostContentRepositoryForeignRepostTicket,
    ) -> anyhow::Result<RisePostContentPresentationSnapshot> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(CancellationError.into());
        }
        let client_session_id = self.session_id.uuid_string().to_lowercase();
        let client_sequence = i64::try_from(ticket.generation).unwrap_or(i64::MAX);
        let response: DeleteRepostResponse = self
            .rpc
            .call(
                RisePostEndpoints::delete_repost(),
                &DeleteRepostRequest {
                    client_request_id: Some(ticket.request_id.clone()),
                    client_sequence: Some(client_sequence),
                    client_session_id: Some(client_session_id.clone()),
                    post_id: Some(ticket.root_id),
                    repost_user_id: Some(ticket.owner_id.clone()),
                },
            )
            .await?;
        let response = RisePostRepostAcknowledgement::from(&response);
        if response.client_request_id.as_deref() != Some(ticket.request_id.as_str())
            || response.client_session_id.as_deref() != Some(client_session_id.as_str())
            || response.client_sequence != Some(client_sequence)
        {
            return Err(RisePostMutationError::Server("mutation_ack_mismatch".to_owned()).into());
        }
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(CancellationError.into());
        }
        Ok(self.refresh_active_pages_from_store().await)
    }

    pub async fn rollback_foreign_repost_removal(
        &self,
        ticket: &RisePostContentRepositoryForeignRepostTicket,
    ) -> RisePostContentPresentationSnapshot {
        let mutations = self.state.with(|s| {
            if s.is_shutdown || ticket.memberships.is_empty() {
                return None;
            }
            Self::advance_store_version(s);
            Some(Self::restored_memberships(&ticket.memberships, s.store_version))
        });
        let Some(mutations) = mutations else {
            return self.state.with(|s| self.make_snapshot(s));
        };
        let _ = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await;
        self.refresh_active_pages_from_store().await
    }

    /// recorded positions written back at the given version
    /// store silently drops a membership older than its entity and the entity may have been
    /// rewritten meanwhile
    fn restored_memberships(
        memberships: &HashMap<String, RiseEntityViewMembership>,
        version: i64,
    ) -> Vec<RiseStoreMutation> {
        memberships
            .iter()
            .map(|(view, membership)| RiseStoreMutation::UpsertEntityViewMembership {
                namespace: ENTITY_NAMESPACE.clone(),
                view: view.clone(),
                membership: RiseEntityViewMembership::new(
                    membership.local_id.clone(),
                    membership.position_key.clone(),
                    version,
                ),
            })
            .collect()
    }

    pub async fn rollback_repost(
        &self,
        ticket: &RisePostContentRepositoryRepostTicket,
    ) -> RisePostContentPresentationSnapshot {
        let root_key = ticket.root_id.to_string();
        let _release = RisePostContentRepositoryRepostTurnRelease {
            repository: self,
            root_key: root_key.clone(),
            generation: ticket.generation,
        };
        if self.state.with(|s| s.is_shutdown) {
            return self.state.with(|s| self.make_snapshot(s));
        }
        if self.state.with(|s| s.repost_generations.get(&root_key).copied()) != Some(ticket.generation) {
            return self.refresh_active_pages_from_store().await;
        }
        self.state.with(Self::advance_store_version);
        let Ok(previous_root_data) = serde_json::to_vec(&ticket.previous_root) else {
            return self.refresh_active_pages_from_store().await;
        };
        let now = Self::now_milliseconds();
        let mutations = self.state.with(|s| {
            let store_version = s.store_version;
            let mut mutations = vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(ticket.root_local_id.clone(), previous_root_data, store_version, now),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }];
            if ticket.desired {
                mutations.extend(Self::membership_mutations(
                    &ticket.root_local_id,
                    &[],
                    &ticket.repost_tab_views,
                    store_version,
                ));
            } else {
                mutations.extend(Self::restored_memberships(&ticket.repost_tab_memberships, store_version));
            }
            mutations
        });
        let _ = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await;
        self.state.with(Self::trim_mutation_generation_state);
        self.refresh_active_pages_from_store().await
    }

    pub async fn begin_opinion_update(
        &self,
        post: &GetPostFeedResponse,
        content: String,
        entities: Option<String>,
    ) -> anyhow::Result<RisePostContentRepositoryOpinionTicket> {
        let Ok(post_id) = post.id.parse::<i64>() else {
            return Err(RisePostRepositoryError::StaleAccount.into());
        };
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        let key = post.id.clone();
        let mut stored_entities: Vec<RiseStoreEntity> = Vec::new();
        if let Some(stored) = self.entity_by_server_id(&key).await? {
            stored_entities.push(stored);
        }
        let ticket_entities = if content.is_empty() { None } else { entities };
        let mut rewritten: Vec<(String, Vec<u8>)> = Vec::new();
        for entity in &stored_entities {
            let Ok(mut stored) = serde_json::from_slice::<RisePostStoredEntity>(&entity.data) else {
                continue;
            };
            stored.post.my_repost_content = if content.is_empty() { None } else { Some(content.clone()) };
            stored.post.my_repost_entities = ticket_entities.clone();
            rewritten.push((entity.local_id.clone(), serde_json::to_vec(&stored)?));
        }
        let now = Self::now_milliseconds();
        let (generation, mutations) = self.state.with(|s| {
            let generation = Self::next_mutation_generation(s);
            s.opinion_generations.insert(key.clone(), generation);
            s.in_flight_opinion_keys.insert(key.clone());
            Self::trim_mutation_generation_state(s);
            Self::advance_store_version(s);
            let store_version = s.store_version;
            let mutations: Vec<RiseStoreMutation> = rewritten
                .into_iter()
                .map(|(local_id, data)| RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(local_id, data, store_version, now),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                })
                .collect();
            (generation, mutations)
        });
        if !mutations.is_empty() {
            self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        }
        let snapshot = self.refresh_active_pages_from_store().await;
        Ok(RisePostContentRepositoryOpinionTicket {
            post_id,
            content,
            entities: ticket_entities,
            generation,
            previous_entities: stored_entities,
            snapshot,
        })
    }

    pub async fn finish_opinion_update(
        &self,
        ticket: &RisePostContentRepositoryOpinionTicket,
    ) -> anyhow::Result<RisePostContentPresentationSnapshot> {
        let response: UpdateRepostResponse = self
            .rpc
            .call(
                RisePostEndpoints::update_repost(),
                &UpdateRepostRequest {
                    content: Some(ticket.content.clone()),
                    entities: ticket.entities.clone(),
                    post_id: Some(ticket.post_id),
                    repost_user_id: None,
                },
            )
            .await?;
        let key = ticket.post_id.to_string();
        if self.state.with(|s| s.is_shutdown)
            || !self.is_current_account().await
            || self.state.with(|s| s.opinion_generations.get(&key).copied()) != Some(ticket.generation)
        {
            return Err(CancellationError.into());
        }
        if let Some(entity) = self.entity_by_server_id(&key).await?
            && let Ok(mut stored) = serde_json::from_slice::<RisePostStoredEntity>(&entity.data)
        {
            stored.post.is_reposted = response.is_reposted;
            stored.post.repost_count = response.repost_count.max(0);
            stored.post.my_repost_id = response.my_repost_id.map(|id| id.to_string());
            stored.post.my_repost_content = response
                .my_repost_content
                .clone()
                .or_else(|| (!ticket.content.is_empty()).then(|| ticket.content.clone()));
            stored.post.my_repost_entities = response.my_repost_entities.clone().or_else(|| ticket.entities.clone());
            let data = serde_json::to_vec(&stored)?;
            let now = Self::now_milliseconds();
            let mutations = self.state.with(|s| {
                Self::advance_store_version(s);
                Self::record_mutation_version(s, &key);
                vec![RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(entity.local_id.clone(), data, s.store_version, now),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                }]
            });
            self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        }
        self.state.with(|s| {
            s.in_flight_opinion_keys.remove(&key);
            Self::trim_mutation_generation_state(s);
        });
        Ok(self.refresh_active_pages_from_store().await)
    }

    pub async fn rollback_opinion_update(
        &self,
        ticket: &RisePostContentRepositoryOpinionTicket,
    ) -> RisePostContentPresentationSnapshot {
        let key = ticket.post_id.to_string();
        let now = Self::now_milliseconds();
        let mutations = self.state.with(|s| {
            if s.is_shutdown || s.opinion_generations.get(&key).copied() != Some(ticket.generation) {
                return None;
            }
            Self::advance_store_version(s);
            let store_version = s.store_version;
            Some(
                ticket
                    .previous_entities
                    .iter()
                    .map(|entity| RiseStoreMutation::UpsertEntity {
                        namespace: ENTITY_NAMESPACE.clone(),
                        entity: RiseStoreEntity::new(entity.local_id.clone(), entity.data.clone(), store_version, now),
                        policy: RiseEntityWritePolicy::ServerAuthoritative,
                    })
                    .collect::<Vec<_>>(),
            )
        });
        let Some(mutations) = mutations else {
            return self.state.with(|s| self.make_snapshot(s));
        };
        if !mutations.is_empty() {
            let _ = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await;
        }
        self.state.with(|s| {
            s.in_flight_opinion_keys.remove(&key);
            Self::trim_mutation_generation_state(s);
        });
        self.refresh_active_pages_from_store().await
    }

    pub async fn begin_delete(&self, post_id: i64) -> anyhow::Result<RisePostContentRepositoryDeleteTicket> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await || post_id <= 0 {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        self.load_scope_index_if_needed().await;
        let key = post_id.to_string();
        self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &key);
            s.deleted_post_versions.insert(key.clone(), s.store_version);
            Self::trim_mutation_versions(s);
        });
        let entity = self.entity_by_server_id(&key).await?;
        let Some(local_id) = entity.map(|entity| entity.local_id) else {
            return Ok(RisePostContentRepositoryDeleteTicket {
                post_id,
                local_id: None,
                memberships: HashMap::new(),
                snapshot: self.state.with(|s| self.make_snapshot(s)),
            });
        };
        let views: Vec<String> = self.state.with(|s| s.scope_index.slots.keys().cloned().collect());
        let memberships = self.current_memberships(&views, &local_id).await;
        let mutations: Vec<RiseStoreMutation> = memberships
            .keys()
            .map(|view| RiseStoreMutation::RemoveEntityViewMembership {
                namespace: ENTITY_NAMESPACE.clone(),
                view: view.clone(),
                local_id: local_id.clone(),
                if_version_at_most: None,
            })
            .collect();
        if !mutations.is_empty() {
            self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        }
        let snapshot = self.refresh_active_pages_from_store().await;
        Ok(RisePostContentRepositoryDeleteTicket { post_id, local_id: Some(local_id), memberships, snapshot })
    }

    pub async fn finish_delete(
        &self,
        ticket: &RisePostContentRepositoryDeleteTicket,
    ) -> anyhow::Result<RisePostContentPresentationSnapshot> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(CancellationError.into());
        }
        let _: DeletePostResponse = self
            .rpc
            .call(RisePostEndpoints::delete_post(), &DeletePostRequest { post_id: Some(ticket.post_id) })
            .await?;
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(CancellationError.into());
        }
        if let Some(local_id) = &ticket.local_id {
            self.engine
                .write(RiseStoreWriteBatch::new(
                    vec![RiseStoreMutation::RemoveEntity { namespace: ENTITY_NAMESPACE.clone(), local_id: local_id.clone() }],
                    HashSet::new(),
                ))
                .await?;
        }
        Ok(self.refresh_active_pages_from_store().await)
    }

    pub async fn rollback_delete(
        &self,
        ticket: &RisePostContentRepositoryDeleteTicket,
    ) -> RisePostContentPresentationSnapshot {
        let mutations = self.state.with(|s| {
            s.deleted_post_versions.remove(&ticket.post_id.to_string());
            if s.is_shutdown || ticket.local_id.is_none() {
                return None;
            }
            Self::advance_store_version(s);
            Some(Self::restored_memberships(&ticket.memberships, s.store_version))
        });
        let Some(mutations) = mutations else {
            return self.state.with(|s| self.make_snapshot(s));
        };
        if !mutations.is_empty() {
            let _ = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await;
        }
        self.refresh_active_pages_from_store().await
    }

    pub async fn begin_collaboration(
        &self,
        post_id: i64,
        operation: RisePostContentRepositoryCollaborationOperation,
    ) -> anyhow::Result<RisePostContentRepositoryCollaborationTicket> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await || post_id <= 0 {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        self.load_scope_index_if_needed().await;
        let key = post_id.to_string();
        let generation = self.state.with(|s| {
            let generation = Self::next_mutation_generation(s);
            s.collaboration_generations.insert(key.clone(), generation);
            s.in_flight_collaboration_keys.insert(key.clone());
            Self::trim_mutation_generation_state(s);
            generation
        });
        let viewer = self.self_collaborator().await;
        let previous_collaborators = self.state.with(|s| {
            let previous = s.collaborators_by_post_id.get(&key).cloned();
            Self::apply_optimistic_collaborator_rows(s, &operation, &key, &viewer);
            previous
        });

        let stored_entity = self.entity_by_server_id(&key).await?;
        let decoded = stored_entity.and_then(|entity| {
            serde_json::from_slice::<RisePostStoredEntity>(&entity.data).ok().map(|stored| (entity, stored))
        });
        let Some((stored_entity, mut stored)) = decoded else {
            return Ok(RisePostContentRepositoryCollaborationTicket {
                post_id,
                local_id: None,
                operation,
                previous: None,
                removed_memberships: HashMap::new(),
                added_views: Vec::new(),
                previous_collaborators,
                generation,
                snapshot: self.state.with(|s| self.make_snapshot(s)),
            });
        };
        let previous = stored.clone();
        Self::apply_optimistic_collaboration(&operation, &mut stored.post, &viewer);

        let (grid_plan, mentions_plan) = Self::collaboration_membership_plan(&operation);
        let (grid_views, grid_candidate_views, mention_views) = self.state.with(|s| {
            let (grid, candidates) = if grid_plan.is_some() {
                (
                    self.own_profile_views(s, &Self::grid_kinds(&stored.post)),
                    self.own_profile_views(s, &Self::grid_candidate_kinds(&stored.post)),
                )
            } else {
                (Vec::new(), Vec::new())
            };
            let mentions = if mentions_plan.is_some() {
                self.own_profile_views(s, &HashSet::from([RisePostProfileKind::Mentions]))
            } else {
                Vec::new()
            };
            (grid, candidates, mentions)
        });
        let mut added_views: Vec<String> = Vec::new();
        let mut removed_views: Vec<String> = Vec::new();
        if grid_plan == Some(false) {
            removed_views.extend(grid_candidate_views.iter().cloned());
        }
        if mentions_plan == Some(false) {
            removed_views.extend(mention_views.iter().cloned());
        }
        if grid_plan == Some(true) {
            added_views.extend(grid_views.iter().cloned());
        }
        if mentions_plan == Some(true) {
            added_views.extend(mention_views.iter().cloned());
        }
        let removed_memberships = self.current_memberships(&removed_views, &stored_entity.local_id).await;

        let data = serde_json::to_vec(&stored)?;
        let store_version = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &key);
            s.store_version
        });
        let mut mutations: Vec<RiseStoreMutation> = vec![RiseStoreMutation::UpsertEntity {
            namespace: ENTITY_NAMESPACE.clone(),
            entity: RiseStoreEntity::new(
                stored_entity.local_id.clone(),
                data,
                store_version,
                Self::now_milliseconds(),
            ),
            policy: RiseEntityWritePolicy::ServerAuthoritative,
        }];
        mutations.extend(Self::membership_mutations(
            &stored_entity.local_id,
            &added_views,
            &removed_views,
            store_version,
        ));
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(RisePostContentRepositoryCollaborationTicket {
            post_id,
            local_id: Some(stored_entity.local_id),
            operation,
            previous: Some(previous),
            removed_memberships,
            added_views,
            previous_collaborators,
            generation,
            snapshot: self.refresh_active_pages_from_store().await,
        })
    }

    pub async fn finish_collaboration(
        &self,
        ticket: &RisePostContentRepositoryCollaborationTicket,
    ) -> anyhow::Result<(Vec<PostCollaboratorRejection>, RisePostContentPresentationSnapshot)> {
        let key = ticket.post_id.to_string();
        let response = self.request_collaboration(ticket).await?;
        if self.state.with(|s| s.is_shutdown)
            || !self.is_current_account().await
            || self.state.with(|s| s.collaboration_generations.get(&key).copied()) != Some(ticket.generation)
        {
            return Err(CancellationError.into());
        }
        let rejected = response.rejected.clone();
        if let Some(dto) = response.post.as_ref()
            && let Some(incoming) = dto.as_feed_response()
            && let Some(target) = self.entity_by_server_id(&key).await?
        {
            let existing = serde_json::from_slice::<RisePostStoredEntity>(&target.data).ok();
            let mut post = Self::canonicalize(
                incoming,
                dto,
                existing.as_ref().map(|stored| &stored.post).or(ticket.previous.as_ref().map(|stored| &stored.post)),
            );
            post.presentation_id = existing
                .as_ref()
                .and_then(|stored| stored.post.presentation_id.clone())
                .or_else(|| Some(target.local_id.clone()));
            let store_version = self.state.with(|s| {
                Self::advance_store_version(s);
                Self::record_mutation_version(s, &post.id);
                s.store_version
            });
            let data = serde_json::to_vec(&RisePostStoredEntity { post: post.clone() })?;
            let mut mutations: Vec<RiseStoreMutation> = vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(
                    target.local_id.clone(),
                    data,
                    store_version,
                    post.updated_at_ms.unwrap_or_else(Self::now_milliseconds),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }];
            let membership = self
                .state
                .with(|s| self.authoritative_membership_mutations(s, &target.local_id, &post, store_version));
            mutations.extend(membership);
            self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
            self.state.with(|s| self.apply_authoritative_collaborator_rows(s, &post, &key));
        }
        self.state.with(|s| {
            s.in_flight_collaboration_keys.remove(&key);
            Self::trim_mutation_generation_state(s);
        });
        Ok((rejected, self.refresh_active_pages_from_store().await))
    }

    pub async fn rollback_collaboration(
        &self,
        ticket: &RisePostContentRepositoryCollaborationTicket,
    ) -> RisePostContentPresentationSnapshot {
        let key = ticket.post_id.to_string();
        let restore = self.state.with(|s| {
            s.in_flight_collaboration_keys.remove(&key);
            let is_current_generation = s.collaboration_generations.get(&key).copied() == Some(ticket.generation);
            if is_current_generation {
                match &ticket.previous_collaborators {
                    Some(previous) => {
                        s.collaborators_by_post_id.insert(key.clone(), previous.clone());
                    }
                    None => {
                        s.collaborators_by_post_id.remove(&key);
                    }
                }
            }
            let restorable = !s.is_shutdown && is_current_generation;
            let restore = match (restorable, &ticket.local_id, &ticket.previous) {
                (true, Some(local_id), Some(previous)) => serde_json::to_vec(previous).ok().map(|data| {
                    Self::advance_store_version(s);
                    (local_id.clone(), data, s.store_version)
                }),
                _ => None,
            };
            if restore.is_none() {
                Self::trim_mutation_generation_state(s);
            }
            restore
        });
        let Some((local_id, data, version)) = restore else {
            return self.refresh_active_pages_from_store().await;
        };
        let mut mutations: Vec<RiseStoreMutation> = vec![RiseStoreMutation::UpsertEntity {
            namespace: ENTITY_NAMESPACE.clone(),
            entity: RiseStoreEntity::new(local_id.clone(), data, version, Self::now_milliseconds()),
            policy: RiseEntityWritePolicy::ServerAuthoritative,
        }];
        for view in &ticket.added_views {
            mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                namespace: ENTITY_NAMESPACE.clone(),
                view: view.clone(),
                local_id: local_id.clone(),
                if_version_at_most: None,
            });
        }
        mutations.extend(Self::restored_memberships(&ticket.removed_memberships, version));
        let _ = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await;
        self.state.with(Self::trim_mutation_generation_state);
        self.refresh_active_pages_from_store().await
    }

    pub async fn load_post_collaborators(
        &self,
        post_id: i64,
        kind: Option<PostCollaboratorKind>,
        status: Option<PostCollaboratorStatus>,
        reset: bool,
    ) -> RisePostContentPresentationSnapshot {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await || post_id <= 0 {
            return self.state.with(|s| self.make_snapshot(s));
        }
        let key = post_id.to_string();
        let started = self.state.with(|s| {
            let mut entry = s.collaborators_by_post_id.get(&key).cloned().unwrap_or_default();
            if entry.is_loading {
                return None;
            }
            let starts_over = reset || !entry.loaded_once || entry.kind != kind || entry.status != status;
            if !(starts_over || entry.has_more) {
                return None;
            }
            s.collaborators_access_clock = s.collaborators_access_clock.wrapping_add(1);
            let generation = Self::next_mutation_generation(s);
            entry.kind = kind;
            entry.status = status;
            entry.is_loading = true;
            entry.generation = generation;
            entry.access = s.collaborators_access_clock;
            if starts_over {
                entry.relative_id = None;
            }
            let cursor = if starts_over { None } else { entry.relative_id };
            s.collaborators_by_post_id.insert(key.clone(), entry);
            Self::trim_collaborator_entries(s);
            Some((generation, starts_over, cursor))
        });
        let Some((generation, starts_over, cursor)) = started else {
            return self.state.with(|s| self.make_snapshot(s));
        };
        let request = GetPostCollaboratorsRequest {
            kind: kind.map(|kind| kind.raw_value().to_owned()),
            limit: Some(Self::COLLABORATORS_PAGE_LIMIT),
            post_id: Some(post_id),
            relative_id: cursor,
            status: status.map(|status| status.raw_value().to_owned()),
        };
        let result: anyhow::Result<GetPostCollaboratorsResponse> =
            self.rpc.call(RisePostEndpoints::get_post_collaborators(), &request).await;
        match result {
            Ok(response) => {
                if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
                    return self.state.with(|s| self.make_snapshot(s));
                }
                self.state.with(|s| {
                    let current = s.collaborators_by_post_id.get(&key).map(|entry| entry.generation);
                    if current != Some(generation) {
                        return self.make_snapshot(s);
                    }
                    let mut updated = s.collaborators_by_post_id.get(&key).cloned().unwrap_or_default();
                    let rows: Vec<PostCollaboratorRow> = response.items.iter().filter_map(|row| row.as_row()).collect();
                    let mut merged: Vec<PostCollaboratorRow> =
                        if starts_over { Vec::new() } else { std::mem::take(&mut updated.items) };
                    let mut seen: HashSet<String> = merged.iter().map(PostCollaboratorRow::id).collect();
                    let last_since = rows.last().map(|row| row.user.since_ms);
                    for row in rows {
                        if seen.insert(row.id()) {
                            merged.push(row);
                        }
                    }
                    merged.truncate(Self::MAXIMUM_COLLABORATOR_ITEMS);
                    updated.items = merged;
                    updated.has_more =
                        response.has_more && updated.items.len() < Self::MAXIMUM_COLLABORATOR_ITEMS;
                    updated.relative_id = response.relative_id.or(last_since);
                    updated.is_loading = false;
                    updated.loaded_once = true;
                    s.collaborators_by_post_id.insert(key.clone(), updated);
                    self.make_snapshot(s)
                })
            }
            Err(_) => self.state.with(|s| {
                let current = s.collaborators_by_post_id.get(&key).map(|entry| entry.generation);
                if current != Some(generation) {
                    return self.make_snapshot(s);
                }
                let mut failed = s.collaborators_by_post_id.get(&key).cloned().unwrap_or_default();
                failed.is_loading = false;
                failed.loaded_once = true;
                s.collaborators_by_post_id.insert(key.clone(), failed);
                self.make_snapshot(s)
            }),
        }
    }

    async fn request_collaboration(
        &self,
        ticket: &RisePostContentRepositoryCollaborationTicket,
    ) -> anyhow::Result<RisePostCollaborationResponse> {
        let post_id = Some(ticket.post_id);
        let response: RisePostCollaborationResponse = match &ticket.operation {
            RisePostContentRepositoryCollaborationOperation::AcceptInvite => {
                self.rpc
                    .call(RisePostEndpoints::accept_coauthor_invite(), &AcceptCoauthorInviteRequest { post_id })
                    .await?
            }
            RisePostContentRepositoryCollaborationOperation::DeclineInvite => {
                self.rpc
                    .call(RisePostEndpoints::decline_coauthor_invite(), &DeclineCoauthorInviteRequest { post_id })
                    .await?
            }
            RisePostContentRepositoryCollaborationOperation::Leave => {
                self.rpc
                    .call(
                        RisePostEndpoints::remove_coauthor(),
                        &RemoveCoauthorRequest { post_id, target_user_id: None },
                    )
                    .await?
            }
            RisePostContentRepositoryCollaborationOperation::SetGridVisibility(show_in_grid) => {
                self.rpc
                    .call(
                        RisePostEndpoints::set_coauthor_grid_visibility(),
                        &SetCoauthorGridVisibilityRequest { post_id, show_in_grid: Some(*show_in_grid) },
                    )
                    .await?
            }
            RisePostContentRepositoryCollaborationOperation::RemoveOwnTag => {
                self.rpc
                    .call(
                        RisePostEndpoints::remove_post_tag(),
                        &RemovePostTagRequest { post_id, target_user_id: None },
                    )
                    .await?
            }
            RisePostContentRepositoryCollaborationOperation::Invite(users) => {
                self.rpc
                    .call(
                        RisePostEndpoints::invite_coauthors(),
                        &InviteCoauthorsRequest {
                            post_id,
                            user_ids: Some(users.iter().map(|user| user.id.clone()).collect()),
                        },
                    )
                    .await?
            }
            RisePostContentRepositoryCollaborationOperation::RevokeCoauthor(user_id) => {
                self.rpc
                    .call(
                        RisePostEndpoints::remove_coauthor(),
                        &RemoveCoauthorRequest { post_id, target_user_id: Some(user_id.clone()) },
                    )
                    .await?
            }
            RisePostContentRepositoryCollaborationOperation::SetTags(drafts) => {
                self.rpc
                    .call(
                        RisePostEndpoints::set_post_tags(),
                        &SetPostTagsRequest {
                            post_id,
                            tagged_users: Some(drafts.iter().map(SetPostTagsTaggedUsersElement::from).collect()),
                        },
                    )
                    .await?
            }
            RisePostContentRepositoryCollaborationOperation::RemoveTag(user_id) => {
                self.rpc
                    .call(
                        RisePostEndpoints::remove_post_tag(),
                        &RemovePostTagRequest { post_id, target_user_id: Some(user_id.clone()) },
                    )
                    .await?
            }
        };
        Ok(response)
    }

    fn apply_optimistic_collaboration(
        operation: &RisePostContentRepositoryCollaborationOperation,
        post: &mut GetPostFeedResponse,
        viewer: &PostCollaboratorUser,
    ) {
        match operation {
            RisePostContentRepositoryCollaborationOperation::AcceptInvite => {
                post.my_collaboration = Some(PostViewerCollaboration {
                    kind: PostCollaboratorKind::Coauthor.raw_value().to_owned(),
                    show_in_grid: post.my_collaboration.as_ref().map(|mine| mine.show_in_grid).unwrap_or(true),
                    status: PostCollaboratorStatus::Accepted.raw_value().to_owned(),
                });
                if !post.coauthors.iter().any(|user| user.id == viewer.id) {
                    post.coauthors.push(viewer.clone());
                    post.coauthors_count = (post.coauthors.len() as i64).max(post.coauthors_count + 1);
                }
            }
            RisePostContentRepositoryCollaborationOperation::DeclineInvite => {
                post.my_collaboration = None;
            }
            RisePostContentRepositoryCollaborationOperation::Leave => {
                post.coauthors.retain(|user| user.id != viewer.id);
                post.coauthors_count = (post.coauthors_count - 1).max(0);
                post.my_collaboration = None;
            }
            RisePostContentRepositoryCollaborationOperation::SetGridVisibility(show_in_grid) => {
                if let Some(mine) = post.my_collaboration.as_mut() {
                    mine.show_in_grid = *show_in_grid;
                }
            }
            RisePostContentRepositoryCollaborationOperation::RemoveOwnTag => {
                post.tagged_users.retain(|tagged| tagged.user.as_ref().map(|user| &user.id) != Some(&viewer.id));
                post.tagged_users_count = (post.tagged_users_count - 1).max(0);
                post.my_collaboration = None;
            }
            RisePostContentRepositoryCollaborationOperation::Invite(users) => {
                let fresh = users
                    .iter()
                    .filter(|user| !post.coauthors.iter().any(|coauthor| coauthor.id == user.id))
                    .count();
                post.pending_coauthors_count += fresh as i64;
            }
            RisePostContentRepositoryCollaborationOperation::RevokeCoauthor(user_id) => {
                if post.coauthors.iter().any(|user| &user.id == user_id) {
                    post.coauthors.retain(|user| &user.id != user_id);
                    post.coauthors_count = (post.coauthors_count - 1).max(0);
                } else {
                    post.pending_coauthors_count = (post.pending_coauthors_count - 1).max(0);
                }
            }
            RisePostContentRepositoryCollaborationOperation::SetTags(drafts) => {
                post.tagged_users = drafts
                    .iter()
                    .map(|draft| PostTaggedUser {
                        placements: draft.placements.clone(),
                        user: Some(draft.user.clone().unwrap_or_else(|| PostCollaboratorUser {
                            id: draft.user_id.clone(),
                            ..PostCollaboratorUser::default()
                        })),
                    })
                    .collect();
                post.tagged_users_count = post.tagged_users.len() as i64;
            }
            RisePostContentRepositoryCollaborationOperation::RemoveTag(user_id) => {
                post.tagged_users.retain(|tagged| tagged.user.as_ref().map(|user| &user.id) != Some(user_id));
                post.tagged_users_count = (post.tagged_users_count - 1).max(0);
                if user_id == &viewer.id {
                    post.my_collaboration = None;
                }
            }
        }
    }

    fn apply_optimistic_collaborator_rows(
        s: &mut RisePostContentRepositoryState,
        operation: &RisePostContentRepositoryCollaborationOperation,
        key: &str,
        viewer: &PostCollaboratorUser,
    ) {
        let Some(entry) = s.collaborators_by_post_id.get_mut(key) else {
            return;
        };
        let coauthor = PostCollaboratorKind::Coauthor.raw_value();
        let tag = PostCollaboratorKind::Tag.raw_value();
        match operation {
            RisePostContentRepositoryCollaborationOperation::AcceptInvite => {
                for row in entry.items.iter_mut() {
                    if row.user.id == viewer.id && row.kind == coauthor {
                        row.status = PostCollaboratorStatus::Accepted.raw_value().to_owned();
                    }
                }
            }
            RisePostContentRepositoryCollaborationOperation::DeclineInvite
            | RisePostContentRepositoryCollaborationOperation::Leave => {
                entry.items.retain(|row| !(row.user.id == viewer.id && row.kind == coauthor));
            }
            RisePostContentRepositoryCollaborationOperation::SetGridVisibility(show_in_grid) => {
                for row in entry.items.iter_mut() {
                    if row.user.id == viewer.id && row.kind == coauthor {
                        row.show_in_grid = *show_in_grid;
                    }
                }
            }
            RisePostContentRepositoryCollaborationOperation::RemoveOwnTag => {
                entry.items.retain(|row| !(row.user.id == viewer.id && row.kind == tag));
            }
            RisePostContentRepositoryCollaborationOperation::Invite(users) => {
                for user in users {
                    if entry.items.iter().any(|row| row.user.id == user.id && row.kind == coauthor) {
                        continue;
                    }
                    entry.items.push(PostCollaboratorRow {
                        user: user.clone(),
                        kind: coauthor.to_owned(),
                        status: PostCollaboratorStatus::Pending.raw_value().to_owned(),
                        show_in_grid: true,
                        placements: Vec::new(),
                    });
                }
            }
            RisePostContentRepositoryCollaborationOperation::RevokeCoauthor(user_id) => {
                entry.items.retain(|row| !(&row.user.id == user_id && row.kind == coauthor));
            }
            RisePostContentRepositoryCollaborationOperation::SetTags(drafts) => {
                entry.items.retain(|row| row.kind != tag);
                entry.items.extend(drafts.iter().map(|draft| PostCollaboratorRow {
                    user: draft.user.clone().unwrap_or_else(|| PostCollaboratorUser {
                        id: draft.user_id.clone(),
                        ..PostCollaboratorUser::default()
                    }),
                    kind: tag.to_owned(),
                    status: PostCollaboratorStatus::Accepted.raw_value().to_owned(),
                    show_in_grid: true,
                    placements: draft.placements.clone(),
                }));
            }
            RisePostContentRepositoryCollaborationOperation::RemoveTag(user_id) => {
                entry.items.retain(|row| !(&row.user.id == user_id && row.kind == tag));
            }
        }
        entry.items.truncate(Self::MAXIMUM_COLLABORATOR_ITEMS);
    }

    fn collaboration_membership_plan(
        operation: &RisePostContentRepositoryCollaborationOperation,
    ) -> (Option<bool>, Option<bool>) {
        match operation {
            RisePostContentRepositoryCollaborationOperation::AcceptInvite => (Some(true), Some(true)),
            RisePostContentRepositoryCollaborationOperation::DeclineInvite
            | RisePostContentRepositoryCollaborationOperation::Leave => (Some(false), Some(false)),
            RisePostContentRepositoryCollaborationOperation::SetGridVisibility(show_in_grid) => {
                (Some(*show_in_grid), None)
            }
            RisePostContentRepositoryCollaborationOperation::RemoveOwnTag => (None, Some(false)),
            RisePostContentRepositoryCollaborationOperation::Invite(_)
            | RisePostContentRepositoryCollaborationOperation::RevokeCoauthor(_)
            | RisePostContentRepositoryCollaborationOperation::SetTags(_)
            | RisePostContentRepositoryCollaborationOperation::RemoveTag(_) => (None, None),
        }
    }

    fn grid_kinds(post: &GetPostFeedResponse) -> HashSet<RisePostProfileKind> {
        let subject = RisePostContentRepositoryMembershipSubject::from_post(post);
        if subject.content_kind != Some(RisePostContentKind::Publication) {
            return HashSet::from([RisePostProfileKind::Threads]);
        }
        let mut kinds: HashSet<RisePostProfileKind> = HashSet::new();
        if subject.lists_in_publications() {
            kinds.insert(RisePostProfileKind::Publications);
        }
        if subject.lists_in_riffs() {
            kinds.insert(RisePostProfileKind::Riffs);
        }
        kinds
    }

    /// every own tab the post could sit in whatever its placement
    /// so leaving a grid clears the tab the placement no longer names too
    fn grid_candidate_kinds(post: &GetPostFeedResponse) -> HashSet<RisePostProfileKind> {
        if RisePostContentKind::from_wire(&post.content_kind) == Some(RisePostContentKind::Publication) {
            HashSet::from([RisePostProfileKind::Publications, RisePostProfileKind::Riffs])
        } else {
            HashSet::from([RisePostProfileKind::Threads])
        }
    }

    /// live views of the own profile
    /// a slot without a parsed scope is skipped on purpose since reposts and mentions tabs share
    /// a signature and only an open view tells them apart
    fn own_profile_views(
        &self,
        s: &RisePostContentRepositoryState,
        kinds: &HashSet<RisePostProfileKind>,
    ) -> Vec<String> {
        s.scope_index
            .slots
            .iter()
            .filter(|(_, slot)| {
                matches!(
                    s.scopes_by_key.get(&slot.scope_key),
                    Some(RisePostPageScope::Profile { user_id, kind, .. })
                        if *user_id == self.account_id && kinds.contains(kind)
                )
            })
            .map(|(view, _)| view.clone())
            .collect()
    }

    /// own favorites tab follows the flag at once
    /// a saved post goes to the head of an open favorites view, an unsaved one leaves it
    fn favorites_shelf_mutations(
        &self,
        s: &RisePostContentRepositoryState,
        local_id: &str,
        favorited: bool,
    ) -> Vec<RiseStoreMutation> {
        let views = self.own_profile_views(s, &HashSet::from([RisePostProfileKind::Favorites]));
        let (added, removed): (&[String], &[String]) = if favorited { (&views, &[]) } else { (&[], &views) };
        Self::membership_mutations(local_id, added, removed, s.store_version)
    }

    /// own view memberships from the authoritative response
    /// not for the author since the post sits in the profile by authorship
    fn authoritative_membership_mutations(
        &self,
        s: &RisePostContentRepositoryState,
        local_id: &str,
        post: &GetPostFeedResponse,
        version: i64,
    ) -> Vec<RiseStoreMutation> {
        if post.author_id == self.account_id {
            return Vec::new();
        }
        let mine = post.my_collaboration.as_ref();
        let is_accepted_coauthor = mine.is_some_and(|mine| {
            mine.kind == PostCollaboratorKind::Coauthor.raw_value()
                && mine.status == PostCollaboratorStatus::Accepted.raw_value()
        });
        let is_tagged = mine.is_some_and(|mine| mine.kind == PostCollaboratorKind::Tag.raw_value());
        let wants_grid = is_accepted_coauthor && mine.is_some_and(|mine| mine.show_in_grid);
        let wants_mentions = is_accepted_coauthor || is_tagged;
        let kept_grid_views =
            if wants_grid { self.own_profile_views(s, &Self::grid_kinds(post)) } else { Vec::new() };
        let candidate_grid_views = self.own_profile_views(s, &Self::grid_candidate_kinds(post));
        let mention_views = self.own_profile_views(s, &HashSet::from([RisePostProfileKind::Mentions]));
        let mut added_views = kept_grid_views.clone();
        let mut removed_views: Vec<String> =
            candidate_grid_views.into_iter().filter(|view| !kept_grid_views.contains(view)).collect();
        if wants_mentions {
            added_views.extend(mention_views);
        } else {
            removed_views.extend(mention_views);
        }
        Self::membership_mutations(local_id, &added_views, &removed_views, version)
    }

    async fn self_collaborator(&self) -> PostCollaboratorUser {
        let now = Self::now_milliseconds();
        let account_id = self.account_id.clone();
        let fallback = PostCollaboratorUser { id: account_id.clone(), since_ms: now, ..PostCollaboratorUser::default() };
        let Some(main) = RiseEngineMainHop::installed() else {
            return fallback;
        };
        let read = main
            .run(move |cx: AsyncApp| {
                async move {
                    Ok(cx.update(|cx| {
                        let profile = AuthServicesStore::try_read(cx).and_then(|store| store.profile());
                        let logo = profile.and_then(|profile| {
                            profile.avatar_url.clone().or_else(|| Some(profile.more.logo.clone()))
                        });
                        PostCollaboratorUser {
                            id: account_id,
                            is_friend: false,
                            is_official: profile.is_some_and(|profile| profile.more.is_official),
                            is_premium: profile.is_some_and(|profile| profile.more.is_premium),
                            logo: logo.unwrap_or_default(),
                            name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
                            name_color: profile.and_then(|profile| profile.more.name_color.clone()),
                            premium_badge_emoji: profile.and_then(|profile| profile.more.premium_badge_emoji.clone()),
                            since_ms: now,
                            tag: profile.map(|profile| profile.tag.clone()).unwrap_or_default(),
                        }
                    }))
                }
                .boxed_local()
            })
            .await;
        read.unwrap_or(fallback)
    }

    fn trim_collaborator_entries(s: &mut RisePostContentRepositoryState) {
        while s.collaborators_by_post_id.len() > Self::MAXIMUM_COLLABORATOR_POSTS {
            let idle = s
                .collaborators_by_post_id
                .iter()
                .filter(|(_, entry)| !entry.is_loading)
                .min_by_key(|(_, entry)| entry.access)
                .map(|(key, _)| key.clone());
            let oldest = idle.or_else(|| {
                s.collaborators_by_post_id.iter().min_by_key(|(_, entry)| entry.access).map(|(key, _)| key.clone())
            });
            let Some(oldest) = oldest else {
                return;
            };
            s.collaborators_by_post_id.remove(&oldest);
        }
    }

    fn canonicalize(
        incoming: GetPostFeedResponse,
        dto: &GetFeedPostItemDTO,
        existing: Option<&GetPostFeedResponse>,
    ) -> GetPostFeedResponse {
        let mut post = incoming;
        post.presentation_id = existing.and_then(|existing| existing.presentation_id.clone());
        post.sparks_total = post.sparks_total.max(existing.map(|existing| existing.sparks_total).unwrap_or(0));
        post.my_sparks = post.my_sparks.max(existing.map(|existing| existing.my_sparks).unwrap_or(0));
        let authoritative = RisePostContentKind::from_wire(&dto.content_kind);
        let existing_kind = existing.and_then(|existing| RisePostContentKind::from_wire(&existing.content_kind));
        let resolved = authoritative.or(existing_kind).unwrap_or_else(|| {
            <RisePostContentKind as RisePostContentKindResolve>::resolve(
                None,
                &post.images,
                post.video.as_deref(),
                post.is_short,
                post.is_image_reel,
                post.track_id.as_deref(),
            )
        });
        post.content_kind = resolved.wire().to_owned();
        if resolved == RisePostContentKind::Thread {
            let prior = existing.and_then(|existing| existing.link_preview.clone());
            post.link_preview = match (prior, post.link_preview.take()) {
                (Some(prior), Some(next)) => Some(if next.revision >= prior.revision { next } else { prior }),
                (prior, next) => next.or(prior),
            };
        } else {
            post.link_preview = None;
        }
        post
    }

    pub async fn apply_moderation_status(
        &self,
        post_id: &str,
        status: &str,
    ) -> anyhow::Result<Option<RisePostContentPresentationSnapshot>> {
        if PostModerationStatus::from_wire(status).is_none() || post_id.is_empty() {
            return Ok(None);
        }
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        let Some(stored_entity) = self.entity_by_server_id(post_id).await? else {
            return Ok(None);
        };
        let Ok(mut stored) = serde_json::from_slice::<RisePostStoredEntity>(&stored_entity.data) else {
            return Ok(None);
        };
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        if stored.post.moderation_status == status {
            return Ok(None);
        }
        stored.post.moderation_status = status.to_owned();
        let store_version = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, post_id);
            s.store_version
        });
        let data = serde_json::to_vec(&stored)?;
        self.engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(
                        stored_entity.local_id.clone(),
                        data,
                        store_version.max(stored_entity.version + 1),
                        Self::now_milliseconds(),
                    ),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                }],
                HashSet::new(),
            ))
            .await?;
        Ok(Some(self.refresh_active_pages_from_store().await))
    }

    pub fn link_preview_push_handler(&self) -> RiseSyncPushHandler {
        let this = self.this.clone();
        RiseSyncPushHandler::new(rise_events::name::POST_LINK_PREVIEW_UPDATED, move |envelope| {
            let this = this.clone();
            async move {
                let Some(this) = this.upgrade() else {
                    return Ok(None);
                };
                this.make_link_preview_batch(envelope).await
            }
        })
    }

    pub fn post_created_push_handler(&self) -> RiseSyncPushHandler {
        let this = self.this.clone();
        RiseSyncPushHandler::new(rise_events::name::POST_CREATED, move |envelope| {
            let this = this.clone();
            async move {
                let Some(this) = this.upgrade() else {
                    return Ok(None);
                };
                this.make_post_created_batch(envelope).await
            }
        })
    }

    pub fn post_deleted_push_handler(&self) -> RiseSyncPushHandler {
        let this = self.this.clone();
        RiseSyncPushHandler::new(rise_events::name::POST_DELETED, move |envelope| {
            let this = this.clone();
            async move {
                let Some(this) = this.upgrade() else {
                    return Ok(None);
                };
                this.make_post_deleted_batch(envelope).await
            }
        })
    }

    pub fn post_updated_push_handler(&self) -> RiseSyncPushHandler {
        let this = self.this.clone();
        RiseSyncPushHandler::new(rise_events::name::POST_UPDATED, move |envelope| {
            let this = this.clone();
            async move {
                let Some(this) = this.upgrade() else {
                    return Ok(None);
                };
                this.make_post_updated_batch(envelope).await
            }
        })
    }

    pub fn media_ready_push_handler(&self) -> RiseSyncPushHandler {
        let this = self.this.clone();
        RiseSyncPushHandler::new(rise_events::name::POST_MEDIA_READY, move |envelope| {
            let this = this.clone();
            async move {
                let Some(this) = this.upgrade() else {
                    return Ok(None);
                };
                this.make_media_ready_batch(envelope).await
            }
        })
    }

    pub fn collaborators_changed_push_handler(&self) -> RiseSyncPushHandler {
        let this = self.this.clone();
        RiseSyncPushHandler::new(rise_events::name::POST_COLLABORATORS_CHANGED, move |envelope| {
            let this = this.clone();
            async move {
                let Some(this) = this.upgrade() else {
                    return Ok(None);
                };
                this.make_collaborators_changed_batch(envelope).await
            }
        })
    }

    pub async fn observe_slots(&self, range: Range<usize>) -> anyhow::Result<RiseStoreObservation> {
        let keys: HashSet<RiseEntityViewKey> = range
            .map(|slot| RiseEntityViewKey::new(ENTITY_NAMESPACE.clone(), format!("content-scope-{slot}")))
            .collect();
        self.engine
            .observe(RiseStoreSnapshotRequest { entity_view_states: keys, ..RiseStoreSnapshotRequest::default() })
            .await
    }

    pub async fn refresh_pages_from_store(
        &self,
        commit: RiseStoreCommit,
    ) -> Option<RisePostContentPresentationSnapshot> {
        let candidates: Vec<(String, String, RisePostPageScope)> = self.state.with(|s| {
            if s.is_shutdown {
                return Vec::new();
            }
            s.scope_index
                .slots
                .iter()
                .filter_map(|(view, slot)| {
                    let view_key = RiseStoreViewKey::entity_view(&ENTITY_NAMESPACE, view);
                    if !(commit.requires_full_refresh || commit.changed_views.contains(&view_key)) {
                        return None;
                    }
                    let scope = s.scopes_by_key.get(&slot.scope_key)?;
                    if s.in_flight_scopes.contains(&slot.scope_key)
                        || !s.pages_by_scope_key.contains_key(&slot.scope_key)
                    {
                        return None;
                    }
                    Some((view.clone(), slot.scope_key.clone(), scope.clone()))
                })
                .collect()
        });
        if self.state.with(|s| s.is_shutdown) {
            return None;
        }
        let mut changed = false;
        for (view, scope_key, scope) in candidates {
            let stored = self.load_page(&scope, &view, Some(RisePostCacheSource::PersistentStore)).await;
            changed |= self.state.with(|s| {
                if s.is_shutdown
                    || s.in_flight_scopes.contains(&scope_key)
                    || s.scope_index.slots.get(&view).map(|slot| &slot.scope_key) != Some(&scope_key)
                    || stored.status == RisePostPageStatus::Idle
                {
                    return false;
                }
                let Some(current) = s.pages_by_scope_key.get(&scope_key) else {
                    return false;
                };
                if stored.items == current.items {
                    return false;
                }
                let page = RisePostPagePresentation {
                    status: if stored.items.is_empty() { current.status } else { RisePostPageStatus::Loaded },
                    error_message: current.error_message.clone(),
                    has_more: stored.has_more,
                    is_refreshing: current.is_refreshing,
                    is_loading_more: current.is_loading_more,
                    cache_source: current.cache_source,
                    revision: current.revision.max(stored.revision),
                    items: stored.items,
                };
                s.pages_by_scope_key.insert(scope_key.clone(), page);
                true
            });
        }
        if !changed {
            return None;
        }
        Some(self.state.with(|s| self.make_snapshot(s)))
    }

    pub async fn refresh_active_pages_from_store(&self) -> RisePostContentPresentationSnapshot {
        let active: Vec<(String, RisePostPageScope, String)> = self.state.with(|s| {
            s.scopes_by_key
                .iter()
                .filter_map(|(scope_key, scope)| {
                    let view = s
                        .scope_index
                        .slots
                        .iter()
                        .find(|(_, slot)| &slot.scope_key == scope_key)
                        .map(|(view, _)| view.clone())?;
                    Some((scope_key.clone(), scope.clone(), view))
                })
                .collect()
        });
        x251_mark(|| format!("x251 refresh_active scopes={:?}", active.iter().map(|(key, _, _)| key.clone()).collect::<Vec<_>>()));
        for (scope_key, scope, view) in active {
            let page = self.load_page(&scope, &view, Some(RisePostCacheSource::PersistentStore)).await;
            self.state.with(|s| s.pages_by_scope_key.insert(scope_key, page));
        }
        self.state.with(|s| self.make_snapshot(s))
    }

    pub async fn begin_load(
        &self,
        scope: RisePostPageScope,
        intent: RisePostContentRepositoryLoadIntent,
    ) -> RisePostContentRepositoryLoadTicket {
        let held = |s: &mut RisePostContentRepositoryState,
                    scope: RisePostPageScope,
                    view: String,
                    intent: RisePostContentRepositoryLoadIntent,
                    processing_post_ids: Vec<String>| RisePostContentRepositoryLoadTicket {
            scope,
            view,
            intent,
            generation: None,
            seed_version: s.store_version,
            cursor: None,
            snapshot: self.make_snapshot(s),
            previous: None,
            processing_post_ids,
        };
        if self.state.with(|s| s.is_shutdown) {
            return self.state.with(|s| held(s, scope, String::new(), intent, Vec::new()));
        }

        self.load_scope_index_if_needed().await;
        let scope_key = scope.key();
        let view = self.state.with(|s| {
            let view = Self::allocate_view(s, &scope);
            s.scopes_by_key.insert(scope_key.clone(), scope.clone());
            view
        });
        let cached = self.load_page(&scope, &view, None).await;
        let current = self.state.with(|s| {
            let current = s.pages_by_scope_key.get(&scope_key).cloned().unwrap_or(cached);
            s.pages_by_scope_key.insert(scope_key.clone(), current.clone());
            current
        });

        let held_in_flight = self.state.with(|s| {
            if !s.in_flight_scopes.contains(&scope_key) {
                return false;
            }
            let age = s
                .in_flight_started_at
                .get(&scope_key)
                .map_or(0, |started| Self::now_milliseconds() - started);
            if age < STALE_IN_FLIGHT_MILLISECONDS {
                return true;
            }
            tracing::warn!(target: "riseonly::X1613", scope = %scope_key, age, ?intent, "stale in flight load released");
            s.in_flight_scopes.remove(&scope_key);
            false
        });
        if held_in_flight {
            tracing::warn!(target: "riseonly::X1613", scope = %scope_key, ?intent, status = ?current.status, "load held in flight");
            if intent == RisePostContentRepositoryLoadIntent::LoadMore {
                tracing::warn!(target: "riseonly::post_list_pagination", "load more held in flight scope {scope_key}");
            }
            return self.state.with(|s| held(s, scope, view, intent, Vec::new()));
        }

        if intent == RisePostContentRepositoryLoadIntent::LoadMore && (!current.has_more || current.items.is_empty()) {
            return self.state.with(|s| held(s, scope, view, intent, Vec::new()));
        }

        let awaits_media_processing = current.items.iter().any(GetPostFeedResponse::is_processing_media);
        let forced_refresh = self.state.with(|s| {
            let forced = s.forced_refresh_scope_keys.contains(&scope_key);
            if forced && intent == RisePostContentRepositoryLoadIntent::Activate {
                s.forced_refresh_scope_keys.remove(&scope_key);
            }
            forced
        });
        let mut intent = if forced_refresh && intent == RisePostContentRepositoryLoadIntent::Activate {
            RisePostContentRepositoryLoadIntent::Refresh
        } else {
            intent
        };

        let network_loaded = self.state.with(|s| s.network_loaded_scope_keys.contains(&scope_key));
        let ranked_feed_held =
            matches!(scope, RisePostPageScope::Feed(_)) && network_loaded && !current.items.is_empty();
        if intent == RisePostContentRepositoryLoadIntent::Activate
            && !forced_refresh
            && ranked_feed_held
            && current.status == RisePostPageStatus::Loaded
            && current.error_message.is_none()
        {
            let processing_post_ids: Vec<String> = if awaits_media_processing {
                current
                    .items
                    .iter()
                    .filter(|post| post.is_processing_media())
                    .take(4)
                    .map(|post| post.id.clone())
                    .collect()
            } else {
                Vec::new()
            };
            return self.state.with(|s| held(s, scope, view, intent, processing_post_ids));
        }

        if intent == RisePostContentRepositoryLoadIntent::Activate
            && !forced_refresh
            && !awaits_media_processing
            && current.status == RisePostPageStatus::Loaded
            && current.error_message.is_none()
            && Self::now_milliseconds() - current.revision < self.refresh_interval_milliseconds
        {
            return self.state.with(|s| held(s, scope, view, intent, Vec::new()));
        }

        if scope == RisePostPageScope::Shorts && intent == RisePostContentRepositoryLoadIntent::Activate {
            intent = RisePostContentRepositoryLoadIntent::Refresh;
        }
        if matches!(scope, RisePostPageScope::Feed(_))
            && intent == RisePostContentRepositoryLoadIntent::Activate
            && !network_loaded
        {
            intent = RisePostContentRepositoryLoadIntent::Refresh;
        }
        let generation = self.state.with(|s| {
            let generation = s.generations.get(&scope_key).copied().unwrap_or(0).wrapping_add(1);
            s.generations.insert(scope_key.clone(), generation);
            s.in_flight_scopes.insert(scope_key.clone());
            s.in_flight_started_at.insert(scope_key.clone(), Self::now_milliseconds());
            generation
        });

        let cursor = if matches!(
            intent,
            RisePostContentRepositoryLoadIntent::LoadMore | RisePostContentRepositoryLoadIntent::Activate
        ) {
            self.load_cursor(&scope_key, &view).await
        } else {
            None
        };
        self.state.with(|s| {
            s.pages_by_scope_key.insert(
                scope_key.clone(),
                RisePostPagePresentation {
                    items: current.items.clone(),
                    status: if current.items.is_empty() {
                        RisePostPageStatus::Loading
                    } else {
                        RisePostPageStatus::Loaded
                    },
                    error_message: None,
                    has_more: current.has_more,
                    is_refreshing: intent != RisePostContentRepositoryLoadIntent::LoadMore && !current.items.is_empty(),
                    is_loading_more: intent == RisePostContentRepositoryLoadIntent::LoadMore,
                    cache_source: current.cache_source,
                    revision: current.revision,
                },
            );
            RisePostContentRepositoryLoadTicket {
                scope,
                view,
                intent,
                generation: Some(generation),
                seed_version: s.store_version,
                cursor,
                snapshot: self.make_snapshot(s),
                previous: Some(current),
                processing_post_ids: Vec::new(),
            }
        })
    }

    pub async fn finish_load(
        &self,
        ticket: &RisePostContentRepositoryLoadTicket,
    ) -> RisePostContentPresentationSnapshot {
        let scope_key = ticket.scope.key();
        let Some(generation) = ticket.generation else {
            return self.state.with(|s| self.make_snapshot(s));
        };
        let is_current = |s: &RisePostContentRepositoryState| s.generations.get(&scope_key).copied() == Some(generation);
        if self.state.with(|s| s.is_shutdown || !is_current(s)) {
            return self.state.with(|s| self.make_snapshot(s));
        }

        let outcome: anyhow::Result<()> = async {
            let deadline = RiseRuntimeTimer::sleep(Duration::from_millis(STALE_IN_FLIGHT_MILLISECONDS as u64));
            let request = self.request(ticket);
            futures::pin_mut!(deadline, request);
            let futures::future::Either::Left((response, _)) = futures::future::select(request, deadline).await else {
                tracing::warn!(target: "riseonly::X1613", scope = %scope_key, generation, "load passed the in flight deadline");
                return Err(rise_engine::RiseWireError::Disconnected("post page load deadline".to_owned()).into());
            };
            let response = response?;
            if self.state.with(|s| s.is_shutdown || !is_current(s)) || !self.is_current_account().await {
                return Err(CancellationError.into());
            }
            self.commit(response, ticket, generation).await
        }
        .await;
        match outcome {
            Ok(()) => self.state.with(|s| {
                s.in_flight_scopes.remove(&scope_key);
                self.make_snapshot(s)
            }),
            Err(error) if error.downcast_ref::<CancellationError>().is_some() => self.state.with(|s| {
                tracing::warn!(target: "riseonly::X1613", scope = %scope_key, generation, "load cancelled");
                Self::clear_cancelled_load(s, &scope_key, generation, ticket);
                self.make_snapshot(s)
            }),
            Err(error) => self.state.with(|s| {
                tracing::warn!(target: "riseonly::X1613", scope = %scope_key, generation, %error, "load failed");
                s.in_flight_scopes.remove(&scope_key);
                let current = s.pages_by_scope_key.get(&scope_key).cloned().unwrap_or(RisePostPagePresentation::IDLE);
                let is_final_search_refusal = matches!(ticket.scope, RisePostPageScope::Search { .. })
                    && CanonErrors::failure(&error)
                        .is_some_and(|failure| !CanonErrors::is_retryable(failure.code.as_deref()));
                let page = RisePostPagePresentation {
                    status: if current.items.is_empty() && !is_final_search_refusal {
                        RisePostPageStatus::Failed
                    } else {
                        RisePostPageStatus::Loaded
                    },
                    error_message: Some(CanonErrors::load_failure_text(&error)),
                    has_more: current.has_more,
                    is_refreshing: false,
                    is_loading_more: false,
                    cache_source: current.cache_source,
                    revision: current.revision,
                    items: current.items,
                };
                s.pages_by_scope_key.insert(scope_key.clone(), page);
                self.make_snapshot(s)
            }),
        }
    }

    pub fn abandon_load(
        &self,
        ticket: &RisePostContentRepositoryLoadTicket,
    ) -> Option<RisePostContentPresentationSnapshot> {
        let generation = ticket.generation?;
        let scope_key = ticket.scope.key();
        Some(self.state.with(|s| {
            Self::clear_cancelled_load(s, &scope_key, generation, ticket);
            self.make_snapshot(s)
        }))
    }

    pub fn load_guard<'a>(
        &'a self,
        ticket: &'a RisePostContentRepositoryLoadTicket,
        on_abandon: impl FnOnce(RisePostContentPresentationSnapshot) + Send + 'static,
    ) -> RisePostContentLoadGuard<'a> {
        RisePostContentLoadGuard {
            repository: self,
            ticket,
            on_abandon: ticket.generation.is_some().then(|| Box::new(on_abandon) as RisePostContentAbandonHandler),
        }
    }

    fn clear_cancelled_load(
        s: &mut RisePostContentRepositoryState,
        scope_key: &str,
        generation: u64,
        ticket: &RisePostContentRepositoryLoadTicket,
    ) {
        s.in_flight_scopes.remove(scope_key);
        if s.generations.get(scope_key).copied() == Some(generation)
            && let Some(current) = s.pages_by_scope_key.get(scope_key)
        {
            let previous = ticket.previous.clone().unwrap_or(RisePostPagePresentation::IDLE);
            let status = if current.items.is_empty() {
                if previous.status == RisePostPageStatus::Loading {
                    RisePostPageStatus::Idle
                } else {
                    previous.status
                }
            } else {
                RisePostPageStatus::Loaded
            };
            let page = RisePostPagePresentation {
                items: current.items.clone(),
                status,
                error_message: if status == RisePostPageStatus::Failed { previous.error_message } else { None },
                has_more: current.has_more,
                is_refreshing: false,
                is_loading_more: false,
                cache_source: current.cache_source,
                revision: current.revision,
            };
            s.pages_by_scope_key.insert(scope_key.to_owned(), page);
        }
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            for generation in s.generations.values_mut() {
                *generation = generation.wrapping_add(1);
            }
            s.in_flight_scopes.clear();
            for (_, task) in s.engagement_tasks.drain() {
                task.abort();
            }
            s.engagement_refusal_handlers.clear();
            for (_, waiters) in s.repost_turn_waiters.drain() {
                for (_, waiter) in waiters {
                    let _ = waiter.send(());
                }
            }
            s.repost_queues.clear();
            s.repost_desired_states.clear();
            s.in_flight_repost_keys.clear();
            s.in_flight_opinion_keys.clear();
            s.in_flight_collaboration_keys.clear();
            s.collaboration_generations.clear();
            s.collaborators_by_post_id.clear();
            s.pages_by_scope_key.clear();
            s.scopes_by_key.clear();
            s.network_loaded_scope_keys.clear();
            s.pending_create_memberships.clear();
            s.session_create_local_ids.clear();
            s.discarding_create_local_ids.clear();
            s.create_progress_by_request_id.clear();
            s.server_accepted_create_request_ids.clear();
            s.in_flight_update_post_ids.clear();
        });
    }

    async fn commit(
        &self,
        response: Box<dyn RisePostWirePage>,
        ticket: &RisePostContentRepositoryLoadTicket,
        generation: u64,
    ) -> anyhow::Result<()> {
        let scope_key = ticket.scope.key();
        let incoming = response.list();
        let mut posts: Vec<(GetPostFeedResponse, String)> = Vec::with_capacity(incoming.len());
        let mut incoming_ids: HashSet<String> = HashSet::new();
        for dto in incoming {
            let Some(mut post) = dto.as_feed_response() else {
                continue;
            };
            let deleted_since_seed = self.state.with(|s| {
                s.deleted_post_versions
                    .get(&post.id)
                    .is_some_and(|deleted_version| *deleted_version > ticket.seed_version)
            });
            if deleted_since_seed {
                continue;
            }
            let existing_entity = self.entity_by_server_id(&post.id).await?;
            let existing = existing_entity
                .as_ref()
                .and_then(|entity| serde_json::from_slice::<RisePostStoredEntity>(&entity.data).ok())
                .map(|stored| stored.post);
            let held_locally = self.state.with(|s| {
                s.entity_mutation_versions
                    .get(&post.id)
                    .is_some_and(|version| *version > ticket.seed_version)
                    || s.in_flight_update_post_ids.values().any(|id| *id == post.id)
            });
            post = match (held_locally, existing) {
                (true, Some(existing)) => existing,
                (_, existing) => Self::canonicalize(post, dto, existing.as_ref()),
            };
            if !Self::accepts(&post, &ticket.scope) {
                continue;
            }
            if !incoming_ids.insert(post.id.clone()) {
                continue;
            }
            let local_id = existing_entity.map_or_else(|| post.id.clone(), |entity| entity.local_id);
            posts.push((post, local_id));
        }

        let version = self.state.with(|s| {
            Self::advance_store_version(s);
            s.store_version
        });
        let now = Self::now_milliseconds();
        let mut vanished_local_id: Option<String> = None;
        if let RisePostPageScope::Detail { post_id } = &ticket.scope
            && incoming.is_empty()
            && !self.state.with(|s| {
                s.pending_create_memberships
                    .values()
                    .any(|pending| pending.server_id.as_deref() == Some(post_id.as_str()))
                    || s.in_flight_update_post_ids.values().any(|id| id == post_id)
            })
            && let Some(stored) = self.entity_by_server_id(post_id).await?
        {
            vanished_local_id = Some(stored.local_id);
            self.state.with(|s| {
                Self::record_mutation_version(s, post_id);
                s.deleted_post_versions.insert(post_id.clone(), version);
            });
        }
        let existing_count: usize;
        let existing_ids: HashSet<String>;
        let mut stale_head_local_ids: Vec<String> = Vec::new();
        let mut search_page_is_whole_result = false;
        let mut profile_page_is_whole_list = false;
        let preserves_existing_memberships = matches!(
            ticket.intent,
            RisePostContentRepositoryLoadIntent::LoadMore | RisePostContentRepositoryLoadIntent::Activate
        );
        if preserves_existing_memberships {
            let existing_window = self
                .engine
                .entity_window(RiseEntityWindowQuery::new(
                    ENTITY_NAMESPACE.clone(),
                    ticket.view.clone(),
                    RiseEntityWindow::First { limit: Self::MAXIMUM_VIEW_ITEMS },
                ))
                .await
                .ok();
            existing_count = existing_window.as_ref().map_or(0, |window| window.total_count);
            existing_ids = existing_window
                .as_ref()
                .map(|window| window.items.iter().map(|item| item.entity.local_id.clone()).collect())
                .unwrap_or_default();
            let pending_local_ids: HashSet<String> =
                self.state.with(|s| s.pending_create_memberships.keys().cloned().collect());
            if ticket.intent == RisePostContentRepositoryLoadIntent::Activate
                && let RisePostPageScope::Profile { kind, .. } = &ticket.scope
                && let Some(window) = &existing_window
            {
                let orders_by_id = matches!(
                    kind,
                    RisePostProfileKind::Publications | RisePostProfileKind::Riffs | RisePostProfileKind::Threads
                );
                let page_has_more = response.is_have_more();
                profile_page_is_whole_list = !page_has_more;
                let lower_bound = if orders_by_id {
                    posts.last().and_then(|(post, _)| post.id.parse::<i64>().ok())
                } else {
                    None
                };
                if !page_has_more || lower_bound.is_some() {
                    for item in window.items.iter().filter(|item| !incoming_ids.contains(&item.entity.local_id)) {
                        if pending_local_ids.contains(&item.entity.local_id) {
                            continue;
                        }
                        let Ok(stored) = serde_json::from_slice::<RisePostStoredEntity>(&item.entity.data) else {
                            continue;
                        };
                        if incoming_ids.contains(&stored.post.id) {
                            continue;
                        }
                        let Ok(server_id) = stored.post.id.parse::<i64>() else {
                            continue;
                        };
                        if !page_has_more || server_id >= lower_bound.unwrap_or(i64::MAX) {
                            stale_head_local_ids.push(item.entity.local_id.clone());
                        }
                    }
                }
            }
            if ticket.intent == RisePostContentRepositoryLoadIntent::Activate
                && matches!(ticket.scope, RisePostPageScope::Search { .. })
                && !response.is_have_more()
                && let Some(window) = &existing_window
            {
                search_page_is_whole_result = true;
                for item in window.items.iter().filter(|item| !incoming_ids.contains(&item.entity.local_id)) {
                    if pending_local_ids.contains(&item.entity.local_id) {
                        continue;
                    }
                    let Ok(stored) = serde_json::from_slice::<RisePostStoredEntity>(&item.entity.data) else {
                        continue;
                    };
                    if incoming_ids.contains(&stored.post.id) {
                        continue;
                    }
                    stale_head_local_ids.push(item.entity.local_id.clone());
                }
            }
        } else {
            existing_count = 0;
            existing_ids = HashSet::new();
        }
        let continues_ranked_session = matches!(ticket.scope, RisePostPageScope::Feed(_));
        let repositions_head =
            ticket.intent == RisePostContentRepositoryLoadIntent::Activate && !continues_ranked_session;
        let membership_posts: Vec<&(GetPostFeedResponse, String)> = if preserves_existing_memberships {
            posts.iter().filter(|(_, local_id)| !existing_ids.contains(local_id)).collect()
        } else {
            posts.iter().collect()
        };

        let mut mutations: Vec<RiseStoreMutation> = Vec::new();
        let evicted_views = self.state.with(|s| {
            let local_ids: Vec<String> = s.pending_create_memberships.keys().cloned().collect();
            for local_id in local_ids {
                let Some(pending) = s.pending_create_memberships.get_mut(&local_id) else {
                    continue;
                };
                let expired = pending
                    .settled_at_milliseconds
                    .is_some_and(|settled_at| now - settled_at > PENDING_CREATE_PROTECTION_MILLISECONDS);
                if expired {
                    s.pending_create_memberships.remove(&local_id);
                    continue;
                }
                if !pending.server_id.as_ref().is_some_and(|server_id| incoming_ids.contains(server_id)) {
                    continue;
                }
                pending.memberships_by_view.remove(&ticket.view);
                if pending.memberships_by_view.is_empty() {
                    s.pending_create_memberships.remove(&local_id);
                }
            }

            if !preserves_existing_memberships {
                mutations.push(RiseStoreMutation::RemoveAllEntityViewMemberships {
                    namespace: ENTITY_NAMESPACE.clone(),
                    view: ticket.view.clone(),
                    if_version_at_most: Some(ticket.seed_version),
                });
                for pending in s.pending_create_memberships.values() {
                    let Some(membership) = pending.memberships_by_view.get(&ticket.view) else {
                        continue;
                    };
                    mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                        namespace: ENTITY_NAMESPACE.clone(),
                        view: ticket.view.clone(),
                        membership: RiseEntityViewMembership::new(
                            membership.local_id.clone(),
                            membership.position_key.clone(),
                            version,
                        ),
                    });
                }
            }
            std::mem::take(&mut s.evicted_views)
        });
        for view in evicted_views {
            mutations.push(RiseStoreMutation::RemoveAllEntityViewMemberships {
                namespace: ENTITY_NAMESPACE.clone(),
                view,
                if_version_at_most: None,
            });
        }
        for local_id in &stale_head_local_ids {
            mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                namespace: ENTITY_NAMESPACE.clone(),
                view: ticket.view.clone(),
                local_id: local_id.clone(),
                if_version_at_most: Some(ticket.seed_version),
            });
        }
        if let Some(vanished_local_id) = &vanished_local_id {
            mutations.extend(self.membership_removals(vanished_local_id).await);
            mutations.push(RiseStoreMutation::RemoveEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                local_id: vanished_local_id.clone(),
            });
        }

        for (post, local_id) in &posts {
            let mut post = post.clone();
            post.presentation_id = Some(local_id.clone());
            let updated_at = post.updated_at_ms.unwrap_or(now);
            let data = serde_json::to_vec(&RisePostStoredEntity { post: post.clone() })?;
            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(local_id.clone(), data, version, updated_at),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            });
            mutations.push(RiseStoreMutation::SetEntityAlias {
                namespace: ENTITY_NAMESPACE.clone(),
                alias: RiseEntityAlias::new(RiseEntityAliasKind::server_id(), post.id.clone()),
                local_id: local_id.clone(),
            });
        }

        let ordinal_base: i64 = if preserves_existing_memberships {
            (existing_count as i64)
                .max(ticket.cursor.as_ref().and_then(|cursor| cursor.next_ordinal).unwrap_or(0))
        } else {
            0
        };
        let is_shorts = matches!(ticket.scope, RisePostPageScope::Shorts);
        for (offset, (post, local_id)) in membership_posts.iter().map(|entry| (&entry.0, &entry.1)).enumerate() {
            let offset = offset as i64;
            let position = RisePositionKey::make(|builder| {
                if repositions_head {
                    builder.append_signed_int64(-(version / 1_000));
                    builder.append_signed_int64(offset);
                } else {
                    let short_id = if is_shorts { post.id.parse::<i64>().ok() } else { None };
                    builder.append_signed_int64(short_id.map_or(ordinal_base + offset, |id| -id));
                }
                builder.append_string(&post.id);
            });
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: ENTITY_NAMESPACE.clone(),
                view: ticket.view.clone(),
                membership: RiseEntityViewMembership::new(local_id.clone(), position, version),
            });
        }

        let retained_tail: Option<(RisePostPageCursor, bool)> = match (&ticket.cursor, &ticket.previous) {
            (Some(cursor), Some(previous))
                if ticket.intent == RisePostContentRepositoryLoadIntent::Activate && existing_count > 0 =>
            {
                match &ticket.scope {
                    RisePostPageScope::Search { .. } if search_page_is_whole_result => None,
                    RisePostPageScope::Profile { .. } if profile_page_is_whole_list => None,
                    RisePostPageScope::Profile { .. } | RisePostPageScope::Search { .. } => {
                        Some((cursor.clone(), previous.has_more))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let mut has_more = retained_tail.as_ref().map_or_else(|| response.is_have_more(), |(_, has_more)| *has_more);
        let relative_id = response
            .relative_id()
            .or_else(|| posts.last().and_then(|(post, _)| post.id.parse::<i64>().ok()));
        if is_shorts {
            let cursor_relative_id = ticket.cursor.as_ref().and_then(|cursor| cursor.relative_id);
            has_more = has_more
                && relative_id.is_some()
                && (ticket.intent != RisePostContentRepositoryLoadIntent::LoadMore || relative_id != cursor_relative_id);
        }
        let cursor = match retained_tail {
            Some((cursor, _)) => cursor,
            None => RisePostPageCursor {
                scope_key: scope_key.clone(),
                relative_id,
                feed_session_id: response
                    .page_feed_session_id()
                    .or_else(|| ticket.cursor.as_ref().and_then(|cursor| cursor.feed_session_id.clone())),
                page: if ticket.intent == RisePostContentRepositoryLoadIntent::LoadMore {
                    (ticket.cursor.as_ref().map_or(1, |cursor| cursor.page) + 1).max(2)
                } else {
                    1
                },
                next_ordinal: Some(ordinal_base + membership_posts.len() as i64),
            },
        };
        let cursor_data = serde_json::to_vec(&cursor)?;
        mutations.push(RiseStoreMutation::SetEntityViewState {
            namespace: ENTITY_NAMESPACE.clone(),
            view: ticket.view.clone(),
            state: RiseEntityViewState::new(
                version,
                None,
                Some(cursor_data),
                false,
                has_more,
                RiseEntityViewLoadState::Loaded,
                None,
                now,
            ),
        });
        mutations.push(RiseStoreMutation::TrimEntityView {
            namespace: ENTITY_NAMESPACE.clone(),
            view: ticket.view.clone(),
            keeping: if Self::slides_window(&ticket.scope) {
                RiseEntityViewRetention::Latest(Self::MAXIMUM_VIEW_ITEMS)
            } else {
                RiseEntityViewRetention::First(Self::MAXIMUM_VIEW_ITEMS)
            },
            if_version_at_most: None,
        });
        mutations.push(RiseStoreMutation::PruneUnreferencedEntities {
            namespace: ENTITY_NAMESPACE.clone(),
            limit: Self::MAXIMUM_VIEW_ITEMS as i64,
        });
        let scope_index_data = self.state.with(|s| {
            if s.scope_index_dirty {
                Some(serde_json::to_vec(&s.scope_index))
            } else {
                None
            }
        });
        if let Some(index_data) = scope_index_data {
            mutations.push(RiseStoreMutation::SetValue {
                namespace: RESOURCE_NAMESPACE.clone(),
                key: SCOPE_INDEX_KEY.to_owned(),
                value: RiseStoredValue::new(index_data?, version, now),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            });
        }

        let invalidated_views = HashSet::from([RiseStoreViewKey::entity_view(&ENTITY_NAMESPACE, &ticket.view)]);
        self.engine.write(RiseStoreWriteBatch::new(mutations, invalidated_views)).await?;
        self.state.with(|s| {
            if let Some(vanished_local_id) = &vanished_local_id {
                s.session_create_local_ids.remove(vanished_local_id);
            }
            s.scope_index_dirty = false;
        });
        if vanished_local_id.is_some() {
            self.refresh_active_pages_from_store().await;
        }
        if self.state.with(|s| s.generations.get(&scope_key).copied()) != Some(generation) {
            return Ok(());
        }
        self.state.with(|s| {
            s.network_loaded_scope_keys.insert(scope_key.clone());
        });
        let page = self.load_page(&ticket.scope, &ticket.view, Some(RisePostCacheSource::Network)).await;
        self.state.with(|s| {
            s.pages_by_scope_key.insert(scope_key, page);
        });
        Ok(())
    }

    pub async fn set_like_intent(
        &self,
        post_id: &str,
        liked: bool,
        seed: Option<GetPostFeedResponse>,
        on_refused: Option<RisePostEngagementRefusalHandler>,
    ) -> anyhow::Result<RisePostContentPresentationSnapshot> {
        self.set_engagement_intent(post_id, RisePostContentRepositoryEngagementKind::Like, liked, seed, on_refused)
            .await
    }

    pub async fn set_favorite_intent(
        &self,
        post_id: &str,
        favorited: bool,
        seed: Option<GetPostFeedResponse>,
        on_refused: Option<RisePostEngagementRefusalHandler>,
    ) -> anyhow::Result<RisePostContentPresentationSnapshot> {
        self.set_engagement_intent(post_id, RisePostContentRepositoryEngagementKind::Favorite, favorited, seed, on_refused)
            .await
    }

    async fn set_engagement_intent(
        &self,
        post_id: &str,
        kind: RisePostContentRepositoryEngagementKind,
        desired: bool,
        seed: Option<GetPostFeedResponse>,
        on_refused: Option<RisePostEngagementRefusalHandler>,
    ) -> anyhow::Result<RisePostContentPresentationSnapshot> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await || post_id.parse::<i64>().is_err() {
            x251_mark(|| format!("x251 engagement stale post={post_id}"));
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        let entity = self.entity_by_server_id(post_id).await?;
        let local_id = entity.as_ref().map_or_else(|| post_id.to_owned(), |entity| entity.local_id.clone());
        let decoded = entity
            .as_ref()
            .and_then(|entity| serde_json::from_slice::<RisePostStoredEntity>(&entity.data).ok());
        x251_mark(|| format!("x251 engagement entity={} decoded={}", entity.is_some(), decoded.is_some()));
        let Some(mut stored) = decoded.or_else(|| seed.map(|post| RisePostStoredEntity { post })) else {
            return Err(RisePostRepositoryError::StaleAccount.into());
        };
        stored.post.id = post_id.to_owned();
        let key = format!("{}:{post_id}", kind.raw_value());
        match kind {
            RisePostContentRepositoryEngagementKind::Like => {
                if stored.post.is_liked != desired {
                    stored.post.likes_count = (stored.post.likes_count + if desired { 1 } else { -1 }).max(0);
                }
                stored.post.is_liked = desired;
            }
            RisePostContentRepositoryEngagementKind::Favorite => {
                if stored.post.is_favorited != desired {
                    stored.post.favorites_count = (stored.post.favorites_count + if desired { 1 } else { -1 }).max(0);
                }
                stored.post.is_favorited = desired;
            }
        }
        let data = serde_json::to_vec(&stored)?;
        let mutations = self.state.with(|s| {
            let generation = Self::next_mutation_generation(s);
            s.engagement_generations.insert(key.clone(), generation);
            match on_refused {
                Some(handler) => {
                    s.engagement_refusal_handlers.insert(key.clone(), handler);
                }
                None => {
                    s.engagement_refusal_handlers.remove(&key);
                }
            }
            Self::trim_mutation_generation_state(s);
            Self::advance_store_version(s);
            Self::record_mutation_version(s, post_id);
            let mut mutations = vec![
                RiseStoreMutation::UpsertEntity {
                    namespace: ENTITY_NAMESPACE.clone(),
                    entity: RiseStoreEntity::new(local_id.clone(), data, s.store_version, Self::now_milliseconds()),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                },
                RiseStoreMutation::SetEntityAlias {
                    namespace: ENTITY_NAMESPACE.clone(),
                    alias: RiseEntityAlias::new(RiseEntityAliasKind::server_id(), post_id.to_owned()),
                    local_id: local_id.clone(),
                },
            ];
            if kind == RisePostContentRepositoryEngagementKind::Favorite {
                mutations.extend(self.favorites_shelf_mutations(s, &local_id, desired));
            }
            mutations
        });
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        self.state.with(|s| {
            if !s.engagement_tasks.contains_key(&key) {
                let this = self.this.clone();
                let worker_post_id = post_id.to_owned();
                let task = RiseAccountRuntime::current().spawn(async move {
                    if let Some(this) = this.upgrade() {
                        this.run_engagement_worker(worker_post_id, kind).await;
                    }
                });
                s.engagement_tasks.insert(key.clone(), task);
            }
        });
        Ok(self.refresh_active_pages_from_store().await)
    }

    async fn run_engagement_worker(&self, post_id: String, kind: RisePostContentRepositoryEngagementKind) {
        let key = format!("{}:{post_id}", kind.raw_value());
        while !self.state.with(|s| s.is_shutdown) {
            let Some(generation) = self.state.with(|s| s.engagement_generations.get(&key).copied()) else {
                break;
            };
            RiseRuntimeTimer::sleep(Duration::from_millis(ENGAGEMENT_COALESCE_MILLISECONDS)).await;
            if self.state.with(|s| s.engagement_generations.get(&key).copied()) != Some(generation) {
                continue;
            }
            self.commit_engagement(&post_id, kind, generation).await;
            let settled = self.state.with(|s| {
                if s.engagement_generations.get(&key).copied() != Some(generation) {
                    return false;
                }
                s.engagement_tasks.remove(&key);
                s.engagement_refusal_handlers.remove(&key);
                Self::trim_mutation_generation_state(s);
                true
            });
            if settled {
                return;
            }
        }
        self.state.with(|s| {
            s.engagement_tasks.remove(&key);
            s.engagement_refusal_handlers.remove(&key);
            Self::trim_mutation_generation_state(s);
        });
    }

    async fn commit_engagement(&self, post_id: &str, kind: RisePostContentRepositoryEngagementKind, generation: u64) {
        let key = format!("{}:{post_id}", kind.raw_value());
        let is_current_generation =
            |s: &RisePostContentRepositoryState| s.engagement_generations.get(&key).copied() == Some(generation);
        if self.state.with(|s| s.is_shutdown || !is_current_generation(s)) {
            return;
        }
        let Ok(post_id_value) = post_id.parse::<i64>() else {
            return;
        };
        let Ok(Some(stored_entity)) = self.entity_by_server_id(post_id).await else {
            return;
        };
        let Ok(stored) = serde_json::from_slice::<RisePostStoredEntity>(&stored_entity.data) else {
            return;
        };
        let desired = match kind {
            RisePostContentRepositoryEngagementKind::Like => stored.post.is_liked,
            RisePostContentRepositoryEngagementKind::Favorite => stored.post.is_favorited,
        };
        let outcome: anyhow::Result<()> = async {
            let request_id = Uuid::new_v4().uuid_string().to_lowercase();
            let client_session_id = self.session_id.uuid_string().to_lowercase();
            let client_sequence = i64::try_from(generation).unwrap_or(i64::MAX);
            let response: RisePostEngagementAcknowledgement = match kind {
                RisePostContentRepositoryEngagementKind::Like => {
                    let reply: SetPostLikeResponse = self
                        .rpc
                        .call(
                            RisePostEndpoints::set_post_like(),
                            &SetPostLikeRequest {
                                client_request_id: Some(request_id.clone()),
                                client_sequence: Some(client_sequence),
                                client_session_id: Some(client_session_id.clone()),
                                liked: Some(desired),
                                post_id: Some(post_id_value),
                            },
                        )
                        .await?;
                    RisePostEngagementAcknowledgement::from(&reply)
                }
                RisePostContentRepositoryEngagementKind::Favorite => {
                    let reply: SetPostFavoriteResponse = self
                        .rpc
                        .call(
                            RisePostEndpoints::set_post_favorite(),
                            &SetPostFavoriteRequest {
                                client_request_id: Some(request_id.clone()),
                                client_sequence: Some(client_sequence),
                                client_session_id: Some(client_session_id.clone()),
                                favorited: Some(desired),
                                post_id: Some(post_id_value),
                            },
                        )
                        .await?;
                    RisePostEngagementAcknowledgement::from(&reply)
                }
            };
            if response.client_request_id.as_deref() != Some(request_id.as_str())
                || response.client_session_id.as_deref() != Some(client_session_id.as_str())
                || response.client_sequence != Some(client_sequence)
            {
                return Err(RisePostMutationError::Server("mutation_ack_mismatch".to_owned()).into());
            }
            if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
                return Ok(());
            }
            if !self.state.with(|s| is_current_generation(s)) {
                return Ok(());
            }
            let Some(current_entity) = self.entity_by_server_id(post_id).await? else {
                return Ok(());
            };
            let Ok(mut current) = serde_json::from_slice::<RisePostStoredEntity>(&current_entity.data) else {
                return Ok(());
            };
            match kind {
                RisePostContentRepositoryEngagementKind::Like => {
                    current.post.is_liked = response.is_liked.unwrap_or(desired);
                    current.post.old_is_liked = current.post.is_liked;
                }
                RisePostContentRepositoryEngagementKind::Favorite => {
                    current.post.is_favorited = response.is_favorited.unwrap_or(desired);
                    current.post.old_is_favorited = current.post.is_favorited;
                }
            }
            current.post.likes_count = response.likes_count.max(0);
            current.post.favorites_count = response.favorites_count.max(0);
            let data = serde_json::to_vec(&current)?;
            let store_version = self.state.with(|s| {
                Self::advance_store_version(s);
                Self::record_mutation_version(s, post_id);
                s.store_version
            });
            let mutations = vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(current_entity.local_id, data, store_version, Self::now_milliseconds()),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }];
            self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
            Ok(())
        }
        .await;
        let Err(error) = outcome else {
            return;
        };
        if !self.state.with(|s| is_current_generation(s)) {
            return;
        }
        let Ok(Some(current_entity)) = self.entity_by_server_id(post_id).await else {
            return;
        };
        let Ok(mut current) = serde_json::from_slice::<RisePostStoredEntity>(&current_entity.data) else {
            return;
        };
        match kind {
            RisePostContentRepositoryEngagementKind::Like => {
                if current.post.is_liked != current.post.old_is_liked {
                    current.post.likes_count =
                        (current.post.likes_count + if current.post.old_is_liked { 1 } else { -1 }).max(0);
                }
                current.post.is_liked = current.post.old_is_liked;
            }
            RisePostContentRepositoryEngagementKind::Favorite => {
                if current.post.is_favorited != current.post.old_is_favorited {
                    current.post.favorites_count =
                        (current.post.favorites_count + if current.post.old_is_favorited { 1 } else { -1 }).max(0);
                }
                current.post.is_favorited = current.post.old_is_favorited;
            }
        }
        let data = serde_json::to_vec(&current).unwrap_or_else(|_| current_entity.data.clone());
        let restored = match kind {
            RisePostContentRepositoryEngagementKind::Like => current.post.is_liked,
            RisePostContentRepositoryEngagementKind::Favorite => current.post.is_favorited,
        };
        let mutations = self.state.with(|s| {
            Self::advance_store_version(s);
            let mut mutations = vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(
                    current_entity.local_id.clone(),
                    data,
                    s.store_version,
                    Self::now_milliseconds(),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }];
            if kind == RisePostContentRepositoryEngagementKind::Favorite {
                mutations.extend(self.favorites_shelf_mutations(s, &current_entity.local_id, restored));
            }
            mutations
        });
        let _ = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await;
        let handler = self.state.with(|s| s.engagement_refusal_handlers.remove(&key));
        let failure = CanonErrors::failure(&error)
            .or_else(|| CanonErrors::is_maintenance_refusal(&error).then(|| MAINTENANCE_FAILURE.clone()));
        if let (Some(handler), Some(failure)) = (handler, failure)
            && let Some(main) = RiseEngineMainHop::installed()
        {
            let _ = main
                .run(move |cx: AsyncApp| {
                    async move {
                        cx.update(|cx| handler(failure, restored, cx));
                        Ok(())
                    }
                    .boxed_local()
                })
                .await;
        }
    }

    pub fn sparks_push_handler(&self) -> RiseSyncPushHandler {
        let this = self.this.clone();
        RiseSyncPushHandler::new(rise_events::name::POST_SPARKS_UPDATED, move |envelope| {
            let this = this.clone();
            async move {
                let Some(this) = this.upgrade() else {
                    return Ok(None);
                };
                this.make_sparks_batch(envelope).await
            }
        })
    }

    /// a pending send drawn on the post while its undo window runs, or taken back
    /// relative so whatever realtime changed meanwhile survives
    pub async fn apply_sparks_delta(
        &self,
        post_id: &str,
        delta: i64,
    ) -> anyhow::Result<Option<RisePostContentPresentationSnapshot>> {
        if delta == 0 {
            return Ok(None);
        }
        self.patch_sparks(post_id, |post| {
            post.sparks_total = (post.sparks_total + delta).max(0);
            post.my_sparks = (post.my_sparks + delta).max(0);
        })
        .await
    }

    pub async fn send_sparks(
        &self,
        post_id: &str,
        amount: i64,
        anonymous: bool,
        idempotency_key: &str,
    ) -> anyhow::Result<(RiseStoredSparksSendResult, Option<RisePostContentPresentationSnapshot>)> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        let Ok(post_id_value) = post_id.parse::<i64>() else {
            return Err(RisePostMutationError::Server("invalid_post_id".to_owned()).into());
        };
        let reply: SendPostSparksResponse = self
            .rpc
            .attested_call(
                RisePostEndpoints::send_post_sparks(),
                &SendPostSparksRequest {
                    amount: Some(amount),
                    anonymous: Some(anonymous),
                    idempotency_key: Some(idempotency_key.to_owned()),
                    post_id: Some(post_id_value),
                },
            )
            .await?;
        let response = RiseStoredSparksSendResult::from_post_canon(&reply);
        let (sparks_total, my_sparks) = (response.sparks_total, response.my_sparks);
        let snapshot = self
            .patch_sparks(post_id, |post| {
                let others = (post.sparks_total - amount).max(0);
                post.sparks_total = sparks_total.max(others);
                post.my_sparks = my_sparks.max(0);
            })
            .await?;
        Ok((response, snapshot))
    }

    pub async fn sparks_top(&self, post_id: &str, limit: i64) -> anyhow::Result<RiseStoredSparksTop> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        let Ok(post_id_value) = post_id.parse::<i64>() else {
            return Err(RisePostMutationError::Server("invalid_post_id".to_owned()).into());
        };
        let reply: GetPostSparksResponse = self
            .rpc
            .call(
                RisePostEndpoints::get_post_sparks(),
                &GetPostSparksRequest { limit: Some(limit), post_id: Some(post_id_value) },
            )
            .await?;
        Ok(RiseStoredSparksTop::from_post_canon(&reply))
    }

    async fn patch_sparks(
        &self,
        post_id: &str,
        change: impl FnOnce(&mut GetPostFeedResponse),
    ) -> anyhow::Result<Option<RisePostContentPresentationSnapshot>> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Err(RisePostRepositoryError::StaleAccount.into());
        }
        let Some(stored_entity) = self.entity_by_server_id(post_id).await? else {
            x251_mark(|| format!("x398 patch_sparks entity none post={post_id}"));
            return Ok(None);
        };
        let Ok(mut stored) = serde_json::from_slice::<RisePostStoredEntity>(&stored_entity.data) else {
            x251_mark(|| format!("x398 patch_sparks decode failed post={post_id}"));
            return Ok(None);
        };
        let before = (stored.post.sparks_total, stored.post.my_sparks);
        change(&mut stored.post);
        x251_mark(|| {
            format!("x398 patch_sparks post={post_id} before={before:?} after={:?}", (stored.post.sparks_total, stored.post.my_sparks))
        });
        if (stored.post.sparks_total, stored.post.my_sparks) == before {
            return Ok(None);
        }
        let data = serde_json::to_vec(&stored)?;
        let store_version = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, post_id);
            s.store_version
        });
        let mutations = vec![RiseStoreMutation::UpsertEntity {
            namespace: ENTITY_NAMESPACE.clone(),
            entity: RiseStoreEntity::new(stored_entity.local_id, data, store_version, Self::now_milliseconds()),
            policy: RiseEntityWritePolicy::ServerAuthoritative,
        }];
        self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(Some(self.refresh_active_pages_from_store().await))
    }

    async fn make_sparks_batch(&self, envelope: RiseWirePushEnvelope) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Ok(None);
        }
        let Some(root) = envelope.data.object_value() else {
            return Ok(None);
        };
        let Some(post_id) = Self::sparks_integer(root.get(post_key::POST_ID)).map(|id| id.to_string()) else {
            return Ok(None);
        };
        let Some(stored_entity) = self.entity_by_server_id(&post_id).await? else {
            return Ok(None);
        };
        let Ok(mut stored) = serde_json::from_slice::<RisePostStoredEntity>(&stored_entity.data) else {
            return Ok(None);
        };
        let mut changed = false;
        if let Some(total) = Self::sparks_integer(root.get(post_key::SPARKS_TOTAL))
            && total > stored.post.sparks_total
        {
            stored.post.sparks_total = total;
            changed = true;
        }
        if let Some(mine) = Self::sparks_integer(root.get(post_key::MY_SPARKS))
            && mine > stored.post.my_sparks
        {
            stored.post.my_sparks = mine;
            changed = true;
        }
        if !changed {
            return Ok(None);
        }
        let data = serde_json::to_vec(&stored)?;
        let store_version = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &post_id);
            s.store_version
        });
        Ok(Some(RiseStoreWriteBatch::new(
            vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(stored_entity.local_id, data, store_version, Self::now_milliseconds()),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }],
            HashSet::new(),
        )))
    }

    fn sparks_integer(value: Option<&RiseJSONValue>) -> Option<i64> {
        match value? {
            RiseJSONValue::Integer(number) => Some(*number),
            RiseJSONValue::UnsignedInteger(number) => i64::try_from(*number).ok(),
            RiseJSONValue::Number(number) => {
                let rounded = number.round();
                (rounded.is_finite() && rounded >= i64::MIN as f64 && rounded < i64::MAX as f64)
                    .then_some(rounded as i64)
            }
            RiseJSONValue::String(text) => text.parse::<i64>().ok(),
            _ => None,
        }
    }

    async fn request(&self, ticket: &RisePostContentRepositoryLoadTicket) -> anyhow::Result<Box<dyn RisePostWirePage>> {
        let is_first_page = ticket.intent != RisePostContentRepositoryLoadIntent::LoadMore;
        let cursor_session = ticket.cursor.as_ref().and_then(|cursor| cursor.feed_session_id.clone());
        let starts_new_session = ticket.intent == RisePostContentRepositoryLoadIntent::Refresh || cursor_session.is_none();
        let relative_id = if ticket.intent == RisePostContentRepositoryLoadIntent::LoadMore {
            ticket.cursor.as_ref().and_then(|cursor| cursor.relative_id)
        } else {
            None
        };
        let feed_session_id = if starts_new_session { None } else { cursor_session };
        match &ticket.scope {
            RisePostPageScope::Shorts => {
                let page: GetShortsFeedResponse = self
                    .rpc
                    .call(
                        RisePostEndpoints::get_shorts_feed(),
                        &GetShortsFeedRequest {
                            include_image_reels: Some(true),
                            limit: Some(Self::PAGE_LIMIT),
                            relative_id,
                        },
                    )
                    .await?;
                Ok(Box::new(page))
            }
            RisePostPageScope::Feed(kind) => {
                let page_number = if is_first_page {
                    1
                } else {
                    (ticket.cursor.as_ref().map_or(1, |cursor| cursor.page) + 1).max(2)
                };
                let page: GetFeedResponse = self
                    .rpc
                    .call(
                        RisePostEndpoints::feed(*kind),
                        &GetFeedRequest {
                            content_kind: kind.content_kind().map(|value| value.wire().to_owned()),
                            feed_session_id,
                            limit: Some(Self::PAGE_LIMIT),
                            new_feed: Some(starts_new_session),
                            page: Some(page_number),
                            relative_id,
                            sort: Some(Self::FEED_SORT.to_owned()),
                            up: Some(false),
                            ..Default::default()
                        },
                    )
                    .await?;
                Ok(Box::new(page))
            }
            RisePostPageScope::Profile { tag, kind, .. } => {
                if *kind == RisePostProfileKind::Reposts {
                    let page: GetUserRepostsResponse = self
                        .rpc
                        .call(
                            RisePostEndpoints::profile(*kind),
                            &GetUserRepostsRequest {
                                limit: Some(Self::PAGE_LIMIT),
                                relative_id,
                                tag: Some(tag.clone()),
                                up: Some(false),
                            },
                        )
                        .await?;
                    return Ok(Box::new(page));
                }
                if *kind == RisePostProfileKind::Mentions {
                    let page: GetUserMentionsResponse = self
                        .rpc
                        .call(
                            RisePostEndpoints::profile(*kind),
                            &GetUserMentionsRequest {
                                limit: Some(Self::PAGE_LIMIT),
                                relative_id,
                                tag: Some(tag.clone()),
                                up: Some(false),
                            },
                        )
                        .await?;
                    return Ok(Box::new(page));
                }
                let page: GetUserPostsResponse = self
                    .rpc
                    .call(
                        RisePostEndpoints::profile(*kind),
                        &GetUserPostsRequest {
                            content_kind: kind.content_kind().map(|value| value.wire().to_owned()),
                            feed_session_id,
                            limit: Some(Self::PAGE_LIMIT),
                            new_feed: Some(starts_new_session),
                            relative_id,
                            sort: Some(Self::PROFILE_SORT.to_owned()),
                            tag: Some(tag.clone()),
                            up: Some(false),
                            ..Default::default()
                        },
                    )
                    .await?;
                Ok(Box::new(page))
            }
            RisePostPageScope::Search { query, kind } => {
                let normalized = RisePostPageScope::normalized_query(query);
                if let Some(hashtag) = normalized.strip_prefix('#') {
                    let page: SearchPostsByHashtagResponse = self
                        .rpc
                        .call(
                            RisePostEndpoints::search_posts_by_hashtag(),
                            &SearchPostsByHashtagRequest {
                                content_kind: Some(kind.content_kind().wire().to_owned()),
                                hashtag: Some(hashtag.to_owned()),
                                limit: Some(Self::PAGE_LIMIT),
                                relative_id,
                                up: Some(false),
                            },
                        )
                        .await?;
                    return Ok(Box::new(page));
                }
                let page: SearchPostsResponse = self
                    .rpc
                    .call(
                        RisePostEndpoints::search(*kind),
                        &SearchPostsRequest {
                            content_kind: Some(kind.content_kind().wire().to_owned()),
                            limit: Some(Self::PAGE_LIMIT),
                            q: Some(normalized.clone()),
                            relative_id,
                            up: Some(false),
                            ..Default::default()
                        },
                    )
                    .await?;
                Ok(Box::new(page))
            }
            RisePostPageScope::Detail { post_id } => {
                let value = match post_id.parse::<i64>() {
                    Ok(value) if value > 0 => value,
                    _ => return Err(RisePostMutationError::Server("invalid_post_id".to_owned()).into()),
                };
                let result: anyhow::Result<GetPostResponse> = self
                    .rpc
                    .call(RisePostEndpoints::get_post(), &GetPostRequest { post_id: Some(value) })
                    .await;
                let post = match result {
                    Ok(response) => response.post,
                    Err(error)
                        if error.downcast_ref::<RiseWireRemoteError>().is_some_and(|remote| {
                            CanonErrors::category(Some(&remote.code)) == ErrorCategory::NotFound
                        }) =>
                    {
                        None
                    }
                    Err(error) => return Err(error),
                };
                let total = if post.is_none() { 0 } else { 1 };
                Ok(Box::new(GetFeedResponse {
                    is_have_more: false,
                    limit: 1,
                    list: post.into_iter().collect(),
                    total,
                    ..Default::default()
                }))
            }
        }
    }

    async fn make_link_preview_batch(
        &self,
        envelope: RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Ok(None);
        }
        let event: RisePostLinkPreviewEvent = envelope.data.decode_as()?;
        let Some(post_id) = event.post_id.as_ref().or(event.id.as_ref()).map(WireID::as_string) else {
            return Ok(None);
        };
        let Some(preview) = event.link_preview.as_ref().map(PostLinkPreviewDTOExt::as_domain) else {
            return Ok(None);
        };
        let Some(entity) = self.entity_by_server_id(&post_id).await? else {
            return Ok(None);
        };
        let Ok(mut stored) = serde_json::from_slice::<RisePostStoredEntity>(&entity.data) else {
            return Ok(None);
        };
        let kind = RisePostContentKind::from_wire(event.content_kind.as_deref().unwrap_or(&stored.post.content_kind));
        if kind != Some(RisePostContentKind::Thread) {
            return Ok(None);
        }
        self.load_scope_index_if_needed().await;
        if preview.revision <= stored.post.link_preview.as_ref().map_or(-1, |current| current.revision) {
            return Ok(None);
        }
        stored.post.content_kind = RisePostContentKind::Thread.wire().to_owned();
        stored.post.link_preview = Some(preview);
        let store_version = self.state.with(|s| {
            Self::advance_store_version(s);
            s.store_version
        });
        let now = Self::now_milliseconds();
        Ok(Some(RiseStoreWriteBatch::new(
            vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(
                    entity.local_id.clone(),
                    serde_json::to_vec(&stored)?,
                    store_version.max(entity.version + 1),
                    now,
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }],
            HashSet::new(),
        )))
    }

    async fn make_media_ready_batch(
        &self,
        envelope: RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Ok(None);
        }
        let event: RisePostMediaReadyEvent = envelope.data.decode_as()?;
        let Some(post_id) = event.post_id.as_ref().map(WireID::as_string).filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        let video = RemoteMediaURL::optional_normalized(event.video.as_deref());
        let status = event.media_status.clone().unwrap_or_else(|| PostMediaStatus::Ready.wire().to_owned());

        if let Some(main) = RiseEngineMainHop::installed() {
            let (hop_post_id, hop_video, hop_track, hop_status) =
                (post_id.clone(), video.clone(), event.track.clone(), status.clone());
            let audio_baked = event.audio_baked;
            let _ = main
                .run(move |cx: AsyncApp| {
                    async move {
                        cx.update(|cx| {
                            PostServicesStore::shared(cx).update(cx, |store, cx| {
                                store.apply_media_ready(
                                    &hop_post_id,
                                    hop_video,
                                    hop_track,
                                    audio_baked,
                                    &hop_status,
                                    cx,
                                );
                            });
                        });
                        Ok(())
                    }
                    .boxed_local()
                })
                .await;
        }

        let Some(entity) = self.entity_by_server_id(&post_id).await? else {
            return Ok(None);
        };
        let Ok(mut stored) = serde_json::from_slice::<RisePostStoredEntity>(&entity.data) else {
            return Ok(None);
        };

        let mut changed = false;
        if let Some(video) = video.filter(|video| !video.is_empty())
            && stored.post.video.as_deref() != Some(video.as_str())
        {
            stored.post.video = Some(video);
            changed = true;
        }
        if let Some(track) = event.track.clone()
            && stored.post.track.as_ref() != Some(&track)
        {
            if let Some(track_id) = track.id.clone().filter(|id| !id.is_empty()) {
                stored.post.track_id = Some(track_id);
            }
            stored.post.track = Some(track);
            changed = true;
        }
        if let Some(baked) = event.audio_baked
            && stored.post.audio_baked != baked
        {
            stored.post.audio_baked = baked;
            changed = true;
        }
        if stored.post.media_status != status {
            stored.post.media_status = status;
            changed = true;
        }
        if !changed {
            return Ok(None);
        }

        let store_version = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &stored.post.id);
            s.store_version
        });
        let now = Self::now_milliseconds();
        Ok(Some(RiseStoreWriteBatch::new(
            vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(
                    entity.local_id.clone(),
                    serde_json::to_vec(&stored)?,
                    store_version.max(entity.version + 1),
                    now,
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }],
            HashSet::new(),
        )))
    }

    async fn make_collaborators_changed_batch(
        &self,
        envelope: RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Ok(None);
        }
        let event: RisePostCollaboratorsChangedEvent = envelope.data.decode_as()?;
        let Some(post_id) = event.post_id.as_ref().map(WireID::as_string).filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        self.load_scope_index_if_needed().await;
        let Some(entity) = self.entity_by_server_id(&post_id).await? else {
            return Ok(None);
        };
        let Ok(mut stored) = serde_json::from_slice::<RisePostStoredEntity>(&entity.data) else {
            return Ok(None);
        };
        if let Some(coauthors) = event.coauthors {
            stored.post.coauthors = coauthors;
        }
        stored.post.coauthors_count = event.coauthors_count.unwrap_or(stored.post.coauthors.len() as i64);
        stored.post.pending_coauthors_count = event.pending_coauthors_count.unwrap_or(0);
        stored.post.my_collaboration = event.my_collaboration;
        if let Some(tagged_users) = event.tagged_users {
            stored.post.tagged_users = tagged_users;
        }
        stored.post.tagged_users_count = event.tagged_users_count.unwrap_or(stored.post.tagged_users.len() as i64);
        let data = serde_json::to_vec(&stored)?;
        let updated_at = event.updated_at_milliseconds.unwrap_or_else(Self::now_milliseconds);
        let mutations = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &stored.post.id);
            let version = s.store_version.max(entity.version + 1);
            let mut mutations = vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(entity.local_id.clone(), data, version, updated_at),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }];
            mutations.extend(self.authoritative_membership_mutations(s, &entity.local_id, &stored.post, version));
            self.apply_authoritative_collaborator_rows(s, &stored.post, &post_id);
            mutations
        });
        Ok(Some(RiseStoreWriteBatch::new(mutations, HashSet::new())))
    }

    async fn make_post_deleted_batch(
        &self,
        envelope: RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Ok(None);
        }
        let event: RisePostAuthorEvent = envelope.data.decode_as()?;
        let typed_id = match Self::canon_event(&envelope) {
            Some(RiseEvent::PostDeleted(deleted)) if deleted.post_id > 0 => Some(deleted.post_id.to_string()),
            _ => None,
        };
        let Some(post_id) =
            typed_id.or_else(|| event.post_id.as_ref().map(WireID::as_string)).filter(|id| !id.is_empty())
        else {
            return Ok(None);
        };
        if let Some(main) = RiseEngineMainHop::installed() {
            let hop_post_id = post_id.clone();
            let _ = main
                .run(move |cx: AsyncApp| {
                    async move {
                        cx.update(|cx| {
                            PostServicesStore::shared(cx).update(cx, |store, cx| {
                                store.apply_live_deletion(&hop_post_id, cx);
                            });
                        });
                        Ok(())
                    }
                    .boxed_local()
                })
                .await;
        }
        let Some(entity) = self.entity_by_server_id(&post_id).await? else {
            return Ok(None);
        };
        self.load_scope_index_if_needed().await;
        self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &post_id);
            s.deleted_post_versions.insert(post_id.clone(), s.store_version);
            Self::trim_mutation_versions(s);
        });
        let mut mutations = self.membership_removals(&entity.local_id).await;
        mutations.push(RiseStoreMutation::RemoveEntity {
            namespace: ENTITY_NAMESPACE.clone(),
            local_id: entity.local_id.clone(),
        });
        Ok(Some(RiseStoreWriteBatch::new(mutations, HashSet::new())))
    }

    async fn membership_removals(&self, local_id: &str) -> Vec<RiseStoreMutation> {
        let views: Vec<String> = self.state.with(|s| s.scope_index.slots.keys().cloned().collect());
        let mut mutations = Vec::new();
        for view in views {
            let Ok(window) = self
                .engine
                .entity_window(RiseEntityWindowQuery::new(
                    ENTITY_NAMESPACE.clone(),
                    view.clone(),
                    RiseEntityWindow::First { limit: Self::MAXIMUM_VIEW_ITEMS },
                ))
                .await
            else {
                continue;
            };
            if !window.items.iter().any(|item| item.entity.local_id == local_id) {
                continue;
            }
            mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                namespace: ENTITY_NAMESPACE.clone(),
                view,
                local_id: local_id.to_owned(),
                if_version_at_most: None,
            });
        }
        mutations
    }

    async fn make_post_updated_batch(
        &self,
        envelope: RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Ok(None);
        }
        let event: RisePostAuthorEvent = envelope.data.decode_as()?;
        let Some(dto) = event.post else {
            return Ok(None);
        };
        let Some(post) = dto.as_feed_response() else {
            return Ok(None);
        };
        let Some(server_entity) = self.entity_by_server_id(&post.id).await? else {
            return Ok(None);
        };
        let Ok(existing) = serde_json::from_slice::<RisePostStoredEntity>(&server_entity.data) else {
            return Ok(None);
        };
        let mut post = Self::canonicalize(post, &dto, Some(&existing.post));
        post.count_visibility = existing.post.count_visibility;
        Self::settle_pushed_post(&mut post, &server_entity.local_id);
        let store_version = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &post.id);
            s.store_version
        });
        let updated_at = post.updated_at_ms.unwrap_or_else(Self::now_milliseconds);
        Ok(Some(RiseStoreWriteBatch::new(
            vec![RiseStoreMutation::UpsertEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                entity: RiseStoreEntity::new(
                    server_entity.local_id.clone(),
                    serde_json::to_vec(&RisePostStoredEntity { post })?,
                    store_version,
                    updated_at,
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }],
            HashSet::new(),
        )))
    }

    /// a pushed row is the servers final word, no upload or compression is pending for it
    fn settle_pushed_post(post: &mut GetPostFeedResponse, local_id: &str) {
        post.presentation_id = Some(local_id.to_owned());
        post.is_temp = false;
        post.is_uploading = false;
        post.is_compressing = false;
        post.upload_progress = 100.0;
        post.compression_progress = 100.0;
        post.local_preview_url = None;
    }

    /// frame read through its canon event type, none if it doesnt decode as one
    fn canon_event(envelope: &RiseWirePushEnvelope) -> Option<RiseEvent> {
        let data = envelope.data.data(false).ok()?;
        RiseEvent::decode(&envelope.r#type, &data).ok()
    }

    async fn make_post_created_batch(
        &self,
        envelope: RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_account().await {
            return Ok(None);
        }
        let event: RisePostCreatedEvent = envelope.data.decode_as()?;
        let Some(dto) = event.post.clone() else {
            return Ok(None);
        };
        let Some(post) = dto.as_feed_response() else {
            return Ok(None);
        };
        let typed_request_id = match Self::canon_event(&envelope) {
            Some(RiseEvent::PostCreated(created)) if !created.client_request_id.is_empty() => {
                Some(created.client_request_id)
            }
            _ => None,
        };
        let lookup_request_id = typed_request_id.or_else(|| event.client_request_id.clone()).filter(|id| !id.is_empty());
        let client_entity = match lookup_request_id {
            Some(request_id) => {
                self.engine
                    .entity_alias(
                        ENTITY_NAMESPACE.clone(),
                        RiseEntityAlias::new(RiseEntityAliasKind::client_id(), request_id),
                    )
                    .await?
            }
            None => None,
        };
        let server_entity = self.entity_by_server_id(&post.id).await?;
        let survivor = client_entity.as_ref().or(server_entity.as_ref());
        let existing = survivor
            .and_then(|entity| serde_json::from_slice::<RisePostStoredEntity>(&entity.data).ok())
            .map(|stored| stored.post);
        if let Some(existing) = &existing
            && !existing.is_temp
            && existing.id == post.id
            && existing.updated_at_ms.unwrap_or(0) >= post.updated_at_ms.unwrap_or(0)
        {
            return Ok(None);
        }
        let mut post = Self::canonicalize(post, &dto, existing.as_ref());
        let local_id = survivor.map_or_else(|| post.id.clone(), |entity| entity.local_id.clone());
        Self::settle_pushed_post(&mut post, &local_id);
        let store_version = self.state.with(|s| {
            Self::advance_store_version(s);
            Self::record_mutation_version(s, &post.id);
            s.store_version
        });
        let mut mutations = Vec::new();
        if let (Some(client_entity), Some(server_entity)) = (&client_entity, &server_entity)
            && client_entity.local_id != server_entity.local_id
        {
            mutations.push(RiseStoreMutation::MergeEntities {
                namespace: ENTITY_NAMESPACE.clone(),
                survivor_local_id: client_entity.local_id.clone(),
                duplicate_local_id: server_entity.local_id.clone(),
            });
        }
        let updated_at = post.updated_at_ms.unwrap_or_else(Self::now_milliseconds);
        mutations.push(RiseStoreMutation::UpsertEntity {
            namespace: ENTITY_NAMESPACE.clone(),
            entity: RiseStoreEntity::new(
                local_id.clone(),
                serde_json::to_vec(&RisePostStoredEntity { post: post.clone() })?,
                store_version,
                updated_at,
            ),
            policy: RiseEntityWritePolicy::ServerAuthoritative,
        });
        mutations.push(RiseStoreMutation::SetEntityAlias {
            namespace: ENTITY_NAMESPACE.clone(),
            alias: RiseEntityAlias::new(RiseEntityAliasKind::server_id(), post.id.clone()),
            local_id: local_id.clone(),
        });
        if let Some(request_id) = event.client_request_id.filter(|id| !id.is_empty()) {
            mutations.push(RiseStoreMutation::SetEntityAlias {
                namespace: ENTITY_NAMESPACE.clone(),
                alias: RiseEntityAlias::new(RiseEntityAliasKind::client_id(), request_id),
                local_id: local_id.clone(),
            });
        }
        self.load_scope_index_if_needed().await;
        let subject = RisePostContentRepositoryMembershipSubject::from_post(&post);
        self.state.with(|s| {
            for (view, slot) in &s.scope_index.slots {
                match Self::membership_of_slot(s, slot, &subject) {
                    RisePostContentRepositoryScopeMembership::Accepts => {
                        if !Self::can_prepend_created_post(s, slot) {
                            continue;
                        }
                        let position = RisePositionKey::make(|builder| {
                            builder.append_signed_int64(-store_version);
                            builder.append_string(&local_id);
                        });
                        mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                            namespace: ENTITY_NAMESPACE.clone(),
                            view: view.clone(),
                            membership: RiseEntityViewMembership::new(local_id.clone(), position, store_version),
                        });
                    }
                    RisePostContentRepositoryScopeMembership::Rejects => {
                        mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                            namespace: ENTITY_NAMESPACE.clone(),
                            view: view.clone(),
                            local_id: local_id.clone(),
                            if_version_at_most: None,
                        });
                    }
                    RisePostContentRepositoryScopeMembership::Unknown => {}
                }
            }
        });
        Ok(Some(RiseStoreWriteBatch::new(mutations, HashSet::new())))
    }

    async fn load_page(
        &self,
        scope: &RisePostPageScope,
        view: &str,
        source: Option<RisePostCacheSource>,
    ) -> RisePostPagePresentation {
        let scope_key = scope.key();
        if source.is_none()
            && let Some(memory) = self.state.with(|s| s.pages_by_scope_key.get(&scope_key).cloned())
        {
            RisePostDebugTrace::cache(None, RisePostCacheSource::Memory, memory.revision);
            return memory;
        }
        let window = if Self::slides_window(scope) {
            RiseEntityWindow::Latest { limit: self.maximum_presentation_items() }
        } else {
            RiseEntityWindow::First { limit: self.maximum_presentation_items() }
        };
        let Ok(result) = self
            .engine
            .entity_window(RiseEntityWindowQuery::new(ENTITY_NAMESPACE.clone(), view.to_owned(), window))
            .await
        else {
            return RisePostPagePresentation::IDLE;
        };
        let mut posts: Vec<GetPostFeedResponse> = Vec::with_capacity(result.items.len());
        let mut interrupted_creates: Vec<String> = Vec::new();
        self.state.with(|s| {
            for item in &result.items {
                let Ok(stored) = serde_json::from_slice::<RisePostStoredEntity>(&item.entity.data) else {
                    continue;
                };
                if !stored.post.is_temp {
                    s.session_create_local_ids.remove(&item.entity.local_id);
                }
                if stored.post.is_temp && !s.session_create_local_ids.contains(&item.entity.local_id) {
                    interrupted_creates.push(item.entity.local_id.clone());
                    continue;
                }
                let mut post = stored.post;
                post.presentation_id = Some(item.entity.local_id.clone());
                posts.push(post);
            }
        });
        if !interrupted_creates.is_empty() {
            self.discard_interrupted_creates(interrupted_creates).await;
        }
        let Some(state) = result.state.as_ref() else {
            return RisePostPagePresentation::IDLE;
        };
        let Some(cursor) = Self::decode_cursor(state.after_cursor.as_deref()) else {
            return RisePostPagePresentation::IDLE;
        };
        if cursor.scope_key != scope_key {
            return RisePostPagePresentation::IDLE;
        }
        let resolved_source = source.unwrap_or(RisePostCacheSource::PersistentStore);
        RisePostDebugTrace::cache(None, resolved_source, state.revision);
        RisePostPagePresentation {
            status: if state.load_state == RiseEntityViewLoadState::Failed && posts.is_empty() {
                RisePostPageStatus::Failed
            } else {
                RisePostPageStatus::Loaded
            },
            items: posts,
            error_message: state.last_error_code.clone(),
            has_more: state.has_more_after
                && (Self::slides_window(scope) || result.items.len() < Self::MAXIMUM_VIEW_ITEMS),
            is_refreshing: false,
            is_loading_more: false,
            cache_source: resolved_source,
            revision: state.updated_at_milliseconds,
        }
    }

    async fn discard_interrupted_creates(&self, local_ids: Vec<String>) {
        let fresh: Vec<String> = self.state.with(|s| {
            if s.is_shutdown {
                return Vec::new();
            }
            local_ids.into_iter().filter(|local_id| s.discarding_create_local_ids.insert(local_id.clone())).collect()
        });
        if fresh.is_empty() {
            return;
        }
        let mutations: Vec<RiseStoreMutation> = fresh
            .iter()
            .map(|local_id| RiseStoreMutation::RemoveEntity {
                namespace: ENTITY_NAMESPACE.clone(),
                local_id: local_id.clone(),
            })
            .collect();
        let _ = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await;
        self.state.with(|s| {
            for local_id in &fresh {
                s.discarding_create_local_ids.remove(local_id);
            }
        });
    }

    async fn load_cursor(&self, scope_key: &str, view: &str) -> Option<RisePostPageCursor> {
        let state = self.engine.entity_view_state(ENTITY_NAMESPACE.clone(), view.to_owned()).await.ok()??;
        let cursor = Self::decode_cursor(state.after_cursor.as_deref())?;
        (cursor.scope_key == scope_key).then_some(cursor)
    }

    async fn stored_post(&self, id: &str) -> anyhow::Result<Option<GetPostFeedResponse>> {
        let Some(entity) = self.entity_by_server_id(id).await? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice::<RisePostStoredEntity>(&entity.data)?.post))
    }

    async fn entity_by_server_id(&self, server_id: &str) -> anyhow::Result<Option<RiseStoreEntity>> {
        if let Some(aliased) = self
            .engine
            .entity_alias(
                ENTITY_NAMESPACE.clone(),
                RiseEntityAlias::new(RiseEntityAliasKind::server_id(), server_id.to_owned()),
            )
            .await?
        {
            return Ok(Some(aliased));
        }
        self.engine.entity_local_id(ENTITY_NAMESPACE.clone(), server_id.to_owned()).await
    }

    async fn entity_by_local_id(&self, local_id: &str) -> anyhow::Result<Option<RiseStoreEntity>> {
        self.engine.entity_local_id(ENTITY_NAMESPACE.clone(), local_id.to_owned()).await
    }

    fn validate_create(draft: &RisePostCreateDraft) -> Result<(), RisePostMutationError> {
        let has_track = draft
            .track_id
            .as_deref()
            .is_some_and(|track_id| !trimming_whitespaces_and_newlines(track_id).is_empty());
        let has_achievement = draft.achievement_id.unwrap_or(0) > 0;
        let carries_media = !draft.local_uploads.is_empty()
            || draft.local_video.is_some()
            || draft.is_short()
            || draft.is_image_reel()
            || has_track;
        if has_achievement && carries_media {
            return Err(RisePostMutationError::InvalidDraft("achievement_cannot_contain_media".to_owned()));
        }
        match draft.content_kind {
            RisePostContentKind::Thread => {
                if carries_media {
                    return Err(RisePostMutationError::InvalidDraft("thread_cannot_contain_media".to_owned()));
                }
                if PostFieldRules::is_description_missing(draft.content_kind, &draft.content) {
                    return Err(RisePostMutationError::InvalidDraft("thread_requires_description".to_owned()));
                }
            }
            RisePostContentKind::Publication => {
                if draft.local_uploads.is_empty() && draft.local_video.is_none() && !has_track && !has_achievement {
                    return Err(RisePostMutationError::InvalidDraft("publication_requires_media".to_owned()));
                }
                if draft.is_short() && draft.local_video.is_none() {
                    return Err(RisePostMutationError::InvalidDraft("short_requires_video".to_owned()));
                }
            }
        }
        Ok(())
    }

    fn make_slot(scope: &RisePostPageScope, access: u64) -> RisePostContentRepositoryScopeSlot {
        let mut owner: Option<String> = None;
        let mut content_kind: Option<String> = None;
        let mut profile_kind: Option<String> = None;
        let family = match scope {
            RisePostPageScope::Shorts => "shorts",
            RisePostPageScope::Feed(kind) => {
                content_kind = kind.content_kind().map(|value| value.wire().to_owned());
                "feed"
            }
            RisePostPageScope::Profile { user_id, kind, .. } => {
                owner = Some(user_id.clone());
                content_kind = kind.content_kind().map(|value| value.wire().to_owned());
                profile_kind = Some(kind.wire().to_owned());
                "profile"
            }
            RisePostPageScope::Search { .. } => "search",
            RisePostPageScope::Detail { .. } => "detail",
        };
        RisePostContentRepositoryScopeSlot {
            scope_key: scope.key(),
            access,
            family: Some(family.to_owned()),
            owner_user_id: owner,
            content_kind,
            profile_kind,
        }
    }

    /// only profiles have a locally known position for a new post
    /// recommendation and search ordering belongs to their page responses
    fn can_prepend_created_post(
        s: &RisePostContentRepositoryState,
        slot: &RisePostContentRepositoryScopeSlot,
    ) -> bool {
        if let Some(scope) = s.scopes_by_key.get(&slot.scope_key) {
            return matches!(scope, RisePostPageScope::Profile { .. });
        }
        slot.family.as_deref() == Some("profile")
    }

    /// whether the post is eligible for the slot regardless of who decides its position
    /// live scope asked first, else the slot is parsed
    /// unknown means a legacy format slot
    fn membership_of_slot(
        s: &RisePostContentRepositoryState,
        slot: &RisePostContentRepositoryScopeSlot,
        subject: &RisePostContentRepositoryMembershipSubject,
    ) -> RisePostContentRepositoryScopeMembership {
        use RisePostContentRepositoryScopeMembership::{Accepts, Rejects, Unknown};
        let verdict = |accepted: bool| if accepted { Accepts } else { Rejects };
        if let Some(scope) = s.scopes_by_key.get(&slot.scope_key) {
            return verdict(Self::membership_of_scope(scope, subject));
        }
        let Some(family) = slot.family.as_deref() else {
            return Unknown;
        };
        match family {
            "shorts" => verdict(subject.lists_in_riffs()),
            "feed" => {
                let Some(wanted) = slot.content_kind.as_deref() else {
                    return verdict(subject.lists_in_publications());
                };
                verdict(
                    subject.content_kind.map(|kind| kind.wire()) == Some(wanted) && subject.lists_in_publications(),
                )
            }
            "profile" => {
                let (Some(owner), Some(wanted)) = (slot.owner_user_id.as_deref(), slot.content_kind.as_deref()) else {
                    return Rejects;
                };
                if owner != subject.author_id || subject.content_kind.map(|kind| kind.wire()) != Some(wanted) {
                    return Rejects;
                }
                if slot.profile_kind.as_deref() == Some(RisePostProfileKind::Riffs.wire()) {
                    return verdict(subject.lists_in_riffs());
                }
                verdict(subject.lists_in_publications())
            }
            _ => Rejects,
        }
    }

    fn membership_of_scope(
        scope: &RisePostPageScope,
        subject: &RisePostContentRepositoryMembershipSubject,
    ) -> bool {
        let is_publication = subject.content_kind == Some(RisePostContentKind::Publication);
        let is_thread = subject.content_kind == Some(RisePostContentKind::Thread);
        match scope {
            RisePostPageScope::Shorts => subject.lists_in_riffs(),
            RisePostPageScope::Feed(RisePostFeedKind::All) => subject.lists_in_publications(),
            RisePostPageScope::Feed(RisePostFeedKind::Publications) => {
                is_publication && subject.lists_in_publications()
            }
            RisePostPageScope::Feed(RisePostFeedKind::Threads) => is_thread,
            RisePostPageScope::Profile { user_id, kind: RisePostProfileKind::Publications, .. } => {
                *user_id == subject.author_id && is_publication && subject.lists_in_publications()
            }
            RisePostPageScope::Profile { user_id, kind: RisePostProfileKind::Riffs, .. } => {
                *user_id == subject.author_id && subject.lists_in_riffs()
            }
            RisePostPageScope::Profile { user_id, kind: RisePostProfileKind::Threads, .. } => {
                *user_id == subject.author_id && is_thread
            }
            RisePostPageScope::Profile {
                kind: RisePostProfileKind::Reposts | RisePostProfileKind::Mentions | RisePostProfileKind::Favorites,
                ..
            }
            | RisePostPageScope::Search { .. }
            | RisePostPageScope::Detail { .. } => false,
        }
    }

    fn accepts(post: &GetPostFeedResponse, scope: &RisePostPageScope) -> bool {
        let kind = RisePostContentKind::from_wire(&post.content_kind);
        let is_publication = kind == Some(RisePostContentKind::Publication);
        let is_thread = kind == Some(RisePostContentKind::Thread);
        let subject = RisePostContentRepositoryMembershipSubject::from_post(post);
        match scope {
            RisePostPageScope::Shorts => subject.lists_in_riffs(),
            RisePostPageScope::Profile {
                kind: RisePostProfileKind::Reposts | RisePostProfileKind::Mentions | RisePostProfileKind::Favorites,
                ..
            } => true,
            RisePostPageScope::Feed(RisePostFeedKind::All) => subject.lists_in_publications(),
            RisePostPageScope::Feed(RisePostFeedKind::Publications) => {
                is_publication && subject.lists_in_publications()
            }
            RisePostPageScope::Feed(RisePostFeedKind::Threads) => is_thread,
            RisePostPageScope::Profile { kind: RisePostProfileKind::Publications, .. } => {
                is_publication && subject.lists_in_publications()
            }
            RisePostPageScope::Profile { kind: RisePostProfileKind::Riffs, .. } => subject.lists_in_riffs(),
            RisePostPageScope::Profile { kind: RisePostProfileKind::Threads, .. } => is_thread,
            RisePostPageScope::Search { kind: RisePostSearchKind::Publications, .. } => is_publication,
            RisePostPageScope::Search { kind: RisePostSearchKind::Threads, .. } => is_thread,
            RisePostPageScope::Detail { post_id } => post.id == *post_id,
        }
    }

    async fn load_scope_index_if_needed(&self) {
        let already_loaded = self.state.with(|s| std::mem::replace(&mut s.scope_index_loaded, true));
        if already_loaded {
            return;
        }
        let Ok(Some(stored)) = self.engine.value(RESOURCE_NAMESPACE.clone(), SCOPE_INDEX_KEY.to_owned()).await else {
            return;
        };
        let Ok(decoded) = serde_json::from_slice::<RisePostContentRepositoryScopeIndex>(&stored.data) else {
            return;
        };
        self.state.with(|s| {
            s.scope_index = decoded;
            s.store_version = s.store_version.max(stored.version);
        });
    }

    fn allocate_view(s: &mut RisePostContentRepositoryState, scope: &RisePostPageScope) -> String {
        let scope_key = scope.key();
        s.scope_index.access_clock = s.scope_index.access_clock.wrapping_add(1);
        let access_clock = s.scope_index.access_clock;
        let existing = s
            .scope_index
            .slots
            .iter()
            .find(|(_, slot)| slot.scope_key == scope_key)
            .map(|(view, _)| view.clone());
        if let Some(view) = existing {
            s.scope_index.slots.insert(view.clone(), Self::make_slot(scope, access_clock));
            s.scope_index_dirty = true;
            return view;
        }

        let view = if s.scope_index.slots.len() < Self::MAXIMUM_SCOPE_COUNT {
            (0..Self::MAXIMUM_SCOPE_COUNT)
                .map(|index| format!("content-scope-{index}"))
                .find(|candidate| !s.scope_index.slots.contains_key(candidate))
                .unwrap_or_else(|| "content-scope-0".to_owned())
        } else if let Some((evicted_view, evicted_scope_key)) = Self::eviction_candidate(s) {
            s.pages_by_scope_key.remove(&evicted_scope_key);
            s.evicted_views.insert(evicted_view.clone());
            evicted_view
        } else {
            "content-scope-0".to_owned()
        };
        s.scope_index.slots.insert(view.clone(), Self::make_slot(scope, access_clock));
        s.scope_index_dirty = true;
        view
    }

    fn eviction_candidate(s: &RisePostContentRepositoryState) -> Option<(String, String)> {
        let protected: HashSet<&String> = s
            .pending_create_memberships
            .values()
            .flat_map(|pending| pending.memberships_by_view.keys())
            .collect();
        let pick = |candidate: Option<(&String, &RisePostContentRepositoryScopeSlot)>| {
            candidate.map(|(view, slot)| (view.clone(), slot.scope_key.clone()))
        };
        pick(
            s.scope_index
                .slots
                .iter()
                .filter(|(view, _)| !protected.contains(view))
                .min_by_key(|(_, slot)| slot.access),
        )
        .or_else(|| pick(s.scope_index.slots.iter().min_by_key(|(_, slot)| slot.access)))
    }

    fn decode_cursor(data: Option<&[u8]>) -> Option<RisePostPageCursor> {
        serde_json::from_slice(data?).ok()
    }

    fn make_snapshot(&self, s: &mut RisePostContentRepositoryState) -> RisePostContentPresentationSnapshot {
        s.presentation_revision = (s.presentation_revision + 1).max(Self::now_milliseconds());
        RisePostContentPresentationSnapshot {
            account_id: self.account_id.clone(),
            account_epoch: self.account_epoch.clone(),
            session_id: self.session_id,
            revision: s.presentation_revision,
            pages_by_scope_key: s.pages_by_scope_key.clone(),
            collaborators_by_post_id: s
                .collaborators_by_post_id
                .iter()
                .map(|(post_id, entry)| {
                    (
                        post_id.clone(),
                        PostCollaboratorsSnapshot {
                            items: entry.items.clone(),
                            has_more: entry.has_more,
                            is_loading: entry.is_loading,
                            loaded_once: entry.loaded_once,
                        },
                    )
                })
                .collect(),
        }
    }

    fn advance_store_version(s: &mut RisePostContentRepositoryState) {
        s.store_version = (s.store_version + 1).max(Self::now_milliseconds());
    }

    fn record_mutation_version(s: &mut RisePostContentRepositoryState, post_id: &str) {
        s.entity_mutation_versions.insert(post_id.to_owned(), s.store_version);
        Self::trim_mutation_versions(s);
    }

    fn trim_mutation_versions(s: &mut RisePostContentRepositoryState) {
        let limit = 256;
        if s.entity_mutation_versions.len() <= limit {
            return;
        }
        let mut retained: Vec<(String, i64)> = s.entity_mutation_versions.drain().collect();
        retained.sort_by(|left, right| right.1.cmp(&left.1));
        retained.truncate(limit);
        s.entity_mutation_versions = retained.into_iter().collect();
        let versions = &s.entity_mutation_versions;
        s.deleted_post_versions.retain(|post_id, _| versions.contains_key(post_id));
    }

    fn next_mutation_generation(s: &mut RisePostContentRepositoryState) -> u64 {
        s.mutation_generation_clock = s.mutation_generation_clock.wrapping_add(1);
        s.mutation_generation_clock
    }

    async fn wait_for_repost_turn(&self, root_key: &str, generation: u64) {
        let waiter = self.state.with(|s| {
            let queue = s.repost_queues.get(root_key);
            if queue.and_then(|queue| queue.first()) == Some(&generation) {
                return None;
            }
            if !queue.is_some_and(|queue| queue.contains(&generation)) {
                return None;
            }
            let (sender, receiver) = oneshot::channel();
            s.repost_turn_waiters.entry(root_key.to_owned()).or_default().insert(generation, sender);
            Some(receiver)
        });
        if let Some(receiver) = waiter {
            let _ = receiver.await;
        }
    }

    fn complete_repost_turn(s: &mut RisePostContentRepositoryState, root_key: &str, generation: u64) {
        let Some(queue) = s.repost_queues.get_mut(root_key) else {
            return;
        };
        let Some(index) = queue.iter().position(|value| *value == generation) else {
            return;
        };
        let completed_head = index == 0;
        queue.remove(index);
        if queue.is_empty() {
            s.repost_queues.remove(root_key);
            s.repost_turn_waiters.remove(root_key);
            s.in_flight_repost_keys.remove(root_key);
            return;
        }
        let next = queue.first().copied();
        s.in_flight_repost_keys.insert(root_key.to_owned());
        let (true, Some(next)) = (completed_head, next) else {
            return;
        };
        let Some(waiters) = s.repost_turn_waiters.get_mut(root_key) else {
            return;
        };
        let Some(continuation) = waiters.remove(&next) else {
            return;
        };
        if waiters.is_empty() {
            s.repost_turn_waiters.remove(root_key);
        }
        let _ = continuation.send(());
    }

    fn trim_mutation_generation_state(s: &mut RisePostContentRepositoryState) {
        let limit = 512;
        while s.engagement_generations.len() > limit {
            let Some(key) = s
                .engagement_generations
                .iter()
                .filter(|(key, _)| !s.engagement_tasks.contains_key(*key))
                .min_by_key(|(_, value)| **value)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            s.engagement_generations.remove(&key);
        }
        while s.repost_generations.len() > limit {
            let Some(key) = s
                .repost_generations
                .iter()
                .filter(|(key, _)| !s.in_flight_repost_keys.contains(*key))
                .min_by_key(|(_, value)| **value)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            s.repost_generations.remove(&key);
            s.repost_desired_states.remove(&key);
        }
        while s.opinion_generations.len() > limit {
            let Some(key) = s
                .opinion_generations
                .iter()
                .filter(|(key, _)| !s.in_flight_opinion_keys.contains(*key))
                .min_by_key(|(_, value)| **value)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            s.opinion_generations.remove(&key);
        }
        while s.collaboration_generations.len() > limit {
            let Some(key) = s
                .collaboration_generations
                .iter()
                .filter(|(key, _)| !s.in_flight_collaboration_keys.contains(*key))
                .min_by_key(|(_, value)| **value)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            s.collaboration_generations.remove(&key);
        }
    }

    async fn is_current_account(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(Some(self.account_id.as_str()), Some(self.account_epoch.as_str()))
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}

fn x251_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X251", "{}", stage());
}
