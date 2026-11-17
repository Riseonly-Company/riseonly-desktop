use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_platform::process_activity::{ProcessActivityLease, begin_process_activity};
use rise_canon::{RiseEvent, RiseGuideSignal, rise_request};
use rise_canon::rise_key::stories as key;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_errors::ErrorCategory;
use rise_events::name;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_platform::uniform_type::UTType;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::desktop_platform::rise_app_quit_drain::{QuitDrain, QuitLease};
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_identity::{RiseIdentity, RiseIdentitySignatureValue};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_ws::sai_ws_chunked_file_upload::{SaiWsChunkedFileUpload, SaiWsUploadOnProgress};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_outbound_cancellation::SaiWsOutboundCancelledError;
use crate::core::lib::sai::sai::Sai;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::maintenance::maintenance_gate::MaintenanceGate;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::services::auth_token_refresh::AuthTokenRefresh;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::chat::engine::messages::rise_message_contracts::RiseMessageRepositoryError;
use crate::modules::story::stores::story::story_services::chats_header_status_hooks::ChatsHeaderStatus;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerEditedVideo, MediaPickerItem, MediaPickerItemKind, MediaPickerTrackSelection,
};
use crate::modules::media_picker::export::media_exporter::MediaExporter;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::post::stores::post_actions::post_types::AuthorInfoMore;
use crate::modules::share_extension::shared::rise_share_container::{RiseShareContainer, RiseShareContainerError};
use crate::modules::share_extension::shared::rise_share_models::RiseShareStoryPublishCheckpoint;
use crate::modules::story::components::story_music_media_renderer::StoryMusicMediaRenderer;
use crate::modules::story::shared::story_canvas_publish::StoryCanvasPublish;
use crate::modules::story::shared::story_track_plate_wire::StoryTrackPlateWire;
use crate::modules::user::shared::rise_blocked_content::RiseBlockedContent;
use crate::modules::story::components::story_player::story_media_view::{StoryImageAspectProbe, StoryMediaPrefetcher};
use crate::modules::story::shared::story_guide_sandbox_hooks::story_guide_sandbox_hooks;
use crate::modules::story::shared::story_publish_staging::{
    StoryPublishStaging, StoryUploadPhase, StoryUploadProgressCenter,
};
use crate::modules::story::stores::story::story_actions::story_actions::{
    StoryActionDoneHandler, StoryActionErrorHandler, StoryActionHighlightHandler, StoryActionResultHandler,
    StoryActionStoryHandler, StoryActionTextHandler, StoryActionUserStoriesHandler, StoryActionsStore,
};
use crate::modules::story::stores::story::story_actions::story_failure_text::StoryFailureText;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_actions::story_types::{
    StoryAudience, StoryPendingPublish, StoryPendingPublishPhase, StoryPlayerEntrance, StoryPlayerPresentationStyle,
    StoryPlayerRequest, StoryPlayerSharedElementOrigin,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::{
    GetHighlightStoriesGatewayDTO, GetStoriesFeedGatewayDTO, GetStoryViewersGatewayDTO, GetUserHighlightsGatewayDTO,
    GetUserStoriesGatewayDTO, HighlightDTO, HighlightScope, StoriesStateDTO, StoryAuthorDTO, StoryDTO,
    StoryDTOFromCanon, StoryGroupDTO, StoryTrackPlateDTO, StoryUploadWire, StoryViewerDTO,
};
use crate::modules::story::stores::story::story_services::story_share_handoff_models::{
    StoryShareHandoffPhase, StoryShareHandoffPresentationStage, StorySharePublishContext, StorySharePublishRequest,
};

pub type StoryCacheTransform = Rc<dyn Fn(Option<Map<String, Value>>) -> Option<Map<String, Value>>>;

#[derive(Clone)]
pub struct StoryMutationToken {
    id: Uuid,
    cache_keys: HashSet<String>,
}

#[derive(Clone)]
enum StoryMutationResolution {
    Pending,
    Committed(Option<StoryCacheTransform>),
    RolledBack,
}

#[derive(Clone)]
struct PendingStoryCacheMutation {
    id: Uuid,
    optimistic: StoryCacheTransform,
    resolution: StoryMutationResolution,
}

#[derive(Clone)]
enum StoryCachedRoot {
    Missing,
    Value(Map<String, Value>),
}

impl StoryCachedRoot {
    fn dictionary(&self) -> Option<Map<String, Value>> {
        match self {
            Self::Value(root) => Some(root.clone()),
            Self::Missing => None,
        }
    }

    fn from_dictionary(root: Option<Map<String, Value>>) -> Self {
        root.map_or(Self::Missing, Self::Value)
    }
}

#[derive(Clone, Default)]
struct StoryViewersProjection {
    viewers: Vec<StoryViewerDTO>,
    total: i64,
    like_count: i64,
    has_more: bool,
    next_cursor: Option<i64>,
    loaded: bool,
}

#[derive(Clone)]
struct PendingStoryPublish {
    idempotency_key: String,
    remote_file_url: Option<String>,
}

#[derive(Clone)]
struct SettledStoryPublishMedia {
    story_id: i64,
    staged_url: PathBuf,
    is_video: bool,
    remote_key: Option<String>,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

pub struct StoryServicesStore {
    pub is_publishing: bool,
    pub publish_error: Option<String>,
    pub pending_story_publish: Option<StoryPendingPublish>,
    pub publish_ring_is_settling: bool,

    publish_task: Option<Task<()>>,
    pending_publish_mutation: Option<StoryMutationToken>,
    pending_publish_signature: Option<String>,
    cancelled_publish_signature: Option<String>,
    pending_publish_staged_url: Option<PathBuf>,
    #[allow(dead_code)]
    pending_publish_lease: Option<(ProcessActivityLease, QuitLease)>,
    pending_publish_unwinds_on_player_dismiss: bool,
    settled_publish_media: Option<SettledStoryPublishMedia>,

    pub story_share_handoff_phase: StoryShareHandoffPhase,
    pub pending_story_share_session_ids: Vec<String>,
    pub story_share_handoff_task: Option<Task<()>>,
    pub(crate) story_share_handoff_exporter: Option<MediaExporter>,

    pending_story_publishes: HashMap<String, PendingStoryPublish>,

    is_publish_parked_on_network: bool,
    publish_transport_bounces: u32,

    active_story_user_ids: HashSet<String>,
    unseen_story_user_ids: HashSet<String>,
    known_story_state_user_ids: HashSet<String>,
    muted_story_user_ids: HashSet<String>,
    feed_story_user_ids: HashSet<String>,

    pub player_request: Option<StoryPlayerRequest>,
    pub is_loading_player: bool,
    active_highlight_id: Option<i64>,

    last_user_stories_target: Option<String>,
    last_highlights_target: Option<String>,
    pending_player_user_id: Option<String>,
    link_open_generation: u64,
    viewers_in_flight: HashSet<i64>,
    viewers_errored: HashSet<i64>,
    viewers_stale: HashSet<i64>,

    pending_highlight_open: Option<i64>,
    pending_highlight_origin: Option<StoryPlayerSharedElementOrigin>,
    like_overrides: HashMap<i64, bool>,
    like_count_deltas: HashMap<i64, i64>,
    locally_viewed_story_ids: HashSet<i64>,
    locally_seen_story_user_ids: HashSet<String>,
    gone_story_ids: HashSet<i64>,
    muted_story_groups_by_user_id: HashMap<String, Map<String, Value>>,

    story_mutation_base_roots: HashMap<String, StoryCachedRoot>,
    pending_story_cache_mutations: HashMap<String, Vec<PendingStoryCacheMutation>>,
    next_temporary_story_id: i64,
    next_temporary_highlight_id: i64,
}

impl StoryServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            is_publishing: false,
            publish_error: None,
            pending_story_publish: None,
            publish_ring_is_settling: false,
            publish_task: None,
            pending_publish_mutation: None,
            pending_publish_signature: None,
            cancelled_publish_signature: None,
            pending_publish_staged_url: None,
            pending_publish_lease: None,
            pending_publish_unwinds_on_player_dismiss: false,
            settled_publish_media: None,
            story_share_handoff_phase: StoryShareHandoffPhase::Idle,
            pending_story_share_session_ids: Vec::new(),
            story_share_handoff_task: None,
            story_share_handoff_exporter: None,
            pending_story_publishes: HashMap::new(),
            is_publish_parked_on_network: false,
            publish_transport_bounces: 0,
            active_story_user_ids: HashSet::new(),
            unseen_story_user_ids: HashSet::new(),
            known_story_state_user_ids: HashSet::new(),
            muted_story_user_ids: HashSet::new(),
            feed_story_user_ids: HashSet::new(),
            player_request: None,
            is_loading_player: false,
            active_highlight_id: None,
            last_user_stories_target: None,
            last_highlights_target: None,
            pending_player_user_id: None,
            link_open_generation: 0,
            viewers_in_flight: HashSet::new(),
            viewers_errored: HashSet::new(),
            viewers_stale: HashSet::new(),
            pending_highlight_open: None,
            pending_highlight_origin: None,
            like_overrides: HashMap::new(),
            like_count_deltas: HashMap::new(),
            locally_viewed_story_ids: HashSet::new(),
            locally_seen_story_user_ids: HashSet::new(),
            gone_story_ids: HashSet::new(),
            muted_story_groups_by_user_id: HashMap::new(),
            story_mutation_base_roots: HashMap::new(),
            pending_story_cache_mutations: HashMap::new(),
            next_temporary_story_id: -1,
            next_temporary_highlight_id: -1,
        }
    }

    fn with_actions(
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut StoryActionsStore, &mut Context<StoryActionsStore>) + 'static,
    ) {
        cx.defer(move |cx| {
            StoryActionsStore::shared(cx).update(cx, action);
        });
    }

    fn update_shared(cx: &mut App, update: impl FnOnce(&mut Self, &mut Context<Self>)) {
        Self::shared(cx).update(cx, update);
    }

    pub fn shared_story_handoff_presentation_stage(&self) -> StoryShareHandoffPresentationStage {
        self.story_share_handoff_phase.presentation_stage(self.is_publishing, self.publish_error.is_some())
    }

    pub fn is_shared_story_background_activity(&self) -> bool {
        self.shared_story_handoff_presentation_stage().shows_profile_avatar_activity()
    }

    pub fn is_publish_waiting_for_network(&self, cx: &mut App) -> bool {
        (self.is_publishing || self.is_shared_story_background_activity())
            && (RiseTransportStatus::is_waiting_for_network(cx) || self.is_publish_parked_on_network)
    }

    pub fn is_publish_parked_on_network(&self) -> bool {
        self.is_publish_parked_on_network
    }

    pub fn can_retry_shared_story_background_work(&self, _cx: &App) -> bool {
        self.shared_story_handoff_presentation_stage() == StoryShareHandoffPresentationStage::BackgroundFailure
    }

    pub fn active_story_user_ids(&self) -> &HashSet<String> {
        &self.active_story_user_ids
    }

    pub fn unseen_story_user_ids(&self) -> &HashSet<String> {
        &self.unseen_story_user_ids
    }

    pub fn known_story_state_user_ids(&self) -> &HashSet<String> {
        &self.known_story_state_user_ids
    }

    pub fn muted_story_user_ids(&self) -> &HashSet<String> {
        &self.muted_story_user_ids
    }

    pub fn active_highlight_id(&self) -> Option<i64> {
        self.active_highlight_id
    }

    pub fn viewers_in_flight(&self) -> &HashSet<i64> {
        &self.viewers_in_flight
    }

    pub fn viewers_errored(&self) -> &HashSet<i64> {
        &self.viewers_errored
    }

    pub fn gone_story_ids(&self) -> &HashSet<i64> {
        &self.gone_story_ids
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        let auth = AuthServicesStore::shared(cx);
        let preserves_cold_launch_queue = {
            let auth = auth.read(cx);
            !auth.is_authenticated()
                && auth.active_account_id().is_none()
                && self.story_share_handoff_phase.active_session_id().is_none()
        };
        if !preserves_cold_launch_queue {
            self.discard_shared_story_handoff(true, false, cx);
        }
        self.publish_task = None;
        self.is_publish_parked_on_network = false;
        self.publish_transport_bounces = 0;
        self.pending_publish_mutation = None;
        self.pending_publish_signature = None;
        self.cancelled_publish_signature = None;
        self.pending_publish_unwinds_on_player_dismiss = false;
        StoryPublishStaging::discard(self.pending_publish_staged_url.as_deref());
        self.pending_publish_staged_url = None;
        self.pending_publish_lease = None;
        StoryPublishStaging::discard(self.settled_publish_media.as_ref().map(|media| media.staged_url.as_path()));
        self.settled_publish_media = None;
        self.pending_story_publish = None;
        self.publish_ring_is_settling = false;
        StoryUploadProgressCenter::clear(cx);
        self.is_publishing = false;
        self.publish_error = None;
        self.pending_story_publishes.clear();
        self.active_story_user_ids.clear();
        self.unseen_story_user_ids.clear();
        self.known_story_state_user_ids.clear();
        self.muted_story_user_ids.clear();
        self.feed_story_user_ids.clear();
        self.player_request = None;
        self.is_loading_player = false;
        self.active_highlight_id = None;
        self.last_user_stories_target = None;
        self.last_highlights_target = None;
        self.pending_player_user_id = None;
        self.link_open_generation = self.link_open_generation.wrapping_add(1);
        self.viewers_in_flight.clear();
        self.viewers_errored.clear();
        self.viewers_stale.clear();
        self.pending_highlight_open = None;
        self.pending_highlight_origin = None;
        self.like_overrides.clear();
        self.like_count_deltas.clear();
        self.locally_viewed_story_ids.clear();
        self.gone_story_ids.clear();
        self.locally_seen_story_user_ids.clear();
        self.muted_story_groups_by_user_id.clear();
        self.story_mutation_base_roots.clear();
        self.pending_story_cache_mutations.clear();
        self.next_temporary_story_id = -1;
        self.next_temporary_highlight_id = -1;
        cx.notify();
    }

    pub fn present_player(&mut self, request: StoryPlayerRequest, highlight_id: Option<i64>, cx: &mut Context<Self>) {
        self.is_loading_player = false;
        self.active_highlight_id = highlight_id;
        self.player_request = Some(request);
        cx.notify();
    }

    pub fn dismiss_player(&mut self, cx: &mut Context<Self>) {
        self.player_request = None;
        self.is_loading_player = false;
        self.pending_player_user_id = None;
        self.link_open_generation = self.link_open_generation.wrapping_add(1);
        self.pending_highlight_open = None;
        self.pending_highlight_origin = None;
        self.active_highlight_id = None;
        self.unwind_cancelled_story_publish_if_needed(cx);
        self.release_settled_publish_media(cx);
        cx.notify();
    }

    pub fn open_stories_for_user(
        &mut self,
        user_id: &str,
        on_unavailable: Option<StoryActionDoneHandler>,
        cx: &mut Context<Self>,
    ) {
        let on_unavailable: StoryActionDoneHandler = on_unavailable.unwrap_or_else(|| Rc::new(|_: &mut App| {}));
        let id = trimming_whitespaces_and_newlines(user_id).to_owned();
        if id.is_empty() || self.is_loading_player {
            return;
        }
        let has_wire_user = AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .is_some_and(|user_id| !user_id.is_empty());
        if !has_wire_user {
            on_unavailable(cx);
            return;
        }

        self.note_user_stories_target(&id, cx);

        let user_stories = self.user_stories(&id, cx);
        let cached = self.live_stories(user_stories, cx);
        if !cached.is_empty() {
            let group = self.group_for_stories(&id, cached);
            self.present_player(StoryPlayerRequest::new(vec![group], 0), None, cx);
            let on_success: StoryActionUserStoriesHandler =
                Rc::new(move |response: GetUserStoriesGatewayDTO, cx: &mut App| {
                    if response.list.is_empty() {
                        on_unavailable(cx);
                    }
                });
            Self::with_actions(cx, move |actions, cx| {
                actions.get_user_stories_action(Some(id), None, Some(on_success), None, cx);
            });
            return;
        }

        self.pending_player_user_id = Some(id.clone());
        self.is_loading_player = true;
        cx.notify();
        let success_id = id.clone();
        let success_unavailable = Rc::clone(&on_unavailable);
        let on_success: StoryActionUserStoriesHandler = Rc::new(move |response: GetUserStoriesGatewayDTO, cx: &mut App| {
            let id = success_id.clone();
            let on_unavailable = Rc::clone(&success_unavailable);
            Self::update_shared(cx, move |this, cx| {
                if this.pending_player_user_id.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.pending_player_user_id = None;
                let stories = this.live_stories(response.list, cx);
                this.is_loading_player = false;
                cx.notify();
                if stories.is_empty() {
                    this.clear_stories_state(&id);
                    cx.defer(move |cx| on_unavailable(cx));
                    return;
                }
                let group = this.group_for_stories(&id, stories);
                this.present_player(StoryPlayerRequest::new(vec![group], 0), None, cx);
            });
        });
        let error_id = id.clone();
        let on_error: StoryActionErrorHandler = Rc::new(move |_: &anyhow::Error, cx: &mut App| {
            let id = error_id.clone();
            let on_unavailable = Rc::clone(&on_unavailable);
            Self::update_shared(cx, move |this, cx| {
                if this.pending_player_user_id.as_deref() != Some(id.as_str()) {
                    return;
                }
                this.pending_player_user_id = None;
                this.is_loading_player = false;
                cx.notify();
                cx.defer(move |cx| on_unavailable(cx));
            });
        });
        Self::with_actions(cx, move |actions, cx| {
            actions.get_user_stories_action(Some(id), None, Some(on_success), Some(on_error), cx);
        });
    }

    pub fn open_stories_feed(&mut self, groups: Vec<StoryGroupDTO>, start_group_index: usize, cx: &mut Context<Self>) {
        if self.player_request.is_some() {
            return;
        }
        let now_ms = Self::server_now_ms(cx);
        let playable_groups: Vec<StoryGroupDTO> =
            groups.iter().filter_map(|group| self.playable_feed_group(group, now_ms, cx)).collect();
        let requested_author_id = groups.get(start_group_index).map(Self::story_group_author_id);
        let requested_index = requested_author_id.as_ref().and_then(|author_id| {
            playable_groups.iter().position(|group| Self::story_group_author_id(group) == *author_id)
        });
        if playable_groups.is_empty() || (requested_author_id.is_some() && requested_index.is_none()) {
            let is_pending = StoryActionsStore::shared(cx).read(cx).get_stories_feed_sai.is_pending();
            if !is_pending {
                Self::with_actions(cx, |actions, cx| actions.get_stories_feed_action(None, cx));
            }
            return;
        }

        self.apply_stories_feed(playable_groups.clone(), false, false, cx);
        let resolved_index = requested_index.unwrap_or(0);
        self.present_player(
            StoryPlayerRequest::new(playable_groups, resolved_index)
                .with_presentation_style(StoryPlayerPresentationStyle::FeedCube),
            None,
            cx,
        );
    }

    fn playable_feed_group(&self, group: &StoryGroupDTO, now_ms: i64, cx: &App) -> Option<StoryGroupDTO> {
        let author_id = Self::story_group_author_id(group);
        if author_id.is_empty() || RiseBlockedContent::hides(Some(author_id.as_str()), cx) {
            return None;
        }
        let live = Self::live_stories_at(&group.stories, now_ms);
        if live.is_empty() {
            return None;
        }
        if live.len() == group.stories.len() {
            return Some(group.clone());
        }
        let unseen = live.iter().filter(|story| !self.viewed(story)).count();
        Some(StoryGroupDTO { all_seen: unseen == 0, author: group.author.clone(), stories: live, unseen_count: unseen as i64 })
    }

    pub fn append_feed_page_to_open_player(&mut self, page: &[StoryGroupDTO], cx: &mut Context<Self>) {
        let Some(request) = self.player_request.clone() else {
            return;
        };
        if request.presentation_style != StoryPlayerPresentationStyle::FeedCube || request.owns_explicit_stories {
            return;
        }
        let now_ms = Self::server_now_ms(cx);
        let mut known: HashSet<String> = request.groups.iter().map(Self::story_group_author_id).collect();
        let mut groups = request.groups.clone();
        for group in page {
            let Some(playable) = self.playable_feed_group(group, now_ms, cx) else {
                continue;
            };
            if known.insert(Self::story_group_author_id(&playable)) {
                groups.push(playable);
            }
        }
        if groups.len() == request.groups.len() {
            return;
        }
        self.player_request = Some(Self::rebuilt_request(&request, groups, request.start_group_index));
        cx.notify();
    }

    fn clear_stories_state(&mut self, user_id: &str) {
        self.known_story_state_user_ids.insert(user_id.to_owned());
        self.active_story_user_ids.remove(user_id);
        self.unseen_story_user_ids.remove(user_id);
        self.locally_seen_story_user_ids.remove(user_id);
    }

    fn server_now_ms(cx: &mut App) -> i64 {
        MaintenanceGate::shared(cx).read(cx).server_now().ms()
    }

    fn live_stories_at(stories: &[StoryDTO], now_ms: i64) -> Vec<StoryDTO> {
        stories
            .iter()
            .filter(|story| story.expires_at_ms <= 0 || story.expires_at_ms > now_ms)
            .cloned()
            .collect()
    }

    fn live_stories(&self, stories: Vec<StoryDTO>, cx: &mut App) -> Vec<StoryDTO> {
        let now_ms = Self::server_now_ms(cx);
        Self::live_stories_at(&stories, now_ms)
    }

    fn group_for_stories(&self, _user_id: &str, mut stories: Vec<StoryDTO>) -> StoryGroupDTO {
        stories.sort_by_key(|story| story.created_at_ms);
        let author = stories.first().and_then(|story| story.author.clone());
        let unseen = stories.iter().filter(|story| !self.viewed(story)).count();
        StoryGroupDTO { all_seen: unseen == 0, author, stories, unseen_count: unseen as i64 }
    }

    fn story_group_author_id(group: &StoryGroupDTO) -> String {
        let stable = group
            .stories
            .first()
            .map(|story| trimming_whitespaces_and_newlines(&story.author_id).to_owned())
            .unwrap_or_default();
        let nested = group
            .author
            .as_ref()
            .map(|author| trimming_whitespaces_and_newlines(&author.id).to_owned())
            .unwrap_or_default();
        if stable.is_empty() { nested } else { stable }
    }

    pub fn story_id(from: Option<&Value>) -> i64 {
        match from {
            Some(Value::Number(number)) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value as i64))
                .unwrap_or(0),
            Some(Value::String(text)) => text.parse::<i64>().unwrap_or(0),
            _ => 0,
        }
    }

    pub fn is_story_liked(&self, story: &StoryDTO, _cx: &App) -> bool {
        self.like_overrides.get(&story.id).copied().unwrap_or(story.is_liked)
    }

    fn viewed(&self, story: &StoryDTO) -> bool {
        story.has_viewed == Some(true) || self.locally_viewed_story_ids.contains(&story.id)
    }

    pub fn is_story_viewed(&self, story: &StoryDTO, _cx: &App) -> bool {
        self.viewed(story)
    }

    pub fn story_like_count(&self, story: &StoryDTO, _cx: &App) -> i64 {
        let base = story.like_count.max(0);
        (base + self.like_count_deltas.get(&story.id).copied().unwrap_or(0)).max(0)
    }

    pub fn apply_like_optimistic(&mut self, story_id: i64, liked: bool, cx: &mut Context<Self>) {
        if self.like_overrides.get(&story_id).copied() == Some(liked) {
            return;
        }
        self.like_overrides.insert(story_id, liked);
        *self.like_count_deltas.entry(story_id).or_insert(0) += if liked { 1 } else { -1 };
        cx.notify();
    }

    pub fn reconcile_like(&mut self, story_id: i64, like_count: Option<i64>, is_liked: bool, cx: &mut Context<Self>) {
        self.like_overrides.insert(story_id, is_liked);
        if let Some(like_count) = like_count {
            self.like_count_deltas.remove(&story_id);
            self.patch_story_like_cache(story_id, like_count, is_liked, cx);
        }
        cx.notify();
    }

    fn patch_story_like_cache(&self, story_id: i64, like_count: i64, is_liked: bool, cx: &mut Context<Self>) {
        let Some(target) = self.last_user_stories_target.as_deref() else {
            return;
        };
        let cache_key = Self::user_stories_cache_key(target);
        let mut patch = Map::new();
        patch.insert(key::LIKE_COUNT.to_owned(), Value::from(like_count));
        patch.insert(key::IS_LIKED.to_owned(), Value::Bool(is_liked));
        let _ = SaiUpdater::patch_row(
            &cache_key,
            key::LIST,
            key::ID,
            &story_id.to_string(),
            &patch,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
    }

    fn trimmed_user_id(user_id: Option<&str>) -> Option<String> {
        user_id.map(trimming_whitespaces_and_newlines).filter(|id| !id.is_empty()).map(str::to_owned)
    }

    pub fn has_active_stories(&self, user_id: Option<&str>, cx: &App) -> bool {
        Self::trimmed_user_id(user_id)
            .is_some_and(|id| self.active_story_user_ids.contains(&id) && !RiseBlockedContent::hides(Some(id.as_str()), cx))
    }

    pub fn has_unseen_stories(&self, user_id: Option<&str>, _cx: &App) -> bool {
        Self::trimmed_user_id(user_id).is_some_and(|id| self.unseen_story_user_ids.contains(&id))
    }

    pub fn is_stories_muted(&self, user_id: Option<&str>, _cx: &App) -> bool {
        Self::trimmed_user_id(user_id).is_some_and(|id| self.muted_story_user_ids.contains(&id))
    }

    pub fn has_known_stories_state(&self, user_id: Option<&str>, _cx: &App) -> bool {
        Self::trimmed_user_id(user_id).is_some_and(|id| self.known_story_state_user_ids.contains(&id))
    }

    pub fn apply_stories_state(
        &mut self,
        states: Vec<StoriesStateDTO>,
        requested_user_ids: &[String],
        cx: &mut Context<Self>,
    ) {
        let requested: HashSet<String> = requested_user_ids
            .iter()
            .map(|id| trimming_whitespaces_and_newlines(id).to_owned())
            .filter(|id| !id.is_empty())
            .collect();
        let returned: HashSet<String> = states
            .iter()
            .map(|state| trimming_whitespaces_and_newlines(&state.user_id).to_owned())
            .filter(|id| !id.is_empty())
            .collect();
        let touched: Vec<String> = requested.union(&returned).cloned().collect();
        let drawn_before: Vec<(bool, bool, bool)> = touched.iter().map(|id| self.drawn_story_state(id)).collect();
        for id in requested.difference(&returned) {
            self.clear_stories_state(id);
        }
        for state in states {
            let id = trimming_whitespaces_and_newlines(&state.user_id).to_owned();
            if id.is_empty() {
                continue;
            }
            self.known_story_state_user_ids.insert(id.clone());
            if state.is_muted {
                self.muted_story_user_ids.insert(id.clone());
            } else {
                self.muted_story_user_ids.remove(&id);
            }
            if state.has_active_stories {
                self.active_story_user_ids.insert(id.clone());
                if state.has_unseen && !self.locally_seen_story_user_ids.contains(&id) {
                    self.unseen_story_user_ids.insert(id);
                } else {
                    self.unseen_story_user_ids.remove(&id);
                }
            } else {
                self.active_story_user_ids.remove(&id);
                self.unseen_story_user_ids.remove(&id);
                self.locally_seen_story_user_ids.remove(&id);
            }
        }
        if touched.iter().zip(&drawn_before).any(|(id, before)| self.drawn_story_state(id) != *before) {
            cx.notify();
        }
    }

    fn drawn_story_state(&self, user_id: &str) -> (bool, bool, bool) {
        (
            self.active_story_user_ids.contains(user_id),
            self.unseen_story_user_ids.contains(user_id),
            self.muted_story_user_ids.contains(user_id),
        )
    }

    pub fn apply_stories_feed(
        &mut self,
        groups: Vec<StoryGroupDTO>,
        replacing: bool,
        has_more: bool,
        cx: &mut Context<Self>,
    ) -> Vec<String> {
        let incoming_ids: HashSet<String> = groups
            .iter()
            .filter_map(|group| {
                let id = Self::story_group_author_id(group);
                (!id.is_empty() && !group.stories.is_empty()).then_some(id)
            })
            .collect();
        let mut unsettled = Vec::new();
        if replacing {
            let publishing_author_id = self.pending_story_publish.as_ref().map(|pending| pending.author_id.clone());
            let own_user_id = AuthServicesStore::shared(cx)
                .read(cx)
                .effective_user_id_for_wire()
                .map(|id| trimming_whitespaces_and_newlines(&id).to_owned());
            let dropped: Vec<String> = self.feed_story_user_ids.difference(&incoming_ids).cloned().collect();
            for id in dropped {
                if Some(&id) == publishing_author_id.as_ref() || Some(&id) == own_user_id.as_ref() {
                    continue;
                }
                if has_more {
                    unsettled.push(id);
                } else {
                    self.clear_stories_state(&id);
                }
            }
            let mut feed_ids = incoming_ids;
            if let Some(publishing_author_id) = publishing_author_id {
                feed_ids.insert(publishing_author_id);
            }
            self.feed_story_user_ids = feed_ids;
        } else {
            self.feed_story_user_ids.extend(incoming_ids);
        }
        for group in &groups {
            let id = Self::story_group_author_id(group);
            if id.is_empty() || group.stories.is_empty() {
                continue;
            }
            self.known_story_state_user_ids.insert(id.clone());
            self.active_story_user_ids.insert(id.clone());
            if group.stories.iter().any(|story| !self.viewed(story)) {
                self.locally_seen_story_user_ids.remove(&id);
                self.unseen_story_user_ids.insert(id);
            } else {
                self.unseen_story_user_ids.remove(&id);
            }
        }
        cx.notify();
        unsettled
    }

    pub fn mark_story_viewed_locally(
        &mut self,
        story_id: i64,
        author_id: &str,
        stories: &[StoryDTO],
        cx: &mut Context<Self>,
    ) {
        if story_id <= 0 {
            return;
        }

        self.locally_viewed_story_ids.insert(story_id);

        let id = trimming_whitespaces_and_newlines(author_id).to_owned();
        if !id.is_empty() {
            let user_stories_key = Self::user_stories_cache_key(&id);
            let mut patch = Map::new();
            patch.insert(key::HAS_VIEWED.to_owned(), Value::Bool(true));
            let _ = SaiUpdater::patch_row(
                &user_stories_key,
                key::LIST,
                key::ID,
                &story_id.to_string(),
                &patch,
                SaiUpdaterWriteScope::Cache,
                cx,
            );
        }

        let feed_key = Self::feed_cache_key();
        let feed_observed = SaiWsResponseCache::observed_payload(
            &feed_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
        .is_some();
        if feed_observed {
            let locally_viewed = self.locally_viewed_story_ids.clone();
            let author = id.clone();
            let _ = SaiUpdater::update_json_value(
                &feed_key,
                move |context| {
                    let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                    let mut tray = AnyDictionaryBridge::array_of_dictionaries(root.get(key::TRAY));
                    for group in &mut tray {
                        let group_author = AnyDictionaryBridge::dictionary(group.get(key::AUTHOR))
                            .and_then(|author| author.get(key::ID).and_then(Value::as_str).map(str::to_owned));
                        let mut rows = AnyDictionaryBridge::array_of_dictionaries(group.get(key::STORIES));
                        let contains_story = rows.iter().any(|row| Self::story_id(row.get(key::ID)) == story_id);
                        if !(contains_story || (!author.is_empty() && group_author.as_deref() == Some(author.as_str()))) {
                            continue;
                        }
                        for row in &mut rows {
                            if Self::story_id(row.get(key::ID)) == story_id {
                                row.insert(key::HAS_VIEWED.to_owned(), Value::Bool(true));
                            }
                        }
                        let unseen = rows
                            .iter()
                            .filter(|row| {
                                let row_id = Self::story_id(row.get(key::ID));
                                row.get(key::HAS_VIEWED).and_then(Value::as_bool) != Some(true)
                                    && !locally_viewed.contains(&row_id)
                            })
                            .count();
                        group.insert(key::STORIES.to_owned(), Self::rows_value(rows));
                        group.insert(key::UNSEEN_COUNT.to_owned(), Value::from(unseen));
                        group.insert(key::ALL_SEEN.to_owned(), Value::Bool(unseen == 0));
                    }
                    root.insert(key::TRAY.to_owned(), Self::rows_value(tray));
                    Ok(Value::Object(root))
                },
                SaiUpdaterWriteScope::Cache,
                cx,
            );
        }

        if !id.is_empty() && !stories.is_empty() && stories.iter().all(|story| self.viewed(story)) {
            self.mark_user_stories_seen_locally(&id, cx);
        }
        cx.notify();
    }

    pub fn mark_user_stories_seen_locally(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let id = trimming_whitespaces_and_newlines(user_id).to_owned();
        if id.is_empty() {
            return;
        }
        self.unseen_story_user_ids.remove(&id);
        self.locally_seen_story_user_ids.insert(id);
        cx.notify();
    }

    pub fn begin_story_mutation(
        &mut self,
        transforms: HashMap<String, StoryCacheTransform>,
        cx: &mut Context<Self>,
    ) -> StoryMutationToken {
        let id = Uuid::new_v4();
        let cache_keys: HashSet<String> = transforms.keys().cloned().collect();
        for (cache_key, transform) in transforms {
            if self.pending_story_cache_mutations.get(&cache_key).is_none_or(Vec::is_empty) {
                let root = self.cached_story_root(&cache_key, cx);
                self.story_mutation_base_roots.insert(cache_key.clone(), root);
            }
            self.pending_story_cache_mutations.entry(cache_key.clone()).or_default().push(PendingStoryCacheMutation {
                id,
                optimistic: transform,
                resolution: StoryMutationResolution::Pending,
            });
            self.render_story_mutation_cache(&cache_key, cx);
        }
        StoryMutationToken { id, cache_keys }
    }

    pub fn commit_story_mutation(
        &mut self,
        token: &StoryMutationToken,
        authoritative_transforms: HashMap<String, StoryCacheTransform>,
        cx: &mut Context<Self>,
    ) {
        self.resolve_story_mutation(
            token,
            |cache_key| StoryMutationResolution::Committed(authoritative_transforms.get(cache_key).cloned()),
            cx,
        );
    }

    pub fn rollback_story_mutation(&mut self, token: &StoryMutationToken, cx: &mut Context<Self>) {
        self.resolve_story_mutation(token, |_| StoryMutationResolution::RolledBack, cx);
    }

    fn resolve_story_mutation(
        &mut self,
        token: &StoryMutationToken,
        resolution: impl Fn(&str) -> StoryMutationResolution,
        cx: &mut Context<Self>,
    ) {
        for cache_key in &token.cache_keys {
            let Some(mut mutations) = self.pending_story_cache_mutations.get(cache_key).cloned() else {
                continue;
            };
            let Some(index) = mutations.iter().position(|mutation| mutation.id == token.id) else {
                continue;
            };
            mutations[index].resolution = resolution(cache_key);

            let mut base = self.story_mutation_base_roots.get(cache_key).and_then(StoryCachedRoot::dictionary);
            while let Some(first) = mutations.first() {
                match &first.resolution {
                    StoryMutationResolution::Pending => break,
                    StoryMutationResolution::Committed(authoritative) => {
                        let transform = authoritative.clone().unwrap_or_else(|| Rc::clone(&first.optimistic));
                        base = transform(base);
                        mutations.remove(0);
                    }
                    StoryMutationResolution::RolledBack => {
                        mutations.remove(0);
                    }
                }
            }

            if mutations.is_empty() {
                self.write_story_root(base, cache_key, cx);
                self.pending_story_cache_mutations.remove(cache_key);
                self.story_mutation_base_roots.remove(cache_key);
            } else {
                self.story_mutation_base_roots.insert(cache_key.clone(), StoryCachedRoot::from_dictionary(base));
                self.pending_story_cache_mutations.insert(cache_key.clone(), mutations);
                self.render_story_mutation_cache(cache_key, cx);
            }
        }
    }

    pub fn rebase_pending_story_publish(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_story_publish.as_ref() else {
            return;
        };
        if self.pending_publish_mutation.is_none()
            || self.pending_story_cache_mutations.get(cache_key).is_none_or(Vec::is_empty)
        {
            return;
        }

        let strip = if cache_key == Self::feed_cache_key() {
            Self::remove_feed_story_transform(pending.story_id)
        } else {
            Self::remove_story_transform(pending.story_id)
        };
        let fresh = self.cached_story_root(cache_key, cx).dictionary();
        self.story_mutation_base_roots.insert(cache_key.to_owned(), StoryCachedRoot::from_dictionary(strip(fresh)));
        self.render_story_mutation_cache(cache_key, cx);
    }

    pub fn did_load_authoritative_stories(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        self.rebase_pending_story_publish(&Self::user_stories_cache_key(target_user_id), cx);
        self.refresh_open_player_stories_from(target_user_id, true, cx);
    }

    fn render_story_mutation_cache(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        let mut root = self.story_mutation_base_roots.get(cache_key).and_then(StoryCachedRoot::dictionary);
        for mutation in self.pending_story_cache_mutations.get(cache_key).map(Vec::as_slice).unwrap_or_default() {
            match &mutation.resolution {
                StoryMutationResolution::RolledBack => continue,
                StoryMutationResolution::Pending => root = (mutation.optimistic)(root),
                StoryMutationResolution::Committed(authoritative) => {
                    let transform = authoritative.as_ref().unwrap_or(&mutation.optimistic);
                    root = transform(root);
                }
            }
        }
        self.write_story_root(root, cache_key, cx);
    }

    fn cached_story_root(&self, cache_key: &str, cx: &mut App) -> StoryCachedRoot {
        let payload = SaiWsResponseCache::current_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::MemoryOnly,
            cx,
        );
        StoryCachedRoot::from_dictionary(AnyDictionaryBridge::dictionary(payload.as_ref()))
    }

    fn write_story_root(&self, root: Option<Map<String, Value>>, cache_key: &str, cx: &mut App) {
        let Some(root) = root else {
            SaiWsResponseCache::remove(cache_key, SaiCacheCleanLayer::Memory, cx);
            return;
        };
        let _ = SaiUpdater::replace_root(cache_key, root, SaiUpdaterWriteScope::Cache, cx);
    }

    fn memory_payload_exists(cache_key: &str, cx: &mut App) -> bool {
        SaiWsResponseCache::current_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::MemoryOnly,
            cx,
        )
        .is_some()
    }

    pub fn mute_user_stories(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(id) = Self::trimmed_user_id(user_id) else {
            return;
        };
        let feed_key = Self::feed_cache_key();
        let feed = SaiWsResponseCache::current_payload(
            &feed_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::MemoryOnly,
            cx,
        );
        let tray = AnyDictionaryBridge::dictionary(feed.as_ref())
            .map(|root| AnyDictionaryBridge::array_of_dictionaries(root.get(key::TRAY)))
            .unwrap_or_default();
        if let Some(group) = tray.into_iter().find(|group| Self::raw_story_group_author_id(group) == id) {
            self.muted_story_groups_by_user_id.insert(id.clone(), group);
        }

        let player_snapshot = self.player_request.clone();
        let highlight_snapshot = self.active_highlight_id;
        let token = self.begin_story_mutation(HashMap::from([(feed_key, Self::remove_feed_author_transform(&id))]), cx);
        let was_muted = self.muted_story_user_ids.contains(&id);
        self.active_story_user_ids.remove(&id);
        self.unseen_story_user_ids.remove(&id);
        self.feed_story_user_ids.remove(&id);
        self.muted_story_user_ids.insert(id.clone());
        self.dismiss_player(cx);
        let target = id.clone();
        let on_result: StoryActionResultHandler = Rc::new(move |succeeded: bool, cx: &mut App| {
            let id = id.clone();
            let token = token.clone();
            let player_snapshot = player_snapshot.clone();
            Self::update_shared(cx, move |this, cx| {
                if succeeded {
                    this.commit_story_mutation(&token, HashMap::new(), cx);
                    return;
                }
                if !was_muted {
                    this.muted_story_user_ids.remove(&id);
                }
                this.rollback_story_mutation(&token, cx);
                this.reconcile_story_state_from_caches(&id, cx);
                if this.player_request.is_none()
                    && let Some(player_snapshot) = player_snapshot
                {
                    this.present_player(player_snapshot, highlight_snapshot, cx);
                }
                cx.notify();
            });
        });
        Self::with_actions(cx, move |actions, cx| actions.mute_user_stories_action(&target, on_result, cx));
        cx.notify();
    }

    pub fn unmute_user_stories(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(id) = Self::trimmed_user_id(user_id) else {
            return;
        };
        let feed_key = Self::feed_cache_key();
        let transforms: HashMap<String, StoryCacheTransform> = match self.muted_story_groups_by_user_id.get(&id) {
            Some(group) => HashMap::from([(feed_key, Self::upsert_feed_group_transform(group.clone()))]),
            None => HashMap::new(),
        };
        let token = self.begin_story_mutation(transforms, cx);
        let was_muted = self.muted_story_user_ids.contains(&id);
        self.muted_story_user_ids.remove(&id);
        self.reconcile_story_state_from_caches(&id, cx);
        let target = id.clone();
        let on_result: StoryActionResultHandler = Rc::new(move |succeeded: bool, cx: &mut App| {
            let id = id.clone();
            let token = token.clone();
            Self::update_shared(cx, move |this, cx| {
                if succeeded {
                    this.commit_story_mutation(&token, HashMap::new(), cx);
                    this.muted_story_groups_by_user_id.remove(&id);
                    let user_ids = vec![id];
                    Self::with_actions(cx, move |actions, cx| actions.get_stories_state_action(&user_ids, cx));
                } else {
                    if was_muted {
                        this.muted_story_user_ids.insert(id.clone());
                    }
                    this.rollback_story_mutation(&token, cx);
                    this.reconcile_story_state_from_caches(&id, cx);
                }
                cx.notify();
            });
        });
        Self::with_actions(cx, move |actions, cx| actions.unmute_user_stories_action(&target, on_result, cx));
        cx.notify();
    }

    pub fn open_story(&mut self, story_id: i64, cx: &mut Context<Self>) {
        if story_id <= 0 {
            return;
        }

        let generation = self.begin_link_open(cx);
        let on_success: StoryActionStoryHandler = Rc::new(move |story: StoryDTO, cx: &mut App| {
            Self::update_shared(cx, move |this, cx| {
                if generation != this.link_open_generation {
                    return;
                }
                this.is_loading_player = false;
                let owner = story.author_id.clone();
                this.note_user_stories_target(&owner, cx);
                let group = this.group_for_stories(&owner, vec![story]);
                this.present_player(
                    StoryPlayerRequest::new(vec![group], 0).with_owns_explicit_stories(true),
                    None,
                    cx,
                );
            });
        });
        let on_error: StoryActionTextHandler = Rc::new(move |_: String, cx: &mut App| {
            let current = Self::update_shared_returning(cx, |this, cx| {
                if generation != this.link_open_generation {
                    return false;
                }
                this.is_loading_player = false;
                cx.notify();
                true
            });
            if current {
                Notifier::send(
                    tr("chat_story_reply_expired"),
                    NotifierSendOptions { r#type: NotifierType::Error, ..NotifierSendOptions::default() },
                    cx,
                );
            }
        });
        Self::with_actions(cx, move |actions, cx| actions.get_story_action(story_id, on_success, on_error, cx));
    }

    fn update_shared_returning<R>(cx: &mut App, update: impl FnOnce(&mut Self, &mut Context<Self>) -> R) -> R {
        Self::shared(cx).update(cx, update)
    }

    fn begin_link_open(&mut self, cx: &mut Context<Self>) -> u64 {
        self.link_open_generation = self.link_open_generation.wrapping_add(1);
        self.pending_player_user_id = None;
        self.pending_highlight_open = None;
        self.pending_highlight_origin = None;
        self.is_loading_player = true;
        cx.notify();
        self.link_open_generation
    }

    pub fn story_created_ring_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::StoryCreated(event)) = RiseEvent::decode_payload(name::STORY_CREATED, payload) else {
            return;
        };
        let wire_author_id = if event.author_id.is_empty() { event.author.id.clone() } else { event.author_id.clone() };
        let author_id = trimming_whitespaces_and_newlines(&wire_author_id).to_owned();
        if author_id.is_empty() {
            return;
        }

        self.known_story_state_user_ids.insert(author_id.clone());
        self.active_story_user_ids.insert(author_id.clone());
        self.locally_seen_story_user_ids.remove(&author_id);
        self.unseen_story_user_ids.insert(author_id.clone());

        if event.story_id > 0 {
            let story = StoryDTO::from_canon(&event);
            let mut transforms: HashMap<String, StoryCacheTransform> =
                HashMap::from([(Self::feed_cache_key(), Self::upsert_feed_story_transform(story.clone(), None))]);
            if self.profile_stories_loaded(&author_id, StoryProfileScope::Profile, cx) {
                transforms.insert(
                    Self::profile_stories_cache_key(&author_id, StoryProfileScope::Profile),
                    Self::upsert_story_transform(story.clone(), None),
                );
            }
            if self.user_stories_loaded(&author_id, cx) {
                transforms.insert(Self::user_stories_cache_key(&author_id), Self::upsert_story_transform(story, None));
            }
            let mutation = self.begin_story_mutation(transforms, cx);
            self.commit_story_mutation(&mutation, HashMap::new(), cx);
            self.reconcile_story_state_from_caches(&author_id, cx);
        }
        cx.notify();
    }

    pub fn story_deleted_ring_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::StoryDeleted(event)) = RiseEvent::decode_payload(name::STORY_DELETED, payload) else {
            return;
        };

        let author_id = trimming_whitespaces_and_newlines(&event.author_id).to_owned();
        if author_id.is_empty() {
            return;
        }
        let story_id = event.story_id;
        if story_id <= 0 {
            let user_ids = vec![author_id];
            Self::with_actions(cx, move |actions, cx| actions.get_stories_state_action(&user_ids, cx));
            return;
        }

        let transforms = self.delete_story_mutation_transforms(story_id, &author_id, cx);
        let mutation = self.begin_story_mutation(transforms, cx);
        self.commit_story_mutation(&mutation, HashMap::new(), cx);
        self.reconcile_story_state_from_caches(&author_id, cx);
        self.note_story_gone(story_id, cx);
    }

    pub fn note_story_gone(&mut self, story_id: i64, cx: &mut Context<Self>) {
        if story_id <= 0 || self.gone_story_ids.contains(&story_id) {
            return;
        }
        self.gone_story_ids.insert(story_id);
        cx.notify();
    }

    pub fn is_story_gone(&self, story_id: i64, _cx: &App) -> bool {
        story_id > 0 && self.gone_story_ids.contains(&story_id)
    }

    pub fn story_viewed_ring_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::StoryViewed(event)) = RiseEvent::decode_payload(name::STORY_VIEWED, payload) else {
            return;
        };

        let viewer_id = trimming_whitespaces_and_newlines(&event.viewer_id).to_owned();
        let my_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        let author_id = trimming_whitespaces_and_newlines(&event.author_id).to_owned();
        if !my_id.is_empty() && author_id == my_id && viewer_id != my_id {
            self.refresh_own_story_viewers(event.story_id, cx);
            return;
        }
        if viewer_id != my_id || my_id.is_empty() || author_id.is_empty() {
            return;
        }
        let story_id = event.story_id;
        if story_id <= 0 {
            let user_ids = vec![author_id];
            Self::with_actions(cx, move |actions, cx| actions.get_stories_state_action(&user_ids, cx));
            return;
        }
        let stories = if self.user_stories_loaded(&author_id, cx) {
            self.user_stories(&author_id, cx)
        } else {
            self.stories_tray(cx)
                .into_iter()
                .find(|group| Self::story_group_author_id(group) == author_id)
                .map(|group| group.stories)
                .unwrap_or_default()
        };
        self.mark_story_viewed_locally(story_id, &author_id, &stories, cx);
        self.reconcile_story_state_from_caches(&author_id, cx);
    }

    pub fn feed_cache_key() -> String {
        SaiCache::new(["getStoriesFeed"]).key
    }

    pub fn user_stories_cache_key(target_user_id: &str) -> String {
        SaiCache::new(["getUserStories", target_user_id]).key
    }

    pub fn highlights_cache_key(target_user_id: &str, scope: HighlightScope) -> String {
        match scope {
            HighlightScope::Profile => SaiCache::new(["getUserHighlights", target_user_id]).key,
            HighlightScope::Archive => {
                SaiCache::new(["getUserHighlights", target_user_id, HighlightScope::Archive.raw_value()]).key
            }
        }
    }

    pub fn highlights_cache_exists(&self, target_user_id: &str, scope: HighlightScope, cx: &mut App) -> bool {
        Self::memory_payload_exists(&Self::highlights_cache_key(target_user_id, scope), cx)
    }

    pub fn archive_highlight_transforms(
        &self,
        highlight: &HighlightDTO,
        archived: bool,
        owner_user_id: &str,
        cx: &mut App,
    ) -> HashMap<String, StoryCacheTransform> {
        let (from, to) =
            if archived { (HighlightScope::Profile, HighlightScope::Archive) } else { (HighlightScope::Archive, HighlightScope::Profile) };
        let mut moved = highlight.clone();
        moved.archived_at_ms = archived.then(|| Date::now().ms());
        let mut transforms: HashMap<String, StoryCacheTransform> = HashMap::from([(
            Self::highlights_cache_key(owner_user_id, from),
            Self::remove_highlight_transform(highlight.id),
        )]);
        if self.highlights_cache_exists(owner_user_id, to, cx) {
            transforms.insert(Self::highlights_cache_key(owner_user_id, to), Self::upsert_highlight_transform(moved, None));
        }
        transforms
    }

    pub fn open_highlight_link(
        &mut self,
        highlight_id: i64,
        on_owner_resolved: Rc<dyn Fn(String, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        if highlight_id <= 0 {
            return;
        }
        let generation = self.begin_link_open(cx);
        let on_success: StoryActionHighlightHandler = Rc::new(move |highlight: HighlightDTO, cx: &mut App| {
            let current = Self::update_shared_returning(cx, |this, cx| {
                if generation != this.link_open_generation {
                    return false;
                }
                this.is_loading_player = false;
                cx.notify();
                true
            });
            if !current {
                return;
            }
            let owner = trimming_whitespaces_and_newlines(&highlight.owner_id).to_owned();
            if owner.is_empty() {
                Notifier::send(
                    tr("highlight_unavailable"),
                    NotifierSendOptions { r#type: NotifierType::Error, ..NotifierSendOptions::default() },
                    cx,
                );
                return;
            }
            on_owner_resolved(owner.clone(), cx);
            Self::update_shared(cx, |this, cx| {
                this.open_highlight(highlight_id, &owner, Uuid::new_v4(), StoryPlayerEntrance::Slide, None, cx);
            });
        });
        let on_error: StoryActionErrorHandler = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let current = Self::update_shared_returning(cx, |this, cx| {
                if generation != this.link_open_generation {
                    return false;
                }
                this.is_loading_player = false;
                cx.notify();
                true
            });
            if !current {
                return;
            }
            let code = error.downcast_ref::<SaiWsResponseError>().and_then(|error| error.code.clone());
            let title = if CanonErrors::category(code.as_deref()) == ErrorCategory::NotFound {
                tr("highlight_unavailable")
            } else {
                tr("error_fetching_data")
            };
            Notifier::send(title, NotifierSendOptions { r#type: NotifierType::Error, ..NotifierSendOptions::default() }, cx);
        });
        Self::with_actions(cx, move |actions, cx| {
            actions.get_highlight_action(highlight_id, on_success, on_error, cx);
        });
    }

    pub fn highlight_stories_cache_key(highlight_id: i64) -> String {
        SaiCache::new(["getHighlightStories".to_owned(), highlight_id.to_string()]).key
    }

    pub fn highlight_stories(&self, highlight_id: i64, cx: &mut App) -> Vec<StoryDTO> {
        if let Some(invented) = (story_guide_sandbox_hooks().highlight_stories)(highlight_id, cx) {
            return invented;
        }
        self.decode_gateway::<GetHighlightStoriesGatewayDTO>(&Self::highlight_stories_cache_key(highlight_id), cx)
            .map(|gateway| gateway.stories)
            .unwrap_or_default()
    }

    pub fn highlight_stories_has_more(&self, highlight_id: i64, cx: &mut App) -> bool {
        self.decode_gateway::<GetHighlightStoriesGatewayDTO>(&Self::highlight_stories_cache_key(highlight_id), cx)
            .is_some_and(|gateway| gateway.has_more)
    }

    pub fn highlight_stories_next_cursor(&self, highlight_id: i64, cx: &mut App) -> Option<String> {
        self.decode_gateway::<GetHighlightStoriesGatewayDTO>(&Self::highlight_stories_cache_key(highlight_id), cx)
            .and_then(|gateway| gateway.relative_id)
    }

    pub fn open_highlight(
        &mut self,
        highlight_id: i64,
        owner_user_id: &str,
        request_id: Uuid,
        entrance: StoryPlayerEntrance,
        shared_element_origin: Option<StoryPlayerSharedElementOrigin>,
        cx: &mut Context<Self>,
    ) {
        let owner = trimming_whitespaces_and_newlines(owner_user_id).to_owned();
        if self.is_loading_player {
            return;
        }

        self.note_user_stories_target(&owner, cx);

        let cached = self.highlight_stories(highlight_id, cx);
        if !cached.is_empty() {
            self.pending_highlight_open = None;
            self.pending_highlight_origin = None;
            let group = self.group_for_stories(&owner, cached);
            self.present_player(
                StoryPlayerRequest::new(vec![group], 0)
                    .with_id(request_id)
                    .with_owns_explicit_stories(true)
                    .with_entrance(entrance)
                    .with_shared_element_origin(shared_element_origin),
                Some(highlight_id),
                cx,
            );
            return;
        }

        self.pending_highlight_open = Some(highlight_id);
        self.pending_highlight_origin = shared_element_origin;
        self.is_loading_player = true;
        cx.notify();
        Self::with_actions(cx, move |actions, cx| actions.get_highlight_stories_action(highlight_id, None, cx));
    }

    pub fn fail_highlight_open(&mut self, highlight_id: i64, cx: &mut Context<Self>) {
        if self.pending_highlight_open != Some(highlight_id) {
            return;
        }
        self.pending_highlight_open = None;
        self.pending_highlight_origin = None;
        self.is_loading_player = false;
        cx.notify();
    }

    pub fn present_highlight_stories_if_ready(&mut self, highlight_id: i64, cx: &mut Context<Self>) {
        if self.pending_highlight_open != Some(highlight_id) {
            return;
        }
        self.pending_highlight_open = None;
        let origin = self.pending_highlight_origin.take();
        self.is_loading_player = false;
        cx.notify();

        let stories = self.highlight_stories(highlight_id, cx);
        if stories.is_empty() {
            return;
        }
        let owner = self
            .last_user_stories_target
            .clone()
            .unwrap_or_else(|| stories.first().map(|story| story.author_id.clone()).unwrap_or_default());
        let group = self.group_for_stories(&owner, stories);
        self.present_player(
            StoryPlayerRequest::new(vec![group], 0)
                .with_owns_explicit_stories(true)
                .with_shared_element_origin(origin),
            Some(highlight_id),
            cx,
        );
    }

    fn rebuilt_request(
        request: &StoryPlayerRequest,
        groups: Vec<StoryGroupDTO>,
        start_group_index: usize,
    ) -> StoryPlayerRequest {
        StoryPlayerRequest::new(groups, start_group_index)
            .with_id(request.id)
            .with_presentation_style(request.presentation_style)
            .with_owns_explicit_stories(request.owns_explicit_stories)
            .with_entrance(request.entrance)
            .with_shared_element_origin(request.shared_element_origin.clone())
    }

    pub fn refresh_open_player_highlight(&mut self, highlight_id: i64, cx: &mut Context<Self>) {
        if self.active_highlight_id != Some(highlight_id) {
            return;
        }
        let Some(request) = self.player_request.clone() else {
            return;
        };
        let stories = self.highlight_stories(highlight_id, cx);
        if stories.is_empty() {
            return;
        }
        let owner = request
            .groups
            .first()
            .and_then(|group| group.author.as_ref().map(|author| author.id.clone()))
            .or_else(|| stories.first().map(|story| story.author_id.clone()))
            .unwrap_or_default();
        let group = self.group_for_stories(&owner, stories);
        self.player_request = Some(Self::rebuilt_request(&request, vec![group], request.start_group_index));
        cx.notify();
    }

    pub fn remove_story_from_open_player(&mut self, story_id: i64, cx: &mut Context<Self>) {
        let Some(request) = self.player_request.clone() else {
            return;
        };
        if !request.groups.iter().any(|group| group.stories.iter().any(|story| story.id == story_id)) {
            return;
        }
        let groups: Vec<StoryGroupDTO> = request
            .groups
            .iter()
            .filter_map(|group| {
                let stories: Vec<StoryDTO> = group.stories.iter().filter(|story| story.id != story_id).cloned().collect();
                if stories.is_empty() {
                    return None;
                }
                let unseen = stories.iter().filter(|story| !self.viewed(story)).count();
                Some(StoryGroupDTO {
                    all_seen: unseen == 0,
                    author: group.author.clone(),
                    stories,
                    unseen_count: unseen as i64,
                })
            })
            .collect();
        if groups.is_empty() {
            return;
        }
        let start = request.start_group_index.min(groups.len() - 1);
        self.player_request = Some(Self::rebuilt_request(&request, groups, start));
        cx.notify();
    }

    pub fn decode_gateway<T: DeserializeOwned>(&self, cache_key: &str, cx: &mut App) -> Option<T> {
        let payload = SaiWsResponseCache::observed_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )?;
        SaiJSONCodec::decode::<T>(Some(&payload))
    }

    pub fn stories_tray(&self, cx: &mut App) -> Vec<StoryGroupDTO> {
        self.decode_gateway::<GetStoriesFeedGatewayDTO>(&Self::feed_cache_key(), cx)
            .map(|gateway| gateway.tray)
            .unwrap_or_default()
    }

    pub fn stories_feed_loaded(&self, cx: &mut App) -> bool {
        self.decode_gateway::<GetStoriesFeedGatewayDTO>(&Self::feed_cache_key(), cx).is_some()
    }

    pub fn feed_has_more(&self, cx: &mut App) -> bool {
        self.decode_gateway::<GetStoriesFeedGatewayDTO>(&Self::feed_cache_key(), cx)
            .is_some_and(|gateway| gateway.has_more)
    }

    pub fn feed_next_cursor(&self, cx: &mut App) -> Option<String> {
        self.decode_gateway::<GetStoriesFeedGatewayDTO>(&Self::feed_cache_key(), cx)
            .and_then(|gateway| gateway.relative_id)
    }

    pub fn user_stories(&self, target_user_id: &str, cx: &mut App) -> Vec<StoryDTO> {
        if let Some(invented) = (story_guide_sandbox_hooks().user_stories)(target_user_id, cx) {
            return invented;
        }
        self.decode_gateway::<GetUserStoriesGatewayDTO>(&Self::user_stories_cache_key(target_user_id), cx)
            .map(|gateway| gateway.list)
            .unwrap_or_default()
    }

    pub fn user_stories_loaded(&self, target_user_id: &str, cx: &mut App) -> bool {
        if (story_guide_sandbox_hooks().user_stories)(target_user_id, cx).is_some() {
            return true;
        }
        self.decode_gateway::<GetUserStoriesGatewayDTO>(&Self::user_stories_cache_key(target_user_id), cx)
            .is_some()
    }

    pub fn user_stories_has_more(&self, target_user_id: &str, cx: &mut App) -> bool {
        if (story_guide_sandbox_hooks().user_stories)(target_user_id, cx).is_some() {
            return false;
        }
        self.decode_gateway::<GetUserStoriesGatewayDTO>(&Self::user_stories_cache_key(target_user_id), cx)
            .is_some_and(|gateway| gateway.has_more)
    }

    pub fn user_stories_next_cursor(&self, target_user_id: &str, cx: &mut App) -> Option<i64> {
        if (story_guide_sandbox_hooks().user_stories)(target_user_id, cx).is_some() {
            return None;
        }
        self.decode_gateway::<GetUserStoriesGatewayDTO>(&Self::user_stories_cache_key(target_user_id), cx)
            .and_then(|gateway| gateway.relative_id)
    }

    pub fn refresh_open_player_stories(&mut self, target_user_id: &str, cx: &mut Context<Self>) {
        self.refresh_open_player_stories_from(target_user_id, false, cx);
    }

    /// authoritative is a fresh server list,
    /// any other refresh reads a cache that can be older so it only adds and updates
    fn refresh_open_player_stories_from(&mut self, target_user_id: &str, authoritative: bool, cx: &mut Context<Self>) {
        let Some(request) = self.player_request.clone() else {
            return;
        };
        if request.owns_explicit_stories || !self.user_stories_loaded(target_user_id, cx) {
            return;
        }
        let cached = self.user_stories(target_user_id, cx);

        let mut groups = request.groups.clone();
        let Some(index) = groups.iter().position(|group| {
            group.author.as_ref().is_some_and(|author| author.id == target_user_id)
                || group.stories.first().is_some_and(|story| story.author_id == target_user_id)
        }) else {
            return;
        };
        let stories = if authoritative { cached } else { Self::merged_player_stories(&groups[index].stories, cached) };
        if stories.is_empty() {
            self.clear_stories_state(target_user_id);
            groups.remove(index);
            if groups.is_empty() {
                self.dismiss_player(cx);
            } else {
                let start = request.start_group_index.min(groups.len() - 1);
                self.player_request = Some(Self::rebuilt_request(&request, groups, start));
                cx.notify();
            }
            return;
        }
        groups[index] = self.group_for_stories(target_user_id, stories);
        self.player_request = Some(Self::rebuilt_request(&request, groups, request.start_group_index));
        cx.notify();
    }

    fn merged_player_stories(open: &[StoryDTO], cached: Vec<StoryDTO>) -> Vec<StoryDTO> {
        let mut merged = cached;
        for story in open {
            if story.id < 0 {
                continue;
            }
            if !merged.iter().any(|known| known.id == story.id) {
                merged.push(story.clone());
            }
        }
        merged.sort_by_key(|story| story.created_at_ms);
        merged
    }

    pub fn highlights(&self, target_user_id: &str, scope: HighlightScope, cx: &mut App) -> Vec<HighlightDTO> {
        let stored = self
            .decode_gateway::<GetUserHighlightsGatewayDTO>(&Self::highlights_cache_key(target_user_id, scope), cx)
            .map(|gateway| gateway.highlights)
            .unwrap_or_default();
        if scope != HighlightScope::Profile {
            return stored;
        }
        (story_guide_sandbox_hooks().highlights)(target_user_id, stored, cx)
    }

    pub fn highlights_loaded(&self, target_user_id: &str, scope: HighlightScope, cx: &mut App) -> bool {
        self.decode_gateway::<GetUserHighlightsGatewayDTO>(&Self::highlights_cache_key(target_user_id, scope), cx)
            .is_some()
    }

    pub fn viewers_cache_key(story_id: i64) -> String {
        SaiCache::new(["getStoryViewers".to_owned(), story_id.to_string()]).key
    }

    fn viewers_projection(&self, story_id: i64, cx: &mut App) -> StoryViewersProjection {
        SaiWsResponseCache::observed_projection(
            &Self::viewers_cache_key(story_id),
            "story-viewers-projection-v1",
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            |payload| {
                let Some(dto) = SaiJSONCodec::decode::<GetStoryViewersGatewayDTO>(payload) else {
                    return StoryViewersProjection::default();
                };
                StoryViewersProjection {
                    viewers: dto.viewers,
                    total: dto.total,
                    like_count: dto.like_count,
                    has_more: dto.has_more,
                    next_cursor: dto.relative_id,
                    loaded: true,
                }
            },
            cx,
        )
    }

    pub fn story_viewers(&self, story_id: i64, cx: &mut App) -> Vec<StoryViewerDTO> {
        self.viewers_projection(story_id, cx).viewers
    }

    pub fn story_viewers_total(&self, story_id: i64, cx: &mut App) -> i64 {
        self.viewers_projection(story_id, cx).total
    }

    pub fn story_viewers_like_count(&self, story_id: i64, cx: &mut App) -> i64 {
        self.viewers_projection(story_id, cx).like_count
    }

    pub fn own_story_like_count(&self, story: &StoryDTO, cx: &mut App) -> i64 {
        self.story_like_count(story, cx).max(self.story_viewers_like_count(story.id, cx))
    }

    pub fn story_viewers_has_more(&self, story_id: i64, cx: &mut App) -> bool {
        self.viewers_projection(story_id, cx).has_more
    }

    pub fn story_viewers_next_cursor(&self, story_id: i64, cx: &mut App) -> Option<i64> {
        self.viewers_projection(story_id, cx).next_cursor
    }

    pub fn mark_viewers_in_flight(&mut self, story_id: i64, cx: &mut Context<Self>) {
        self.viewers_errored.remove(&story_id);
        self.viewers_in_flight.insert(story_id);
        cx.notify();
    }

    pub fn clear_viewers_in_flight(&mut self, story_id: i64, errored: bool, cx: &mut Context<Self>) {
        self.viewers_in_flight.remove(&story_id);
        if errored {
            self.viewers_errored.insert(story_id);
        } else {
            self.viewers_errored.remove(&story_id);
        }
        if self.viewers_stale.remove(&story_id) && !errored {
            Self::with_actions(cx, move |actions, cx| actions.get_story_viewers_action(story_id, None, cx));
        }
        cx.notify();
    }

    fn refresh_own_story_viewers(&mut self, story_id: i64, cx: &mut Context<Self>) {
        if story_id <= 0 || !self.story_viewers_loaded(story_id, cx) {
            return;
        }
        if self.viewers_in_flight.contains(&story_id) {
            self.viewers_stale.insert(story_id);
            return;
        }
        Self::with_actions(cx, move |actions, cx| actions.get_story_viewers_action(story_id, None, cx));
    }

    pub fn is_viewers_in_flight(&self, story_id: i64, _cx: &App) -> bool {
        self.viewers_in_flight.contains(&story_id)
    }

    pub fn story_viewers_loaded(&self, story_id: i64, cx: &mut App) -> bool {
        self.viewers_projection(story_id, cx).loaded
    }

    pub fn note_user_stories_target(&mut self, target_user_id: &str, _cx: &mut Context<Self>) {
        self.last_user_stories_target = Some(target_user_id.to_owned());
    }

    pub fn note_highlights_target(&mut self, target_user_id: &str, _cx: &mut Context<Self>) {
        self.last_highlights_target = Some(target_user_id.to_owned());
    }

    pub fn make_temporary_highlight(
        &mut self,
        title: &str,
        owner_id: &str,
        story_ids: &[i64],
        cx: &mut Context<Self>,
    ) -> HighlightDTO {
        let id = self.next_temporary_highlight_id;
        self.next_temporary_highlight_id -= 1;
        let stories_by_id: HashMap<i64, StoryDTO> =
            self.user_stories(owner_id, cx).into_iter().map(|story| (story.id, story)).collect();
        HighlightDTO {
            cover_story: story_ids.first().and_then(|first| stories_by_id.get(first).cloned()),
            created_at_ms: Date::now().ms(),
            id,
            owner_id: owner_id.to_owned(),
            stories_count: story_ids.len() as i64,
            title: title.to_owned(),
            ..HighlightDTO::default()
        }
    }

    fn rows_value(rows: Vec<Map<String, Value>>) -> Value {
        Value::Array(rows.into_iter().map(Value::Object).collect())
    }

    fn seeded_root(array_key: &str, third_key: &str, third_value: Value) -> Map<String, Value> {
        let mut root = Map::new();
        root.insert(key::SUCCESS.to_owned(), Value::Bool(true));
        root.insert(array_key.to_owned(), Value::Array(Vec::new()));
        root.insert(third_key.to_owned(), third_value);
        root
    }

    pub fn upsert_story_transform(story: StoryDTO, replacing_story_id: Option<i64>) -> StoryCacheTransform {
        Rc::new(move |existing| {
            let Some(row) = SaiJSONCodec::encode(&story) else {
                return existing;
            };
            let mut root = existing.unwrap_or_else(|| Self::seeded_root(key::LIST, key::HAS_MORE, Value::Bool(false)));
            let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::LIST));
            let replace_id = replacing_story_id.unwrap_or(story.id);
            if replace_id != story.id {
                rows.retain(|row| Self::story_id(row.get(key::ID)) != story.id);
            }
            let index = rows
                .iter()
                .position(|row| Self::story_id(row.get(key::ID)) == replace_id)
                .or_else(|| rows.iter().position(|row| Self::story_id(row.get(key::ID)) == story.id));
            if let Some(index) = index {
                rows[index] = row;
            } else {
                rows.insert(0, row);
                if let Some(total) = root.get(key::TOTAL).and_then(Value::as_i64) {
                    root.insert(key::TOTAL.to_owned(), Value::from(total + 1));
                }
            }
            root.insert(key::LIST.to_owned(), Self::rows_value(rows));
            Some(root)
        })
    }

    pub fn upsert_published_profile_story_transform(
        story: StoryDTO,
        replacing_story_id: Option<i64>,
    ) -> StoryCacheTransform {
        let upsert = Self::upsert_story_transform(story.clone(), replacing_story_id);
        Rc::new(move |existing| {
            existing.as_ref()?;
            let Some(mut root) = upsert(existing.clone()) else {
                return existing;
            };
            let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::LIST));
            let Some(index) = rows.iter().position(|row| Self::story_id(row.get(key::ID)) == story.id) else {
                return Some(root);
            };
            let row = rows.remove(index);
            let first_unpinned = rows
                .iter()
                .position(|row| row.get(key::PINNED_AT_MS).and_then(Value::as_i64).unwrap_or(0) <= 0)
                .unwrap_or(rows.len());
            rows.insert(first_unpinned, row);
            root.insert(key::LIST.to_owned(), Self::rows_value(rows));
            Some(root)
        })
    }

    pub fn remove_story_transform(target_id: i64) -> StoryCacheTransform {
        Rc::new(move |existing| {
            let mut root = existing?;
            let rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::LIST));
            let count = rows.len();
            let filtered: Vec<Map<String, Value>> =
                rows.into_iter().filter(|row| Self::story_id(row.get(key::ID)) != target_id).collect();
            if filtered.len() != count
                && let Some(total) = root.get(key::TOTAL).and_then(Value::as_i64)
            {
                root.insert(key::TOTAL.to_owned(), Value::from((total - 1).max(0)));
            }
            root.insert(key::LIST.to_owned(), Self::rows_value(filtered));
            Some(root)
        })
    }

    fn unseen_rows(rows: &[Map<String, Value>]) -> usize {
        rows.iter().filter(|row| row.get(key::HAS_VIEWED).and_then(Value::as_bool) != Some(true)).count()
    }

    pub fn upsert_feed_story_transform(story: StoryDTO, replacing_story_id: Option<i64>) -> StoryCacheTransform {
        Rc::new(move |existing| {
            let Some(story_row) = SaiJSONCodec::encode(&story) else {
                return existing;
            };
            let author_id = trimming_whitespaces_and_newlines(&story.author_id).to_owned();
            if author_id.is_empty() {
                return existing;
            }

            let mut root = existing.unwrap_or_else(|| Self::seeded_root(key::TRAY, key::HAS_MORE, Value::Bool(false)));
            let mut tray = AnyDictionaryBridge::array_of_dictionaries(root.get(key::TRAY));
            let group_index = tray.iter().position(|group| Self::raw_story_group_author_id(group) == author_id);
            let mut group = group_index.map(|index| tray[index].clone()).unwrap_or_default();
            let mut rows = AnyDictionaryBridge::array_of_dictionaries(group.get(key::STORIES));
            let replace_id = replacing_story_id.unwrap_or(story.id);
            if replace_id != story.id {
                rows.retain(|row| Self::story_id(row.get(key::ID)) != story.id);
            }
            let story_index = rows
                .iter()
                .position(|row| Self::story_id(row.get(key::ID)) == replace_id)
                .or_else(|| rows.iter().position(|row| Self::story_id(row.get(key::ID)) == story.id));
            match story_index {
                Some(index) => rows[index] = story_row,
                None => rows.push(story_row),
            }
            let author = story
                .author
                .as_ref()
                .and_then(SaiJSONCodec::encode)
                .map(Value::Object)
                .or_else(|| group.get(key::AUTHOR).cloned())
                .unwrap_or_else(|| Value::Object(Map::new()));
            group.insert(key::AUTHOR.to_owned(), author);
            let unseen = Self::unseen_rows(&rows);
            group.insert(key::STORIES.to_owned(), Self::rows_value(rows));
            group.insert(key::UNSEEN_COUNT.to_owned(), Value::from(unseen));
            group.insert(key::ALL_SEEN.to_owned(), Value::Bool(unseen == 0));
            match (group_index, story_index) {
                (Some(group_index), Some(_)) => tray[group_index] = group,
                (group_index, _) => {
                    if let Some(group_index) = group_index {
                        tray.remove(group_index);
                    }
                    tray.insert(0, group);
                }
            }
            root.insert(key::TRAY.to_owned(), Self::rows_value(tray));
            Some(root)
        })
    }

    pub fn remove_feed_story_transform(target_id: i64) -> StoryCacheTransform {
        Rc::new(move |existing| {
            let mut root = existing?;
            let mut tray = AnyDictionaryBridge::array_of_dictionaries(root.get(key::TRAY));
            for index in (0..tray.len()).rev() {
                let rows: Vec<Map<String, Value>> = AnyDictionaryBridge::array_of_dictionaries(tray[index].get(key::STORIES))
                    .into_iter()
                    .filter(|row| Self::story_id(row.get(key::ID)) != target_id)
                    .collect();
                if rows.is_empty() {
                    tray.remove(index);
                } else {
                    let unseen = Self::unseen_rows(&rows);
                    let group = &mut tray[index];
                    group.insert(key::STORIES.to_owned(), Self::rows_value(rows));
                    group.insert(key::UNSEEN_COUNT.to_owned(), Value::from(unseen));
                    group.insert(key::ALL_SEEN.to_owned(), Value::Bool(unseen == 0));
                }
            }
            root.insert(key::TRAY.to_owned(), Self::rows_value(tray));
            Some(root)
        })
    }

    pub fn remove_feed_author_transform(user_id: &str) -> StoryCacheTransform {
        let user_id = user_id.to_owned();
        Rc::new(move |existing| {
            let mut root = existing?;
            let tray: Vec<Map<String, Value>> = AnyDictionaryBridge::array_of_dictionaries(root.get(key::TRAY))
                .into_iter()
                .filter(|group| Self::raw_story_group_author_id(group) != user_id)
                .collect();
            root.insert(key::TRAY.to_owned(), Self::rows_value(tray));
            Some(root)
        })
    }

    pub fn upsert_feed_group_transform(group: Map<String, Value>) -> StoryCacheTransform {
        Rc::new(move |existing| {
            let user_id = Self::raw_story_group_author_id(&group);
            if user_id.is_empty() {
                return existing;
            }
            let mut root = existing.unwrap_or_else(|| Self::seeded_root(key::TRAY, key::HAS_MORE, Value::Bool(false)));
            let mut tray = AnyDictionaryBridge::array_of_dictionaries(root.get(key::TRAY));
            match tray.iter().position(|row| Self::raw_story_group_author_id(row) == user_id) {
                Some(index) => tray[index] = group.clone(),
                None => tray.insert(0, group.clone()),
            }
            root.insert(key::TRAY.to_owned(), Self::rows_value(tray));
            Some(root)
        })
    }

    pub fn upsert_highlight_transform(highlight: HighlightDTO, replacing_highlight_id: Option<i64>) -> StoryCacheTransform {
        Rc::new(move |existing| {
            let Some(row) = SaiJSONCodec::encode(&highlight) else {
                return existing;
            };
            let mut root = existing.unwrap_or_else(|| Self::seeded_root(key::HIGHLIGHTS, key::TOTAL, Value::from(0)));
            let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::HIGHLIGHTS));
            let replace_id = replacing_highlight_id.unwrap_or(highlight.id);
            let index = rows
                .iter()
                .position(|row| Self::story_id(row.get(key::ID)) == replace_id)
                .or_else(|| rows.iter().position(|row| Self::story_id(row.get(key::ID)) == highlight.id));
            if let Some(index) = index {
                rows[index] = row;
            } else {
                rows.insert(0, row);
                let previous = root.get(key::TOTAL).and_then(Value::as_i64).unwrap_or(rows.len() as i64 - 1);
                root.insert(key::TOTAL.to_owned(), Value::from(previous + 1));
            }
            root.insert(key::HIGHLIGHTS.to_owned(), Self::rows_value(rows));
            Some(root)
        })
    }

    pub fn patch_highlight_transform(highlight_id: i64, fields: Map<String, Value>) -> StoryCacheTransform {
        Rc::new(move |existing| {
            let mut root = existing?;
            let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::HIGHLIGHTS));
            if let Some(row) = rows.iter_mut().find(|row| Self::story_id(row.get(key::ID)) == highlight_id) {
                for (field, value) in &fields {
                    row.insert(field.clone(), value.clone());
                }
            }
            root.insert(key::HIGHLIGHTS.to_owned(), Self::rows_value(rows));
            Some(root)
        })
    }

    pub fn remove_highlight_transform(highlight_id: i64) -> StoryCacheTransform {
        Rc::new(move |existing| {
            let mut root = existing?;
            let rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::HIGHLIGHTS));
            let count = rows.len();
            let filtered: Vec<Map<String, Value>> =
                rows.into_iter().filter(|row| Self::story_id(row.get(key::ID)) != highlight_id).collect();
            if filtered.len() != count {
                let total = root.get(key::TOTAL).and_then(Value::as_i64).unwrap_or(count as i64);
                root.insert(key::TOTAL.to_owned(), Value::from((total - 1).max(0)));
            }
            root.insert(key::HIGHLIGHTS.to_owned(), Self::rows_value(filtered));
            Some(root)
        })
    }

    pub fn highlight_stories_transform(stories: Vec<StoryDTO>, removing_story_ids: HashSet<i64>) -> StoryCacheTransform {
        Rc::new(move |existing| {
            let mut root = existing?;
            let mut rows: Vec<Map<String, Value>> = AnyDictionaryBridge::array_of_dictionaries(root.get(key::STORIES))
                .into_iter()
                .filter(|row| !removing_story_ids.contains(&Self::story_id(row.get(key::ID))))
                .collect();
            let existing_ids: HashSet<i64> = rows.iter().map(|row| Self::story_id(row.get(key::ID))).collect();
            let additions: Vec<Map<String, Value>> = stories
                .iter()
                .filter(|story| !existing_ids.contains(&story.id))
                .filter_map(SaiJSONCodec::encode)
                .collect();
            rows.splice(0..0, additions);
            let total = rows.len();
            root.insert(key::STORIES.to_owned(), Self::rows_value(rows));
            root.insert(key::TOTAL.to_owned(), Value::from(total));
            Some(root)
        })
    }

    pub fn delete_story_mutation_transforms(
        &self,
        story_id: i64,
        user_id: &str,
        cx: &mut App,
    ) -> HashMap<String, StoryCacheTransform> {
        let mut transforms: HashMap<String, StoryCacheTransform> = HashMap::from([
            (Self::user_stories_cache_key(user_id), Self::remove_story_transform(story_id)),
            (Self::feed_cache_key(), Self::remove_feed_story_transform(story_id)),
            (
                Self::profile_stories_cache_key(user_id, StoryProfileScope::Profile),
                Self::remove_story_transform(story_id),
            ),
            (
                Self::profile_stories_cache_key(user_id, StoryProfileScope::Archive),
                Self::remove_story_transform(story_id),
            ),
        ]);

        let mut affected_highlight_ids: HashSet<i64> = HashSet::new();
        let mut emptied_highlight_ids: HashSet<i64> = HashSet::new();
        let mut next_cover_by_highlight_id: HashMap<i64, Map<String, Value>> = HashMap::new();
        for highlight in self.highlights(user_id, HighlightScope::Profile, cx) {
            let cache_key = Self::highlight_stories_cache_key(highlight.id);
            let members = self.highlight_stories(highlight.id, cx);
            if !Self::memory_payload_exists(&cache_key, cx) || !members.iter().any(|story| story.id == story_id) {
                continue;
            }
            affected_highlight_ids.insert(highlight.id);
            let remaining: Vec<&StoryDTO> = members.iter().filter(|story| story.id != story_id).collect();
            if remaining.is_empty() {
                emptied_highlight_ids.insert(highlight.id);
            } else if let Some(next) = remaining.first().and_then(|story| SaiJSONCodec::encode(*story)) {
                next_cover_by_highlight_id.insert(highlight.id, next);
            }
            transforms.insert(cache_key, Self::highlight_stories_transform(Vec::new(), HashSet::from([story_id])));
        }

        if !affected_highlight_ids.is_empty() {
            let highlights_key = Self::highlights_cache_key(user_id, HighlightScope::Profile);
            let transform: StoryCacheTransform = Rc::new(move |existing| {
                let mut root = existing?;
                let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::HIGHLIGHTS));
                for row in &mut rows {
                    let highlight_id = Self::story_id(row.get(key::ID));
                    if !affected_highlight_ids.contains(&highlight_id) {
                        continue;
                    }
                    let count = row.get(key::STORIES_COUNT).and_then(Value::as_i64).unwrap_or(0);
                    row.insert(key::STORIES_COUNT.to_owned(), Value::from((count - 1).max(0)));
                    let covers_story = AnyDictionaryBridge::dictionary(row.get(key::COVER_STORY))
                        .is_some_and(|cover| Self::story_id(cover.get(key::ID)) == story_id);
                    if covers_story {
                        let cover = next_cover_by_highlight_id.get(&highlight_id).cloned().map_or(Value::Null, Value::Object);
                        row.insert(key::COVER_STORY.to_owned(), cover);
                    }
                }
                let count = rows.len();
                let kept: Vec<Map<String, Value>> = rows
                    .into_iter()
                    .filter(|row| !emptied_highlight_ids.contains(&Self::story_id(row.get(key::ID))))
                    .collect();
                if kept.len() != count
                    && let Some(total) = root.get(key::TOTAL).and_then(Value::as_i64)
                {
                    root.insert(key::TOTAL.to_owned(), Value::from((total - (count - kept.len()) as i64).max(0)));
                }
                root.insert(key::HIGHLIGHTS.to_owned(), Self::rows_value(kept));
                Some(root)
            });
            transforms.insert(highlights_key, transform);
        }
        transforms
    }

    pub fn highlight_story_mutation_transforms(
        &self,
        highlight_id: i64,
        user_id: &str,
        adding_story_ids: &[i64],
        removing_story_ids: &HashSet<i64>,
        cx: &mut App,
    ) -> HashMap<String, StoryCacheTransform> {
        let stories_by_id: HashMap<i64, StoryDTO> =
            self.user_stories(user_id, cx).into_iter().map(|story| (story.id, story)).collect();
        let additions: Vec<StoryDTO> = adding_story_ids.iter().filter_map(|id| stories_by_id.get(id).cloned()).collect();
        let highlight_stories_key = Self::highlight_stories_cache_key(highlight_id);
        let mut transforms: HashMap<String, StoryCacheTransform> = HashMap::new();
        if Self::memory_payload_exists(&highlight_stories_key, cx) {
            transforms.insert(
                highlight_stories_key,
                Self::highlight_stories_transform(additions.clone(), removing_story_ids.clone()),
            );
        }

        let removing = removing_story_ids.clone();
        let remaining_cover = self
            .highlight_stories(highlight_id, cx)
            .into_iter()
            .find(|story| !removing.contains(&story.id))
            .and_then(|story| SaiJSONCodec::encode(&story));
        let delta = adding_story_ids.len() as i64 - removing.len() as i64;
        let first_addition = additions.first().and_then(SaiJSONCodec::encode);
        let highlights_key = Self::highlights_cache_key(user_id, HighlightScope::Profile);
        let transform: StoryCacheTransform = Rc::new(move |existing| {
            let mut root = existing?;
            let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::HIGHLIGHTS));
            let Some(row) = rows.iter_mut().find(|row| Self::story_id(row.get(key::ID)) == highlight_id) else {
                return Some(root);
            };
            let current_count = row.get(key::STORIES_COUNT).and_then(Value::as_i64).unwrap_or(0);
            row.insert(key::STORIES_COUNT.to_owned(), Value::from((current_count + delta).max(0)));
            let cover = AnyDictionaryBridge::dictionary(row.get(key::COVER_STORY));
            match (cover, &first_addition) {
                (None, Some(first)) => {
                    row.insert(key::COVER_STORY.to_owned(), Value::Object(first.clone()));
                }
                (Some(cover), _) if removing.contains(&Self::story_id(cover.get(key::ID))) => {
                    let replacement = remaining_cover.clone().map_or(Value::Null, Value::Object);
                    row.insert(key::COVER_STORY.to_owned(), replacement);
                }
                _ => {}
            }
            root.insert(key::HIGHLIGHTS.to_owned(), Self::rows_value(rows));
            Some(root)
        });
        transforms.insert(highlights_key, transform);
        transforms
    }

    fn raw_story_group_author_id(group: &Map<String, Value>) -> String {
        let stable = AnyDictionaryBridge::array_of_dictionaries(group.get(key::STORIES))
            .first()
            .and_then(|row| row.get(key::AUTHOR_ID).and_then(Value::as_str).map(str::to_owned));
        let nested = AnyDictionaryBridge::dictionary(group.get(key::AUTHOR))
            .and_then(|author| author.get(key::ID).and_then(Value::as_str).map(str::to_owned));
        match stable {
            Some(stable) if !stable.is_empty() => stable,
            _ => nested.unwrap_or_default(),
        }
    }

    pub fn reconcile_story_state_from_caches(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let id = trimming_whitespaces_and_newlines(user_id).to_owned();
        if id.is_empty() {
            return;
        }

        let has_user_stories_cache = Self::memory_payload_exists(&Self::user_stories_cache_key(&id), cx);
        let cached_user_stories = self.user_stories(&id, cx);
        let tray = self.stories_tray(cx);
        let feed_group = tray.iter().find(|group| Self::story_group_author_id(group) == id);
        let stories = if has_user_stories_cache {
            cached_user_stories
        } else {
            feed_group.map(|group| group.stories.clone()).unwrap_or_default()
        };

        if has_user_stories_cache || feed_group.is_some() {
            self.known_story_state_user_ids.insert(id.clone());
            if stories.is_empty() {
                self.clear_stories_state(&id);
            } else {
                self.active_story_user_ids.insert(id.clone());
                if stories.iter().any(|story| !self.viewed(story)) && !self.locally_seen_story_user_ids.contains(&id) {
                    self.unseen_story_user_ids.insert(id.clone());
                } else {
                    self.unseen_story_user_ids.remove(&id);
                }
            }
        }

        self.feed_story_user_ids = tray
            .iter()
            .map(Self::story_group_author_id)
            .filter(|author_id| !author_id.is_empty())
            .collect();
        self.refresh_open_player_stories(&id, cx);
        cx.notify();
    }
}

impl StoryServicesStore {
    const PUBLISH_COMPLETION_FEEDBACK: ImpactFeedbackGenerator = ImpactFeedbackGenerator::new(ImpactStyle::Light);
    const PUBLISH_RING_SETTLE: Duration = Duration::from_millis(420);
    const PENDING_STORY_PUBLISHES_LIMIT: usize = 16;
    const PUBLISH_NETWORK_PAUSE: Duration = Duration::from_secs(2);
    const PUBLISH_NETWORK_POLL: Duration = Duration::from_secs(1);
    const PUBLISH_TRANSPORT_BOUNCE_LIMIT: u32 = 4;

    pub fn reset_publish_state(&mut self, cx: &mut Context<Self>) {
        if self.pending_story_publish.is_some() || self.publish_task.is_some() {
            self.publish_error = None;
            cx.notify();
            return;
        }
        self.is_publishing = false;
        self.publish_error = None;
        cx.notify();
    }

    pub fn dismiss_publish_error(&mut self, cx: &mut Context<Self>) {
        self.publish_error = None;
        cx.notify();
    }

    pub fn make_temporary_story(
        &mut self,
        body: &rise_request::story::CreateStory,
        user_id: &str,
        cx: &mut Context<Self>,
    ) -> StoryDTO {
        let id = self.next_temporary_story_id;
        self.next_temporary_story_id -= 1;
        let now = Date::now().ms();
        let auth = AuthServicesStore::shared(cx);
        let profile = auth.read(cx).profile().cloned();
        let duration_days = body.duration_days.unwrap_or(0);
        StoryDTO {
            allow_screenshots: body.allow_screenshots.unwrap_or(false),
            audience: body.audience.clone().unwrap_or_default(),
            author: Some(StoryAuthorDTO {
                id: user_id.to_owned(),
                is_online: true,
                is_premium: profile.as_ref().is_some_and(|profile| profile.more.is_premium),
                last_seen_ms: Some(now),
                logo: profile
                    .as_ref()
                    .and_then(|profile| profile.avatar_url.clone().or_else(|| Some(profile.more.logo.clone())))
                    .unwrap_or_default(),
                name: profile.as_ref().map(|profile| profile.name.clone()).unwrap_or_default(),
                name_color: profile.as_ref().and_then(|profile| profile.more.name_color.clone()),
                premium_badge_emoji: profile.as_ref().and_then(|profile| profile.more.premium_badge_emoji.clone()),
                tag: profile.as_ref().map(|profile| profile.tag.clone()).unwrap_or_default(),
                ..StoryAuthorDTO::default()
            }),
            author_id: user_id.to_owned(),
            caption: body.caption.clone(),
            created_at_ms: now,
            duration_days,
            expires_at_ms: now + duration_days * 86_400_000,
            file_id: body.file_id.clone().unwrap_or_default(),
            has_viewed: Some(true),
            id,
            is_liked: false,
            like_count: 0,
            media_type: body.media_type.clone().unwrap_or_default(),
            post_to_profile: body.post_to_profile.unwrap_or(false),
            track_end_ms: body.track_end_ms,
            track_id: body.track_id.clone(),
            track_plate: body.track_plate.as_ref().map(|plate| StoryTrackPlateDTO {
                artist: plate.artist.clone().unwrap_or_default(),
                center_x: plate.center_x.unwrap_or(0.0),
                center_y: plate.center_y.unwrap_or(0.0),
                cover_url: plate.cover_url.clone().unwrap_or_default(),
                is_dark: plate.is_dark.unwrap_or(false),
                rotation: plate.rotation.unwrap_or(0.0),
                scale: plate.scale.unwrap_or(0.0),
                title: plate.title.clone().unwrap_or_default(),
            }),
            track_start_ms: body.track_start_ms,
            view_count: 0,
            ..StoryDTO::default()
        }
    }

    fn set_pending_story_publish_phase(&mut self, phase: StoryPendingPublishPhase, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_story_publish.as_mut() else {
            return;
        };
        if pending.phase == phase {
            return;
        }
        pending.phase = phase;
        match phase {
            StoryPendingPublishPhase::Preparing => StoryUploadProgressCenter::publish(0.0, StoryUploadPhase::Preparing),
            StoryPendingPublishPhase::Uploading => StoryUploadProgressCenter::publish(0.0, StoryUploadPhase::Uploading),
            StoryPendingPublishPhase::Creating => StoryUploadProgressCenter::publish(1.0, StoryUploadPhase::Processing),
            StoryPendingPublishPhase::Failed => StoryUploadProgressCenter::publish(1.0, StoryUploadPhase::Failed),
        }
        cx.notify();
    }

    fn adopt_rendered_pending_media(&mut self, url: PathBuf, cx: &mut Context<Self>) {
        let previous = self.pending_publish_staged_url.replace(url.clone());
        if let Some(pending) = self.pending_story_publish.as_mut() {
            pending.media_source = Self::file_url_string(&url);
            pending.is_video = true;
        }
        if previous.as_deref() != Some(url.as_path()) {
            StoryPublishStaging::discard(previous.as_deref());
        }
        cx.notify();
    }

    pub fn pending_story_media(&self, story_id: i64, _cx: &App) -> Option<(String, bool)> {
        if let Some(settled) = self.settled_publish_media.as_ref()
            && settled.story_id == story_id
        {
            return Some((Self::file_url_string(&settled.staged_url), settled.is_video));
        }
        let pending = self.pending_story_publish.as_ref()?;
        if pending.story_id != story_id || pending.media_source.is_empty() {
            return None;
        }
        Some((pending.media_source.clone(), pending.is_video))
    }

    pub fn settled_publish_story_id(&self) -> Option<i64> {
        self.settled_publish_media.as_ref().map(|settled| settled.story_id)
    }

    pub fn is_pending_story(&self, story_id: i64, _cx: &App) -> bool {
        self.pending_story_publish.as_ref().is_some_and(|pending| pending.story_id == story_id)
    }

    pub fn cancel_active_story_publish(&mut self, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X807", "cancel phase={:?} task={} player={} signature={:?}", self.pending_story_publish.as_ref().map(|pending| pending.phase), self.publish_task.is_some(), self.player_request.is_some(), self.pending_publish_signature);
        let Some(phase) = self.pending_story_publish.as_ref().map(|pending| pending.phase) else {
            return;
        };
        self.publish_task = None;
        self.is_publish_parked_on_network = false;
        self.publish_transport_bounces = 0;
        if phase == StoryPendingPublishPhase::Creating {
            self.cancelled_publish_signature = self.pending_publish_signature.clone();
        }
        self.pending_publish_signature = None;
        self.publish_ring_is_settling = false;
        self.publish_error = None;
        self.is_publishing = false;

        if self.player_request.is_some() {
            self.pending_publish_unwinds_on_player_dismiss = true;
        } else {
            self.unwind_pending_story_publish(cx);
        }
        cx.notify();
    }

    fn unwind_cancelled_story_publish_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.pending_publish_unwinds_on_player_dismiss {
            return;
        }
        self.pending_publish_unwinds_on_player_dismiss = false;
        self.unwind_pending_story_publish(cx);
    }

    fn unwind_pending_story_publish(&mut self, cx: &mut Context<Self>) {
        let author_id = self.pending_story_publish.as_ref().map(|pending| pending.author_id.clone());
        if let Some(mutation) = self.pending_publish_mutation.take() {
            self.rollback_story_mutation(&mutation, cx);
        }
        StoryPublishStaging::discard(self.pending_publish_staged_url.as_deref());
        self.pending_publish_staged_url = None;
        self.pending_publish_lease = None;
        self.pending_story_publish = None;
        self.pending_publish_unwinds_on_player_dismiss = false;
        StoryUploadProgressCenter::clear(cx);
        if let Some(author_id) = author_id {
            self.reconcile_story_state_from_caches(&author_id, cx);
        }
        cx.notify();
    }

    fn release_settled_publish_media(&mut self, cx: &mut Context<Self>) {
        let Some(media) = self.settled_publish_media.take() else {
            return;
        };
        Self::file_staged_media(media, cx);
    }

    fn finish_pending_story_publish(&mut self, committed: bool, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X807", "finish committed={committed} unwinds_on_dismiss={}", self.pending_publish_unwinds_on_player_dismiss);
        self.publish_task = None;
        self.pending_publish_signature = None;

        if self.pending_publish_unwinds_on_player_dismiss {
            return;
        }

        self.pending_publish_mutation = None;
        StoryPublishStaging::discard(self.pending_publish_staged_url.as_deref());
        self.pending_publish_staged_url = None;
        self.pending_publish_lease = None;

        let had_pending_row = self.pending_story_publish.is_some();
        self.pending_story_publish = None;
        cx.notify();

        if !committed {
            self.publish_ring_is_settling = false;
            StoryUploadProgressCenter::clear(cx);
            return;
        }
        self.publish_ring_is_settling = true;
        StoryUploadProgressCenter::publish(1.0, StoryUploadPhase::Completed);
        if had_pending_row {
            let _ = Self::PUBLISH_COMPLETION_FEEDBACK.prepare();
            let _ = Self::PUBLISH_COMPLETION_FEEDBACK.impact_occurred();
        }
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor().timer(Self::PUBLISH_RING_SETTLE).await;
            let _ = this.update(cx, |store, cx| {
                if store.pending_story_publish.is_some() {
                    return;
                }
                StoryUploadProgressCenter::clear(cx);
                store.publish_ring_is_settling = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn file_url_string(path: &Path) -> String {
        Url::from_file_path(path).map(|url| url.absolute_string().to_owned()).unwrap_or_default()
    }

    fn trim_pending_story_publishes_if_needed(&mut self, signature: &str) {
        if self.pending_story_publishes.len() <= Self::PENDING_STORY_PUBLISHES_LIMIT {
            return;
        }
        let keys: Vec<String> = self.pending_story_publishes.keys().cloned().collect();
        for key in keys {
            if key == signature {
                continue;
            }
            self.pending_story_publishes.remove(&key);
            if self.pending_story_publishes.len() <= Self::PENDING_STORY_PUBLISHES_LIMIT {
                break;
            }
        }
    }

    pub fn send_story_reply(
        &mut self,
        story: Option<&StoryDTO>,
        author_id: &str,
        author: Option<&StoryAuthorDTO>,
        text: &str,
        on_failure: Option<StoryActionDoneHandler>,
        on_refused: Option<StoryActionDoneHandler>,
        cx: &mut Context<Self>,
    ) {
        let on_failure: StoryActionDoneHandler = on_failure.unwrap_or_else(|| Rc::new(|_: &mut App| {}));
        let on_refused: StoryActionDoneHandler = on_refused.unwrap_or_else(|| Rc::new(|_: &mut App| {}));
        let trimmed = trimming_whitespaces_and_newlines(text).to_owned();
        if trimmed.is_empty() {
            return;
        }

        let id = trimming_whitespaces_and_newlines(author_id).to_owned();
        let story = story.filter(|story| story.id > 0).cloned();
        let Some(story) = story.filter(|_| !id.is_empty()) else {
            cx.defer(move |cx| on_failure(cx));
            return;
        };

        let author = author.cloned();
        let target = User {
            id: id.clone(),
            name: author.as_ref().map(|author| author.name.clone()).unwrap_or_default(),
            tag: author.as_ref().map(|author| author.tag.clone()).unwrap_or_default(),
            more: AuthorInfoMore {
                logo: author.as_ref().map(|author| author.logo.clone()).unwrap_or_default(),
                is_premium: author.as_ref().is_some_and(|author| author.is_premium),
                is_official: false,
                premium_badge_emoji: author.as_ref().and_then(|author| author.premium_badge_emoji.clone()),
                name_color: author.as_ref().and_then(|author| author.name_color.clone()),
                ..AuthorInfoMore::default()
            },
            ..User::default()
        };

        let failure_on_error = on_failure.clone();
        let on_open: Box<dyn FnOnce(String, &mut App)> = Box::new(move |chat_id: String, cx: &mut App| {
            let is_online = SaiWebSocketManager::shared(cx).read(cx).is_connected();
            let completion: Box<dyn FnOnce(anyhow::Result<()>, &mut App)> =
                Box::new(move |result: anyhow::Result<()>, cx: &mut App| match result {
                    Ok(()) => {
                        if !is_online {
                            return;
                        }
                        Notifier::send(
                            tr("story_reply_sent"),
                            NotifierSendOptions {
                                r#type: NotifierType::Success,
                                position: NotifierPosition::Top,
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    }
                    Err(error) => {
                        if error.downcast_ref::<CancellationError>().is_some() {
                            return;
                        }
                        on_refused(cx);
                        Notifier::send(
                            Self::story_reply_refusal_text(&error),
                            NotifierSendOptions {
                                r#type: NotifierType::Error,
                                position: NotifierPosition::Top,
                                ..NotifierSendOptions::default()
                            },
                            cx,
                        );
                    }
                });
            let sent = MessageInteractionsStore::shared(cx).update(cx, |messages, cx| {
                messages.send_story_reply(&chat_id, &trimmed, &story, author.as_ref(), &id, Some(completion), cx)
            });
            if sent.is_none() {
                on_failure(cx);
                return;
            }
            if is_online {
                return;
            }
            let status = ChatsHeaderStatus::resolve(SaiWebSocketManager::shared(cx).read(cx).phase(), false, false, false);
            Notifier::send(
                status.text(),
                NotifierSendOptions {
                    r#type: NotifierType::Warning,
                    position: NotifierPosition::Top,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        let on_error: Box<dyn FnOnce(String, &mut App)> = Box::new(move |_: String, cx: &mut App| failure_on_error(cx));
        cx.defer(move |cx| {
            ChatActionsStore::shared(cx).update(cx, |actions, cx| {
                actions.open_direct_chat_action(target, on_open, Some(on_error), cx);
            });
        });
    }

    fn story_reply_refusal_text(error: &anyhow::Error) -> String {
        let Some(RiseMessageRepositoryError::RemoteFailure(code)) = error.downcast_ref::<RiseMessageRepositoryError>()
        else {
            return tr("error_fetching_data");
        };
        CanonErrors::text(Some(code), &HashMap::new()).unwrap_or_else(|| tr("block_type_chat_desc"))
    }

    /// story caption ceiling per tier from canon,
    /// an over limit caption is refused with the canon phrase not cut
    fn caption_rule() -> RiseFieldRule {
        RiseFieldRule::new("story_create_story", "/caption")
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_story(
        &mut self,
        item: MediaPickerItem,
        caption: String,
        audience: StoryAudience,
        allow_screenshots: bool,
        post_to_profile: bool,
        duration_days: i64,
        selected_user_ids: Vec<String>,
        attached_track: Option<MediaPickerTrackSelection>,
        idempotency_key: Option<String>,
        shared_handoff_context: Option<StorySharePublishContext>,
        on_success: StoryActionDoneHandler,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        tracing::warn!(target: "riseonly::X807", "publish_story busy={} task={} key={:?}", self.is_publishing, self.publish_task.is_some(), idempotency_key);
        if self.is_publishing {
            let message = tr("story_publish_wait_previous");
            self.publish_error = Some(message.clone());
            cx.notify();
            cx.defer(move |cx| on_error(message, cx));
            return;
        }

        let Some(user_id) =
            AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().filter(|user_id| !user_id.is_empty())
        else {
            cx.defer(move |cx| on_error(tr("story_create_error"), cx));
            return;
        };
        let (Some(account_user_id), Some(account_epoch)) =
            (SaiAccountCacheScope::active_user_id(), SaiAccountCacheScope::active_epoch())
        else {
            cx.defer(move |cx| on_error(tr("story_create_error"), cx));
            return;
        };
        if account_user_id != user_id
            || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_user_id), Some(&account_epoch))
        {
            cx.defer(move |cx| on_error(tr("story_create_error"), cx));
            return;
        }

        let explicit_idempotency_key =
            idempotency_key.as_deref().map(|key| trimming_whitespaces_and_newlines(key).to_owned());
        if idempotency_key.is_some()
            && explicit_idempotency_key.as_deref().is_none_or(|key| key.is_empty() || key.len() > 128)
        {
            cx.defer(move |cx| on_error(tr("story_create_error"), cx));
            return;
        }
        if let Some(context) = shared_handoff_context.as_ref()
            && (context.account_id != account_user_id
                || explicit_idempotency_key.as_deref() != Some(context.idempotency_key().as_str()))
        {
            cx.defer(move |cx| on_error(tr("story_create_error"), cx));
            return;
        }

        let is_premium = AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium);
        let caption_rule = Self::caption_rule().for_tier(is_premium);
        let trimmed_caption = trimming_whitespaces_and_newlines(&caption).to_owned();
        if let (Some(maximum), Some(unit)) = (caption_rule.maximum(), caption_rule.unit())
            && unit.count(&trimmed_caption) > maximum
        {
            let caption_error = caption_rule.error_text(&trimmed_caption).unwrap_or_else(|| tr("story_create_error"));
            self.publish_error = Some(caption_error.clone());
            cx.notify();
            cx.defer(move |cx| on_error(caption_error, cx));
            return;
        }

        let is_video = matches!(item.kind, MediaPickerItemKind::Video);
        let canvas_state = StoryCanvasPublish::canvas_state(&item);
        let canvas_animates = canvas_state.as_ref().is_some_and(StoryCanvasPublish::animates);
        let media_type = if is_video || attached_track.is_some() || canvas_animates { "video" } else { "image" };
        let source_url = if is_video { item.video_payload.as_ref().map_or(&item.url, |payload| &payload.url) } else { &item.url };
        let Some(staged_media_url) = source_url.to_file_path().and_then(|path| StoryPublishStaging::stage(&path)) else {
            cx.defer(move |cx| on_error(tr("story_create_error"), cx));
            return;
        };

        self.is_publishing = true;
        self.publish_error = None;

        let clamped_duration = duration_days.clamp(1, 3);
        let selected: Vec<String> = if audience == StoryAudience::Selected {
            let unique: HashSet<String> = selected_user_ids
                .iter()
                .map(|user_id| trimming_whitespaces_and_newlines(user_id).to_owned())
                .filter(|user_id| !user_id.is_empty())
                .collect();
            let mut unique: Vec<String> = unique.into_iter().collect();
            unique.sort();
            unique
        } else {
            Vec::new()
        };
        let base_operation_signature = self.story_publish_signature(
            &account_user_id,
            &item,
            media_type,
            &trimmed_caption,
            audience.wire(),
            allow_screenshots,
            post_to_profile,
            clamped_duration,
            &selected,
        );
        let operation_signature = shared_handoff_context
            .as_ref()
            .map_or(base_operation_signature, |context| format!("shared-story:{}", context.idempotency_key()));

        let durable_checkpoint = if let Some(context) = shared_handoff_context.as_ref() {
            let loaded = match RiseShareContainer::story_publish_checkpoint(&context.session_id, &account_user_id) {
                Ok(loaded) => loaded,
                Err(error) => {
                    Sai::log("StoryShare", move || format!("checkpoint load failed: {error}"), SaiLogKind::Network, cx);
                    StoryPublishStaging::discard(Some(&staged_media_url));
                    self.is_publishing = false;
                    let message = tr("story_share_progress_error_message");
                    self.publish_error = Some(message.clone());
                    cx.notify();
                    cx.defer(move |cx| on_error(message, cx));
                    return;
                }
            };
            if let Some(checkpoint) = loaded.as_ref()
                && !self.story_checkpoint(
                    checkpoint,
                    context,
                    explicit_idempotency_key.as_deref().unwrap_or_default(),
                    media_type,
                    &trimmed_caption,
                    audience.wire(),
                    allow_screenshots,
                    post_to_profile,
                    clamped_duration,
                    &selected,
                )
            {
                StoryPublishStaging::discard(Some(&staged_media_url));
                self.is_publishing = false;
                let message = tr("story_share_progress_error_message");
                self.publish_error = Some(message.clone());
                cx.notify();
                cx.defer(move |cx| on_error(message, cx));
                return;
            }
            self.record_shared_story_publish_request(
                StorySharePublishRequest {
                    caption: caption.clone(),
                    audience,
                    allow_screenshots,
                    post_to_profile,
                    duration_days: clamped_duration,
                    selected_user_ids: selected.clone(),
                    track: attached_track.clone(),
                },
                context,
                cx,
            );
            loaded
        } else {
            None
        };

        let operation = if let Some(checkpoint) = durable_checkpoint.as_ref() {
            let operation = PendingStoryPublish {
                idempotency_key: checkpoint.idempotency_key.clone(),
                remote_file_url: Some(checkpoint.remote_file_url.clone()),
            };
            self.pending_story_publishes.insert(operation_signature.clone(), operation.clone());
            operation
        } else if let Some(pending) = self.pending_story_publishes.get(&operation_signature)
            && explicit_idempotency_key.as_ref().is_none_or(|key| *key == pending.idempotency_key)
        {
            pending.clone()
        } else {
            let pending = PendingStoryPublish {
                idempotency_key: explicit_idempotency_key.clone().unwrap_or_else(|| Uuid::new_v4().uuid_string().to_lowercase()),
                remote_file_url: None,
            };
            self.pending_story_publishes.insert(operation_signature.clone(), pending.clone());
            self.trim_pending_story_publishes_if_needed(&operation_signature);
            pending
        };

        let mut pending_body = rise_request::story::CreateStory {
            allow_screenshots: Some(allow_screenshots),
            audience: Some(audience.wire().to_owned()),
            caption: (!trimmed_caption.is_empty()).then(|| trimmed_caption.clone()),
            duration_days: Some(clamped_duration),
            excluded_user_ids: Some(Vec::new()),
            file_id: Some(Self::file_url_string(&staged_media_url)),
            idempotency_key: Some(operation.idempotency_key.clone()),
            media_type: Some(media_type.to_owned()),
            post_to_profile: Some(post_to_profile),
            selected_user_ids: Some(selected.clone()),
            ..rise_request::story::CreateStory::default()
        };
        self.apply_track(attached_track.as_ref(), &mut pending_body);

        self.pending_publish_signature = Some(operation_signature.clone());
        self.persist_story_reedit_document(&item, &operation_signature, cx);
        let phase = if media_type == "video" { StoryPendingPublishPhase::Preparing } else { StoryPendingPublishPhase::Uploading };
        self.begin_pending_story_publish(&pending_body, &user_id, staged_media_url.clone(), is_video, phase, cx);
        self.seed_pending_media_aspect(&item, &Self::file_url_string(&staged_media_url));

        let audience_wire = audience.wire().to_owned();
        self.publish_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let is_current = |cx: &mut AsyncApp| {
                this.read_with(cx, |store, _| {
                    store.is_current_publish(&operation_signature, &account_user_id, &account_epoch)
                })
                .unwrap_or(false)
            };
            let mut upload_url = staged_media_url.clone();
            let mut rendered_media_url: Option<PathBuf> = None;
            let mut media_item = item.clone();

            let needs_render = |cx: &mut AsyncApp| {
                this.read_with(cx, |store, _| {
                    store
                        .pending_story_publishes
                        .get(&operation_signature)
                        .is_none_or(|pending| pending.remote_file_url.is_none())
                })
                .unwrap_or(false)
            };
            if canvas_animates
                && let Some(canvas_state) = canvas_state.clone()
                && needs_render(cx)
            {
                let video = cx
                    .update(|cx| StoryCanvasPublish::animated_video_item(item.clone(), canvas_state, cx))
                    .await;
                match video {
                    Ok(video) => {
                        if !is_current(cx) {
                            return;
                        }
                        let video_path = video.url.to_file_path();
                        let staged = video_path.as_deref().and_then(StoryPublishStaging::stage);
                        let staged_url = staged.as_deref().and_then(Url::from_file_path);
                        let (Some(staged), Some(staged_url)) = (staged, staged_url) else {
                            let on_error = on_error.clone();
                            let _ = this.update(cx, |store, cx| {
                                store.resolve_failed_story_publish_message(
                                    tr("story_create_error"),
                                    &operation_signature,
                                    on_error,
                                    cx,
                                );
                            });
                            return;
                        };
                        if let Some(path) = video_path.as_deref() {
                            StoryPublishStaging::discard_temporary(path);
                        }
                        rendered_media_url = Some(staged.clone());
                        upload_url = staged.clone();
                        let video_payload = video.video_payload.as_ref().map(|payload| {
                            let mut edited = MediaPickerEditedVideo::new(
                                staged_url.clone(),
                                payload.start_seconds,
                                payload.end_seconds,
                                payload.duration,
                                payload.is_muted,
                                payload.caption.clone(),
                            );
                            edited.pixel_size = payload.pixel_size;
                            edited.poster_url = payload.poster_url.clone();
                            edited.spatial_edit = None;
                            edited
                        });
                        media_item = MediaPickerItem::new(
                            video.id.clone(),
                            MediaPickerItemKind::Video,
                            staged_url.clone(),
                            staged_url.last_path_component(),
                            None,
                            None,
                            video_payload,
                            None,
                            false,
                        );
                        if attached_track.is_none() {
                            let _ = this.update(cx, |store, cx| store.adopt_rendered_pending_media(staged, cx));
                        }
                    }
                    Err(error) => {
                        let on_error = on_error.clone();
                        let _ = this.update(cx, |store, cx| {
                            store.resolve_failed_story_publish(
                                &error,
                                &account_user_id,
                                &account_epoch,
                                &operation_signature,
                                on_error,
                                cx,
                            );
                        });
                        return;
                    }
                }
            }

            if let Some(track) = attached_track.as_ref()
                && needs_render(cx)
            {
                match StoryMusicMediaRenderer::render(media_item.clone(), &track.track, track.start_ms, track.end_ms, cx).await {
                    Ok(rendered) => {
                        if !is_current(cx) {
                            return;
                        }
                        let rendered_path = rendered.url.to_file_path();
                        let Some(staged) = rendered_path.as_deref().and_then(StoryPublishStaging::stage) else {
                            let on_error = on_error.clone();
                            let _ = this.update(cx, |store, cx| {
                                store.resolve_failed_story_publish_message(
                                    tr("story_create_error"),
                                    &operation_signature,
                                    on_error,
                                    cx,
                                );
                            });
                            return;
                        };
                        if let Some(path) = rendered_path.as_deref() {
                            StoryPublishStaging::discard_temporary(path);
                        }
                        rendered_media_url = Some(staged.clone());
                        upload_url = staged.clone();
                        let _ = this.update(cx, |store, cx| store.adopt_rendered_pending_media(staged, cx));
                    }
                    Err(error) => {
                        let on_error = on_error.clone();
                        let _ = this.update(cx, |store, cx| {
                            store.resolve_failed_story_publish(
                                &error,
                                &account_user_id,
                                &account_epoch,
                                &operation_signature,
                                on_error,
                                cx,
                            );
                        });
                        return;
                    }
                }
            }

            let content_type = if media_type == "video" {
                "video/mp4".to_owned()
            } else {
                let upload_extension = upload_url.extension().and_then(|extension| extension.to_str()).unwrap_or_default();
                let path_extension = if upload_extension.is_empty() {
                    Path::new(&item.suggested_file_name)
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .unwrap_or_default()
                } else {
                    upload_extension
                };
                UTType::from_filename_extension(path_extension)
                    .filter(|r#type| r#type.conforms_to(&UTType::image()))
                    .and_then(|r#type| r#type.preferred_mime_type())
                    .unwrap_or_else(|| "image/jpeg".to_owned())
            };
            let upload_filename = if media_type == "video" && rendered_media_url.is_some() {
                upload_url.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_owned()
            } else {
                item.suggested_file_name.clone()
            };

            let mut uploaded_file_url = this
                .read_with(cx, |store, _| {
                    store.pending_story_publishes.get(&operation_signature).and_then(|pending| pending.remote_file_url.clone())
                })
                .ok()
                .flatten();
            while uploaded_file_url.is_none() {
                let attempt: anyhow::Result<Option<String>> = async {
                    cx.update(AuthTokenRefresh::ensure_valid_access_token).await?;
                    if !is_current(cx) {
                        return Ok(None);
                    }
                    let compresses_as_preparing = media_type == "video";
                    let upload_share = if compresses_as_preparing { 0.0 } else { 0.15 };
                    this.update(cx, |store, cx| {
                        store.set_pending_story_publish_phase(
                            if compresses_as_preparing {
                                StoryPendingPublishPhase::Preparing
                            } else {
                                StoryPendingPublishPhase::Uploading
                            },
                            cx,
                        );
                    })?;
                    let mut original =
                        RiseOutgoingMediaFile::new(upload_url.clone(), upload_filename.clone(), content_type.clone());
                    original.file_context = Some("story".to_owned());
                    original.media_type = Some(media_type.to_owned());
                    let compression_progress: Arc<dyn Fn(f64) + Send + Sync> = {
                        let account_user_id = account_user_id.clone();
                        let account_epoch = account_epoch.clone();
                        Arc::new(move |fraction: f64| {
                            if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_user_id), Some(&account_epoch)) {
                                return;
                            }
                            if compresses_as_preparing {
                                StoryUploadProgressCenter::publish(fraction, StoryUploadPhase::Preparing);
                            } else {
                                StoryUploadProgressCenter::publish(fraction * upload_share, StoryUploadPhase::Uploading);
                            }
                        })
                    };
                    let compressed = OutgoingMediaCompressor::compress_if_needed(
                        original.clone(),
                        None,
                        MediaCompressionConfiguration::publication(),
                        Some(compression_progress),
                        None,
                    )
                    .await?;
                    let compressed_path = compressed.url.to_file_path();
                    let generated_temporary_url = compressed_path.clone().filter(|path| *path != upload_url);
                    let prepared = RiseConversationUploadSupport::prepared_local_file(&original, &compressed);
                    let outcome: anyhow::Result<Option<String>> = async {
                        if !is_current(cx) {
                            return Ok(None);
                        }
                        this.update(cx, |store, cx| {
                            store.set_pending_story_publish_phase(StoryPendingPublishPhase::Uploading, cx);
                        })?;
                        let Some(compressed_path) = compressed_path.clone() else {
                            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
                        };
                        let on_progress: SaiWsUploadOnProgress = {
                            let account_user_id = account_user_id.clone();
                            let account_epoch = account_epoch.clone();
                            Rc::new(move |fraction: f64, _: &mut App| {
                                if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_user_id), Some(&account_epoch)) {
                                    return;
                                }
                                StoryUploadProgressCenter::publish(
                                    upload_share + fraction * (1.0 - upload_share),
                                    StoryUploadPhase::Uploading,
                                );
                            })
                        };
                        let upload = cx.update(|cx| {
                            SaiWsChunkedFileUpload::upload(
                                compressed_path,
                                prepared.filename.clone(),
                                compressed.content_type.clone(),
                                "story".to_owned(),
                                None,
                                Some(account_user_id.clone()),
                                Some(account_epoch.clone()),
                                Some(on_progress),
                                None,
                                cx,
                            )
                        });
                        let done = upload.await?;
                        let Some(resolved) = StoryUploadWire::remote_file_id(&done) else {
                            return Err(SaiWsOptimisticError::FileUploadInvalidSlotPayload.into());
                        };
                        Ok(Some(resolved))
                    }
                    .await;
                    if let Some(path) = generated_temporary_url.as_deref() {
                        StoryPublishStaging::discard_temporary(path);
                    }
                    outcome
                }
                .await;

                match attempt {
                    Ok(None) => return,
                    Ok(Some(resolved)) => {
                        let _ = this.update(cx, |store, _| {
                            if let Some(pending) = store.pending_story_publishes.get_mut(&operation_signature)
                                && pending.idempotency_key == operation.idempotency_key
                            {
                                pending.remote_file_url = Some(resolved.clone());
                            }
                        });
                        uploaded_file_url = Some(resolved);
                    }
                    Err(error) => {
                        if !Self::is_cancellation(&error)
                            && cx.update(|cx| StoryFailureText::is_transport_failure(&error, cx))
                            && Self::park_story_publish_until_network(
                                &this,
                                &operation_signature,
                                &account_user_id,
                                &account_epoch,
                                cx,
                            )
                            .await
                        {
                            continue;
                        }
                        let on_error = on_error.clone();
                        let _ = this.update(cx, |store, cx| {
                            store.resolve_failed_story_publish(
                                &error,
                                &account_user_id,
                                &account_epoch,
                                &operation_signature,
                                on_error,
                                cx,
                            );
                        });
                        return;
                    }
                }
            }
            let Some(file_url) = uploaded_file_url else {
                return;
            };
            if !is_current(cx) {
                return;
            }

            let caption_value = (!trimmed_caption.is_empty()).then(|| trimmed_caption.clone());
            let checkpoint = if let Some(context) = shared_handoff_context.as_ref() {
                let candidate = RiseShareStoryPublishCheckpoint {
                    item_id: context.item_id.clone(),
                    idempotency_key: operation.idempotency_key.clone(),
                    remote_file_url: file_url.clone(),
                    media_type: media_type.to_owned(),
                    caption: caption_value.clone(),
                    audience: audience_wire.clone(),
                    allow_screenshots,
                    post_to_profile,
                    duration_days: clamped_duration,
                    excluded_user_ids: Vec::new(),
                    selected_user_ids: selected.clone(),
                };
                match RiseShareContainer::store_story_publish_checkpoint(&context.session_id, &account_user_id, candidate) {
                    Ok(stored) => Some(stored),
                    Err(error) => {
                        if !is_current(cx) {
                            return;
                        }
                        let on_error = on_error.clone();
                        let _ = this.update(cx, |store, cx| {
                            Sai::log("StoryShare", move || format!("checkpoint write failed: {error}"), SaiLogKind::Network, cx);
                            store.resolve_failed_story_publish_message(
                                tr("story_share_progress_error_message"),
                                &operation_signature,
                                on_error,
                                cx,
                            );
                        });
                        return;
                    }
                }
            } else {
                None
            };

            let body = this.update(cx, |store, _| {
                let mut body = checkpoint.as_ref().map(|checkpoint| store.story_create_body(&user_id, checkpoint)).unwrap_or_else(|| {
                    rise_request::story::CreateStory {
                        allow_screenshots: Some(allow_screenshots),
                        audience: Some(audience_wire.clone()),
                        caption: caption_value.clone(),
                        duration_days: Some(clamped_duration),
                        excluded_user_ids: Some(Vec::new()),
                        file_id: Some(file_url.clone()),
                        idempotency_key: Some(operation.idempotency_key.clone()),
                        media_type: Some(media_type.to_owned()),
                        post_to_profile: Some(post_to_profile),
                        selected_user_ids: Some(selected.clone()),
                        ..rise_request::story::CreateStory::default()
                    }
                });
                store.apply_track(attached_track.as_ref(), &mut body);
                body
            });
            let Ok(mut body) = body else {
                return;
            };
            if let Some(canvas_state) = canvas_state.clone()
                && shared_handoff_context.is_none()
            {
                body.link_areas = Some(StoryCanvasPublish::link_areas(&canvas_state));
                body.canvas = cx
                    .update(|cx| {
                        StoryCanvasPublish::upload_canvas(canvas_state, account_user_id.clone(), account_epoch.clone(), cx)
                    })
                    .await
                    .ok()
                    .flatten();
                if !is_current(cx) {
                    return;
                }
            }
            let _ = this.update(cx, |store, cx| {
                store.submit_story_create(
                    body,
                    &user_id,
                    &account_epoch,
                    &operation_signature,
                    on_success.clone(),
                    on_error.clone(),
                    cx,
                );
            });
        }));
        cx.notify();
    }

    fn begin_pending_story_publish(
        &mut self,
        body: &rise_request::story::CreateStory,
        user_id: &str,
        staged_media_url: PathBuf,
        pending_is_video: bool,
        phase: StoryPendingPublishPhase,
        cx: &mut Context<Self>,
    ) {
        self.unwind_cancelled_story_publish_if_needed(cx);
        self.release_settled_publish_media(cx);
        StoryPublishStaging::discard_stale_files();
        self.note_user_stories_target(user_id, cx);
        let temporary_story = self.make_temporary_story(body, user_id, cx);
        let user_stories_key = Self::user_stories_cache_key(user_id);
        let feed_key = Self::feed_cache_key();
        let profile_key = Self::profile_stories_cache_key(user_id, StoryProfileScope::Profile);
        let mutation = self.begin_story_mutation(
            HashMap::from([
                (user_stories_key, Self::upsert_story_transform(temporary_story.clone(), None)),
                (feed_key, Self::upsert_feed_story_transform(temporary_story.clone(), None)),
                (profile_key, Self::upsert_published_profile_story_transform(temporary_story.clone(), None)),
            ]),
            cx,
        );
        self.pending_publish_mutation = Some(mutation);
        self.pending_publish_staged_url = Some(staged_media_url.clone());
        self.pending_publish_lease = Some((
            begin_process_activity("riseonly.story.publish"),
            QuitDrain::lease("riseonly.story.publish", cx),
        ));
        self.pending_publish_unwinds_on_player_dismiss = false;
        self.pending_story_publish = Some(StoryPendingPublish {
            story_id: temporary_story.id,
            author_id: user_id.to_owned(),
            media_source: Self::file_url_string(&staged_media_url),
            is_video: pending_is_video,
            phase,
        });

        StoryUploadProgressCenter::begin(cx);
        StoryUploadProgressCenter::publish(
            0.0,
            if phase == StoryPendingPublishPhase::Preparing { StoryUploadPhase::Preparing } else { StoryUploadPhase::Uploading },
        );

        self.known_story_state_user_ids.insert(user_id.to_owned());
        self.active_story_user_ids.insert(user_id.to_owned());
        self.locally_seen_story_user_ids.remove(user_id);
        self.reconcile_story_state_from_caches(user_id, cx);
        cx.notify();
    }

    fn is_current_publish(&self, signature: &str, account_user_id: &str, account_epoch: &str) -> bool {
        if !SaiAccountCacheScope::is_current_account_epoch(Some(account_user_id), Some(account_epoch)) {
            return false;
        }
        self.pending_publish_signature.as_deref() == Some(signature)
            && self.pending_story_publish.as_ref().is_some_and(StoryPendingPublish::is_active)
    }

    fn resolve_failed_story_publish(
        &mut self,
        error: &anyhow::Error,
        account_user_id: &str,
        account_epoch: &str,
        signature: &str,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        if Self::is_cancellation(error) {
            return;
        }
        if !SaiAccountCacheScope::is_current_account_epoch(Some(account_user_id), Some(account_epoch))
            || self.pending_publish_signature.as_deref() != Some(signature)
        {
            return;
        }
        tracing::warn!(target: "riseonly::X1487", "publish story failed: {error:#} debug={error:?}");
        let description = error.to_string();
        Sai::log("Story", move || format!("publish story upload error: {description}"), SaiLogKind::Network, cx);
        self.resolve_failed_story_publish_message(StoryFailureText::message(error), signature, on_error, cx);
    }

    fn resolve_failed_story_publish_message(
        &mut self,
        message: String,
        signature: &str,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        if self.pending_publish_signature.as_deref() != Some(signature) {
            return;
        }
        tracing::warn!(target: "riseonly::X1487", "publish story resolved as failed message={message}");
        self.pending_publish_signature = None;
        self.publish_task = None;
        self.set_pending_story_publish_phase(StoryPendingPublishPhase::Failed, cx);
        self.unwind_pending_story_publish(cx);
        StoryUploadProgressCenter::clear(cx);
        self.is_publishing = false;
        self.publish_error = Some(message.clone());
        cx.notify();
        cx.defer(move |cx| on_error(message, cx));
    }

    /// holds a publish whose transfer lost the network
    /// row keeps its ring and says it waits, the step repeats when the socket is back
    /// false if cancelled or the account changed
    async fn park_story_publish_until_network(
        this: &WeakEntity<Self>,
        signature: &str,
        account_user_id: &str,
        account_epoch: &str,
        cx: &mut AsyncApp,
    ) -> bool {
        let is_current = |cx: &mut AsyncApp| {
            this.read_with(cx, |store, _| store.is_current_publish(signature, account_user_id, account_epoch))
                .unwrap_or(false)
        };
        if !is_current(cx) {
            return false;
        }
        let parked = this
            .update(cx, |store, cx| {
                Sai::log("Story", || "publish parked until the network is back".to_owned(), SaiLogKind::Network, cx);
                store.is_publish_parked_on_network = true;
                cx.notify();
                RiseTransportStatus::wake_if_offline(cx);
            })
            .is_ok();
        if !parked {
            return false;
        }
        cx.background_executor().timer(Self::PUBLISH_NETWORK_PAUSE).await;
        let mut resumed = true;
        let bounced = cx.update(RiseTransportStatus::is_connected);
        let _ = this.update(cx, |store, _| {
            store.publish_transport_bounces = if bounced { store.publish_transport_bounces + 1 } else { 0 };
        });
        while !cx.update(RiseTransportStatus::is_connected) {
            if !is_current(cx) {
                resumed = false;
                break;
            }
            cx.update(RiseTransportStatus::wake_if_offline);
            cx.background_executor().timer(Self::PUBLISH_NETWORK_POLL).await;
        }
        let _ = this.update(cx, |store, cx| {
            store.is_publish_parked_on_network = false;
            cx.notify();
        });
        resumed && is_current(cx)
    }

    fn is_cancellation(error: &anyhow::Error) -> bool {
        error.downcast_ref::<CancellationError>().is_some() || error.downcast_ref::<SaiWsOutboundCancelledError>().is_some()
    }

    fn seed_pending_media_aspect(&self, item: &MediaPickerItem, source: &str) {
        let size = item
            .video_payload
            .as_ref()
            .map(|payload| (payload.pixel_size.width, payload.pixel_size.height))
            .or_else(|| item.image_payload.as_ref().map(|payload| (payload.pixel_size.width, payload.pixel_size.height)));
        let Some((width, height)) = size.filter(|(width, height)| *width > 0.0 && *height > 0.0) else {
            return;
        };
        StoryImageAspectProbe::seed(width / height, source);
    }

    fn apply_track(&self, attached_track: Option<&MediaPickerTrackSelection>, body: &mut rise_request::story::CreateStory) {
        let Some(attached_track) = attached_track else {
            return;
        };
        body.track_id = attached_track.track.id.clone();
        body.track_start_ms = Some(attached_track.start_ms);
        body.track_end_ms = Some(attached_track.end_ms);
        body.track_plate = StoryTrackPlateWire::plate(
            attached_track.track.id.as_deref(),
            attached_track.plate.as_ref(),
            &attached_track.track,
        );
    }

    fn adopt_staged_media_into_caches(&mut self, remote_file_id: Option<&str>, story_id: i64, cx: &mut Context<Self>) {
        let Some(staged) = self.pending_publish_staged_url.take() else {
            return;
        };
        let normalized = remote_file_id
            .and_then(|remote_file_id| RemoteMediaURL::optional_normalized(Some(trimming_whitespaces_and_newlines(remote_file_id))))
            .filter(|normalized| normalized.to_lowercase().starts_with("http"));
        if let Some(normalized) = normalized.as_deref() {
            StoryImageAspectProbe::copy_seed(&Self::file_url_string(&staged), normalized);
        }
        let media = SettledStoryPublishMedia {
            story_id,
            staged_url: staged,
            is_video: self.pending_story_publish.as_ref().is_some_and(|pending| pending.is_video),
            remote_key: normalized,
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
        };
        self.release_settled_publish_media(cx);
        if self.player_request.is_some() {
            self.settled_publish_media = Some(media);
        } else {
            Self::file_staged_media(media, cx);
        }
    }

    pub fn adopt_reedited_media(
        &mut self,
        staged_url: PathBuf,
        is_video: bool,
        aspect: Option<f32>,
        story: &StoryDTO,
        cx: &mut Context<Self>,
    ) {
        let normalized = RemoteMediaURL::optional_normalized(Some(trimming_whitespaces_and_newlines(&story.file_id)))
            .filter(|normalized| normalized.to_lowercase().starts_with("http"));
        if let Some(aspect) = aspect {
            StoryImageAspectProbe::seed(aspect, &Self::file_url_string(&staged_url));
            if let Some(normalized) = normalized.as_deref() {
                StoryImageAspectProbe::seed(aspect, normalized);
            }
        }
        let media = SettledStoryPublishMedia {
            story_id: story.id,
            staged_url,
            is_video,
            remote_key: normalized,
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
        };
        self.release_settled_publish_media(cx);
        if self.player_request.is_some() {
            self.settled_publish_media = Some(media);
        } else {
            Self::file_staged_media(media, cx);
        }
    }

    /// moves the staged bytes into the cache the committed address reads or drops them,
    /// one owner so nothing is deleted while read
    fn file_staged_media(media: SettledStoryPublishMedia, _cx: &mut App) {
        let staged = media.staged_url;
        let Some(normalized) = media.remote_key else {
            StoryPublishStaging::discard(Some(&staged));
            return;
        };
        let Some(remote) = Url::from_string(&normalized) else {
            StoryPublishStaging::discard(Some(&staged));
            return;
        };
        let account_user_id = media.account_user_id;
        let account_epoch = media.account_epoch;

        if media.is_video {
            let cache_key = StoryMediaPrefetcher::video_cache_key(&remote);
            drop(RiseAccountRuntime::current().spawn_blocking(move || {
                let suggested_filename = staged.file_name().and_then(|name| name.to_str()).map(str::to_owned);
                let adopted = RemoteMediaDiskCache::shared().adopt(
                    &staged,
                    &remote,
                    Some(&cache_key),
                    suggested_filename.as_deref(),
                    None,
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                );
                if adopted.is_err() {
                    StoryPublishStaging::discard(Some(&staged));
                }
            }));
            return;
        }

        let invalidation_token = CleverImageDiskCache::invalidation_token(account_user_id.as_deref());
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            if SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
                && let Ok(data) = std::fs::read(&staged)
                && !data.is_empty()
            {
                CleverImageDiskCache::store_data(&data, &normalized, account_user_id.as_deref(), Some(invalidation_token));
            }
            StoryPublishStaging::discard(Some(&staged));
        }));
    }

    pub fn resume_persisted_shared_story_publish(
        &mut self,
        context: StorySharePublishContext,
        expected_checkpoint: RiseShareStoryPublishCheckpoint,
        on_success: StoryActionDoneHandler,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let (false, Some(user_id), Some(account_user_id), Some(account_epoch)) = (
            self.is_publishing,
            user_id,
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
        ) else {
            cx.defer(move |cx| on_error(tr("story_create_error"), cx));
            return;
        };
        if user_id != context.account_id
            || account_user_id != user_id
            || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_user_id), Some(&account_epoch))
        {
            cx.defer(move |cx| on_error(tr("story_create_error"), cx));
            return;
        }

        let persisted = RiseShareContainer::story_publish_checkpoint(&context.session_id, &account_user_id).and_then(
            |persisted| match persisted {
                Some(persisted)
                    if persisted == expected_checkpoint
                        && persisted.item_id == context.item_id
                        && persisted.idempotency_key == context.idempotency_key() =>
                {
                    Ok(persisted)
                }
                _ => Err(RiseShareContainerError::InvalidManifest),
            },
        );
        let checkpoint = match persisted {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                Sai::log("StoryShare", move || format!("checkpoint resume failed: {error}"), SaiLogKind::Network, cx);
                let message = tr("story_share_progress_error_message");
                self.publish_error = Some(message.clone());
                cx.notify();
                cx.defer(move |cx| on_error(message, cx));
                return;
            }
        };

        self.is_publishing = true;
        self.publish_error = None;
        StoryUploadProgressCenter::begin(cx);
        StoryUploadProgressCenter::publish(1.0, StoryUploadPhase::Processing);
        let operation_signature = format!("shared-story:{}", checkpoint.idempotency_key);
        self.pending_story_publishes.insert(
            operation_signature.clone(),
            PendingStoryPublish {
                idempotency_key: checkpoint.idempotency_key.clone(),
                remote_file_url: Some(checkpoint.remote_file_url.clone()),
            },
        );
        let body = self.story_create_body(&user_id, &checkpoint);
        self.submit_story_create(body, &user_id, &account_epoch, &operation_signature, on_success, on_error, cx);
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_story_create(
        &mut self,
        body: rise_request::story::CreateStory,
        user_id: &str,
        account_epoch: &str,
        operation_signature: &str,
        on_success: StoryActionDoneHandler,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        tracing::warn!(target: "riseonly::X807", "submit create signature={operation_signature} pending={:?} current={:?}", self.pending_story_publish.as_ref().map(|pending| pending.phase), self.pending_publish_signature);
        self.note_user_stories_target(user_id, cx);
        let user_stories_key = Self::user_stories_cache_key(user_id);
        let feed_key = Self::feed_cache_key();
        let profile_key = Self::profile_stories_cache_key(user_id, StoryProfileScope::Profile);

        let existing = self
            .pending_story_publish
            .as_ref()
            .filter(|pending| pending.author_id == user_id)
            .map(|pending| pending.story_id)
            .zip(self.pending_publish_mutation.clone());
        let (mutation, temporary_story_id) = if let Some((story_id, existing)) = existing {
            self.set_pending_story_publish_phase(StoryPendingPublishPhase::Creating, cx);
            (existing, story_id)
        } else {
            let temporary_story = self.make_temporary_story(&body, user_id, cx);
            let mutation = self.begin_story_mutation(
                HashMap::from([
                    (user_stories_key.clone(), Self::upsert_story_transform(temporary_story.clone(), None)),
                    (feed_key.clone(), Self::upsert_feed_story_transform(temporary_story.clone(), None)),
                    (profile_key.clone(), Self::upsert_published_profile_story_transform(temporary_story.clone(), None)),
                ]),
                cx,
            );
            self.known_story_state_user_ids.insert(user_id.to_owned());
            self.active_story_user_ids.insert(user_id.to_owned());
            self.reconcile_story_state_from_caches(user_id, cx);
            (mutation, temporary_story.id)
        };

        let this = cx.weak_entity();
        let user_id = user_id.to_owned();
        let account_epoch = account_epoch.to_owned();
        let operation_signature = operation_signature.to_owned();

        let on_story: StoryActionStoryHandler = {
            let this = this.clone();
            let mutation = mutation.clone();
            let user_id = user_id.clone();
            let account_epoch = account_epoch.clone();
            let operation_signature = operation_signature.clone();
            let file_id = body.file_id.clone();
            let on_success = on_success.clone();
            Rc::new(move |story: StoryDTO, cx: &mut App| {
                let _ = this.update(cx, |store, cx| {
                    if !SaiAccountCacheScope::is_current_account_epoch(Some(&user_id), Some(&account_epoch)) {
                        store.rollback_story_mutation(&mutation, cx);
                        store.finish_pending_story_publish(false, cx);
                        store.is_publishing = false;
                        cx.notify();
                        return;
                    }
                    store.pending_story_publishes.remove(&operation_signature);
                    tracing::warn!(target: "riseonly::X807", "created story={} signature={operation_signature} cancelled={:?}", story.id, store.cancelled_publish_signature);
                    if store.cancelled_publish_signature.as_deref() == Some(operation_signature.as_str()) {
                        store.cancelled_publish_signature = None;
                        store.finish_pending_story_publish(false, cx);
                        store.rollback_story_mutation(&mutation, cx);
                        let story_id = story.id;
                        Self::with_actions(cx, move |actions, cx| actions.delete_story_action(story_id, None, cx));
                        return;
                    }
                    store.is_publishing = false;
                    store.publish_error = None;
                    store.publish_transport_bounces = 0;
                    store.bind_story_reedit_document(&operation_signature, story.id, cx);
                    store.adopt_staged_media_into_caches(file_id.as_deref(), story.id, cx);
                    store.commit_story_mutation(
                        &mutation,
                        HashMap::from([
                            (
                                Self::user_stories_cache_key(&user_id),
                                Self::upsert_story_transform(story.clone(), Some(temporary_story_id)),
                            ),
                            (
                                Self::feed_cache_key(),
                                Self::upsert_feed_story_transform(story.clone(), Some(temporary_story_id)),
                            ),
                            (
                                Self::profile_stories_cache_key(&user_id, StoryProfileScope::Profile),
                                Self::upsert_published_profile_story_transform(story.clone(), Some(temporary_story_id)),
                            ),
                        ]),
                        cx,
                    );
                    store.finish_pending_story_publish(true, cx);
                    store.known_story_state_user_ids.insert(user_id.clone());
                    store.active_story_user_ids.insert(user_id.clone());
                    store.locally_seen_story_user_ids.remove(&user_id);
                    store.reconcile_story_state_from_caches(&user_id, cx);
                    GuideSignalBus::emit(RiseGuideSignal::StoryCreated);
                    let on_success = on_success.clone();
                    cx.defer(move |cx| on_success(cx));
                });
            })
        };

        let on_transport_drop: StoryActionDoneHandler = {
            let this = this.clone();
            let mutation = mutation.clone();
            let user_id = user_id.clone();
            let account_epoch = account_epoch.clone();
            let operation_signature = operation_signature.clone();
            let body = body.clone();
            let on_success = on_success.clone();
            let on_error = on_error.clone();
            Rc::new(move |cx: &mut App| {
                let _ = this.update(cx, |store, cx| {
                    let owns_parked_row = store.pending_publish_signature.as_deref() == Some(operation_signature.as_str())
                        && store.pending_story_publish.as_ref().is_some_and(|pending| pending.story_id == temporary_story_id);
                    if !owns_parked_row {
                        store.fail_story_create(
                            StoryFailureText::connection_text(),
                            &user_id,
                            &account_epoch,
                            &operation_signature,
                            &mutation,
                            on_error.clone(),
                            cx,
                        );
                        return;
                    }
                    let mutation = mutation.clone();
                    let user_id = user_id.clone();
                    let account_epoch = account_epoch.clone();
                    let operation_signature = operation_signature.clone();
                    let body = body.clone();
                    let on_success = on_success.clone();
                    let on_error = on_error.clone();
                    store.publish_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
                        let resumed = Self::park_story_publish_until_network(
                            &this,
                            &operation_signature,
                            &user_id,
                            &account_epoch,
                            cx,
                        )
                        .await;
                        let _ = this.update(cx, |store, cx| {
                            if !resumed {
                                if store.cancelled_publish_signature.as_deref() == Some(operation_signature.as_str()) {
                                    store.cancelled_publish_signature = None;
                                } else if !SaiAccountCacheScope::is_current_account_epoch(Some(&user_id), Some(&account_epoch)) {
                                    store.fail_story_create(
                                        StoryFailureText::connection_text(),
                                        &user_id,
                                        &account_epoch,
                                        &operation_signature,
                                        &mutation,
                                        on_error.clone(),
                                        cx,
                                    );
                                }
                                return;
                            }
                            if store.publish_transport_bounces > Self::PUBLISH_TRANSPORT_BOUNCE_LIMIT {
                                tracing::warn!(target: "riseonly::X807", "create keeps dropping with the socket up, failing signature={operation_signature} bounces={}", store.publish_transport_bounces);
                                store.publish_transport_bounces = 0;
                                store.fail_story_create(
                                    tr("story_create_error"),
                                    &user_id,
                                    &account_epoch,
                                    &operation_signature,
                                    &mutation,
                                    on_error.clone(),
                                    cx,
                                );
                                return;
                            }
                            store.submit_story_create(
                                body.clone(),
                                &user_id,
                                &account_epoch,
                                &operation_signature,
                                on_success.clone(),
                                on_error.clone(),
                                cx,
                            );
                        });
                    }));
                });
            })
        };

        let on_failure: StoryActionTextHandler = Rc::new(move |message: String, cx: &mut App| {
            let _ = this.update(cx, |store, cx| {
                store.fail_story_create(
                    message,
                    &user_id,
                    &account_epoch,
                    &operation_signature,
                    &mutation,
                    on_error.clone(),
                    cx,
                );
            });
        });

        Self::with_actions(cx, move |actions, cx| {
            actions.create_story_action(body, on_story, Some(on_transport_drop), on_failure, cx);
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn fail_story_create(
        &mut self,
        message: String,
        user_id: &str,
        account_epoch: &str,
        operation_signature: &str,
        mutation: &StoryMutationToken,
        on_error: StoryActionTextHandler,
        cx: &mut Context<Self>,
    ) {
        if !SaiAccountCacheScope::is_current_account_epoch(Some(user_id), Some(account_epoch)) {
            self.rollback_story_mutation(mutation, cx);
            self.finish_pending_story_publish(false, cx);
            self.is_publishing = false;
            cx.notify();
            return;
        }
        if self.cancelled_publish_signature.as_deref() == Some(operation_signature) {
            self.cancelled_publish_signature = None;
            self.finish_pending_story_publish(false, cx);
            self.rollback_story_mutation(mutation, cx);
            return;
        }
        self.rollback_story_mutation(mutation, cx);
        self.finish_pending_story_publish(false, cx);
        self.reconcile_story_state_from_caches(user_id, cx);
        self.is_publishing = false;
        self.publish_error = Some(message.clone());
        cx.notify();
        cx.defer(move |cx| on_error(message, cx));
    }

    fn story_create_body(
        &self,
        _user_id: &str,
        checkpoint: &RiseShareStoryPublishCheckpoint,
    ) -> rise_request::story::CreateStory {
        rise_request::story::CreateStory {
            allow_screenshots: Some(checkpoint.allow_screenshots),
            audience: Some(checkpoint.audience.clone()),
            caption: checkpoint.caption.clone(),
            duration_days: Some(checkpoint.duration_days),
            excluded_user_ids: Some(checkpoint.excluded_user_ids.clone()),
            file_id: Some(checkpoint.remote_file_url.clone()),
            idempotency_key: Some(checkpoint.idempotency_key.clone()),
            media_type: Some(checkpoint.media_type.clone()),
            post_to_profile: Some(checkpoint.post_to_profile),
            selected_user_ids: Some(checkpoint.selected_user_ids.clone()),
            ..rise_request::story::CreateStory::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn story_checkpoint(
        &self,
        checkpoint: &RiseShareStoryPublishCheckpoint,
        context: &StorySharePublishContext,
        idempotency_key: &str,
        media_type: &str,
        caption: &str,
        audience: &str,
        allow_screenshots: bool,
        post_to_profile: bool,
        duration_days: i64,
        selected_user_ids: &[String],
    ) -> bool {
        checkpoint.item_id == context.item_id
            && checkpoint.idempotency_key == idempotency_key
            && !trimming_whitespaces_and_newlines(&checkpoint.remote_file_url).is_empty()
            && checkpoint.media_type == media_type
            && checkpoint.caption.as_deref() == (!caption.is_empty()).then_some(caption)
            && checkpoint.audience == audience
            && checkpoint.allow_screenshots == allow_screenshots
            && checkpoint.post_to_profile == post_to_profile
            && checkpoint.duration_days == duration_days
            && checkpoint.excluded_user_ids.is_empty()
            && checkpoint.selected_user_ids == selected_user_ids
    }

    #[allow(clippy::too_many_arguments)]
    fn story_publish_signature(
        &self,
        account_user_id: &str,
        item: &MediaPickerItem,
        media_type: &str,
        caption: &str,
        audience: &str,
        allow_screenshots: bool,
        post_to_profile: bool,
        duration_days: i64,
        selected_user_ids: &[String],
    ) -> String {
        RiseIdentity::publish_signature(&[
            ("account_user_id", RiseIdentitySignatureValue::Text(account_user_id.to_owned())),
            ("media_item_id", RiseIdentitySignatureValue::Text(item.id.clone())),
            (key::MEDIA_TYPE, RiseIdentitySignatureValue::Text(media_type.to_owned())),
            (key::CAPTION, RiseIdentitySignatureValue::Text(caption.to_owned())),
            (key::AUDIENCE, RiseIdentitySignatureValue::Text(audience.to_owned())),
            (key::ALLOW_SCREENSHOTS, RiseIdentitySignatureValue::Flag(allow_screenshots)),
            (key::POST_TO_PROFILE, RiseIdentitySignatureValue::Flag(post_to_profile)),
            (key::DURATION_DAYS, RiseIdentitySignatureValue::Integer(duration_days)),
            (key::SELECTED_USER_IDS, RiseIdentitySignatureValue::TextList(selected_user_ids.to_vec())),
        ])
    }
}
