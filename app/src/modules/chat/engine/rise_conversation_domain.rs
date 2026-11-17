use rise_engine::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

use futures::{FutureExt, StreamExt};
use futures::future::BoxFuture;
use gpui::{App, AsyncApp, Context, Entity};
use rise_canon::{RiseTimingCanon, rise_key};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_sync::{
    RiseDurableLifecycleEvent, RiseDurableLifecycleResult, RiseDurableLifecycleStreamTermination, RiseSyncPushHandler,
};
use rise_engine::rise_store::rise_store_entities::{RiseEntityWindow, RiseEntityWindowQuery};
use rise_engine::rise_store::rise_store_models::{RiseStoreSnapshot, RiseStoreSnapshotRequest, RiseStoreWriteBatch};
use rise_engine::rise_upload::rise_upload_progress::{RiseUploadCancellationSignal, RiseUploadRealtimeProgressHub};
use rise_engine::{CancellationError, RiseEngine, RiseJSONValue, RiseWirePushEnvelope, RiseWireRemoteError};
use serde_json::{Map, Value};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiActiveScreen, SaiNavigationBridge};
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseColdOpenTrace;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::ui::clever_image_disk_cache::CleverImagePrefetchPolicy;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::chat_engine_hooks::{
    ChatEngineMainCallback, ChatEngineRemovedChatName, chat_engine_hooks, chat_engine_rtc_hooks,
};
use crate::modules::chat::engine::chats::rise_chat_repository::RiseChatRepository;
use crate::modules::chat::engine::chats::rise_chat_repository_models::{
    RiseChatRepositoryError, RiseChatResourceInvalidation, RiseChatWireCodec, RiseConversationTopicPresence,
};
use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::{
    RiseChatOverlaySearchPresentationPreparer, RiseChatOverlaySearchScope, RiseChatSearchRecentItem,
    RiseChatSearchResultsPolicy, RiseSearchRequestGenerationPolicy,
};
use crate::modules::chat::engine::core::rise_chat_presentation_preparer::RiseChatPresentationPreparer;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationAuxiliaryLoadState, RiseConversationAuxiliaryPageSnapshot, RiseConversationAuxiliaryRepository,
    RiseConversationAuxiliaryRepositoryError, RiseConversationAuxiliarySnapshot,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseChatListWireSnapshot, RiseChatUnreadBadge, RiseChatUnreadBadgePresentation,
    RiseConversationPresentation, RiseMessagePresentationPreparer, RiseMessageScopePresentationSnapshot,
    RiseMessageTabKind, RiseMessageTabPresentationPreparer, RiseMessageTabPresentationSnapshot, RiseMessageTabScope,
    RiseMessageWireSnapshot, RisePinnedMessagesPresentationPreparer,
};
use crate::modules::chat::engine::core::rise_conversation_resource_repository::RiseConversationResourceKeyPersistence;
use crate::modules::chat::engine::core::rise_conversation_rpc::RiseConversationEndpoints;
use crate::modules::chat::engine::core::rise_conversation_request_state::RiseConversationStateError;
use crate::modules::chat::engine::core::rise_conversation_store_layout::RiseConversationStoreLayout;
use crate::modules::chat::engine::core::rise_global_message_search_presentation::{
    RiseGlobalMessageSearchPresentationPreparer, RiseGlobalMessageSearchScope,
};
use crate::modules::chat::engine::forum::forum_voice_subscription_registry::ForumVoiceSubscriptionRegistry;
use crate::modules::chat::engine::forum::rise_chat_voice_presence::RiseChatVoicePresenceRepository;
use crate::modules::chat::engine::forum::rise_forum_voice_presence::RiseForumVoicePresenceRepository;
use crate::modules::chat::engine::forum::rise_forum_voice_realtime::{RiseForumVoiceRealtime, RiseForumVoiceRealtimeKind};
use crate::modules::chat::engine::media::rise_chat_media_gallery_presentation::RiseChatMediaGalleryPresentation;
use crate::modules::chat::engine::media::rise_chat_media_gallery_repository::RiseChatMediaGalleryRepository;
use crate::modules::chat::engine::messages::rise_chat_calendar::{RiseChatCalendarDay, RiseChatCalendarPage, RiseChatCalendarWindow};
use crate::modules::chat::engine::messages::rise_message_ephemeral_state::{RiseMessageTypingPeer, RiseMessageTypingStream};
use crate::modules::chat::engine::messages::rise_call_history_message::{RiseCallHistoryMessage, RiseCallHistoryRecord};
use crate::modules::chat::engine::messages::rise_message_contracts::{
    RiseAuthoritativeMessage, RiseMessageAuxiliaryInvalidationBatchHook, RiseMessageChatPreviewBatchHook,
    RiseMessageChatReadOptimisticHook, RiseMessageChatReadOptimisticHookPrepared, RiseMessageDraft,
    RiseMessageAuxiliaryProjection, RiseMessagePreviewChange, RiseMessageReadReceiptHintHook, RiseMessageSubmission,
    RiseMessageSubmissionMode,
};
use crate::modules::chat::engine::messages::rise_message_durable_outbox::RiseCreateMessageDurableEndpoint;
use crate::modules::chat::engine::messages::rise_message_mutations::MessageMutationCheckpoint;
use crate::modules::chat::engine::messages::rise_message_read_receipt_presentation::{
    RiseMessageReadReceiptFormatSeed, RiseMessageReadReceiptPresentation, RiseMessageReadReceiptPresentationPreparer,
    RiseMessageReadReceiptScope,
};
use crate::modules::chat::engine::messages::rise_message_read_receipt_wire_types::RiseMessageReadReceiptStoredResource;
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::engine::messages::rise_message_scope::{
    RiseMessageAnchor, RiseMessageJumpOutcome, RiseMessagePageDirection, RiseMessagePageLoadOutcome, RiseMessageScope,
    RiseMessageWindowConfiguration, RiseMessageWindowShift,
};
use crate::modules::chat::engine::messages::rise_message_search_session::{
    ChatSearchMessageKind, RiseMessageSearchError, RiseMessageSearchPresentationSnapshot, RiseMessageSearchRepository,
    RiseMessageSearchScope,
};
use crate::modules::chat::engine::messages::rise_message_unread_marks::{
    RiseChatUnreadFlagKind, RiseUnreadMarkCandidate, RiseUnreadMarkKind, RiseUnreadMarkViewport, RiseUnreadMarksConsumption,
    RiseUnreadMarksRealtimeChange, RiseUnreadMarksRepository, RiseUnreadMarksSnapshot,
};
use crate::modules::chat::engine::messages::rise_message_upload_retry::{RiseMessageUploadResend, RiseMessageUploadRetryCodec};
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::engine::migration::rise_legacy_conversation_importer::RiseLegacyConversationImporter;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::message::message_actions::message_types::{
    ChatTypingPeer, MessageContentType, MessageHistoryDirection, RemoteMediaURL,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    GetChatCalendarGatewayDTO, LinkMessageWireDTO, MediaItemWireDTO, MessageWireDTO,
};
use crate::modules::chat::stores::message::message_services::message_pagination_trace::MessagePaginationTrace;
use crate::modules::post::stores::post_actions::post_wire_types::GetFeedPostItemDTO;
use crate::modules::search::stores::search_actions::search_wire_types::SearchResultItemDTO;
use crate::modules::stickers::engine::rise_pack_collaboration_repository::RisePackCollaborationRepository;
use crate::modules::stickers::engine::rise_sticker_repository::RiseStickerRepository;

pub enum RiseMessageSynchronizationEpochPolicy {}

impl RiseMessageSynchronizationEpochPolicy {
    pub fn should_synchronize(
        force: bool,
        has_materialized_view: bool,
        synchronized_epoch: Option<u64>,
        current_epoch: u64,
    ) -> bool {
        force || !has_materialized_view || synchronized_epoch != Some(current_epoch)
    }

    pub fn consumes_epoch(outcome: &RiseMessagePageLoadOutcome) -> bool {
        match outcome {
            RiseMessagePageLoadOutcome::Loaded { .. }
            | RiseMessagePageLoadOutcome::CursorStalled
            | RiseMessagePageLoadOutcome::EndReached => true,
            RiseMessagePageLoadOutcome::AlreadyLoading | RiseMessagePageLoadOutcome::StaleResponseDropped => false,
        }
    }
}

pub enum RiseMessageSynchronizationFlightPolicy {}

impl RiseMessageSynchronizationFlightPolicy {
    pub fn accepts(
        active_epoch: Option<u64>,
        active_generation: Option<u64>,
        current_epoch: u64,
        expected_epoch: u64,
        expected_generation: u64,
    ) -> bool {
        active_epoch == Some(expected_epoch)
            && active_generation == Some(expected_generation)
            && current_epoch == expected_epoch
    }
}

pub enum RiseMessageTimelineRecoveryPolicy {}

impl RiseMessageTimelineRecoveryPolicy {
    pub fn latest_anchor(
        row_count: i64,
        thread_root_id: Option<&str>,
        topic_id: Option<&str>,
        preview_message_id: Option<&str>,
        preview_topic_id: Option<&str>,
    ) -> Option<String> {
        if row_count != 0 || thread_root_id.is_some() {
            return None;
        }
        let preview_message_id = Self::normalized(preview_message_id)?;
        if let Some(topic_id) = topic_id {
            let requested_topic = Self::normalized_topic(Some(topic_id));
            if requested_topic != Self::normalized_topic(preview_topic_id) {
                return None;
            }
        }
        Some(preview_message_id)
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let normalized = trimming_whitespaces_and_newlines(value?);
        if normalized.is_empty() { None } else { Some(normalized.to_string()) }
    }

    fn normalized_topic(value: Option<&str>) -> String {
        value.map(trimming_whitespaces_and_newlines).unwrap_or_default().to_string()
    }
}

pub enum RiseMessagePresentationInputPolicy {}

impl RiseMessagePresentationInputPolicy {
    pub fn is_duplicate(lhs: &RiseMessageWireSnapshot, rhs: &RiseMessageWireSnapshot) -> bool {
        lhs.revision == rhs.revision && Self::has_same_content(lhs, rhs)
    }

    pub fn has_same_content(lhs: &RiseMessageWireSnapshot, rhs: &RiseMessageWireSnapshot) -> bool {
        lhs.account_id == rhs.account_id
            && lhs.scope == rhs.scope
            && lhs.local_ids == rhs.local_ids
            && lhs.entity_stamps == rhs.entity_stamps
            && lhs.anchor_local_id == rhs.anchor_local_id
            && lhs.has_cached_before == rhs.has_cached_before
            && lhs.has_cached_after == rhs.has_cached_after
            && lhs.before_cursor == rhs.before_cursor
            && lhs.after_cursor == rhs.after_cursor
            && lhs.has_more_before == rhs.has_more_before
            && lhs.has_more_after == rhs.has_more_after
            && lhs.load_state == rhs.load_state
            && lhs.error_code == rhs.error_code
    }
}

pub enum RiseMessageObservationRecoveryPolicy {}

impl RiseMessageObservationRecoveryPolicy {
    pub fn delay_milliseconds(after_completed_attempts: i64) -> Option<i64> {
        match after_completed_attempts {
            ..0 => None,
            0 => Some(0),
            1 => Some(250),
            2 => Some(1_000),
            3 => Some(4_000),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn accepts(
        is_active: bool,
        is_shutdown: bool,
        account_matches: bool,
        has_owner: bool,
        current_generation: Option<u64>,
        expected_generation: u64,
        has_installed_task: bool,
        context_matches: bool,
    ) -> bool {
        is_active
            && !is_shutdown
            && account_matches
            && has_owner
            && current_generation == Some(expected_generation)
            && !has_installed_task
            && context_matches
    }
}

/// recovery for the window request, not the store observation
/// failed seed with no rows renders as empty chat, no error
/// so we ask again on the same ladder
pub enum RiseMessageSeedRecoveryPolicy {}

impl RiseMessageSeedRecoveryPolicy {
    pub fn delay_milliseconds(after_completed_attempts: i64) -> Option<i64> {
        if after_completed_attempts < 0 {
            return None;
        }
        RiseMessageObservationRecoveryPolicy::delay_milliseconds(after_completed_attempts + 1)
    }

    pub fn accepts(
        is_active: bool,
        is_shutdown: bool,
        account_matches: bool,
        has_owner: bool,
        current_navigation_generation: u64,
        expected_navigation_generation: u64,
    ) -> bool {
        is_active
            && !is_shutdown
            && account_matches
            && has_owner
            && current_navigation_generation == expected_navigation_generation
    }
}

static UNREAD_MARK_CONFIRMATION_DELAY_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::debounce("unread_marks_consume").client_debounce_ms);
const PINNED_PAGE_WALK_CEILING: i64 = 64;
const PINNED_PAGE_WALK_RETRY_DELAY: Duration = Duration::from_millis(250);
const PINNED_PAGE_PATCHES_BUFFER: usize = 32;

#[derive(Default)]
struct RiseConversationDomainState {
    unread_mark_viewports: HashMap<RiseMessageScope, RiseUnreadMarkViewport>,
    unread_mark_observation_counts: HashMap<RiseMessageScope, i64>,
    unread_mark_candidates_awaiting_ledger: HashMap<RiseMessageScope, HashMap<String, RiseUnreadMarkCandidate>>,
    pending_unread_mark_consumptions: HashMap<RiseMessageScope, RiseUnreadMarksConsumption>,
    parked_unread_mark_consumptions: HashMap<RiseMessageScope, RiseUnreadMarksConsumption>,
    unread_mark_confirmation_task: Option<JoinHandle<()>>,

    chat_observation_tasks: Vec<JoinHandle<()>>,
    badge_refresh_task: Option<JoinHandle<()>>,
    message_observation_tasks: HashMap<RiseMessageScope, JoinHandle<()>>,
    message_observation_recovery_tasks: HashMap<RiseMessageScope, JoinHandle<()>>,
    message_observation_recovery_tokens: HashMap<RiseMessageScope, Uuid>,
    message_observation_recovery_attempts: HashMap<RiseMessageScope, i64>,
    message_seed_recovery_tasks: HashMap<RiseMessageScope, JoinHandle<()>>,
    message_seed_recovery_tokens: HashMap<RiseMessageScope, Uuid>,
    message_seed_recovery_attempts: HashMap<RiseMessageScope, i64>,
    unknown_chat_refresh_attempts: HashMap<String, i64>,
    unknown_chat_refresh_task: Option<JoinHandle<()>>,
    unknown_chat_refresh_queued: bool,
    unknown_chat_refresh_retried: HashSet<String>,
    chat_detail_revalidations_in_flight: HashSet<String>,
    typing_observation_tasks: HashMap<RiseMessageScope, JoinHandle<()>>,
    typing_observation_generations: HashMap<RiseMessageScope, u64>,
    message_observation_generations: HashMap<RiseMessageScope, u64>,
    message_observation_anchors: HashMap<RiseMessageScope, RiseMessageAnchor>,
    message_observation_configurations: HashMap<RiseMessageScope, RiseMessageWindowConfiguration>,
    message_presentation_generations: HashMap<RiseMessageScope, u64>,
    message_navigation_generations: HashMap<RiseMessageScope, u64>,
    active_message_jumps: HashMap<RiseMessageScope, u64>,
    message_observation_owners: HashMap<RiseMessageScope, HashSet<Uuid>>,
    message_owner_generations: HashMap<RiseMessageScope, HashMap<Uuid, u64>>,
    latest_chat_snapshots: HashMap<RiseChatListKind, RiseChatListWireSnapshot>,
    chat_presentation_preparer: RiseChatPresentationPreparer,
    chat_presentation_ready: bool,
    latest_message_snapshots: HashMap<RiseMessageScope, RiseMessageWireSnapshot>,
    message_history_clear_revisions_by_chat: HashMap<String, i64>,
    global_message_search_tasks: HashMap<ChatSearchMessageKind, JoinHandle<()>>,
    global_message_search_generations: HashMap<ChatSearchMessageKind, u64>,
    global_message_search_scopes: HashMap<ChatSearchMessageKind, RiseGlobalMessageSearchScope>,
    message_search_tasks: HashMap<RiseMessageSearchScope, JoinHandle<()>>,
    message_search_generations: HashMap<String, u64>,
    message_search_scopes: HashMap<String, RiseMessageSearchScope>,
    chat_overlay_entity_search_task: Option<JoinHandle<()>>,
    chat_overlay_chat_search_task: Option<JoinHandle<()>>,
    chat_overlay_post_search_task: Option<JoinHandle<()>>,
    chat_overlay_entity_search_generation: u64,
    chat_overlay_chat_search_generation: u64,
    chat_overlay_post_search_generation: u64,
    chat_overlay_entity_search_scope: Option<RiseChatOverlaySearchScope>,
    chat_overlay_chat_search_scope: Option<RiseChatOverlaySearchScope>,
    chat_overlay_post_search_scope: Option<RiseChatOverlaySearchScope>,
    chat_search_recents: Vec<RiseChatSearchRecentItem>,
    synchronization_epoch: u64,
    synchronized_chat_epochs: HashMap<RiseChatListKind, u64>,
    synchronizing_chat_epochs: HashMap<RiseChatListKind, u64>,
    chat_sync_retries: HashMap<RiseChatListKind, (u64, usize)>,
    synchronized_message_epochs: HashMap<RiseMessageScope, u64>,
    synchronizing_message_epochs: HashMap<RiseMessageScope, u64>,
    message_synchronization_generations: HashMap<RiseMessageScope, u64>,
    synchronizing_message_generations: HashMap<RiseMessageScope, u64>,
    synchronized_message_anchors: HashMap<RiseMessageScope, (RiseMessageAnchor, u64)>,
    pending_forced_message_syncs: HashMap<RiseMessageScope, (Option<RiseMessageAnchor>, Option<String>)>,
    durable_lifecycle_task: Option<JoinHandle<()>>,
    pinned_page_patches_task: Option<JoinHandle<()>>,
    media_gallery_walk_task: Option<JoinHandle<()>>,
    refresh_task: Option<JoinHandle<()>>,
    account_epoch: Option<String>,
    did_schedule_ownership_refresh: bool,
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
    message_warmup_task: Option<JoinHandle<()>>,
    message_warmup_generation: u64,
}

/// account scoped root of the normalized conversation engine
/// an actor and not a ui store, repos own transport and cache invariants
pub struct RiseConversationDomain {
    pub account_id: String,
    pub engine: Arc<RiseEngine>,
    pub chats: Arc<RiseChatRepository>,
    pub messages: Arc<RiseMessageRepository>,
    pub auxiliary: Arc<RiseConversationAuxiliaryRepository>,
    pub unread_marks: Arc<RiseUnreadMarksRepository>,
    pub stickers: Arc<RiseStickerRepository>,
    pub pack_collaboration: Arc<RisePackCollaborationRepository>,
    pub media_gallery: Arc<RiseChatMediaGalleryRepository>,
    pub message_search: Arc<RiseMessageSearchRepository>,
    pub forum_voice: Arc<RiseForumVoicePresenceRepository>,
    pub chat_voice: Arc<RiseChatVoicePresenceRepository>,
    pub upload_realtime_progress: Arc<RiseUploadRealtimeProgressHub>,

    message_presentation_preparer: Arc<RiseMessagePresentationPreparer>,
    pinned_message_presentation_preparer: Arc<RisePinnedMessagesPresentationPreparer>,
    message_read_receipt_presentation_preparer: Arc<RiseMessageReadReceiptPresentationPreparer>,
    message_tab_presentation_preparer: Arc<RiseMessageTabPresentationPreparer>,
    global_message_search_presentation_preparer: Arc<RiseGlobalMessageSearchPresentationPreparer>,
    chat_overlay_search_presentation_preparer: Arc<RiseChatOverlaySearchPresentationPreparer>,
    pinned_page_patches: Mutex<Option<broadcast::Receiver<String>>>,
    this: Weak<RiseConversationDomain>,
    state: RiseIsolated<RiseConversationDomainState>,
}

/// runs work on the main thread, none only before the main hop is installed
async fn on_main<R, W>(work: W) -> Option<R>
where
    R: Send + 'static,
    W: FnOnce(&mut App) -> R + Send + 'static,
{
    let main = RiseEngineMainHop::installed()?;
    main.run(move |cx: AsyncApp| async move { Ok(cx.update(work)) }.boxed_local()).await.ok()
}

impl RiseConversationDomain {
    fn known_voice_photo(user_id: &str, chat_id: &str, cx: &mut App) -> Option<String> {
        if let Some(me) = AuthServicesStore::shared(cx).read(cx).profile()
            && me.id == user_id
        {
            return RemoteMediaURL::optional_normalized(me.avatar_url.as_deref().or(Some(me.more.logo.as_str())));
        }
        let hooks = chat_engine_hooks()?;
        let direct_id = (hooks.known_direct_chat_id)(user_id, cx);
        if !direct_id.is_empty()
            && let Some(logo) = (hooks.chat_info)(&direct_id, cx)
                .and_then(|chat| RemoteMediaURL::optional_normalized(Some(chat.participant.more.logo.as_str())))
        {
            return Some(logo);
        }
        let scope = (hooks.member_scope_key)(chat_id, None);
        SaiWsResponseCache::memory_only(&(hooks.members_cache_key)(&scope), cx)?;
        (hooks.member)(&scope, user_id, cx)
            .and_then(|member| RemoteMediaURL::optional_normalized(Some(member.avatar_url.as_str())))
    }

    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let (pinned_page_patches_continuation, pinned_page_patches) = broadcast::channel(PINNED_PAGE_PATCHES_BUFFER);
        let account_id = engine.account.account_id.clone();
        let chat_repository = RiseChatRepository::new(engine.clone(), account_id.clone(), None, None);
        let auxiliary_repository = RiseConversationAuxiliaryRepository::new(engine.clone(), account_id.clone());
        let upload_realtime_progress = Arc::new(RiseUploadRealtimeProgressHub::default());
        let message_search = RiseMessageSearchRepository::new(engine.clone(), account_id.clone());
        let messages = RiseMessageRepository::new(
            engine.clone(),
            account_id.clone(),
            upload_realtime_progress.clone(),
            Self::preview_hook(chat_repository.clone(), account_id.clone()),
            Self::chat_read_optimistic_hook(chat_repository.clone()),
            Self::auxiliary_invalidation_hook(auxiliary_repository.clone(), pinned_page_patches_continuation),
            Self::read_receipt_hint_hook(auxiliary_repository.clone()),
        );
        let unread_marks = RiseUnreadMarksRepository::new(engine.clone(), account_id.clone());
        let stickers = RiseStickerRepository::new(engine.clone(), account_id.clone());
        let pack_collaboration = RisePackCollaborationRepository::new(engine.clone(), account_id.clone());
        let media_gallery = RiseChatMediaGalleryRepository::new(engine.clone(), account_id.clone());
        Arc::new_cyclic(|this| Self {
            account_id,
            engine,
            chats: chat_repository,
            messages,
            auxiliary: auxiliary_repository,
            unread_marks,
            stickers,
            pack_collaboration,
            media_gallery,
            message_search,
            forum_voice: Arc::new(RiseForumVoicePresenceRepository::new()),
            chat_voice: Arc::new(RiseChatVoicePresenceRepository::new()),
            upload_realtime_progress,
            message_presentation_preparer: Arc::new(RiseMessagePresentationPreparer::new()),
            pinned_message_presentation_preparer: Arc::new(RisePinnedMessagesPresentationPreparer::new()),
            message_read_receipt_presentation_preparer: Arc::new(RiseMessageReadReceiptPresentationPreparer::new()),
            message_tab_presentation_preparer: Arc::new(RiseMessageTabPresentationPreparer::new()),
            global_message_search_presentation_preparer: Arc::new(RiseGlobalMessageSearchPresentationPreparer::new()),
            chat_overlay_search_presentation_preparer: Arc::new(RiseChatOverlaySearchPresentationPreparer::new()),
            pinned_page_patches: Mutex::new(Some(pinned_page_patches)),
            this: this.clone(),
            state: RiseIsolated::new(RiseConversationDomainState::default()),
        })
    }

    fn preview_hook(chats: Arc<RiseChatRepository>, account_id: String) -> RiseMessageChatPreviewBatchHook {
        RiseMessageChatPreviewBatchHook::new(move |change: RiseMessagePreviewChange, version: i64| {
            let chats = chats.clone();
            let account_id = account_id.clone();
            async move {
                let batch = chats.make_message_preview_batch(change.clone(), version).await?;
                Self::mirror_forum_topic_rows(&change, &chats, &account_id).await;
                Self::mirror_own_slow_mode_send(&change, &account_id).await;
                Ok(batch)
            }
            .boxed()
        })
    }

    fn chat_read_optimistic_hook(chats: Arc<RiseChatRepository>) -> RiseMessageChatReadOptimisticHook {
        let badge_generation_at_prepare: Arc<Mutex<HashMap<String, u64>>> = Arc::new(Mutex::new(HashMap::new()));
        let prepare = chats.clone();
        let confirm = chats.clone();
        let rollback = chats.clone();
        let discard = chats;
        let prepared_generations = badge_generation_at_prepare.clone();
        let confirmed_generations = badge_generation_at_prepare.clone();
        let rolled_back_generations = badge_generation_at_prepare.clone();
        let discarded_generations = badge_generation_at_prepare;
        RiseMessageChatReadOptimisticHook::new(
            move |chat_id: String, clearing_topic_unread: Option<i64>| {
                let chats = prepare.clone();
                let generations = prepared_generations.clone();
                async move {
                    let generation = chats.server_inbox_badge_generation();
                    let prepared = match clearing_topic_unread {
                        Some(clearing) => chats.make_topic_read_optimistic_batch(&chat_id, clearing).await?,
                        None => chats.make_read_optimistic_batch(&chat_id).await?,
                    };
                    if let Some((token, _)) = prepared.as_ref()
                        && let Ok(mut generations) = generations.lock()
                    {
                        generations.insert(token.id.to_string(), generation);
                    }
                    Ok(prepared.map(|(token, batch)| RiseMessageChatReadOptimisticHookPrepared { token, batch }))
                }
                .boxed()
            },
            move |token| {
                let chats = confirm.clone();
                let generations = confirmed_generations.clone();
                async move {
                    let prepared_generation =
                        generations.lock().ok().and_then(|mut generations| generations.remove(&token.id.to_string()));
                    let _ = chats.confirm_optimistic(&token).await;
                    if prepared_generation.is_some_and(|generation| generation != chats.server_inbox_badge_generation()) {
                        let _ = chats.refresh_unread_badge(true).await;
                    }
                }
                .boxed()
            },
            move |token| {
                let chats = rollback.clone();
                let generations = rolled_back_generations.clone();
                async move {
                    if let Ok(mut generations) = generations.lock() {
                        generations.remove(&token.id.to_string());
                    }
                    let _ = chats.rollback_optimistic(&token).await;
                }
                .boxed()
            },
            move |token| {
                let chats = discard.clone();
                let generations = discarded_generations.clone();
                async move {
                    if let Ok(mut generations) = generations.lock() {
                        generations.remove(&token.id.to_string());
                    }
                    chats.discard_prepared_optimistic(&token)
                }
                .boxed()
            },
        )
    }

    fn auxiliary_invalidation_hook(
        auxiliary: Arc<RiseConversationAuxiliaryRepository>,
        pinned_page_patches_continuation: broadcast::Sender<String>,
    ) -> RiseMessageAuxiliaryInvalidationBatchHook {
        RiseMessageAuxiliaryInvalidationBatchHook::new(move |invalidation| {
            let auxiliary = auxiliary.clone();
            let pinned_page_patches_continuation = pinned_page_patches_continuation.clone();
            async move {
                let batch = auxiliary.make_message_projection_invalidation_batch(&invalidation).await;
                if invalidation.pinned_message_patch.is_some()
                    && invalidation.projections.contains(&RiseMessageAuxiliaryProjection::Pinned)
                {
                    let _ = pinned_page_patches_continuation.send(invalidation.chat_id.clone());
                }
                batch
            }
            .boxed()
        })
    }

    fn read_receipt_hint_hook(auxiliary: Arc<RiseConversationAuxiliaryRepository>) -> RiseMessageReadReceiptHintHook {
        RiseMessageReadReceiptHintHook::new(move |hint| {
            let auxiliary = auxiliary.clone();
            async move { auxiliary.record_message_read_receipt_hint(hint).await }.boxed()
        })
    }

    pub(crate) fn is_registry_active(&self, cx: &mut App) -> bool {
        RiseConversationDomainRegistry::shared(cx)
            .read(cx)
            .active()
            .is_some_and(|active| std::ptr::eq(Arc::as_ptr(&active), self as *const Self))
    }

    /// imports the old disk cache and installs every reducer before the engine starts
    /// push ownership isnt published until activate completes
    pub async fn prepare(&self, context: &RiseAppEngineBootstrapContext) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared) = self.state.with(|state| (state.is_shutdown, state.is_prepared));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if is_prepared {
            return Ok(());
        }

        context.run_importer(&RiseLegacyConversationImporter::new()).await?;
        self.messages.register(context).await?;
        let invalidation_owner = self.this.clone();
        let chat_handlers = self
            .chats
            .chat_push_handlers(Some(Arc::new(move |invalidations: HashSet<RiseChatResourceInvalidation>| {
                let owner = invalidation_owner.clone();
                async move {
                    if let Some(owner) = owner.upgrade() {
                        owner.apply_resource_invalidations(invalidations).await;
                    }
                }
                .boxed()
            })));
        for handler in chat_handlers {
            context.register_push_handler(handler).await?;
        }
        let unread_marks_owner = self.this.clone();
        context
            .register_push_handler(RiseSyncPushHandler::new(
                rise_events::name::CHAT_UNREAD_MARKS_CHANGED,
                move |envelope| {
                    let owner = unread_marks_owner.clone();
                    async move {
                        let Some(owner) = owner.upgrade() else {
                            return Ok(None);
                        };
                        owner.reduce_unread_marks_changed(envelope).await
                    }
                },
            ))
            .await?;
        let forum_voice = self.forum_voice.clone();
        let chat_voice = self.chat_voice.clone();
        context
            .register_push_handler(RiseSyncPushHandler::new(RiseForumVoiceRealtime::EVENT_TYPE, move |envelope| {
                let forum_voice = forum_voice.clone();
                let chat_voice = chat_voice.clone();
                async move {
                    let Some(mut update) = RiseForumVoiceRealtime::decode(&envelope) else {
                        tracing::warn!(target: "riseonly::X1906", r#type = %envelope.r#type, "voice update not decoded");
                        return Ok(None);
                    };
                    tracing::warn!(target: "riseonly::X1906", chat = %update.chat_id, room = %update.room_id, kind = ?update.kind, scope = ?update.scope, count = update.participant_count, has_participant = update.participant.is_some(), occurred = update.occurred_at_ms, "voice update push");
                    if update.kind == RiseForumVoiceRealtimeKind::Joined
                        && let Some(mut participant) = update.participant.clone()
                        && participant.avatar_url.is_empty()
                    {
                        let chat_id = update.chat_id.clone();
                        let user_id = participant.user_id.clone();
                        let known =
                            on_main(move |cx| Self::known_voice_photo(&user_id, &chat_id, cx)).await.flatten();
                        if let Some(known) = known {
                            participant.avatar_url = known;
                            update.participant = Some(participant);
                        }
                    }
                    forum_voice.apply(update.clone()).await;
                    chat_voice.apply(update).await;
                    Ok(None)
                }
            }))
            .await?;

        if let Ok(initial_durable_lifecycle) = self.engine.durable_lifecycle_events().await {
            let owner = self.this.clone();
            let engine = self.engine.clone();
            let task = RiseAccountRuntime::current().spawn(async move {
                let mut durable_lifecycle = initial_durable_lifecycle;
                loop {
                    let mut should_resubscribe = false;
                    while let Some(event) = durable_lifecycle.next().await {
                        if matches!(
                            event,
                            RiseDurableLifecycleEvent::StreamTerminated {
                                reason: RiseDurableLifecycleStreamTermination::SubscriberEvicted
                            }
                        ) {
                            should_resubscribe = true;
                            break;
                        }
                        match &event {
                            RiseDurableLifecycleEvent::Enqueued { request_id, .. }
                            | RiseDurableLifecycleEvent::RetryScheduled {
                                request_id,
                                result: RiseDurableLifecycleResult::Paused,
                                ..
                            } => {
                                let request_id = request_id.clone();
                                on_main(move |cx| {
                                    SaiWebSocketManager::shared(cx).update(cx, |manager, cx| {
                                        manager.note_durable_outbound_waiting(request_id, cx);
                                    });
                                })
                                .await;
                            }
                            RiseDurableLifecycleEvent::Terminal { request_id, .. }
                            | RiseDurableLifecycleEvent::ReconciliationCommitted { request_id, .. } => {
                                let request_id = request_id.clone();
                                on_main(move |cx| {
                                    SaiWebSocketManager::shared(cx).update(cx, |manager, _| {
                                        manager.note_durable_outbound_settled(request_id);
                                    });
                                })
                                .await;
                            }
                            _ => {}
                        }
                        if let Some(owner) = owner.upgrade() {
                            owner.note_durable_send_outcome(&event).await;
                        }
                        if matches!(
                            event,
                            RiseDurableLifecycleEvent::StreamTerminated {
                                reason: RiseDurableLifecycleStreamTermination::EngineShutdown
                            }
                        ) {
                            return;
                        }
                    }
                    if !should_resubscribe {
                        return;
                    }
                    let Ok(next) = engine.durable_lifecycle_events().await else {
                        return;
                    };
                    durable_lifecycle = next;
                }
            });
            self.state.with(|state| state.durable_lifecycle_task = Some(task));
        }
        let pinned_page_patches = self.pinned_page_patches.lock().ok().and_then(|mut receiver| receiver.take());
        if let Some(mut pinned_page_patches) = pinned_page_patches {
            let owner = self.this.clone();
            let task = RiseAccountRuntime::current().spawn(async move {
                loop {
                    let chat_id = match pinned_page_patches.recv().await {
                        Ok(chat_id) => chat_id,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return,
                    };
                    if let Some(owner) = owner.upgrade() {
                        owner.refresh_pinned_messages_presentation_from_cache(&chat_id).await;
                    }
                }
            });
            self.state.with(|state| state.pinned_page_patches_task = Some(task));
        }

        self.state.with(|state| state.is_prepared = true);
        for kind in RiseChatListKind::ALL_CASES.iter().copied() {
            let repository = self.chats.clone();
            let owner = self.this.clone();
            let task = RiseAccountRuntime::current().spawn(async move {
                let Ok(mut snapshots) = repository.observe(kind).await else {
                    return;
                };
                while let Some(snapshot) = snapshots.next().await {
                    let Ok(snapshot) = snapshot else {
                        return;
                    };
                    if let Some(owner) = owner.upgrade() {
                        owner.accept(snapshot).await;
                    }
                }
            });
            self.state.with(|state| state.chat_observation_tasks.push(task));
        }
        Ok(())
    }

    async fn apply_resource_invalidations(&self, invalidations: HashSet<RiseChatResourceInvalidation>) {
        let expected_epoch = self.state.with(|state| {
            if state.is_shutdown || !state.is_active { None } else { state.account_epoch.clone() }
        });
        let Some(expected_epoch) = expected_epoch else {
            return;
        };
        let account_id = self.account_id.clone();
        let mut removed_chats: HashMap<String, ChatEngineRemovedChatName> = HashMap::new();
        for invalidation in &invalidations {
            let RiseChatResourceInvalidation::RemovedByAdmin { chat_id } = invalidation else {
                continue;
            };
            let Ok(Some(stored)) = self.chats.stored_chat_id(chat_id, false).await else {
                continue;
            };
            let wire = &stored.record.wire;
            removed_chats.insert(
                chat_id.clone(),
                ChatEngineRemovedChatName {
                    title: wire.title.clone().unwrap_or_default(),
                    is_channel: wire.r#type.as_deref().and_then(ChatType::matching) == Some(ChatType::Channel),
                    announces: true,
                },
            );
        }

        let Some(this) = self.this.upgrade() else {
            return;
        };
        on_main(move |cx| {
            if !this.is_registry_active(cx)
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&expected_epoch))
            {
                return;
            }
            let Some(hooks) = chat_engine_hooks() else {
                return;
            };

            let mut permission_chats = HashSet::<String>::new();
            let mut topics_chats = HashSet::<String>::new();
            let mut forum_chats = HashSet::<String>::new();
            let mut settings_chats = HashSet::<String>::new();
            let mut preview_topics: HashMap<String, Option<String>> = HashMap::new();
            let mut reread_preview_chats = HashSet::<String>::new();
            let mut deleted_topic_unread: Vec<(String, i64)> = Vec::new();

            for invalidation in invalidations {
                match invalidation {
                    RiseChatResourceInvalidation::Topics { chat_id } => {
                        topics_chats.insert(chat_id.clone());
                        permission_chats.insert(chat_id);
                    }
                    RiseChatResourceInvalidation::Forum { chat_id }
                    | RiseChatResourceInvalidation::Roles { chat_id } => {
                        forum_chats.insert(chat_id.clone());
                        permission_chats.insert(chat_id);
                    }
                    RiseChatResourceInvalidation::Members { chat_id } => {
                        (hooks.refresh_loaded_member_lists)(&chat_id, cx);
                        permission_chats.insert(chat_id);
                    }
                    RiseChatResourceInvalidation::MemberRole { chat_id, user_id, role, role_ids, display_color } => {
                        let mut fields = Map::new();
                        if let Some(role) = role {
                            fields.insert(rise_key::events::ROLE.to_string(), Value::from(role));
                        }
                        if let Some(role_ids) = role_ids {
                            fields.insert(rise_key::events::ROLE_IDS.to_string(), Value::from(role_ids));
                        }
                        if let Some(display_color) = display_color {
                            fields.insert(rise_key::events::DISPLAY_COLOR.to_string(), Value::from(display_color));
                        }
                        (hooks.apply_member_fields_local)(&chat_id, &user_id, fields, cx);
                    }
                    RiseChatResourceInvalidation::Settings { chat_id } => {
                        settings_chats.insert(chat_id);
                    }
                    RiseChatResourceInvalidation::Roster { chat_id, user_id, joined } => {
                        if joined {
                            (hooks.refresh_loaded_member_lists)(&chat_id, cx);
                        } else {
                            (hooks.remove_member_local)(&chat_id, &user_id, None, false, cx);
                        }
                    }
                    RiseChatResourceInvalidation::Preview { chat_id, topic_id, reread } => {
                        if reread {
                            reread_preview_chats.insert(chat_id.clone());
                        }
                        match preview_topics.get_mut(&chat_id) {
                            Some(known) => {
                                if *known != topic_id {
                                    *known = None;
                                }
                            }
                            None => {
                                preview_topics.insert(chat_id, topic_id);
                            }
                        }
                    }
                    RiseChatResourceInvalidation::TopicDeleted { chat_id, topic_id, deleted_by } => {
                        let mut payload = Map::new();
                        let unread_chat_id = chat_id.clone();
                        payload.insert(rise_key::events::CHAT_ID.to_string(), Value::from(chat_id));
                        payload.insert(rise_key::events::TOPIC_ID.to_string(), Value::from(topic_id));
                        if let Some(deleted_by) = deleted_by {
                            payload.insert(rise_key::events::DELETED_BY.to_string(), Value::from(deleted_by));
                        }
                        let cleared = (hooks.apply_topic_deleted_event)(Some(Value::Object(payload)), cx);
                        if cleared > 0 {
                            deleted_topic_unread.push((unread_chat_id, cleared));
                        }
                    }
                    RiseChatResourceInvalidation::RemovedByAdmin { chat_id } => {
                        let stored = removed_chats.get(&chat_id).cloned();
                        (hooks.announce_removed_from_chat)(&chat_id, stored, cx);
                    }
                }
            }

            let chats_with_cached_base_permissions: HashSet<String> = permission_chats
                .iter()
                .filter(|chat_id| (hooks.effective_permissions)(chat_id, None, cx).is_some())
                .cloned()
                .collect();
            let active_room_config = chat_engine_rtc_hooks().and_then(|rtc| (rtc.active_room_config)(cx));

            for chat_id in &permission_chats {
                (hooks.invalidate_overwrite_caches)(chat_id, cx);
                (hooks.invalidate_effective_permissions)(chat_id, cx);
            }
            for chat_id in &chats_with_cached_base_permissions {
                (hooks.load_effective_permissions)(chat_id, None, true, cx);
            }
            if let Some((room_chat_id, Some(topic_id))) = &active_room_config
                && permission_chats.contains(room_chat_id)
            {
                (hooks.load_effective_permissions)(room_chat_id, Some(topic_id), true, cx);
            }
            for chat_id in &settings_chats {
                if (hooks.settings_by_chat)(chat_id, cx).is_some() {
                    (hooks.get_chat_settings_action)(chat_id, cx);
                }
                if (hooks.chat_info)(chat_id, cx).is_some() {
                    (hooks.get_chat_by_id_action)(chat_id, true, cx);
                }
            }
            for chat_id in &topics_chats {
                (hooks.get_topics_action)(chat_id, true, cx);
            }
            let preview_domain = RiseConversationDomainRegistry::shared(cx)
                .read(cx)
                .active()
                .filter(|domain| domain.account_id == account_id);
            let reread_chats = Rc::new(reread_preview_chats);
            for chat_id in &forum_chats {
                let revalidate_preview: Option<ChatEngineMainCallback> =
                    preview_topics.remove(chat_id).map(|topic_id| {
                        let preview_domain = preview_domain.clone();
                        let account_id = account_id.clone();
                        let expected_epoch = expected_epoch.clone();
                        let chat_id = chat_id.clone();
                        let reread = reread_chats.contains(&chat_id);
                        Rc::new(move |cx: &mut App| {
                            let Some(preview_domain) = preview_domain.clone() else {
                                return;
                            };
                            if !preview_domain.is_registry_active(cx)
                                || !SaiAccountCacheScope::is_current_account_epoch(
                                    Some(&account_id),
                                    Some(&expected_epoch),
                                )
                            {
                                return;
                            }
                            let chat_id = chat_id.clone();
                            let topic_id = topic_id.clone();
                            drop(RiseAccountRuntime::current().spawn(async move {
                                preview_domain
                                    .revalidate_topic_scoped_preview(&chat_id, topic_id.as_deref(), reread)
                                    .await;
                            }));
                        }) as ChatEngineMainCallback
                    });
                (hooks.get_forum_structure_action)(chat_id, true, revalidate_preview.clone(), revalidate_preview, cx);
            }
            let immediate_rereads: Vec<String> = reread_chats.intersection(&forum_chats).cloned().collect();
            if !immediate_rereads.is_empty()
                && let Some(domain) = preview_domain.clone()
            {
                drop(RiseAccountRuntime::current().spawn(async move {
                    for chat_id in immediate_rereads {
                        domain.revalidate_topic_scoped_preview(&chat_id, None, true).await;
                    }
                }));
            }
            if !preview_topics.is_empty()
                && let Some(domain) = preview_domain
            {
                let pending: Vec<(String, Option<String>, bool)> = preview_topics
                    .into_iter()
                    .map(|(chat_id, topic_id)| {
                        let reread = reread_chats.contains(&chat_id);
                        (chat_id, topic_id, reread)
                    })
                    .collect();
                drop(RiseAccountRuntime::current().spawn(async move {
                    for (chat_id, topic_id, reread) in pending {
                        domain.revalidate_topic_scoped_preview(&chat_id, topic_id.as_deref(), reread).await;
                    }
                }));
            }
            if !deleted_topic_unread.is_empty() {
                let chats = this.chats.clone();
                drop(RiseAccountRuntime::current().spawn(async move {
                    for (chat_id, clearing) in deleted_topic_unread {
                        let prepared = chats.make_topic_read_optimistic_batch(&chat_id, clearing).await;
                        tracing::warn!(target: "riseonly::X1677", chat = chat_id.as_str(), clearing, prepared = matches!(prepared, Ok(Some(_))), "deleted topic unread cleared");
                        if let Ok(Some((token, batch))) = prepared
                        {
                            if chats.engine.write(batch).await.is_ok() {
                                let _ = chats.confirm_optimistic(&token).await;
                            } else {
                                chats.discard_prepared_optimistic(&token);
                            }
                        }
                    }
                }));
            }
        })
        .await;
    }

    async fn mirror_own_slow_mode_send(change: &RiseMessagePreviewChange, account_id: &str) {
        let RiseMessagePreviewChange::Created { scope, message, .. } = change else {
            return;
        };
        if message.is_temp == Some(true) {
            return;
        }
        let has_server_id = [message.server_id.as_deref(), message.id.as_deref()]
            .into_iter()
            .flatten()
            .any(|id| !id.is_empty() && !GetSaiServerId::is_temp_id(Some(id)));
        if !has_server_id
            || message.is_system_message == Some(true)
            || MessageContentType::from_raw_value(message.content_type.as_deref().unwrap_or_default())
                == MessageContentType::System
        {
            return;
        }
        let sender_id = message.sender_id.as_deref().or(message.sender.as_ref().and_then(|sender| sender.id.as_deref()));
        if sender_id != Some(account_id) || account_id.is_empty() {
            return;
        }
        let chat_id = scope.chat_id.clone();
        let created_at = RiseChatWireCodec::timestamp(message);
        if created_at <= 0.0 {
            return;
        }
        let account_id = account_id.to_string();
        on_main(move |cx| {
            if !SaiAccountCacheScope::is_current_account(Some(&account_id)) {
                return;
            }
            if let Some(hooks) = chat_engine_hooks() {
                (hooks.record_own_server_send)(&chat_id, created_at, cx);
            }
        })
        .await;
    }

    async fn mirror_forum_topic_rows(
        change: &RiseMessagePreviewChange,
        chats: &Arc<RiseChatRepository>,
        account_id: &str,
    ) {
        let chat_id = match change {
            RiseMessagePreviewChange::Created { scope, .. } | RiseMessagePreviewChange::Edited { scope, .. } => {
                scope.chat_id.clone()
            }
            RiseMessagePreviewChange::Deleted { chat_id, .. }
            | RiseMessagePreviewChange::SendFailed { chat_id, .. }
            | RiseMessagePreviewChange::Receipt { chat_id, .. } => chat_id.clone(),
            RiseMessagePreviewChange::Read { chat_id, topic_id, .. } => {
                if topic_id.is_none() {
                    return;
                }
                chat_id.clone()
            }
            RiseMessagePreviewChange::HistoryCleared { .. } => return,
        };
        let Ok(Some(stored)) = chats.stored_chat_id(&chat_id, false).await else {
            return;
        };
        if stored.record.wire.topics_enabled != Some(true) {
            return;
        }
        let failed_topic_row = match change {
            RiseMessagePreviewChange::SendFailed { local_id, .. } => stored
                .record
                .wire
                .last_message
                .clone()
                .filter(|preview| preview.id.as_deref() == Some(local_id.as_str()))
                .and_then(|mut preview| {
                    preview.send_failed = Some(true);
                    let updated_at = RiseConversationStoreLayout::normalized_timestamp(
                        preview.created_at.or(preview.timestamp).unwrap_or(0.0),
                    );
                    let topic_id = preview.topic_id.clone().unwrap_or_default();
                    SaiJSONCodec::encode(&preview).map(|row| (topic_id, row, updated_at))
                }),
            _ => None,
        };

        let change = change.clone();
        let account_id = account_id.to_string();
        on_main(move |cx| {
            let Some(hooks) = chat_engine_hooks() else {
                return;
            };
            if !SaiAccountCacheScope::is_current_account(Some(&account_id))
                || (SaiWsResponseCache::memory_only(&(hooks.topics_cache_key)(&chat_id), cx).is_none()
                    && SaiWsResponseCache::memory_only(&(hooks.forum_structure_cache_key)(&chat_id), cx).is_none())
            {
                return;
            }
            match change {
                RiseMessagePreviewChange::Created { scope, message, was_already_stored } => {
                    let topic_id = message.topic_id.clone().or(scope.topic_id.clone()).unwrap_or_default();
                    let Some(row) = SaiJSONCodec::encode(&RiseChatWireCodec::last_message(
                        &message,
                        &RiseConversationTopicPresence::Value(topic_id.clone()),
                    )) else {
                        return;
                    };
                    let sender_id = message
                        .sender_id
                        .clone()
                        .or(message.sender.as_ref().and_then(|sender| sender.id.clone()))
                        .unwrap_or_default();
                    (hooks.apply_topic_message_created)(
                        &chat_id,
                        &topic_id,
                        row,
                        RiseChatWireCodec::timestamp(&message),
                        !was_already_stored && !sender_id.is_empty() && sender_id != account_id,
                        cx,
                    );
                }
                RiseMessagePreviewChange::Edited { scope, message } => {
                    let topic_id = message.topic_id.clone().or(scope.topic_id.clone());
                    let Some(message_id) = message.server_id.clone().or(message.id.clone()) else {
                        return;
                    };
                    let presence = match &topic_id {
                        Some(topic_id) => RiseConversationTopicPresence::Value(topic_id.clone()),
                        None => RiseConversationTopicPresence::Absent,
                    };
                    let Some(row) = SaiJSONCodec::encode(&RiseChatWireCodec::last_message(&message, &presence)) else {
                        return;
                    };
                    (hooks.apply_topic_message_edited)(&chat_id, topic_id.as_deref(), &message_id, row, cx);
                }
                RiseMessagePreviewChange::Deleted { server_ids, .. } => {
                    let mut cleared_preview = false;
                    for message_id in &server_ids {
                        if (hooks.apply_topic_message_deleted)(&chat_id, None, message_id, cx) {
                            cleared_preview = true;
                        }
                    }
                    if cleared_preview {
                        (hooks.get_topics_action)(&chat_id, true, cx);
                        (hooks.get_forum_structure_action)(&chat_id, true, None, None, cx);
                    }
                }
                RiseMessagePreviewChange::Read { topic_id: Some(topic_id), .. } => {
                    (hooks.apply_topic_read_local)(&chat_id, &topic_id, cx);
                }
                RiseMessagePreviewChange::SendFailed { local_id, .. } => {
                    (hooks.apply_topic_message_send_failed)(&chat_id, &local_id, failed_topic_row, cx);
                }
                RiseMessagePreviewChange::Receipt { server_ids, .. } => {
                    (hooks.apply_topic_receipt)(&chat_id, &server_ids, cx);
                }
                RiseMessagePreviewChange::Read { .. } | RiseMessagePreviewChange::HistoryCleared { .. } => {}
            }
        })
        .await;
    }

    /// activates cached state at once, network refresh is non fatal
    /// an offline launch stays on encrypted store data and refreshes on reconnect
    pub async fn activate(&self) -> anyhow::Result<()> {
        let (is_shutdown, is_active) = self.state.with(|state| (state.is_shutdown, state.is_active));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if is_active {
            return Ok(());
        }
        let account_id = self.account_id.clone();
        let epoch = on_main(move |_| {
            if !SaiAccountCacheScope::is_current_account(Some(&account_id)) {
                return None;
            }
            SaiAccountCacheScope::active_epoch()
        })
        .await
        .flatten();
        let Some(epoch) = epoch else {
            return Err(RiseConversationStateError::StaleAccount.into());
        };
        self.state.with(|state| {
            state.account_epoch = Some(epoch.clone());
            state.is_active = true;
        });
        let owner = self.this.clone();
        self.chats
            .on_unknown_chat(Arc::new(move |chat_id: String| {
                let owner = owner.clone();
                async move {
                    if let Some(owner) = owner.upgrade() {
                        owner.unknown_chat_received_message(chat_id).await;
                    }
                }
                .boxed()
            }));
        RiseColdOpenTrace::mark(|| "conversation activate begin".to_owned());
        self.stickers.activate(&epoch).await;
        RiseColdOpenTrace::mark(|| "conversation stickers".to_owned());
        self.pack_collaboration.activate().await;
        RiseColdOpenTrace::mark(|| "conversation packs".to_owned());
        self.forum_voice.activate(&self.account_id).await;
        self.chat_voice.activate(&self.account_id).await;
        RiseColdOpenTrace::mark(|| "conversation voice".to_owned());
        let mut recents = self.auxiliary.cached_chat_search_recents().await.unwrap_or_default();
        recents.truncate(RiseChatSearchResultsPolicy::recent_limit().max(0) as usize);
        let cached_chat_search_recents = recents.clone();
        self.state.with(|state| state.chat_search_recents = recents);

        RiseColdOpenTrace::mark(|| "conversation recents".to_owned());

        let message_snapshots: Vec<RiseMessageWireSnapshot> =
            self.state.with(|state| state.latest_message_snapshots.values().cloned().collect());
        let mut committed_message_snapshots: Vec<Arc<RiseMessageScopePresentationSnapshot>> =
            Vec::with_capacity(message_snapshots.len());
        for snapshot in &message_snapshots {
            if let Some(prepared) = self.message_presentation_preparer.prepare(snapshot.clone(), true).await {
                committed_message_snapshots.push(prepared);
            }
        }
        let prepared_chat_snapshots = self.state.with(|state| {
            if state.is_shutdown || !state.is_active || state.account_epoch.as_deref() != Some(epoch.as_str()) {
                return None;
            }
            let snapshots: Vec<RiseChatListWireSnapshot> = state.latest_chat_snapshots.values().cloned().collect();
            Some(
                snapshots
                    .iter()
                    .filter_map(|snapshot| state.chat_presentation_preparer.prepare(snapshot, &HashSet::new()))
                    .collect::<Vec<_>>(),
            )
        });
        let Some(prepared_chat_snapshots) = prepared_chat_snapshots else {
            return Err(CancellationError.into());
        };
        RiseColdOpenTrace::mark(|| {
            format!(
                "conversation prepared messages={} chats={}",
                committed_message_snapshots.len(),
                prepared_chat_snapshots.len()
            )
        });
        let Some(this) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let main_epoch = epoch.clone();
        let prepared_chat_count = prepared_chat_snapshots.len();
        let installed = on_main(move |cx| {
            let account_id = this.account_id.clone();
            if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&main_epoch)) {
                return false;
            }
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.activate(&account_id, cx);
            });
            RiseMessageReadReceiptPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.activate(&account_id, cx);
            });
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                for snapshot in prepared_chat_snapshots {
                    presentation.publish_chat_list(snapshot, cx);
                }
                for snapshot in committed_message_snapshots {
                    presentation.publish_message_scope(snapshot, cx);
                }
                presentation.publish_chat_search_recents(&account_id, cached_chat_search_recents, cx);
            });
            RiseConversationDomainRegistry::shared(cx).update(cx, |registry, cx| registry.install(this, cx));
            RiseColdOpenTrace::mark(|| format!("conversation installed chats={prepared_chat_count}"));
            true
        })
        .await
        .unwrap_or(false);
        let current_chat_snapshots = self.state.with(|state| {
            if !installed
                || state.is_shutdown
                || !state.is_active
                || state.account_epoch.as_deref() != Some(epoch.as_str())
            {
                return None;
            }
            state.chat_presentation_ready = true;
            Some(state.latest_chat_snapshots.clone())
        });
        let Some(mut current_chat_snapshots) = current_chat_snapshots else {
            return Err(CancellationError.into());
        };
        for kind in RiseChatListKind::ALL_CASES.iter() {
            if let Some(snapshot) = current_chat_snapshots.remove(kind) {
                self.accept(snapshot).await;
            }
        }
        Ok(())
    }

    /// called only after the runtime published this session push claims
    /// so the first authoritative read cant open a gap that loses a legacy owned push
    pub async fn did_activate_ownership(&self) {
        let already = self.state.with(|state| std::mem::replace(&mut state.did_schedule_ownership_refresh, true));
        if already {
            return;
        }
        self.schedule_authoritative_refresh(false).await;
        self.start_unread_badge_refresh(false);
    }

    /// revalidates materialized views after every transport reconnect
    /// store snapshots stay visible while the requests run
    pub async fn socket_connected(&self) {
        self.stickers.refresh_visible_picker_after_reconnect().await;
        self.chat_voice.drop_delta_answers_after_reconnect().await;
        self.schedule_authoritative_refresh(true).await;
        self.start_unread_badge_refresh(true);
        self.resend_parked_unread_mark_confirmations();
    }

    async fn unknown_chat_received_message(&self, chat_id: String) {
        let expected = self.state.with(|state| {
            if !state.is_active || state.is_shutdown {
                None
            } else {
                Some((state.account_epoch.clone(), state.synchronization_epoch))
            }
        });
        let Some((expected_account_epoch, expected_synchronization_epoch)) = expected else {
            return;
        };
        let Ok(stored) = self.chats.stored_chat_id(&chat_id, false).await else {
            return;
        };
        let should_start = self.state.with(|state| {
            if !state.is_active
                || state.is_shutdown
                || state.account_epoch != expected_account_epoch
                || state.synchronization_epoch != expected_synchronization_epoch
            {
                return false;
            }
            if let Some(stored) = &stored {
                match &stored.record.tombstone {
                    Some(tombstone) if !tombstone.is_confirmed() => return false,
                    None if stored.record.list_kind.is_some() => {
                        state.unknown_chat_refresh_attempts.remove(&chat_id);
                        return false;
                    }
                    _ => {}
                }
            }
            let deletion_version = stored.as_ref().map_or(0, |stored| stored.entity.version);
            if state.unknown_chat_refresh_attempts.get(&chat_id) == Some(&deletion_version) {
                return false;
            }
            state.unknown_chat_refresh_attempts.insert(chat_id.clone(), deletion_version);
            if state.unknown_chat_refresh_task.is_some() {
                state.unknown_chat_refresh_queued = true;
                return false;
            }
            true
        });
        if should_start {
            self.start_unknown_chat_refresh();
        }
    }

    fn start_unknown_chat_refresh(&self) {
        let owner = self.this.clone();
        let task = RiseAccountRuntime::current().spawn(async move {
            let Some(owner) = owner.upgrade() else {
                return;
            };
            owner.synchronize_chats_if_needed(RiseChatListKind::Inbox, true, None).await;
            owner.synchronize_chats_if_needed(RiseChatListKind::Requests, true, None).await;
            if owner.unknown_chats_still_unlisted().await {
                RiseRuntimeTimer::sleep(Self::UNKNOWN_CHAT_RETRY_DELAY).await;
                owner.synchronize_chats_if_needed(RiseChatListKind::Inbox, true, None).await;
                owner.synchronize_chats_if_needed(RiseChatListKind::Requests, true, None).await;
            }
            owner.finish_unknown_chat_refresh();
        });
        self.state.with(|state| state.unknown_chat_refresh_task = Some(task));
    }

    const UNKNOWN_CHAT_RETRY_DELAY: Duration = Duration::from_secs(2);

    async fn unknown_chats_still_unlisted(&self) -> bool {
        let candidates: Vec<String> = self.state.with(|state| {
            state
                .unknown_chat_refresh_attempts
                .keys()
                .filter(|chat_id| !state.unknown_chat_refresh_retried.contains(*chat_id))
                .cloned()
                .collect()
        });
        let mut unlisted = Vec::new();
        for chat_id in candidates {
            let Ok(stored) = self.chats.stored_chat_id(&chat_id, false).await else {
                continue;
            };
            let listed = stored.as_ref().is_some_and(|stored| {
                stored.record.list_kind.is_some() && stored.record.tombstone.is_none()
            });
            if !listed {
                unlisted.push(chat_id);
            }
        }
        if unlisted.is_empty() {
            return false;
        }
        self.state.with(|state| {
            if !state.is_active || state.is_shutdown {
                return false;
            }
            state.unknown_chat_refresh_retried.extend(unlisted);
            true
        })
    }

    fn finish_unknown_chat_refresh(&self) {
        let queued = self.state.with(|state| {
            state.unknown_chat_refresh_task = None;
            if !state.is_active || state.is_shutdown {
                return None;
            }
            Some(std::mem::take(&mut state.unknown_chat_refresh_queued))
        });
        match queued {
            Some(true) => self.start_unknown_chat_refresh(),
            Some(false) => self.start_unread_badge_refresh(true),
            None => {}
        }
    }

    pub async fn refresh_unread_badge(&self) {
        self.start_unread_badge_refresh(true);
    }

    /// foreground edge over a live socket
    /// gateway never echoes a sender own message
    /// so one from a share extension or notification reply reaches an open chat only here
    pub async fn application_did_become_active(&self) {
        self.schedule_authoritative_refresh(true).await;
        self.start_unread_badge_refresh(true);
    }

    /// started and never awaited, runs on the runtime lifecycle queue
    /// an awaited rpc there would block its own reply
    fn start_unread_badge_refresh(&self, force: bool) {
        let epoch = self.state.with(|state| {
            if !state.is_active || state.is_shutdown {
                return None;
            }
            if let Some(task) = state.badge_refresh_task.take() {
                task.abort();
            }
            Some(state.account_epoch.clone())
        });
        let Some(epoch) = epoch else {
            return;
        };
        let chats = self.chats.clone();
        let account_id = self.account_id.clone();
        let task = RiseAccountRuntime::current().spawn(async move {
            let Some(summary) = chats.refresh_unread_badge(force).await else {
                return;
            };
            on_main(move |cx| {
                if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), epoch.as_deref()) {
                    return;
                }
                RiseChatUnreadBadgePresentation::shared(cx).update(cx, |presentation, cx| {
                    presentation.confirm_server_summary(RiseChatUnreadBadge::from_aggregates(&summary), cx);
                });
            })
            .await;
        });
        self.state.with(|state| state.badge_refresh_task = Some(task));
    }

    async fn schedule_authoritative_refresh(&self, advancing_epoch: bool) {
        let refresh_epoch = self.state.with(|state| {
            if !state.is_active || state.is_shutdown {
                return None;
            }
            if let Some(task) = state.refresh_task.take() {
                task.abort();
            }
            if advancing_epoch {
                state.synchronization_epoch = state.synchronization_epoch.wrapping_add(1);
                state.message_seed_recovery_attempts.clear();
                state.unknown_chat_refresh_attempts.clear();
                state.unknown_chat_refresh_retried.clear();
            }
            Some(state.synchronization_epoch)
        });
        let Some(refresh_epoch) = refresh_epoch else {
            return;
        };
        let active_screen = on_main(SaiNavigationBridge::current_screen).await.flatten();
        let mut active_chat_kinds = Self::chat_kinds_to_synchronize(active_screen.as_ref());
        active_chat_kinds.insert(RiseChatListKind::Inbox);
        let observed: Vec<(RiseMessageScope, Option<String>)> = self.state.with(|state| {
            let mut active_message_scopes: HashSet<RiseMessageScope> =
                state.message_observation_tasks.keys().cloned().collect();
            active_message_scopes.extend(
                state
                    .message_observation_owners
                    .iter()
                    .filter(|(_, owners)| !owners.is_empty())
                    .map(|(scope, _)| scope.clone()),
            );
            active_message_scopes
                .into_iter()
                .map(|scope| {
                    let anchor = state.latest_message_snapshots.get(&scope).and_then(|s| s.anchor_local_id.clone());
                    (scope, anchor)
                })
                .collect()
        });
        let owner = self.this.clone();
        let task = RiseAccountRuntime::current().spawn(async move {
            let mut group = Vec::with_capacity(active_chat_kinds.len() + observed.len());
            for kind in active_chat_kinds {
                let owner = owner.clone();
                group.push(
                    async move {
                        if let Some(owner) = owner.upgrade() {
                            owner.synchronize_chats_if_needed(kind, false, Some(refresh_epoch)).await;
                        }
                    }
                    .boxed(),
                );
            }
            for (scope, anchor_local_id) in observed {
                let owner = owner.clone();
                group.push(
                    async move {
                        if let Some(owner) = owner.upgrade() {
                            owner
                                .synchronize_messages_if_needed(
                                    &scope,
                                    anchor_local_id.map(RiseMessageAnchor::LocalID),
                                    None,
                                    Some(refresh_epoch),
                                    false,
                                )
                                .await;
                        }
                    }
                    .boxed(),
                );
            }
            futures::future::join_all(group).await;
        });
        self.state.with(|state| state.refresh_task = Some(task));
    }
}

impl RiseConversationDomain {
    pub(crate) fn insert_chat_detail_revalidation_in_flight(&self, chat_id: &str) -> bool {
        self.state.with(|state| state.chat_detail_revalidations_in_flight.insert(chat_id.to_string()))
    }

    pub(crate) fn remove_chat_detail_revalidation_in_flight(&self, chat_id: &str) {
        self.state.with(|state| state.chat_detail_revalidations_in_flight.remove(chat_id));
    }

    pub(crate) fn with_media_gallery_walk_task<R>(&self, body: impl FnOnce(&mut Option<JoinHandle<()>>) -> R) -> R {
        self.state.with(|state| body(&mut state.media_gallery_walk_task))
    }

    pub(crate) fn weak_this(&self) -> Weak<Self> {
        self.this.clone()
    }

    pub async fn shutdown(&self) {
        let already = self.state.with(|state| {
            if state.is_shutdown {
                return true;
            }
            state.is_shutdown = true;
            state.message_warmup_generation = state.message_warmup_generation.wrapping_add(1);
            if let Some(task) = state.message_warmup_task.take() {
                task.abort();
            }
            state.is_active = false;
            state.account_epoch = None;
            false
        });
        if already {
            return;
        }
        self.forum_voice.deactivate().await;
        self.chat_voice.deactivate().await;
        self.state.with(|state| {
            if let Some(task) = state.unread_mark_confirmation_task.take() {
                task.abort();
            }
            state.pending_unread_mark_consumptions.clear();
            state.parked_unread_mark_consumptions.clear();
            state.unread_mark_viewports.clear();
            state.unread_mark_observation_counts.clear();
            state.unread_mark_candidates_awaiting_ledger.clear();
        });
        self.unread_marks.release_all().await;
        on_main(|cx| {
            ForumVoiceSubscriptionRegistry::shared(cx).update(cx, |registry, cx| registry.release_all(cx));
        })
        .await;

        let mut cancelled = self.state.with(|state| {
            let mut cancelled: Vec<JoinHandle<()>> = Vec::new();
            cancelled.extend(state.refresh_task.take());
            if let Some(task) = state.badge_refresh_task.take() {
                task.abort();
            }
            cancelled.extend(state.durable_lifecycle_task.take());
            if let Some(task) = state.pinned_page_patches_task.take() {
                task.abort();
            }
            for task in state.message_search_tasks.drain().map(|(_, task)| task) {
                task.abort();
            }
            state.message_search_generations.clear();
            state.message_search_scopes.clear();
            cancelled
        });
        self.message_search.clear_all().await;
        let more = self.state.with(|state| {
            let mut tasks: Vec<JoinHandle<()>> = Vec::new();
            tasks.extend(state.global_message_search_tasks.drain().map(|(_, task)| task));
            state.global_message_search_generations.clear();
            state.global_message_search_scopes.clear();
            tasks.extend(state.chat_overlay_entity_search_task.take());
            tasks.extend(state.chat_overlay_chat_search_task.take());
            tasks.extend(state.chat_overlay_post_search_task.take());
            state.chat_overlay_entity_search_scope = None;
            state.chat_overlay_chat_search_scope = None;
            state.chat_overlay_post_search_scope = None;

            tasks.append(&mut state.chat_observation_tasks);
            tasks.extend(state.message_observation_tasks.drain().map(|(_, task)| task));
            tasks.extend(state.message_observation_recovery_tasks.drain().map(|(_, task)| task));
            state.message_observation_recovery_tokens.clear();
            state.message_observation_recovery_attempts.clear();
            tasks.extend(state.message_seed_recovery_tasks.drain().map(|(_, task)| task));
            state.message_seed_recovery_tokens.clear();
            state.message_seed_recovery_attempts.clear();
            if let Some(task) = state.unknown_chat_refresh_task.take() {
                task.abort();
            }
            state.unknown_chat_refresh_queued = false;
            state.unknown_chat_refresh_attempts.clear();
            state.unknown_chat_refresh_retried.clear();
            state.message_observation_generations.clear();
            state.message_observation_anchors.clear();
            state.message_observation_configurations.clear();
            state.message_presentation_generations.clear();
            state.message_navigation_generations.clear();
            state.active_message_jumps.clear();
            state.message_observation_owners.clear();
            state.message_owner_generations.clear();
            state.synchronized_chat_epochs.clear();
            state.synchronizing_chat_epochs.clear();
            state.synchronized_message_epochs.clear();
            state.synchronizing_message_epochs.clear();
            state.pending_forced_message_syncs.clear();
            state.message_synchronization_generations.clear();
            state.synchronizing_message_generations.clear();
            state.synchronized_message_anchors.clear();
            tasks.extend(state.typing_observation_tasks.drain().map(|(_, task)| task));
            state.typing_observation_generations.clear();
            tasks
        });
        cancelled.extend(more);
        for task in &cancelled {
            task.abort();
        }
        for task in cancelled {
            let _ = task.await;
        }

        self.messages.shutdown().await;
        self.stickers.shutdown().await;
        self.pack_collaboration.shutdown().await;
        self.upload_realtime_progress.shutdown().await;
        self.state.with(|state| {
            if let Some(task) = state.media_gallery_walk_task.take() {
                task.abort();
            }
        });
        self.auxiliary.invalidate_pending_requests().await;
        self.auxiliary.clear_memory().await;
        self.media_gallery.clear_memory().await;
        self.message_presentation_preparer.reset().await;
        self.pinned_message_presentation_preparer.reset().await;
        self.message_tab_presentation_preparer.reset().await;
        self.global_message_search_presentation_preparer.reset().await;
        self.chat_overlay_search_presentation_preparer.reset().await;
        self.state.with(|state| {
            state.chat_search_recents = Vec::new();
            state.latest_chat_snapshots.clear();
            state.chat_presentation_preparer = RiseChatPresentationPreparer::default();
            state.chat_presentation_ready = false;
            state.latest_message_snapshots.clear();
            state.message_history_clear_revisions_by_chat.clear();
        });
        let Some(this) = self.this.upgrade() else {
            return;
        };
        on_main(move |cx| {
            let account_id = this.account_id.clone();
            RiseConversationDomainRegistry::shared(cx).update(cx, |registry, cx| registry.remove(&this, cx));
            RiseChatMediaGalleryPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.deactivate(&account_id, cx);
            });
            let presentation = RiseConversationPresentation::shared(cx);
            if presentation.read(cx).account_id() == Some(account_id.as_str()) {
                presentation.update(cx, |presentation, cx| presentation.deactivate(cx));
                RiseMessageReadReceiptPresentation::shared(cx).update(cx, |presentation, cx| {
                    presentation.deactivate(cx);
                });
                if let Some(hooks) = chat_engine_hooks() {
                    (hooks.remove_all_typing_peers_by_chat)(cx);
                }
            }
        })
        .await;
    }

    pub async fn accept(&self, snapshot: RiseChatListWireSnapshot) {
        let is_newest = |state: &RiseConversationDomainState| {
            snapshot.revision >= state.latest_chat_snapshots.get(&snapshot.kind).map_or(0, |latest| latest.revision)
        };
        let expected_epoch = self.state.with(|state| {
            if state.is_shutdown
                || snapshot.account_id != self.account_id
                || snapshot.rows.len() != snapshot.local_ids.len()
                || snapshot.rows.len() != snapshot.entity_stamps.len()
                || !is_newest(state)
            {
                return None;
            }
            Some(state.account_epoch.clone())
        });
        let Some(expected_epoch) = expected_epoch else {
            return;
        };
        let removed = self.chats.removed_chats().await.ok();
        let still_current =
            self.state.with(|state| !state.is_shutdown && state.account_epoch == expected_epoch && is_newest(state));
        if !still_current {
            return;
        }
        let (removed_local_ids, confirmed_removals) = removed.unwrap_or_default();
        let effective = Self::bounded_chat_head(&snapshot, &removed_local_ids);
        self.publish_chat_snapshot(effective, confirmed_removals, expected_epoch).await;
    }

    async fn publish_chat_snapshot(
        &self,
        snapshot: RiseChatListWireSnapshot,
        confirmed_removals: HashSet<String>,
        expected_epoch: Option<String>,
    ) {
        let prepared = self.state.with(|state| {
            if state.is_shutdown || state.account_epoch != expected_epoch {
                return None;
            }
            state.latest_chat_snapshots.insert(snapshot.kind, snapshot.clone());
            if !state.is_active || !state.chat_presentation_ready {
                return None;
            }
            let expected_epoch = expected_epoch.clone()?;
            let prepared = state.chat_presentation_preparer.prepare(&snapshot, &confirmed_removals)?;
            Some((prepared, expected_epoch))
        });
        let Some((prepared, expected_epoch)) = prepared else {
            return;
        };
        let Some(this) = self.this.upgrade() else {
            return;
        };
        on_main(move |cx| {
            if !this.is_registry_active(cx)
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&this.account_id), Some(&expected_epoch))
            {
                return;
            }
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_chat_list(prepared, cx);
            });
            if let Some(hooks) = chat_engine_hooks() {
                (hooks.adopt_known_chats_for_draft_direct_chats)(cx);
            }
        })
        .await;
    }

    fn bounded_chat_head(
        incoming: &RiseChatListWireSnapshot,
        removed_local_ids: &HashSet<String>,
    ) -> RiseChatListWireSnapshot {
        let mut seen = HashSet::<&str>::new();
        let indices: Vec<usize> = (0..incoming.rows.len())
            .filter(|&index| {
                let local_id = incoming.local_ids[index].as_str();
                !removed_local_ids.contains(local_id) && seen.insert(local_id)
            })
            .take(RiseChatRepository::default_presentation_limit() as usize)
            .collect();
        RiseChatListWireSnapshot {
            account_id: incoming.account_id.clone(),
            kind: incoming.kind,
            revision: incoming.revision,
            rows: indices.iter().map(|&index| incoming.rows[index].clone()).collect(),
            local_ids: indices.iter().map(|&index| incoming.local_ids[index].clone()).collect(),
            entity_stamps: indices.iter().map(|&index| incoming.entity_stamps[index].clone()).collect(),
            loaded_row_count: incoming.loaded_row_count,
            server_total_count: incoming.server_total_count,
            server_total_unread_count: incoming.server_total_unread_count,
            server_unread_chats_count: incoming.server_unread_chats_count,
            server_total_unread_media_count: incoming.server_total_unread_media_count,
            cursor: incoming.cursor.clone(),
            has_more: incoming.has_more,
            load_state: incoming.load_state,
            error_code: incoming.error_code.clone(),
        }
    }

    pub async fn accept_chat_detail(&self, wire: RiseStoredChat, is_archived: Option<bool>) {
        if self.validate_active_account().await.is_err() {
            return;
        }
        let Some(expected_epoch) = self.state.with(|state| state.account_epoch.clone()) else {
            return;
        };
        let model = wire.to_chat_info();
        let id = RiseConversationStoreLayout::canonical_chat_server_id(&wire);
        let Some(this) = self.this.upgrade() else {
            return;
        };
        on_main(move |cx| {
            if !this.is_registry_active(cx)
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&this.account_id), Some(&expected_epoch))
            {
                return;
            }
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_chat_detail_model(model, &id, &this.account_id, is_archived, cx);
            });
        })
        .await;
    }

    pub async fn accept_observation_generation(&self, snapshot: RiseMessageWireSnapshot, observation_generation: u64) {
        let is_observed = |state: &RiseConversationDomainState| {
            state.message_observation_generations.get(&snapshot.scope) == Some(&observation_generation)
                && state.message_observation_tasks.contains_key(&snapshot.scope)
        };
        let accepted = self.state.with(|state| {
            if state.is_shutdown || snapshot.account_id != self.account_id {
                return None;
            }
            if let Some(clear_revision) = state.message_history_clear_revisions_by_chat.get(&snapshot.scope.chat_id)
                && snapshot.revision < *clear_revision
            {
                return None;
            }
            if !is_observed(state) || !state.is_active {
                tracing::warn!(
                    target: "riseonly::X2024",
                    chat = %snapshot.scope.chat_id,
                    rows = snapshot.rows.len(),
                    observed = is_observed(state),
                    generation = observation_generation,
                    "accept drop not observed"
                );
                RiseColdOpenTrace::mark(|| {
                    format!(
                        "x256 accept drop observed={} active={} rows={}",
                        is_observed(state),
                        state.is_active,
                        snapshot.rows.len()
                    )
                });
                return None;
            }
            let Some(expected_account_epoch) = state.account_epoch.clone() else {
                RiseColdOpenTrace::mark(|| "x256 accept drop no_epoch".to_owned());
                return None;
            };
            if let Some(latest) = state.latest_message_snapshots.get(&snapshot.scope)
                && (latest.revision > snapshot.revision
                    || RiseMessagePresentationInputPolicy::is_duplicate(latest, &snapshot))
            {
                tracing::warn!(
                    target: "riseonly::X2024",
                    chat = %snapshot.scope.chat_id,
                    rows = snapshot.rows.len(),
                    revision = snapshot.revision,
                    latest_revision = latest.revision,
                    "accept skip"
                );
                RiseColdOpenTrace::mark(|| {
                    format!(
                        "a3a1 accept skip rows={} rev={} latestRev={}",
                        snapshot.rows.len(),
                        snapshot.revision,
                        latest.revision
                    )
                });
                return None;
            }
            state.latest_message_snapshots.insert(snapshot.scope.clone(), snapshot.clone());
            RiseColdOpenTrace::mark(|| {
                format!(
                    "a3a1 accept rows={} rev={} gen={observation_generation} anchor={} newestTag={}",
                    snapshot.rows.len(),
                    snapshot.revision,
                    if snapshot.anchor_local_id.is_none() { "latest" } else { "id" },
                    snapshot.local_ids.last().map_or_else(|| "none".to_owned(), |id| RiseColdOpenTrace::tag(id))
                )
            });
            let generation = state.message_presentation_generations.entry(snapshot.scope.clone()).or_insert(0);
            *generation = generation.wrapping_add(1);
            Some((expected_account_epoch, *generation))
        });
        let Some((expected_account_epoch, presentation_generation)) = accepted else {
            return;
        };
        let Some(prepared) = self.message_presentation_preparer.prepare(snapshot.clone(), true).await else {
            tracing::warn!(target: "riseonly::X2024", chat = %snapshot.scope.chat_id, rows = snapshot.rows.len(), "accept drop prepare");
            RiseColdOpenTrace::mark(|| format!("x256 accept drop prepare rows={}", snapshot.rows.len()));
            return;
        };
        let is_current = |state: &RiseConversationDomainState| {
            !state.is_shutdown
                && state.is_active
                && state.account_epoch.as_deref() == Some(expected_account_epoch.as_str())
                && is_observed(state)
        };
        let may_publish = self.state.with(|state| {
            is_current(state)
                && state.message_presentation_generations.get(&snapshot.scope) == Some(&presentation_generation)
        });
        if !may_publish {
            tracing::warn!(target: "riseonly::X2024", chat = %snapshot.scope.chat_id, rows = snapshot.rows.len(), "accept drop may_publish");
            RiseColdOpenTrace::mark(|| "x256 accept drop may_publish".to_owned());
            return;
        }
        let Some(this) = self.this.upgrade() else {
            return;
        };
        let main_epoch = expected_account_epoch.clone();
        let published = on_main(move |cx| {
            let registry_active = this.is_registry_active(cx);
            if !registry_active
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&this.account_id), Some(&main_epoch))
            {
                RiseColdOpenTrace::mark(|| format!("x256 accept drop main registry_active={registry_active}"));
                return false;
            }
            let receipt = RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_message_scope(prepared, cx)
            });
            if receipt.is_none() {
                tracing::warn!(target: "riseonly::X2024", "accept publish refused");
            }
            true
        })
        .await
        .unwrap_or(false);
        if !published {
            return;
        }
        self.state.with(|state| {
            if is_current(state) {
                state.message_observation_recovery_attempts.remove(&snapshot.scope);
            }
        });
    }

    async fn note_durable_send_outcome(&self, event: &RiseDurableLifecycleEvent) {
        if self.state.with(|state| state.is_shutdown) {
            return;
        }
        let (RiseDurableLifecycleEvent::ReconciliationCommitted { request_id, kind, .. }
        | RiseDurableLifecycleEvent::Terminal { request_id, kind, .. }) = event
        else {
            return;
        };
        if kind != RiseCreateMessageDurableEndpoint::KIND {
            return;
        }
        self.messages.write_durable_send_outcome_preview(*request_id).await;
    }
}

pub struct RiseConversationDomainRegistry {
    active: Option<Arc<RiseConversationDomain>>,
}

impl RiseConversationDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<Arc<RiseConversationDomain>> {
        self.active.clone()
    }

    pub fn install(&mut self, domain: Arc<RiseConversationDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Arc<RiseConversationDomain>, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        if !Arc::ptr_eq(active, domain) {
            return;
        }
        self.active = None;
        cx.notify();
    }
}

const DEFAULT_PINNED_MESSAGES_LIMIT: i64 = 50;
const DEFAULT_MESSAGE_TAB_LIMIT: i64 = 50;
const DEFAULT_GLOBAL_MESSAGE_SEARCH_LIMIT: i64 = 30;
const DEFAULT_CHAT_OVERLAY_ENTITIES_LIMIT: i64 = 15;
const DEFAULT_CHAT_OVERLAY_CHATS_LIMIT: i64 = 30;
const DEFAULT_CHAT_OVERLAY_POSTS_LIMIT: i64 = 21;
const READ_RECEIPTS_REFRESH_INTERVAL_MS: i64 = 15_000;
const READ_RECEIPTS_OFFLINE_ERROR: &str = "offline";
const HASHTAG_QUERY_PREFIX: &str = "#";
const MENTION_QUERY_PREFIX: &str = "@";
const MEDIA_EDIT_SLOT_INDEX: i64 = 0;
const MESSAGE_LOG_NAME: &str = "Message";

fn is_request_already_in_flight(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<RiseConversationAuxiliaryRepositoryError>(),
        Some(RiseConversationAuxiliaryRepositoryError::RequestAlreadyInFlight(_))
    )
}

fn loading_page<Item: Clone>(
    page: Option<&RiseConversationAuxiliaryPageSnapshot<Item>>,
) -> RiseConversationAuxiliaryPageSnapshot<Item> {
    RiseConversationAuxiliaryPageSnapshot {
        items: page.map_or_else(Vec::new, |page| page.items.clone()),
        cursor: page.and_then(|page| page.cursor.clone()),
        has_more: page.is_some_and(|page| page.has_more),
        revision: page.map_or(0, |page| page.revision),
        load_state: RiseConversationAuxiliaryLoadState::Loading,
        error: None,
        is_persisted: page.is_some_and(|page| page.is_persisted),
    }
}

fn failed_page<Item: Clone>(
    page: Option<&RiseConversationAuxiliaryPageSnapshot<Item>>,
    error: &anyhow::Error,
) -> RiseConversationAuxiliaryPageSnapshot<Item> {
    RiseConversationAuxiliaryPageSnapshot {
        items: page.map_or_else(Vec::new, |page| page.items.clone()),
        cursor: page.and_then(|page| page.cursor.clone()),
        has_more: page.is_some_and(|page| page.has_more),
        revision: page.map_or(0, |page| page.revision),
        load_state: RiseConversationAuxiliaryLoadState::Failed,
        error: Some(CanonErrors::load_failure_text(error)),
        is_persisted: page.is_some_and(|page| page.is_persisted),
    }
}

impl RiseConversationDomain {
    pub async fn load_pinned_messages(
        &self,
        user_id: &str,
        chat_id: &str,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) {
        let limit = limit.unwrap_or(DEFAULT_PINNED_MESSAGES_LIMIT);
        let normalized_chat_id = trimming_whitespaces_and_newlines(chat_id);
        let normalized_user_id = trimming_whitespaces_and_newlines(user_id);
        if normalized_chat_id.is_empty()
            || normalized_user_id.is_empty()
            || self.validate_active_account().await.is_err()
        {
            return;
        }

        let mut holds_complete_list = false;
        if relative_id.is_none()
            && let Ok(cached) = self.auxiliary.cached_pinned_messages(normalized_chat_id).await
        {
            holds_complete_list = !cached.has_more;
            self.publish_pinned_messages(&cached, normalized_chat_id).await;
        }

        let refreshed = match self
            .auxiliary
            .refresh_pinned_messages(normalized_user_id, normalized_chat_id, relative_id, Some(limit))
            .await
        {
            Ok(refreshed) => refreshed,
            Err(error) if is_request_already_in_flight(&error) => return,
            Err(_) => {
                let Ok(failed) = self.auxiliary.cached_pinned_messages(normalized_chat_id).await else {
                    return;
                };
                self.publish_pinned_messages(&failed, normalized_chat_id).await;
                return;
            }
        };
        if !refreshed.has_more {
            self.publish_pinned_messages(&refreshed, normalized_chat_id).await;
            return;
        }
        if !holds_complete_list {
            self.publish_pinned_messages(&refreshed, normalized_chat_id).await;
        }
        let complete = self
            .load_remaining_pinned_messages(normalized_user_id, normalized_chat_id, limit, !holds_complete_list)
            .await;
        if holds_complete_list && let Some(complete) = complete {
            self.publish_pinned_messages(&complete, normalized_chat_id).await;
        }
    }

    async fn load_remaining_pinned_messages(
        &self,
        user_id: &str,
        chat_id: &str,
        limit: i64,
        publishes_each_page: bool,
    ) -> Option<RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>> {
        for _ in 0..PINNED_PAGE_WALK_CEILING {
            self.validate_active_account().await.ok()?;
            let stored = self.auxiliary.cached_pinned_messages(chat_id).await.ok()?;
            if !stored.has_more {
                return Some(stored);
            }
            let cursor = stored.cursor?;
            match self.auxiliary.refresh_pinned_messages(user_id, chat_id, Some(cursor.as_str()), Some(limit)).await {
                Ok(page) => {
                    if publishes_each_page {
                        self.publish_pinned_messages(&page, chat_id).await;
                    }
                }
                Err(error) if is_request_already_in_flight(&error) => {
                    RiseRuntimeTimer::sleep(PINNED_PAGE_WALK_RETRY_DELAY).await;
                }
                Err(_) => return None,
            }
        }
        None
    }

    pub async fn load_message_read_receipts(
        &self,
        user_id: &str,
        chat_id: &str,
        message_id: &str,
        force: bool,
        offline: bool,
    ) {
        let normalized_user_id = trimming_whitespaces_and_newlines(user_id);
        let scope = RiseMessageReadReceiptScope::new(
            trimming_whitespaces_and_newlines(chat_id),
            trimming_whitespaces_and_newlines(message_id),
        );
        if normalized_user_id.is_empty()
            || scope.chat_id.is_empty()
            || scope.message_id.is_empty()
            || self.validate_active_account().await.is_err()
        {
            tracing::warn!(target: "riseonly::X563", "read receipts skipped chat={} message={} user_empty={} (empty scope or inactive account)", scope.chat_id, scope.message_id, normalized_user_id.is_empty());
            return;
        }

        let cached = self.auxiliary.cached_message_read_receipts(&scope.chat_id, &scope.message_id).await.ok();
        if let Some(cached) = &cached {
            self.publish_message_read_receipts(cached.clone(), &scope).await;
        }

        let now_milliseconds = (Date::now().time_interval_since_1970() * 1_000.0) as i64;
        let cached_value = cached.as_ref().and_then(|cached| cached.value.as_ref());
        let should_refresh = force
            || cached_value.is_none()
            || now_milliseconds - cached_value.map_or(0, |value| value.refreshed_at_milliseconds)
                >= READ_RECEIPTS_REFRESH_INTERVAL_MS;
        if !should_refresh {
            return;
        }
        if offline {
            tracing::warn!(target: "riseonly::X563", "read receipts failed chat={} message={} force={force}: transport offline, waits for next connect", scope.chat_id, scope.message_id);
            self.publish_message_read_receipts(
                RiseConversationAuxiliarySnapshot {
                    value: cached_value.cloned(),
                    revision: cached.as_ref().map_or(0, |cached| cached.revision),
                    load_state: RiseConversationAuxiliaryLoadState::Failed,
                    error: Some(READ_RECEIPTS_OFFLINE_ERROR.to_string()),
                    is_persisted: cached.as_ref().is_some_and(|cached| cached.is_persisted),
                },
                &scope,
            )
            .await;
            return;
        }

        match self.auxiliary.refresh_message_read_receipts(normalized_user_id, &scope.chat_id, &scope.message_id).await
        {
            Ok(refreshed) => self.publish_message_read_receipts(refreshed, &scope).await,
            Err(error) if is_request_already_in_flight(&error) => {
                tracing::warn!(target: "riseonly::X563", "read receipts chat={} message={} force={force}: request already in flight", scope.chat_id, scope.message_id);
            }
            Err(error) => {
                tracing::warn!(target: "riseonly::X563", "read receipts failed chat={} message={} force={force}: get_message_read_users error: {error:#}", scope.chat_id, scope.message_id);
                if let Ok(failed) =
                    self.auxiliary.cached_message_read_receipts(&scope.chat_id, &scope.message_id).await
                {
                    self.publish_message_read_receipts(failed, &scope).await;
                }
            }
        }
    }

    fn is_message_projection_current(&self, chat_id: &str, source_revision: i64) -> bool {
        self.state.with(|state| {
            state
                .message_history_clear_revisions_by_chat
                .get(chat_id)
                .is_none_or(|clear_revision| source_revision >= *clear_revision)
        })
    }

    async fn is_projection_publishable(&self, chat_id: &str, source_revision: i64) -> bool {
        self.validate_active_account().await.is_ok() && self.is_message_projection_current(chat_id, source_revision)
    }

    async fn publish_message_read_receipts(
        &self,
        source: RiseConversationAuxiliarySnapshot<RiseMessageReadReceiptStoredResource>,
        scope: &RiseMessageReadReceiptScope,
    ) {
        let revision = source.revision;
        if !self.is_projection_publishable(&scope.chat_id, revision).await {
            return;
        }
        let Some(format_seed) = on_main(RiseMessageReadReceiptFormatSeed::current).await else {
            return;
        };
        let prepared = self
            .message_read_receipt_presentation_preparer
            .prepare(&self.account_id, scope, source, format_seed)
            .await;
        if !self.is_projection_publishable(&scope.chat_id, revision).await {
            return;
        }
        on_main(move |cx| {
            RiseMessageReadReceiptPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(prepared, cx);
            });
        })
        .await;
    }

    pub async fn refresh_pinned_messages_presentation_from_cache(&self, chat_id: &str) {
        let normalized_chat_id = trimming_whitespaces_and_newlines(chat_id);
        if normalized_chat_id.is_empty() || self.validate_active_account().await.is_err() {
            return;
        }
        let Ok(cached) = self.auxiliary.cached_pinned_messages(normalized_chat_id).await else {
            return;
        };
        self.publish_pinned_messages(&cached, normalized_chat_id).await;
    }

    async fn publish_pinned_messages(
        &self,
        page: &RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>,
        chat_id: &str,
    ) {
        let revision = page.revision;
        if !self.is_projection_publishable(chat_id, revision).await {
            return;
        }
        let prepared = self.pinned_message_presentation_preparer.prepare(&self.account_id, chat_id, page).await;
        if !self.is_projection_publishable(chat_id, revision).await {
            return;
        }
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_pinned_messages(prepared, cx);
            });
        })
        .await;
    }

    pub async fn load_message_tab(
        &self,
        user_id: &str,
        scope: &RiseMessageTabScope,
        relative_id: Option<&str>,
        limit: Option<i64>,
        favorites_target_chat_id: Option<&str>,
    ) {
        let limit = limit.unwrap_or(DEFAULT_MESSAGE_TAB_LIMIT);
        let normalized_user_id = trimming_whitespaces_and_newlines(user_id);
        let normalized_chat_id = trimming_whitespaces_and_newlines(&scope.chat_id);
        if normalized_user_id.is_empty()
            || normalized_chat_id.is_empty()
            || self.validate_active_account().await.is_err()
        {
            tracing::warn!(
                target: "riseonly::X357",
                "tab {} chat {normalized_chat_id}: skipped, empty ids or inactive account",
                scope.kind.raw_value()
            );
            return;
        }
        let normalized_scope = RiseMessageTabScope::new(scope.kind, normalized_chat_id.to_string(), scope.topic_id.clone());

        if relative_id.is_none() {
            self.publish_cached_message_tab(&normalized_scope, true).await;
        }

        let outcome = self
            .refresh_message_tab(normalized_user_id, &normalized_scope, relative_id, limit, favorites_target_chat_id)
            .await;
        match &outcome {
            Ok(()) => tracing::warn!(
                target: "riseonly::X357",
                "tab {} chat {normalized_chat_id} relative {relative_id:?}: refreshed",
                normalized_scope.kind.raw_value()
            ),
            Err(error) => tracing::warn!(
                target: "riseonly::X357",
                "tab {} chat {normalized_chat_id} relative {relative_id:?}: failed {error:#}",
                normalized_scope.kind.raw_value()
            ),
        }
        if let Err(error) = outcome
            && !is_request_already_in_flight(&error)
        {
            self.publish_cached_message_tab(&normalized_scope, false).await;
        }
    }

    async fn refresh_message_tab(
        &self,
        user_id: &str,
        scope: &RiseMessageTabScope,
        relative_id: Option<&str>,
        limit: i64,
        favorites_target_chat_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let topic_id = scope.topic_id.as_deref();
        match scope.kind {
            RiseMessageTabKind::Media => {
                let page = self
                    .auxiliary
                    .refresh_media_messages(user_id, &scope.chat_id, topic_id, relative_id, Some(limit))
                    .await?;
                self.publish_message_tab_media(&page, scope).await;
            }
            RiseMessageTabKind::Links => {
                let page = self
                    .auxiliary
                    .refresh_link_messages(user_id, &scope.chat_id, topic_id, relative_id, Some(limit))
                    .await?;
                self.publish_message_tab_links(&page, scope).await;
            }
            RiseMessageTabKind::Files
            | RiseMessageTabKind::Music
            | RiseMessageTabKind::Voice
            | RiseMessageTabKind::Gif
            | RiseMessageTabKind::Favorites => {
                let Some(kind) = scope.kind.message_list_kind() else {
                    return Ok(());
                };
                let favorites_target = if scope.kind == RiseMessageTabKind::Favorites {
                    favorites_target_chat_id.map(trimming_whitespaces_and_newlines)
                } else {
                    None
                };
                let request_chat_id = favorites_target.filter(|target| !target.is_empty());
                let source_chat_id = request_chat_id.map(|_| scope.chat_id.as_str());
                let page = self
                    .auxiliary
                    .refresh_message_list_tab(
                        kind,
                        user_id,
                        &scope.chat_id,
                        topic_id,
                        relative_id,
                        Some(limit),
                        request_chat_id,
                        source_chat_id,
                    )
                    .await?;
                self.publish_message_tab_messages(&page, scope).await;
            }
        }
        Ok(())
    }

    /// refreshing marks a page whose request is about to start
    /// an old failure shows as loading so retry visibly does something
    async fn publish_cached_message_tab(&self, scope: &RiseMessageTabScope, refreshing: bool) {
        let topic_id = scope.topic_id.as_deref();
        match scope.kind {
            RiseMessageTabKind::Media => {
                let Ok(page) = self.auxiliary.cached_media_messages(&scope.chat_id, topic_id).await else {
                    return;
                };
                self.publish_message_tab_media(&if refreshing { page.retrying_failure() } else { page }, scope).await;
            }
            RiseMessageTabKind::Links => {
                let Ok(page) = self.auxiliary.cached_link_messages(&scope.chat_id, topic_id).await else {
                    return;
                };
                self.publish_message_tab_links(&if refreshing { page.retrying_failure() } else { page }, scope).await;
            }
            RiseMessageTabKind::Files
            | RiseMessageTabKind::Music
            | RiseMessageTabKind::Voice
            | RiseMessageTabKind::Gif
            | RiseMessageTabKind::Favorites => {
                let Some(kind) = scope.kind.message_list_kind() else {
                    return;
                };
                let Ok(page) = self.auxiliary.cached_message_list_tab(kind, &scope.chat_id, topic_id).await else {
                    return;
                };
                self.publish_message_tab_messages(&if refreshing { page.retrying_failure() } else { page }, scope)
                    .await;
            }
        }
    }

    async fn publish_message_tab_media(
        &self,
        page: &RiseConversationAuxiliaryPageSnapshot<MediaItemWireDTO>,
        scope: &RiseMessageTabScope,
    ) {
        let revision = page.revision;
        if !self.is_projection_publishable(&scope.chat_id, revision).await {
            return;
        }
        let prepared = self.message_tab_presentation_preparer.prepare_media(&self.account_id, scope.clone(), page).await;
        if !self.is_projection_publishable(&scope.chat_id, revision).await {
            return;
        }
        Self::publish_prepared_message_tab(prepared).await;
    }

    async fn publish_message_tab_messages(
        &self,
        page: &RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>,
        scope: &RiseMessageTabScope,
    ) {
        let revision = page.revision;
        if !self.is_projection_publishable(&scope.chat_id, revision).await {
            return;
        }
        let prepared = self.message_tab_presentation_preparer.prepare_messages(&self.account_id, scope.clone(), page).await;
        if !self.is_projection_publishable(&scope.chat_id, revision).await {
            return;
        }
        Self::publish_prepared_message_tab(prepared).await;
    }

    async fn publish_message_tab_links(
        &self,
        page: &RiseConversationAuxiliaryPageSnapshot<LinkMessageWireDTO>,
        scope: &RiseMessageTabScope,
    ) {
        let revision = page.revision;
        if !self.is_projection_publishable(&scope.chat_id, revision).await {
            return;
        }
        let prepared = self.message_tab_presentation_preparer.prepare_links(&self.account_id, scope.clone(), page).await;
        if !self.is_projection_publishable(&scope.chat_id, revision).await {
            return;
        }
        Self::publish_prepared_message_tab(prepared).await;
    }

    async fn publish_prepared_message_tab(prepared: RiseMessageTabPresentationSnapshot) {
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_message_tab(prepared, cx);
            });
        })
        .await;
    }
}

impl RiseConversationDomain {
    pub async fn load_global_message_search(
        &self,
        kind: ChatSearchMessageKind,
        user_id: &str,
        query: &str,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) {
        let limit = limit.unwrap_or(DEFAULT_GLOBAL_MESSAGE_SEARCH_LIMIT);
        let normalized_user_id = trimming_whitespaces_and_newlines(user_id).to_string();
        let normalized_query = trimming_whitespaces_and_newlines(query).to_string();
        if normalized_user_id.is_empty() {
            return;
        }
        let Some(scope) = RiseGlobalMessageSearchScope::new(kind, &normalized_query) else {
            return;
        };

        let generation = self.state.with(|state| {
            if relative_id.is_none() {
                if let Some(task) = state.global_message_search_tasks.get(&kind) {
                    task.abort();
                }
                let generation = state.global_message_search_generations.get(&kind).copied().unwrap_or(0).wrapping_add(1);
                state.global_message_search_generations.insert(kind, generation);
                state.global_message_search_scopes.insert(kind, scope.clone());
                Some(generation)
            } else if state.global_message_search_scopes.get(&kind) == Some(&scope) {
                Some(state.global_message_search_generations.get(&kind).copied().unwrap_or(0))
            } else {
                None
            }
        });
        let Some(generation) = generation else {
            return;
        };
        let this = self.this.clone();
        let relative_id = relative_id.map(str::to_string);
        let task = RiseAccountRuntime::current().spawn(async move {
            let Some(this) = this.upgrade() else {
                return;
            };
            this.perform_global_message_search(
                kind,
                &normalized_user_id,
                &normalized_query,
                &scope,
                relative_id.as_deref(),
                limit,
                generation,
            )
            .await;
        });
        self.state.with(|state| state.global_message_search_tasks.insert(kind, task));
    }

    #[allow(clippy::too_many_arguments)]
    async fn perform_global_message_search(
        &self,
        kind: ChatSearchMessageKind,
        user_id: &str,
        query: &str,
        scope: &RiseGlobalMessageSearchScope,
        relative_id: Option<&str>,
        limit: i64,
        generation: u64,
    ) {
        if !self.is_current_global_message_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }

        if relative_id.is_none() {
            let cached = self.auxiliary.cached_global_message_search(kind, query).await.ok();
            if let Some(cached) = &cached {
                self.publish_global_message_search(cached, scope, generation).await;
            }
            self.publish_global_message_search(&loading_page(cached.as_ref()), scope, generation).await;
        }

        if !self.is_current_global_message_search(scope, generation) {
            return;
        }
        match self.auxiliary.refresh_global_message_search(kind, user_id, query, relative_id, Some(limit)).await {
            Ok(page) => self.publish_global_message_search(&page, scope, generation).await,
            Err(error) if is_request_already_in_flight(&error) => {}
            Err(error) => {
                if !self.is_current_global_message_search(scope, generation) {
                    return;
                }
                let cached = self.auxiliary.cached_global_message_search(kind, query).await.ok();
                self.publish_global_message_search(&failed_page(cached.as_ref(), &error), scope, generation).await;
            }
        }
    }

    async fn publish_global_message_search(
        &self,
        page: &RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>,
        scope: &RiseGlobalMessageSearchScope,
        generation: u64,
    ) {
        if !self.is_current_global_message_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }
        let prepared = self.global_message_search_presentation_preparer.prepare(&self.account_id, scope, page).await;
        if !self.is_current_global_message_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_global_message_search(prepared, cx);
            });
        })
        .await;
    }

    pub async fn load_chat_overlay_entities(
        &self,
        user_id: &str,
        query: &str,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) {
        let limit = limit.unwrap_or(DEFAULT_CHAT_OVERLAY_ENTITIES_LIMIT);
        let normalized_user_id = trimming_whitespaces_and_newlines(user_id).to_string();
        let normalized_query = trimming_whitespaces_and_newlines(query).to_string();
        if normalized_user_id.is_empty() {
            return;
        }
        let Some(scope) = RiseChatOverlaySearchScope::new(&normalized_query) else {
            return;
        };

        let generation = self.state.with(|state| {
            if relative_id.is_none() {
                if let Some(task) = &state.chat_overlay_entity_search_task {
                    task.abort();
                }
                state.chat_overlay_entity_search_generation = state.chat_overlay_entity_search_generation.wrapping_add(1);
                state.chat_overlay_entity_search_scope = Some(scope.clone());
            } else if state.chat_overlay_entity_search_scope.as_ref() != Some(&scope) {
                return None;
            }
            Some(state.chat_overlay_entity_search_generation)
        });
        let Some(generation) = generation else {
            return;
        };
        let this = self.this.clone();
        let relative_id = relative_id.map(str::to_string);
        let task = RiseAccountRuntime::current().spawn(async move {
            let Some(this) = this.upgrade() else {
                return;
            };
            this.perform_chat_overlay_entity_search(
                &normalized_user_id,
                &normalized_query,
                &scope,
                relative_id.as_deref(),
                limit,
                generation,
            )
            .await;
        });
        self.state.with(|state| state.chat_overlay_entity_search_task = Some(task));
    }

    pub async fn record_chat_search_recent(&self, item: RiseChatSearchRecentItem) {
        let is_live = self.state.with(|state| !state.is_shutdown && state.is_active);
        if !is_live
            || self.validate_active_account().await.is_err()
            || trimming_whitespaces_and_newlines(&item.id).is_empty()
            || trimming_whitespaces_and_newlines(&item.title).is_empty()
        {
            return;
        }

        let snapshot = self.state.with(|state| {
            state.chat_search_recents =
                RiseChatSearchResultsPolicy::inserting_recent(item, &state.chat_search_recents);
            state.chat_search_recents.clone()
        });

        let account_id = self.account_id.clone();
        let items = snapshot.clone();
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_chat_search_recents(&account_id, items, cx);
            });
        })
        .await;
        let _ = self.auxiliary.store_chat_search_recents(snapshot).await;
    }

    pub async fn clear_chat_search_recents(&self) {
        let is_live = self.state.with(|state| !state.is_shutdown && state.is_active);
        if !is_live || self.validate_active_account().await.is_err() {
            return;
        }
        self.state.with(|state| state.chat_search_recents = Vec::new());
        let account_id = self.account_id.clone();
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_chat_search_recents(&account_id, Vec::new(), cx);
            });
        })
        .await;
        let _ = self.auxiliary.clear_chat_search_recents().await;
    }

    async fn perform_chat_overlay_entity_search(
        &self,
        user_id: &str,
        query: &str,
        scope: &RiseChatOverlaySearchScope,
        relative_id: Option<&str>,
        limit: i64,
        generation: u64,
    ) {
        if !self.is_current_chat_overlay_entity_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }

        if relative_id.is_none() {
            let cached = self.auxiliary.cached_chat_overlay_entities(query).await.ok();
            if let Some(cached) = &cached {
                self.publish_chat_overlay_entities(cached, scope, generation).await;
            }
            self.publish_chat_overlay_entities(&loading_page(cached.as_ref()), scope, generation).await;
        }

        if !self.is_current_chat_overlay_entity_search(scope, generation) {
            return;
        }
        match self.auxiliary.refresh_chat_overlay_entities(user_id, query, relative_id, Some(limit)).await {
            Ok(page) => self.publish_chat_overlay_entities(&page, scope, generation).await,
            Err(error) if is_request_already_in_flight(&error) => {}
            Err(error) => {
                if !self.is_current_chat_overlay_entity_search(scope, generation) {
                    return;
                }
                let cached = self.auxiliary.cached_chat_overlay_entities(query).await.ok();
                self.publish_chat_overlay_entities(&failed_page(cached.as_ref(), &error), scope, generation).await;
            }
        }
    }

    pub async fn load_chat_overlay_chats(
        &self,
        user_id: &str,
        query: &str,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) {
        let limit = limit.unwrap_or(DEFAULT_CHAT_OVERLAY_CHATS_LIMIT);
        let normalized_user_id = trimming_whitespaces_and_newlines(user_id).to_string();
        let normalized_query = trimming_whitespaces_and_newlines(query).to_string();
        if normalized_user_id.is_empty() {
            return;
        }
        let Some(scope) = RiseChatOverlaySearchScope::new(&normalized_query) else {
            return;
        };

        let generation = self.state.with(|state| {
            if relative_id.is_none() {
                if let Some(task) = &state.chat_overlay_chat_search_task {
                    task.abort();
                }
                state.chat_overlay_chat_search_generation = state.chat_overlay_chat_search_generation.wrapping_add(1);
                state.chat_overlay_chat_search_scope = Some(scope.clone());
            } else if state.chat_overlay_chat_search_scope.as_ref() != Some(&scope) {
                return None;
            }
            Some(state.chat_overlay_chat_search_generation)
        });
        let Some(generation) = generation else {
            return;
        };
        let this = self.this.clone();
        let relative_id = relative_id.map(str::to_string);
        let task = RiseAccountRuntime::current().spawn(async move {
            let Some(this) = this.upgrade() else {
                return;
            };
            this.perform_chat_overlay_chat_search(
                &normalized_user_id,
                &normalized_query,
                &scope,
                relative_id.as_deref(),
                limit,
                generation,
            )
            .await;
        });
        self.state.with(|state| state.chat_overlay_chat_search_task = Some(task));
    }

    async fn perform_chat_overlay_chat_search(
        &self,
        user_id: &str,
        query: &str,
        scope: &RiseChatOverlaySearchScope,
        relative_id: Option<&str>,
        limit: i64,
        generation: u64,
    ) {
        if !self.is_current_chat_overlay_chat_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }

        if relative_id.is_none() {
            let cached = self.auxiliary.cached_chat_overlay_chats(query).await.ok();
            if let Some(cached) = &cached {
                self.publish_chat_overlay_chats(cached, scope, generation).await;
            }
            self.publish_chat_overlay_chats(&loading_page(cached.as_ref()), scope, generation).await;
        }

        if !self.is_current_chat_overlay_chat_search(scope, generation) {
            return;
        }
        match self.auxiliary.refresh_chat_overlay_chats(user_id, query, relative_id, Some(limit)).await {
            Ok(page) => self.publish_chat_overlay_chats(&page, scope, generation).await,
            Err(error) if is_request_already_in_flight(&error) => {}
            Err(error) => {
                if !self.is_current_chat_overlay_chat_search(scope, generation) {
                    return;
                }
                let cached = self.auxiliary.cached_chat_overlay_chats(query).await.ok();
                self.publish_chat_overlay_chats(&failed_page(cached.as_ref(), &error), scope, generation).await;
            }
        }
    }

    pub async fn load_chat_overlay_posts(&self, query: &str, relative_id: Option<&str>, limit: Option<i64>) {
        let limit = limit.unwrap_or(DEFAULT_CHAT_OVERLAY_POSTS_LIMIT);
        let normalized_query = trimming_whitespaces_and_newlines(query).to_string();
        if normalized_query.is_empty()
            || normalized_query.starts_with(HASHTAG_QUERY_PREFIX)
            || normalized_query.starts_with(MENTION_QUERY_PREFIX)
        {
            return;
        }
        let Some(scope) = RiseChatOverlaySearchScope::new(&normalized_query) else {
            return;
        };

        let generation = self.state.with(|state| {
            if relative_id.is_none() {
                if let Some(task) = &state.chat_overlay_post_search_task {
                    task.abort();
                }
                state.chat_overlay_post_search_generation = state.chat_overlay_post_search_generation.wrapping_add(1);
                state.chat_overlay_post_search_scope = Some(scope.clone());
            } else if state.chat_overlay_post_search_scope.as_ref() != Some(&scope) {
                return None;
            }
            Some(state.chat_overlay_post_search_generation)
        });
        let Some(generation) = generation else {
            return;
        };
        let this = self.this.clone();
        let relative_id = relative_id.map(str::to_string);
        let task = RiseAccountRuntime::current().spawn(async move {
            let Some(this) = this.upgrade() else {
                return;
            };
            this.perform_chat_overlay_post_search(
                &normalized_query,
                &scope,
                relative_id.as_deref(),
                limit,
                generation,
            )
            .await;
        });
        self.state.with(|state| state.chat_overlay_post_search_task = Some(task));
    }

    async fn perform_chat_overlay_post_search(
        &self,
        query: &str,
        scope: &RiseChatOverlaySearchScope,
        relative_id: Option<&str>,
        limit: i64,
        generation: u64,
    ) {
        if !self.is_current_chat_overlay_post_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }

        if relative_id.is_none() {
            let cached = self.auxiliary.cached_chat_overlay_posts(query).await.ok();
            if let Some(cached) = &cached {
                self.publish_chat_overlay_posts(cached, scope, generation).await;
            }
            self.publish_chat_overlay_posts(&loading_page(cached.as_ref()), scope, generation).await;
        }

        if !self.is_current_chat_overlay_post_search(scope, generation) {
            return;
        }
        match self.auxiliary.refresh_chat_overlay_posts(query, relative_id, Some(limit)).await {
            Ok(page) => self.publish_chat_overlay_posts(&page, scope, generation).await,
            Err(error) if is_request_already_in_flight(&error) => {}
            Err(error) => {
                if !self.is_current_chat_overlay_post_search(scope, generation) {
                    return;
                }
                let cached = self.auxiliary.cached_chat_overlay_posts(query).await.ok();
                self.publish_chat_overlay_posts(&failed_page(cached.as_ref(), &error), scope, generation).await;
            }
        }
    }

    async fn publish_chat_overlay_entities(
        &self,
        page: &RiseConversationAuxiliaryPageSnapshot<SearchResultItemDTO>,
        scope: &RiseChatOverlaySearchScope,
        generation: u64,
    ) {
        if !self.is_current_chat_overlay_entity_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }
        let prepared = self.chat_overlay_search_presentation_preparer.prepare_entities(&self.account_id, scope, page).await;
        if !self.is_current_chat_overlay_entity_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_chat_overlay_entity_search(prepared, cx);
            });
        })
        .await;
    }

    async fn publish_chat_overlay_chats(
        &self,
        page: &RiseConversationAuxiliaryPageSnapshot<RiseStoredChat>,
        scope: &RiseChatOverlaySearchScope,
        generation: u64,
    ) {
        if !self.is_current_chat_overlay_chat_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }
        let prepared = self.chat_overlay_search_presentation_preparer.prepare_chats(&self.account_id, scope, page).await;
        if !self.is_current_chat_overlay_chat_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_chat_overlay_chat_search(prepared, cx);
            });
        })
        .await;
    }

    async fn publish_chat_overlay_posts(
        &self,
        page: &RiseConversationAuxiliaryPageSnapshot<GetFeedPostItemDTO>,
        scope: &RiseChatOverlaySearchScope,
        generation: u64,
    ) {
        if !self.is_current_chat_overlay_post_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }
        let prepared = self.chat_overlay_search_presentation_preparer.prepare_posts(&self.account_id, scope, page).await;
        if !self.is_current_chat_overlay_post_search(scope, generation)
            || self.validate_active_account().await.is_err()
        {
            return;
        }
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_chat_overlay_post_search(prepared, cx);
            });
        })
        .await;
    }

    /// starts a search on one surface, a chat or the global section of a chat kind
    /// old query on the same surface is superseded, an identical one replays the cache
    pub async fn search_messages(&self, scope: RiseMessageSearchScope, user_id: &str) {
        let normalized_user_id = trimming_whitespaces_and_newlines(user_id).to_string();
        if normalized_user_id.is_empty() {
            return;
        }
        let key = scope.session_key();
        let generation = self.state.with(|state| {
            if let Some(previous) = state.message_search_scopes.get(&key)
                && *previous != scope
                && let Some(task) = state.message_search_tasks.remove(previous)
            {
                task.abort();
            }
            if let Some(task) = state.message_search_tasks.remove(&scope) {
                task.abort();
            }
            let generation = state.message_search_generations.get(&key).copied().unwrap_or(0).wrapping_add(1);
            state.message_search_generations.insert(key.clone(), generation);
            state.message_search_scopes.insert(key.clone(), scope.clone());
            generation
        });
        let this = self.this.clone();
        let task_scope = scope.clone();
        let task = RiseAccountRuntime::current().spawn(async move {
            let Some(this) = this.upgrade() else {
                return;
            };
            this.perform_message_search(&task_scope, &normalized_user_id, generation).await;
        });
        self.state.with(|state| state.message_search_tasks.insert(scope, task));
    }

    pub async fn load_more_message_search(&self, scope: RiseMessageSearchScope, user_id: &str) {
        self.page_message_search(scope, user_id, false);
    }

    pub async fn load_newer_message_search(&self, scope: RiseMessageSearchScope, user_id: &str) {
        self.page_message_search(scope, user_id, true);
    }

    pub async fn clear_message_search(&self, session_key: &str) {
        let scope = self.state.with(|state| {
            let scope = state.message_search_scopes.remove(session_key)?;
            if let Some(task) = state.message_search_tasks.remove(&scope) {
                task.abort();
            }
            let generation = state.message_search_generations.entry(session_key.to_string()).or_insert(0);
            *generation = generation.wrapping_add(1);
            Some(scope)
        });
        let Some(scope) = scope else {
            return;
        };
        let this = self.this.clone();
        RiseAccountRuntime::current().spawn(async move {
            if let Some(this) = this.upgrade() {
                this.message_search.clear(&scope).await;
            }
        });
    }

    fn page_message_search(&self, scope: RiseMessageSearchScope, user_id: &str, newer: bool) {
        let normalized_user_id = trimming_whitespaces_and_newlines(user_id).to_string();
        if normalized_user_id.is_empty() {
            return;
        }
        let generation = self.state.with(|state| {
            let key = scope.session_key();
            if state.message_search_scopes.get(&key) != Some(&scope) {
                return None;
            }
            state.message_search_generations.get(&key).copied()
        });
        let Some(generation) = generation else {
            return;
        };
        let this = self.this.clone();
        RiseAccountRuntime::current().spawn(async move {
            let Some(this) = this.upgrade() else {
                return;
            };
            if !this.is_current_message_search(&scope, generation) || this.validate_active_account().await.is_err() {
                return;
            }
            let page = if newer {
                this.message_search.load_newer(&scope, &normalized_user_id).await
            } else {
                this.message_search.load_more(&scope, &normalized_user_id).await
            };
            match page {
                Ok(Some(page)) => this.publish_message_search(page, &scope, generation).await,
                Ok(None) => {}
                Err(error) if error.is::<RiseMessageSearchError>() => {}
                Err(error) => {
                    if !this.is_current_message_search(&scope, generation) {
                        return;
                    }
                    let Some(cached) = this.message_search.cached(&scope).await else {
                        return;
                    };
                    let failed = this.failed_message_search_snapshot(Some(&cached), &scope, &error, true);
                    this.publish_message_search(failed, &scope, generation).await;
                }
            }
        });
    }

    async fn perform_message_search(&self, scope: &RiseMessageSearchScope, user_id: &str, generation: u64) {
        if !self.is_current_message_search(scope, generation) || self.validate_active_account().await.is_err() {
            return;
        }
        let cached = self.message_search.cached(scope).await;
        let shown = cached.clone().unwrap_or_else(|| RiseMessageSearchPresentationSnapshot {
            account_id: self.account_id.clone(),
            scope: scope.clone(),
            hits: Vec::new(),
            cursor: None,
            has_more: false,
            has_newer: false,
            total_count: None,
            load_state: RiseConversationAuxiliaryLoadState::Loading,
            error: None,
        });
        self.publish_message_search(shown, scope, generation).await;
        match self.message_search.search(scope, user_id).await {
            Ok(page) => self.publish_message_search(page, scope, generation).await,
            Err(error) if error.is::<RiseMessageSearchError>() => {}
            Err(error) => {
                if !self.is_current_message_search(scope, generation) {
                    return;
                }
                let failed = self.failed_message_search_snapshot(cached.as_ref(), scope, &error, false);
                self.publish_message_search(failed, scope, generation).await;
            }
        }
    }

    fn failed_message_search_snapshot(
        &self,
        cached: Option<&RiseMessageSearchPresentationSnapshot>,
        scope: &RiseMessageSearchScope,
        error: &anyhow::Error,
        can_page: bool,
    ) -> RiseMessageSearchPresentationSnapshot {
        tracing::warn!(
            target: "riseonly::message_search",
            global = scope.is_global(),
            can_page,
            error = %format!("{error:#}"),
            "message search page failed"
        );
        RiseMessageSearchPresentationSnapshot {
            account_id: self.account_id.clone(),
            scope: scope.clone(),
            hits: cached.map_or_else(Vec::new, |cached| cached.hits.clone()),
            cursor: if can_page { cached.and_then(|cached| cached.cursor.clone()) } else { None },
            has_more: can_page && cached.is_some_and(|cached| cached.has_more),
            has_newer: can_page && cached.is_some_and(|cached| cached.has_newer),
            total_count: cached.and_then(|cached| cached.total_count),
            load_state: RiseConversationAuxiliaryLoadState::Failed,
            error: Some(CanonErrors::load_failure_text(error)),
        }
    }

    async fn publish_message_search(
        &self,
        snapshot: RiseMessageSearchPresentationSnapshot,
        scope: &RiseMessageSearchScope,
        generation: u64,
    ) {
        if !self.is_current_message_search(scope, generation) || self.validate_active_account().await.is_err() {
            return;
        }
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_message_search(snapshot, cx);
            });
        })
        .await;
    }

    fn is_current_message_search(&self, scope: &RiseMessageSearchScope, generation: u64) -> bool {
        self.state.with(|state| {
            let key = scope.session_key();
            RiseSearchRequestGenerationPolicy::accepts(
                state.message_search_scopes.get(&key),
                state.message_search_generations.get(&key).copied(),
                scope,
                generation,
                false,
                state.is_shutdown,
            )
        })
    }

    fn is_current_global_message_search(&self, scope: &RiseGlobalMessageSearchScope, generation: u64) -> bool {
        self.state.with(|state| {
            RiseSearchRequestGenerationPolicy::accepts(
                state.global_message_search_scopes.get(&scope.kind),
                state.global_message_search_generations.get(&scope.kind).copied(),
                scope,
                generation,
                false,
                state.is_shutdown,
            )
        })
    }

    fn is_current_chat_overlay_entity_search(&self, scope: &RiseChatOverlaySearchScope, generation: u64) -> bool {
        self.state.with(|state| {
            RiseSearchRequestGenerationPolicy::accepts(
                state.chat_overlay_entity_search_scope.as_ref(),
                Some(state.chat_overlay_entity_search_generation),
                scope,
                generation,
                false,
                state.is_shutdown,
            )
        })
    }

    fn is_current_chat_overlay_chat_search(&self, scope: &RiseChatOverlaySearchScope, generation: u64) -> bool {
        self.state.with(|state| {
            RiseSearchRequestGenerationPolicy::accepts(
                state.chat_overlay_chat_search_scope.as_ref(),
                Some(state.chat_overlay_chat_search_generation),
                scope,
                generation,
                false,
                state.is_shutdown,
            )
        })
    }

    fn is_current_chat_overlay_post_search(&self, scope: &RiseChatOverlaySearchScope, generation: u64) -> bool {
        self.state.with(|state| {
            RiseSearchRequestGenerationPolicy::accepts(
                state.chat_overlay_post_search_scope.as_ref(),
                Some(state.chat_overlay_post_search_generation),
                scope,
                generation,
                false,
                state.is_shutdown,
            )
        })
    }
}

impl RiseConversationDomain {
    pub async fn submit_message(
        &self,
        draft: RiseMessageDraft,
        scope: &RiseMessageScope,
        mode: RiseMessageSubmissionMode,
    ) -> anyhow::Result<RiseMessageSubmission> {
        self.validate_active_account().await?;
        let submission = self.messages.create(draft, scope, mode).await?;
        self.validate_active_account().await?;
        Ok(submission)
    }

    pub async fn submit_inline_result(
        &self,
        body: RiseJSONValue,
        optimistic_wire: MessageWireDTO,
        local_id: &str,
        echo_temp_id: &str,
        scope: &RiseMessageScope,
    ) -> anyhow::Result<RiseJSONValue> {
        self.validate_active_account().await?;
        let response = self
            .messages
            .submit_external_message(
                RiseConversationEndpoints::send_inline_result(),
                &body,
                &optimistic_wire,
                local_id,
                Some(echo_temp_id),
                scope,
            )
            .await?;
        self.validate_active_account().await?;
        Ok(response)
    }

    pub async fn wait_for_message_submission(&self, local_id: &str) -> anyhow::Result<()> {
        self.validate_active_account().await?;
        self.messages.wait_for_submission_resolution(local_id).await?;
        self.validate_active_account().await
    }

    pub async fn cancel_outgoing_message(&self, local_id: &str, scope: &RiseMessageScope) {
        if self.validate_active_account().await.is_err() {
            return;
        }
        self.hide_outgoing_message_from_presentation(local_id).await;
        if let Err(error) = self.messages.cancel_outgoing(local_id, scope).await {
            Self::log_outgoing_failure("Rise cancel outgoing", local_id, " failed: ", &error).await;
        }
    }

    pub async fn ingest_call_history(&self, record: RiseCallHistoryRecord) {
        let is_shutdown = self.state.with(|state| state.is_shutdown);
        if is_shutdown || self.validate_active_account().await.is_err() {
            return;
        }
        let ingested = async {
            let Some(wire) = RiseCallHistoryMessage::wire(&record)? else {
                return Ok(());
            };
            tracing::warn!(target: "riseonly::X1848", call = %record.call_id, chat = %record.chat_id, "call history ingest");
            let scope = RiseMessageScope::new(record.chat_id.clone(), None, None);
            self.messages.ingest_authoritative(vec![RiseAuthoritativeMessage::new(wire, scope, None)]).await
        }
        .await;
        if let Err(error) = ingested {
            Self::log_outgoing_failure("Rise call history ingest", &record.call_id, " failed: ", &error).await;
        }
    }

    pub async fn ingest_device_only_message(&self, wire: MessageWireDTO, chat_id: &str) {
        let is_shutdown = self.state.with(|state| state.is_shutdown);
        if is_shutdown || self.validate_active_account().await.is_err() || !RiseMessageRepository::is_device_only_row(&wire)
        {
            return;
        }
        let scope = RiseMessageScope::new(chat_id.to_string(), None, None);
        let ingested = self.messages.ingest_authoritative(vec![RiseAuthoritativeMessage::new(wire, scope, None)]).await;
        if let Err(error) = ingested
            && !error.is::<CancellationError>()
        {
            Self::log_outgoing_failure("Rise device-only ingest", chat_id, " failed: ", &error).await;
        }
    }

    pub async fn outgoing_upload_resend(&self, local_id: &str) -> Option<RiseMessageUploadResend> {
        self.validate_active_account().await.ok()?;
        let retry = self.messages.failed_upload_retry(local_id).await?;
        self.validate_active_account().await.ok()?;
        RiseMessageUploadRetryCodec::resend(&retry)
    }

    pub async fn retry_outgoing_message(&self, local_id: &str, scope: &RiseMessageScope) {
        if self.validate_active_account().await.is_err() {
            return;
        }
        if let Err(error) = self.messages.retry_outgoing(local_id, scope).await {
            Self::log_outgoing_failure("Rise retry outgoing", local_id, " failed: ", &error).await;
        }
    }

    async fn log_outgoing_failure(action: &str, subject: &str, tail: &str, error: &anyhow::Error) {
        if error.is::<CancellationError>() {
            return;
        }
        let line = format!("{action} {subject}{tail}{error}");
        on_main(move |cx| Sai::log(MESSAGE_LOG_NAME, move || line, SaiLogKind::Network, cx)).await;
    }

    pub async fn cancel_outgoing_media_slot(&self, local_id: &str, slot_index: i64) -> bool {
        if slot_index < 0 || self.validate_active_account().await.is_err() {
            return false;
        }
        if !self.messages.cancel_upload_slot(local_id, slot_index).await {
            return false;
        }
        self.hide_outgoing_media_slot_from_presentation(local_id, slot_index).await;
        true
    }

    pub async fn hide_outgoing_message_from_presentation(&self, local_id: &str) {
        if self.state.with(|state| state.is_shutdown || !state.is_active) {
            return;
        }
        let snapshots = self.message_presentation_preparer.hide_outgoing_message(local_id).await;
        self.publish_local_message_presentation_snapshots(snapshots).await;
    }

    async fn hide_outgoing_media_slot_from_presentation(&self, local_id: &str, local_slot_index: i64) {
        if self.state.with(|state| state.is_shutdown || !state.is_active) {
            return;
        }
        let snapshots = self.message_presentation_preparer.hide_outgoing_media_slot(local_id, local_slot_index).await;
        self.publish_local_message_presentation_snapshots(snapshots).await;
    }

    async fn publish_local_message_presentation_snapshots(
        &self,
        snapshots: Vec<Arc<RiseMessageScopePresentationSnapshot>>,
    ) {
        let accepted = self.state.with(|state| {
            if snapshots.is_empty() || state.is_shutdown || !state.is_active {
                return false;
            }
            for snapshot in &snapshots {
                let generation =
                    state.message_presentation_generations.entry(snapshot.source.scope.clone()).or_insert(0);
                *generation = generation.wrapping_add(1);
            }
            true
        });
        if !accepted {
            return;
        }
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                for snapshot in snapshots {
                    presentation.publish_message_scope(snapshot, cx);
                }
            });
        })
        .await;
    }

    pub async fn is_media_edit_cancelled(&self, local_id: &str) -> bool {
        self.messages.is_upload_cancelled(local_id, MEDIA_EDIT_SLOT_INDEX).await
    }

    pub async fn media_edit_cancellation_signal(&self, local_id: &str) -> RiseUploadCancellationSignal {
        self.messages.upload_cancellation_signal(local_id, MEDIA_EDIT_SLOT_INDEX).await
    }

    pub async fn begin_media_edit(
        &self,
        local_id: &str,
        caption: &str,
        local_media: BTreeMap<String, RiseJSONValue>,
        entities: Option<String>,
    ) -> anyhow::Result<MessageMutationCheckpoint> {
        self.validate_active_account().await?;
        let mut payload = BTreeMap::from([
            (rise_key::events::CONTENT.to_string(), RiseJSONValue::String(caption.to_string())),
            (
                rise_key::events::MEDIA_ITEMS.to_string(),
                RiseJSONValue::Array(vec![RiseJSONValue::Object(local_media)]),
            ),
        ]);
        if let Some(entities) = entities {
            payload.insert(rise_key::events::ENTITIES.to_string(), RiseJSONValue::String(entities));
        }
        self.messages
            .begin_prepared_patch(local_id, move |wire| RiseMessageWireCodec::editing(wire, &payload, false))
            .await
    }

    pub async fn rollback_media_edit(&self, checkpoint: MessageMutationCheckpoint) {
        let _ = self.messages.rollback_prepared_patch(checkpoint).await;
    }

    pub async fn publish_media_edit_progress(&self, local_id: &str, fraction: f64) {
        self.messages.publish_prepared_patch_progress(local_id, fraction).await;
    }

    pub async fn finish_media_edit(&self, local_id: &str) {
        self.messages.finish_prepared_patch(local_id).await;
    }

    /// same guard as validate_active_account in a shape other files can call
    /// the throwing one is file private and its error never leaves
    pub async fn is_active_account_session(&self) -> bool {
        self.validate_active_account().await.is_ok()
    }

    async fn validate_active_account(&self) -> anyhow::Result<()> {
        let account_epoch = self.state.with(|state| {
            if state.is_shutdown || !state.is_active { None } else { state.account_epoch.clone() }
        });
        let Some(account_epoch) = account_epoch else {
            return Err(RiseConversationStateError::MissingActiveSession.into());
        };
        let this = self.this.clone();
        let account_id = self.account_id.clone();
        let valid = on_main(move |cx| {
            this.upgrade().is_some_and(|this| this.is_registry_active(cx))
                && SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(account_epoch.as_str()))
        })
        .await;
        if valid != Some(true) {
            return Err(RiseConversationStateError::StaleAccount.into());
        }
        Ok(())
    }
}

const REACTION_ADDED_CHANGE: &str = "added";
const MEMBERSHIP_REFUSAL_SUFFIX: &str = "Not a member of this chat";
const DEFAULT_WINDOW_SHIFT_STRIDE: i64 = 96;
const MAXIMUM_SHIFTED_WINDOW_SPAN: i64 = 320;
const SCREEN_CHATS: &str = "Chats";
const SCREEN_FORUM_HOME: &str = "ForumHome";
const SCREEN_CHAT_TOPICS: &str = "ChatTopics";
const SCREEN_ARCHIVED_CHATS: &str = "ArchivedChats";
const SCREEN_MESSAGE_REQUESTS: &str = "MessageRequests";
const SCREEN_CHAT: &str = "Chat";
const SCREEN_PARAM_TOPIC_ID: &str = "topicId";
const SCREEN_PARAM_TOPIC_MODE: &str = "topicMode";

/// cleanup over the actor state, also runs when the task is aborted at an await
struct RiseConversationStateDefer<'a, F: FnMut(&mut RiseConversationDomainState)> {
    state: &'a RiseIsolated<RiseConversationDomainState>,
    cleanup: F,
}

impl<F: FnMut(&mut RiseConversationDomainState)> Drop for RiseConversationStateDefer<'_, F> {
    fn drop(&mut self) {
        self.state.with(|state| (self.cleanup)(state));
    }
}

fn bump_generation<K: std::hash::Hash + Eq>(generations: &mut HashMap<K, u64>, key: K) -> u64 {
    let generation = generations.entry(key).or_insert(0);
    *generation = generation.wrapping_add(1);
    *generation
}

fn merged_unique(previous: Option<&[String]>, next: &[String]) -> Vec<String> {
    let unique: HashSet<String> = previous.unwrap_or_default().iter().chain(next).cloned().collect();
    unique.into_iter().collect()
}

fn merged_unread_mark_consumption(
    previous: Option<&RiseUnreadMarksConsumption>,
    consumption: &RiseUnreadMarksConsumption,
) -> RiseUnreadMarksConsumption {
    RiseUnreadMarksConsumption {
        scope: consumption.scope.clone(),
        mention_ids: merged_unique(previous.map(|previous| previous.mention_ids.as_slice()), &consumption.mention_ids),
        reply_ids: merged_unique(previous.map(|previous| previous.reply_ids.as_slice()), &consumption.reply_ids),
        reaction_ids: merged_unique(
            previous.map(|previous| previous.reaction_ids.as_slice()),
            &consumption.reaction_ids,
        ),
        reactions_cleared: previous.map_or(0, |previous| previous.reactions_cleared) + consumption.reactions_cleared,
    }
}

impl RiseConversationDomain {
    pub async fn clear_official_bot_local_history(&self, chat_id: &str) -> anyhow::Result<()> {
        self.clear_local_message_history(chat_id, true, None).await
    }

    /// messages of a deleted chat must not survive on device
    /// a direct chat reopens under the same id when either side writes
    /// the server returns only post deletion rows
    pub async fn clear_deleted_chat_local_history(&self, chat_id: &str) -> anyhow::Result<()> {
        self.clear_local_message_history(chat_id, false, None).await
    }

    pub async fn cached_message_history_bytes(&self) -> anyhow::Result<HashMap<String, i64>> {
        self.validate_active_account().await?;
        self.messages.cached_history_bytes_by_chat().await
    }

    pub async fn evict_cached_message_history(&self, chat_ids: Option<&[String]>) -> anyhow::Result<i64> {
        self.validate_active_account().await?;
        let view_bytes = self.messages.cached_history_view_bytes().await?;
        let mut bytes_by_chat: HashMap<String, i64> = HashMap::new();
        for (view, bytes) in &view_bytes {
            if *bytes <= 0 {
                continue;
            }
            let Some(scope) = RiseConversationStoreLayout::message_scope(view) else {
                continue;
            };
            *bytes_by_chat.entry(scope.chat_id).or_insert(0) += *bytes;
        }
        let targets: Vec<String> = match chat_ids {
            Some(chat_ids) => chat_ids.to_vec(),
            None => bytes_by_chat.keys().cloned().collect(),
        };
        let mut freed = 0;
        for chat_id in &targets {
            if self.clear_local_message_history(chat_id, false, Some(&view_bytes)).await.is_err() {
                continue;
            }
            freed += bytes_by_chat.get(chat_id).copied().unwrap_or(0);
        }
        Ok(freed)
    }

    async fn clear_local_message_history(
        &self,
        chat_id: &str,
        preserving_official_bot_row: bool,
        view_bytes: Option<&HashMap<String, i64>>,
    ) -> anyhow::Result<()> {
        self.validate_active_account().await?;
        let Some(expected_account_epoch) = self.state.with(|state| state.account_epoch.clone()) else {
            return Err(RiseConversationStateError::MissingActiveSession.into());
        };
        let receipt = match view_bytes {
            Some(view_bytes) => self.messages.evict_cached_history(chat_id, view_bytes).await?,
            None => self.messages.clear_local_history(chat_id).await?,
        };
        let is_stale = |state: &RiseConversationDomainState| {
            state.is_shutdown || !state.is_active || state.account_epoch.as_ref() != Some(&expected_account_epoch)
        };
        let resolved_chat_id = receipt.scopes.iter().next().map_or_else(|| chat_id.to_string(), |scope| scope.chat_id.clone());

        let stale = self.state.with(|state| {
            if is_stale(state) {
                return true;
            }
            let clear_revision = state.message_history_clear_revisions_by_chat.entry(resolved_chat_id.clone()).or_insert(0);
            *clear_revision = (*clear_revision).max(receipt.store_revision);
            for scope in &receipt.scopes {
                bump_generation(&mut state.message_synchronization_generations, scope.clone());
                state.synchronizing_message_epochs.remove(scope);
                state.synchronizing_message_generations.remove(scope);
                state.pending_forced_message_syncs.remove(scope);
                state.synchronized_message_epochs.remove(scope);
                state.synchronized_message_anchors.remove(scope);
                bump_generation(&mut state.message_presentation_generations, scope.clone());
                bump_generation(&mut state.message_navigation_generations, scope.clone());
                state.active_message_jumps.remove(scope);
                state.latest_message_snapshots.remove(scope);
            }
            false
        });
        if stale {
            return Err(RiseConversationStateError::StaleAccount.into());
        }
        for scope in &receipt.scopes {
            self.message_presentation_preparer.release(scope).await;
        }
        self.pinned_message_presentation_preparer.release(&resolved_chat_id).await;
        self.message_tab_presentation_preparer.release(&resolved_chat_id).await;

        if self.state.with(|state| is_stale(state)) {
            return Err(RiseConversationStateError::StaleAccount.into());
        }
        let this = self.this.clone();
        let account_id = self.account_id.clone();
        let scopes = receipt.scopes;
        on_main(move |cx| {
            if !this.upgrade().is_some_and(|this| this.is_registry_active(cx))
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(expected_account_epoch.as_str()))
            {
                return;
            }
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.clear_message_history(&resolved_chat_id, cx);
            });
            let Some(hooks) = chat_engine_hooks() else {
                return;
            };
            (hooks.clear_local_history)(&resolved_chat_id, scopes, cx);
            if preserving_official_bot_row {
                (hooks.preserve_official_bot_after_history_clear)(&resolved_chat_id, cx);
            }
        })
        .await;
        Ok(())
    }

    pub async fn observe_messages(&self, scope: &RiseMessageScope) -> anyhow::Result<u64> {
        self.observe_messages_full(
            scope,
            RiseMessageAnchor::Latest,
            RiseMessageWindowConfiguration::presentation(),
            false,
            false,
        )
        .await
    }

    pub async fn observe_messages_full(
        &self,
        scope: &RiseMessageScope,
        anchor: RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
        replacing_existing: bool,
        recovering: bool,
    ) -> anyhow::Result<u64> {
        let opening = self.state.with(|state| {
            if state.is_shutdown {
                return Err(anyhow::Error::from(CancellationError));
            }
            if state.message_observation_tasks.contains_key(scope) && !replacing_existing {
                return Ok(Err(state.message_observation_generations.get(scope).copied().unwrap_or(0)));
            }
            if let Some(task) = state.message_observation_recovery_tasks.remove(scope) {
                task.abort();
            }
            state.message_observation_recovery_tokens.remove(scope);
            if !recovering {
                state.message_observation_recovery_attempts.remove(scope);
            }
            Ok(Ok(bump_generation(&mut state.message_observation_generations, scope.clone())))
        })?;
        let generation = match opening {
            Ok(generation) => generation,
            Err(existing) => {
                RiseColdOpenTrace::mark(|| format!("x256 observe reuse gen={existing} topic={:?}", scope.topic_id));
                return Ok(existing);
            }
        };

        RiseColdOpenTrace::mark(|| format!("x256 observe open gen={generation} topic={:?}", scope.topic_id));
        let observation = match self.messages.observe_with_seed(scope, anchor.clone(), configuration.clone()).await {
            Ok(observation) => observation,
            Err(error) => {
                RiseColdOpenTrace::mark(|| format!("x256 observe seed error {error}"));
                let is_current = self.state.with(|state| {
                    if state.is_shutdown || state.message_observation_generations.get(scope) != Some(&generation) {
                        return false;
                    }
                    if let Some(task) = state.message_observation_tasks.remove(scope) {
                        task.abort();
                    }
                    state.message_observation_anchors.insert(scope.clone(), anchor.clone());
                    state.message_observation_configurations.insert(scope.clone(), configuration.clone());
                    true
                });
                if is_current {
                    self.schedule_message_observation_recovery(scope, generation, &anchor, &configuration);
                }
                return Err(error);
            }
        };
        let is_current = self.state.with(|state| {
            let is_current = !state.is_shutdown && state.message_observation_generations.get(scope) == Some(&generation);
            if is_current && let Some(previous) = state.message_observation_tasks.remove(scope) {
                previous.abort();
            }
            is_current
        });
        if !is_current {
            return Err(CancellationError.into());
        }
        let seed = observation.seed;
        let mut updates = observation.updates;
        let owner = self.this.clone();
        let task_scope = scope.clone();
        let task_anchor = anchor.clone();
        let task_configuration = configuration.clone();
        let task = RiseAccountRuntime::current().spawn(async move {
            let mut unexpected_termination = true;
            while let Some(update) = updates.next().await {
                match update {
                    Ok(snapshot) => {
                        if let Some(owner) = owner.upgrade() {
                            owner.accept_observation_generation(snapshot, generation).await;
                        }
                    }
                    Err(error) => {
                        unexpected_termination = !error.is::<CancellationError>();
                        break;
                    }
                }
            }
            RiseColdOpenTrace::mark(|| format!("x256 observe ended unexpected={unexpected_termination}"));
            if let Some(owner) = owner.upgrade() {
                owner.message_observation_did_terminate(
                    &task_scope,
                    generation,
                    &task_anchor,
                    &task_configuration,
                    unexpected_termination,
                );
            }
        });
        self.state.with(|state| {
            state.message_observation_tasks.insert(scope.clone(), task);
            state.message_observation_anchors.insert(scope.clone(), anchor);
            state.message_observation_configurations.insert(scope.clone(), configuration);
        });
        self.accept_observation_generation(seed, generation).await;

        let typing_generation = self.state.with(|state| {
            if state.typing_observation_tasks.contains_key(scope) {
                return None;
            }
            Some(bump_generation(&mut state.typing_observation_generations, scope.clone()))
        });
        if let Some(typing_generation) = typing_generation {
            let mut typing = self.messages.typing_updates(scope).await;
            let may_install = self.state.with(|state| {
                !state.is_shutdown
                    && state.typing_observation_generations.get(scope) == Some(&typing_generation)
                    && !state.typing_observation_tasks.contains_key(scope)
                    && state.message_observation_tasks.contains_key(scope)
            });
            if !may_install {
                return Ok(generation);
            }
            let owner = self.this.clone();
            let task_scope = scope.clone();
            let task = RiseAccountRuntime::current().spawn(async move {
                while let Some(peers) = typing.next().await {
                    if let Some(owner) = owner.upgrade() {
                        owner.accept_typing(peers, &task_scope, typing_generation).await;
                    }
                }
            });
            self.state.with(|state| state.typing_observation_tasks.insert(scope.clone(), task));
        }
        Ok(generation)
    }

    fn message_observation_did_terminate(
        &self,
        scope: &RiseMessageScope,
        generation: u64,
        anchor: &RiseMessageAnchor,
        configuration: &RiseMessageWindowConfiguration,
        unexpected: bool,
    ) {
        let is_current = self.state.with(|state| {
            if state.message_observation_generations.get(scope) != Some(&generation) {
                return false;
            }
            state.message_observation_tasks.remove(scope);
            true
        });
        if is_current && unexpected {
            self.schedule_message_observation_recovery(scope, generation, anchor, configuration);
        }
    }

    fn schedule_message_observation_recovery(
        &self,
        scope: &RiseMessageScope,
        generation: u64,
        anchor: &RiseMessageAnchor,
        configuration: &RiseMessageWindowConfiguration,
    ) {
        let armed = self.state.with(|state| {
            if state.message_observation_recovery_tasks.contains_key(scope) {
                return None;
            }
            let expected_account_epoch = state.account_epoch.clone()?;
            let accepted = RiseMessageObservationRecoveryPolicy::accepts(
                state.is_active,
                state.is_shutdown,
                true,
                state.message_observation_owners.get(scope).is_some_and(|owners| !owners.is_empty()),
                state.message_observation_generations.get(scope).copied(),
                generation,
                state.message_observation_tasks.contains_key(scope),
                state.message_observation_anchors.get(scope) == Some(anchor)
                    && state.message_observation_configurations.get(scope) == Some(configuration),
            );
            if !accepted {
                return None;
            }
            let completed_attempts = state.message_observation_recovery_attempts.get(scope).copied().unwrap_or(0);
            let delay_milliseconds = RiseMessageObservationRecoveryPolicy::delay_milliseconds(completed_attempts)?;
            state.message_observation_recovery_attempts.insert(scope.clone(), completed_attempts + 1);
            let token = Uuid::new_v4();
            state.message_observation_recovery_tokens.insert(scope.clone(), token);
            Some((expected_account_epoch, delay_milliseconds, token))
        });
        let Some((expected_account_epoch, delay_milliseconds, token)) = armed else {
            return;
        };
        let owner = self.this.clone();
        let task_scope = scope.clone();
        let task_anchor = anchor.clone();
        let task_configuration = configuration.clone();
        let task = RiseAccountRuntime::current().spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_milliseconds.max(0) as u64)).await;
            if let Some(owner) = owner.upgrade() {
                owner
                    .recover_message_observation(
                        &task_scope,
                        generation,
                        task_anchor,
                        task_configuration,
                        &expected_account_epoch,
                        token,
                    )
                    .await;
            }
        });
        self.state.with(|state| state.message_observation_recovery_tasks.insert(scope.clone(), task));
    }

    async fn recover_message_observation(
        &self,
        scope: &RiseMessageScope,
        generation: u64,
        anchor: RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
        expected_account_epoch: &str,
        token: Uuid,
    ) {
        if !self.is_current_message_observation_recovery(
            scope,
            generation,
            &anchor,
            &configuration,
            expected_account_epoch,
            token,
        ) || self.validate_active_account().await.is_err()
            || !self.is_current_message_observation_recovery(
                scope,
                generation,
                &anchor,
                &configuration,
                expected_account_epoch,
                token,
            )
        {
            self.clear_message_observation_recovery(scope, token);
            return;
        }
        self.clear_message_observation_recovery(scope, token);

        let Ok(recovered_generation) =
            self.observe_messages_full(scope, anchor.clone(), configuration.clone(), true, true).await
        else {
            return;
        };
        let holds_recovered = |state: &RiseConversationDomainState| {
            !state.is_shutdown
                && state.is_active
                && state.account_epoch.as_deref() == Some(expected_account_epoch)
                && state.message_observation_generations.get(scope) == Some(&recovered_generation)
                && state.message_observation_tasks.contains_key(scope)
                && state.message_observation_owners.get(scope).is_some_and(|owners| !owners.is_empty())
        };
        let is_same_context = self.state.with(|state| {
            holds_recovered(state)
                && state.message_observation_anchors.get(scope) == Some(&anchor)
                && state.message_observation_configurations.get(scope) == Some(&configuration)
        });
        if !is_same_context {
            return;
        }
        let around = if anchor == RiseMessageAnchor::Latest { None } else { Some(anchor.clone()) };
        self.synchronize_messages_if_needed(scope, around, None, None, true).await;
        if !self.state.with(|state| holds_recovered(state)) {
            return;
        }
        if let Ok(snapshot) = self.messages.snapshot(scope, anchor, configuration).await {
            self.accept_observation_generation(snapshot, recovered_generation).await;
        }
    }

    fn is_current_message_observation_recovery(
        &self,
        scope: &RiseMessageScope,
        generation: u64,
        anchor: &RiseMessageAnchor,
        configuration: &RiseMessageWindowConfiguration,
        expected_account_epoch: &str,
        token: Uuid,
    ) -> bool {
        self.state.with(|state| {
            state.message_observation_recovery_tokens.get(scope) == Some(&token)
                && RiseMessageObservationRecoveryPolicy::accepts(
                    state.is_active,
                    state.is_shutdown,
                    state.account_epoch.as_deref() == Some(expected_account_epoch),
                    state.message_observation_owners.get(scope).is_some_and(|owners| !owners.is_empty()),
                    state.message_observation_generations.get(scope).copied(),
                    generation,
                    state.message_observation_tasks.contains_key(scope),
                    state.message_observation_anchors.get(scope) == Some(anchor)
                        && state.message_observation_configurations.get(scope) == Some(configuration),
                )
        })
    }

    fn clear_message_observation_recovery(&self, scope: &RiseMessageScope, token: Uuid) {
        self.state.with(|state| {
            if state.message_observation_recovery_tokens.get(scope) != Some(&token) {
                return;
            }
            state.message_observation_recovery_tasks.remove(scope);
            state.message_observation_recovery_tokens.remove(scope);
        });
    }

    pub async fn chat_list_typing_updates(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
    ) -> anyhow::Result<RiseMessageTypingStream> {
        self.validate_active_account().await?;
        let scope = RiseMessageScope::new(chat_id.to_string(), topic_id.map(str::to_string), None);
        Ok(self.messages.typing_hub.stream(&scope, true).await)
    }

    async fn accept_typing(&self, peers: Vec<RiseMessageTypingPeer>, scope: &RiseMessageScope, typing_generation: u64) {
        let is_current = |state: &RiseConversationDomainState| {
            state.typing_observation_generations.get(scope) == Some(&typing_generation)
                && state.typing_observation_tasks.contains_key(scope)
        };
        if !self.state.with(|state| is_current(state)) || self.validate_active_account().await.is_err() {
            return;
        }
        let Some(expected_epoch) = self.state.with(|state| is_current(state).then(|| state.account_epoch.clone())) else {
            return;
        };
        let this = self.this.clone();
        let account_id = self.account_id.clone();
        let storage_key = scope.storage_key();
        let chat_id = scope.chat_id.clone();
        on_main(move |cx| {
            if !this.upgrade().is_some_and(|this| this.is_registry_active(cx))
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), expected_epoch.as_deref())
            {
                return;
            }
            let value = (!peers.is_empty()).then(|| {
                peers
                    .into_iter()
                    .map(|peer| ChatTypingPeer {
                        user_id: peer.user_id,
                        name: peer.name,
                        action: peer.action,
                        status_text: peer.status_text,
                        at: peer.updated_at.time_interval_since_1970(),
                    })
                    .collect::<Vec<_>>()
            });
            let Some(hooks) = chat_engine_hooks() else {
                return;
            };
            (hooks.set_typing_peers_by_chat)(&storage_key, value, cx);
        })
        .await;
    }

    /// opens the window of a scope, optionally at a requested message
    /// jump_anchor displaces the saved reading position instead of racing it
    /// outcome is for jump_anchor, empty without
    pub async fn load_initial_messages(
        &self,
        scope: &RiseMessageScope,
        preferred_anchor: Option<&str>,
        jump_anchor: Option<&str>,
        anchor_storage_key: Option<&str>,
        owner_id: Option<Uuid>,
        owner_generation: Option<u64>,
    ) -> RiseMessageJumpOutcome {
        let (navigation_generation, initial_configuration) = self.state.with(|state| {
            (
                state.message_navigation_generations.get(scope).copied().unwrap_or(0),
                state
                    .message_observation_configurations
                    .get(scope)
                    .cloned()
                    .unwrap_or_else(RiseMessageWindowConfiguration::initial_viewport),
            )
        });
        let scroll_anchor_storage_key = match anchor_storage_key {
            Some(key) if !key.is_empty() => key.to_string(),
            _ => scope.storage_key(),
        };
        let outcome = self
            .perform_load_initial_messages(
                scope,
                preferred_anchor,
                jump_anchor,
                &scroll_anchor_storage_key,
                (owner_id, owner_generation),
                navigation_generation,
                initial_configuration,
            )
            .await;
        match outcome {
            Ok(outcome) => outcome,
            Err(_) if self.is_current_message_navigation(scope, navigation_generation) => {
                RiseMessageJumpOutcome::Failed
            }
            Err(_) => RiseMessageJumpOutcome::Superseded,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn perform_load_initial_messages(
        &self,
        scope: &RiseMessageScope,
        preferred_anchor: Option<&str>,
        jump_anchor: Option<&str>,
        scroll_anchor_storage_key: &str,
        (owner_id, owner_generation): (Option<Uuid>, Option<u64>),
        navigation_generation: u64,
        initial_configuration: RiseMessageWindowConfiguration,
    ) -> anyhow::Result<RiseMessageJumpOutcome> {
        if !self.register_message_observation_owner(scope, owner_id, owner_generation) {
            return Ok(RiseMessageJumpOutcome::Superseded);
        }
        self.cancel_message_seed_recovery(scope);
        self.state.with(|state| state.message_seed_recovery_attempts.remove(scope));
        if !self.owns_message_observation(scope, owner_id, owner_generation) {
            return Ok(RiseMessageJumpOutcome::Superseded);
        }
        let is_claimed = |domain: &Self| {
            domain.owns_message_observation(scope, owner_id, owner_generation)
                && domain.is_current_message_navigation(scope, navigation_generation)
        };
        let requested_jump_anchor = jump_anchor.map(trimming_whitespaces_and_newlines).filter(|value| !value.is_empty());
        let normalized_preferred_anchor = requested_jump_anchor
            .or_else(|| preferred_anchor.map(trimming_whitespaces_and_newlines))
            .filter(|value| !value.is_empty());
        if let Some(normalized_preferred_anchor) = normalized_preferred_anchor {
            for anchor in [
                RiseMessageAnchor::ServerID(normalized_preferred_anchor.to_string()),
                RiseMessageAnchor::LocalID(normalized_preferred_anchor.to_string()),
            ] {
                let observed =
                    self.observe_messages_full(scope, anchor.clone(), initial_configuration.clone(), true, false).await;
                if observed.is_ok() && self.state.with(|state| state.latest_message_snapshots.contains_key(scope)) {
                    if !is_claimed(self) {
                        return Ok(RiseMessageJumpOutcome::Superseded);
                    }
                    self.synchronize_messages_if_needed(scope, Some(anchor), Some(scroll_anchor_storage_key), None, true)
                        .await;
                    return Ok(RiseMessageJumpOutcome::Installed);
                }
            }
        }

        if !is_claimed(self) {
            return Ok(RiseMessageJumpOutcome::Superseded);
        }
        self.observe_messages_full(scope, RiseMessageAnchor::Latest, initial_configuration.clone(), false, false).await?;
        let Some((initial_row_count, initial_load_state)) = self.state.with(|state| {
            state
                .latest_message_snapshots
                .get(scope)
                .map(|snapshot| (snapshot.rows.len() as i64, snapshot.load_state.raw_value()))
        }) else {
            return Ok(RiseMessageJumpOutcome::Superseded);
        };
        RiseColdOpenTrace::mark(|| format!("messages cache rows={initial_row_count} state={initial_load_state}"));

        if !is_claimed(self) {
            return Ok(RiseMessageJumpOutcome::Superseded);
        }
        if let Some(requested_jump_anchor) = requested_jump_anchor {
            return Ok(self.jump(scope, Some(requested_jump_anchor), None, initial_configuration).await);
        }
        if let Some(normalized_preferred_anchor) = normalized_preferred_anchor {
            let anchor = RiseMessageAnchor::ServerID(normalized_preferred_anchor.to_string());
            self.synchronize_messages_if_needed(scope, Some(anchor.clone()), Some(scroll_anchor_storage_key), None, true)
                .await;
            if !is_claimed(self) {
                return Ok(RiseMessageJumpOutcome::Superseded);
            }
            let observed = self.observe_messages_full(scope, anchor, initial_configuration.clone(), true, false).await;
            if observed.is_ok() && self.state.with(|state| state.latest_message_snapshots.contains_key(scope)) {
                return Ok(RiseMessageJumpOutcome::Installed);
            }
            if !is_claimed(self) {
                return Ok(RiseMessageJumpOutcome::Superseded);
            }
            self.observe_messages_full(scope, RiseMessageAnchor::Latest, initial_configuration, true, false).await?;
            return Ok(RiseMessageJumpOutcome::Installed);
        }
        let recovery_anchor = self.latest_message_recovery_anchor(scope, initial_row_count).await;
        self.synchronize_messages_if_needed(
            scope,
            recovery_anchor.map(RiseMessageAnchor::ServerID),
            Some(scroll_anchor_storage_key),
            None,
            true,
        )
        .await;
        Ok(RiseMessageJumpOutcome::Failed)
    }

    /// false when owner_generation is older than the one this owner already registered
    fn register_message_observation_owner(
        &self,
        scope: &RiseMessageScope,
        owner_id: Option<Uuid>,
        owner_generation: Option<u64>,
    ) -> bool {
        let (Some(owner_id), Some(owner_generation)) = (owner_id, owner_generation) else {
            return true;
        };
        self.state.with(|state| {
            let generations = state.message_owner_generations.entry(scope.clone()).or_default();
            if owner_generation < generations.get(&owner_id).copied().unwrap_or(0) {
                return false;
            }
            generations.insert(owner_id, owner_generation);
            state.message_observation_owners.entry(scope.clone()).or_default().insert(owner_id);
            true
        })
    }

    fn is_current_message_navigation(&self, scope: &RiseMessageScope, generation: u64) -> bool {
        self.state.with(|state| state.message_navigation_generations.get(scope).copied().unwrap_or(0) == generation)
    }

    async fn latest_message_recovery_anchor(&self, scope: &RiseMessageScope, row_count: i64) -> Option<String> {
        let preview = self.chats.chat(&scope.chat_id).await.ok().flatten().and_then(|chat| chat.wire.last_message);
        RiseMessageTimelineRecoveryPolicy::latest_anchor(
            row_count,
            scope.thread_root_id.as_deref(),
            scope.topic_id.as_deref(),
            preview.as_ref().and_then(|preview| preview.id.as_deref()),
            preview.as_ref().and_then(|preview| preview.topic_id.as_deref()),
        )
    }

    /// publishes the persisted list at once
    /// wire read only if the account never materialized the list, activation and reconnect refresh already
    pub async fn load_chats(&self, kind: RiseChatListKind) {
        if let Ok(snapshot) = self.chats.snapshot(kind).await {
            self.accept(snapshot).await;
        }
        self.synchronize_chats_if_needed(kind, false, None).await;
    }

    pub async fn refresh_chats(&self, kind: RiseChatListKind) {
        self.synchronize_chats_if_needed(kind, true, None).await;
    }

    /// a peer new photo or name arrives as an event that skips the viewer photo rule
    /// and the cached chat read keeps the old peer for an hour so the list is read again
    pub async fn refresh_direct_peer_chat(&self, chat_id: &str) {
        let Ok(Some(stored)) = self.chats.stored_chat_id(chat_id, false).await else {
            return;
        };
        let Some(kind) = stored.record.list_kind else {
            return;
        };
        self.synchronize_chats_if_needed(kind, true, None).await;
    }

    /// a hand made unread flag was set or cleared on the server and the frame has no count
    /// so the list is read again and takes the folded count
    pub async fn refresh_chat_row(&self, chat_id: &str) {
        self.refresh_direct_peer_chat(chat_id).await;
    }

    pub async fn apply_chat_unread_flag(&self, chat_id: &str, unread_count: i64) {
        if self.state.with(|state| state.is_shutdown || !state.is_active) {
            return;
        }
        self.chats.apply_chat_unread_flag(chat_id, unread_count).await;
    }

    async fn synchronize_chats_if_needed(&self, kind: RiseChatListKind, force: bool, expected_epoch: Option<u64>) {
        let current_epoch = self.state.with(|state| state.synchronization_epoch);
        let epoch = expected_epoch.unwrap_or(current_epoch);
        if expected_epoch.is_some() && epoch != current_epoch {
            return;
        }
        let has_materialized_view = self.chats.has_materialized_view(kind).await.unwrap_or(false);
        let may_run = self.state.with(|state| {
            if !force && has_materialized_view && state.synchronized_chat_epochs.get(&kind) == Some(&epoch) {
                return false;
            }
            if state.synchronizing_chat_epochs.get(&kind) == Some(&epoch) {
                return false;
            }
            state.synchronizing_chat_epochs.insert(kind, epoch);
            state.synchronized_chat_epochs.insert(kind, epoch);
            true
        });
        if !may_run {
            return;
        }
        let _flight = RiseConversationStateDefer {
            state: &self.state,
            cleanup: move |state: &mut RiseConversationDomainState| {
                if state.synchronizing_chat_epochs.get(&kind) == Some(&epoch) {
                    state.synchronizing_chat_epochs.remove(&kind);
                }
            },
        };
        match self.chats.refresh(kind).await {
            Ok(()) => {
                if let Ok(snapshot) = self.chats.snapshot(kind).await {
                    self.accept(snapshot).await;
                    self.revalidate_assumed_empty_unread_marks().await;
                }
            }
            Err(error) if !error.is::<CancellationError>() => self.schedule_chat_sync_retry(kind, epoch),
            Err(_) => {}
        }
    }

    const CHAT_SYNC_RETRY_DELAYS: [Duration; 3] =
        [Duration::from_secs(2), Duration::from_secs(4), Duration::from_secs(6)];

    fn schedule_chat_sync_retry(&self, kind: RiseChatListKind, epoch: u64) {
        let delay = self.state.with(|state| {
            let previous = state
                .chat_sync_retries
                .get(&kind)
                .and_then(|(retry_epoch, count)| (*retry_epoch == epoch).then_some(*count))
                .unwrap_or(0);
            let delay = Self::CHAT_SYNC_RETRY_DELAYS.get(previous).copied()?;
            state.chat_sync_retries.insert(kind, (epoch, previous + 1));
            Some(delay)
        });
        let Some(delay) = delay else {
            return;
        };
        drop(RiseAccountRuntime::current().spawn(Self::chat_sync_retry_task(self.this.clone(), kind, epoch, delay)));
    }

    fn chat_sync_retry_task(
        owner: Weak<RiseConversationDomain>,
        kind: RiseChatListKind,
        epoch: u64,
        delay: Duration,
    ) -> BoxFuture<'static, ()> {
        async move {
            RiseRuntimeTimer::sleep(delay).await;
            if let Some(owner) = owner.upgrade() {
                owner.retry_chat_sync(kind, epoch).await;
            }
        }
        .boxed()
    }

    async fn retry_chat_sync(&self, kind: RiseChatListKind, epoch: u64) {
        let may_retry = self.state.with(|state| {
            let may_retry = !state.is_shutdown
                && state.is_active
                && epoch == state.synchronization_epoch
                && state.synchronized_chat_epochs.get(&kind) == Some(&epoch)
                && state.synchronizing_chat_epochs.get(&kind) != Some(&epoch);
            if may_retry {
                state.synchronized_chat_epochs.remove(&kind);
            }
            may_retry
        });
        if may_retry {
            self.synchronize_chats_if_needed(kind, false, Some(epoch)).await;
        }
    }

    fn forced_message_sync_task(
        owner: Weak<RiseConversationDomain>,
        scope: RiseMessageScope,
        anchor: Option<RiseMessageAnchor>,
        anchor_storage_key: Option<String>,
    ) -> BoxFuture<'static, ()> {
        async move {
            if let Some(owner) = owner.upgrade() {
                owner.synchronize_messages_if_needed(&scope, anchor, anchor_storage_key.as_deref(), None, true).await;
            }
        }
        .boxed()
    }

    pub fn chat_kinds_to_synchronize(screen: Option<&SaiActiveScreen>) -> HashSet<RiseChatListKind> {
        let Some(screen) = screen else {
            return HashSet::new();
        };
        match screen.id.as_str() {
            SCREEN_CHATS | SCREEN_FORUM_HOME | SCREEN_CHAT_TOPICS => HashSet::from([RiseChatListKind::Inbox]),
            SCREEN_ARCHIVED_CHATS => HashSet::from([RiseChatListKind::Archive]),
            SCREEN_MESSAGE_REQUESTS => HashSet::from([RiseChatListKind::Requests]),
            SCREEN_CHAT
                if screen.params.contains_key(SCREEN_PARAM_TOPIC_ID)
                    || screen.params.contains_key(SCREEN_PARAM_TOPIC_MODE) =>
            {
                HashSet::from([RiseChatListKind::Inbox])
            }
            _ => HashSet::new(),
        }
    }

    pub async fn refresh_messages(&self, scope: &RiseMessageScope) {
        self.synchronize_messages_if_needed(scope, None, None, None, true).await;
    }

    pub async fn revalidate_forum_feed(&self, chat_id: &str) {
        let scope = RiseMessageScope::new(chat_id.to_string(), None, None);
        let around = self.state.with(|state| {
            let is_observed = state.message_observation_tasks.contains_key(&scope)
                || state.message_observation_owners.get(&scope).is_some_and(|owners| !owners.is_empty());
            is_observed.then(|| {
                state
                    .latest_message_snapshots
                    .get(&scope)
                    .and_then(|snapshot| snapshot.anchor_local_id.clone())
                    .map(RiseMessageAnchor::LocalID)
            })
        });
        let Some(around) = around else {
            return;
        };
        self.synchronize_messages_if_needed(&scope, around, None, None, true).await;
    }

    pub async fn forget_lost_topic_rows(&self, chat_id: &str, topic_ids: HashSet<String>) {
        if self.validate_active_account().await.is_err() {
            return;
        }
        let Ok(fallback) = self.messages.forget_topic_rows(chat_id, topic_ids.clone()).await else {
            return;
        };
        let _ = self.chats.replace_hidden_topic_preview(chat_id, &topic_ids, fallback.as_ref()).await;
    }

    async fn synchronize_messages_if_needed(
        &self,
        scope: &RiseMessageScope,
        around: Option<RiseMessageAnchor>,
        anchor_storage_key: Option<&str>,
        expected_epoch: Option<u64>,
        force: bool,
    ) {
        let anchor = around;
        let scroll_anchor_storage_key = anchor_storage_key.filter(|key| !key.is_empty()).map(str::to_string);
        if self.has_active_message_jump(scope) {
            return;
        }
        let flight = self.state.with(|state| {
            let navigation_generation = state.message_navigation_generations.get(scope).copied().unwrap_or(0);
            let epoch = expected_epoch.unwrap_or(state.synchronization_epoch);
            if expected_epoch.is_some() && epoch != state.synchronization_epoch {
                return None;
            }
            if state.synchronizing_message_epochs.get(scope) == Some(&epoch) {
                if force || (expected_epoch.is_some() && !state.pending_forced_message_syncs.contains_key(scope)) {
                    state
                        .pending_forced_message_syncs
                        .insert(scope.clone(), (anchor.clone(), scroll_anchor_storage_key.clone()));
                }
                RiseColdOpenTrace::mark(|| {
                    format!(
                        "a3a1 sync queued force={force} refresh={} queued={}",
                        expected_epoch.is_some(),
                        state.pending_forced_message_syncs.contains_key(scope)
                    )
                });
                return None;
            }
            let request_generation = bump_generation(&mut state.message_synchronization_generations, scope.clone());
            state.synchronizing_message_epochs.insert(scope.clone(), epoch);
            state.synchronizing_message_generations.insert(scope.clone(), request_generation);
            Some((navigation_generation, epoch, request_generation))
        });
        let Some((navigation_generation, epoch, request_generation)) = flight else {
            return;
        };
        let flight_scope = scope.clone();
        let flight_owner = self.this.clone();
        let _flight = RiseConversationStateDefer {
            state: &self.state,
            cleanup: move |state: &mut RiseConversationDomainState| {
                if state.synchronizing_message_epochs.get(&flight_scope) == Some(&epoch)
                    && state.synchronizing_message_generations.get(&flight_scope) == Some(&request_generation)
                {
                    state.synchronizing_message_epochs.remove(&flight_scope);
                    state.synchronizing_message_generations.remove(&flight_scope);
                    if let Some((anchor, anchor_storage_key)) = state.pending_forced_message_syncs.remove(&flight_scope) {
                        drop(RiseAccountRuntime::current().spawn(Self::forced_message_sync_task(
                            flight_owner.clone(),
                            flight_scope.clone(),
                            anchor,
                            anchor_storage_key,
                        )));
                    }
                }
            },
        };

        let has_materialized_view = self.messages.has_materialized_view(scope).await.unwrap_or(false);
        if !self.is_current_message_synchronization(scope, epoch, request_generation, navigation_generation) {
            return;
        }
        let synchronized_epoch = self.state.with(|state| match &anchor {
            Some(anchor) => state
                .synchronized_message_anchors
                .get(scope)
                .filter(|(synchronized, _)| synchronized == anchor)
                .map(|(_, epoch)| *epoch),
            None => state.synchronized_message_epochs.get(scope).copied(),
        });
        let should_synchronize = RiseMessageSynchronizationEpochPolicy::should_synchronize(
            force,
            has_materialized_view,
            synchronized_epoch,
            epoch,
        );
        RiseColdOpenTrace::mark(|| {
            format!(
                "a3a1 sync decision run={should_synchronize} force={force} anchor={} refresh={} epoch={epoch} gen={request_generation}",
                if anchor.is_none() { "latest" } else { "id" },
                expected_epoch.is_some()
            )
        });
        if !should_synchronize {
            return;
        }

        let operation_observation_generation =
            self.state.with(|state| state.message_observation_generations.get(scope).copied());
        let outcome = self
            .perform_message_synchronization(
                scope,
                anchor.as_ref(),
                scroll_anchor_storage_key.as_deref(),
                epoch,
                request_generation,
                navigation_generation,
                operation_observation_generation,
            )
            .await;
        if let Err(error) = outcome {
            let is_current =
                self.is_current_message_synchronization(scope, epoch, request_generation, navigation_generation);
            let is_live = self.state.with(|state| !state.is_shutdown && state.is_active);
            if is_live
                && scope.thread_root_id.is_none()
                && Self::refuses_membership(&error)
                && self.confirm_removed_by_server(&scope.chat_id).await
            {
                return;
            }
            if is_current && !error.is::<CancellationError>() {
                self.schedule_message_seed_recovery(
                    scope,
                    anchor,
                    scroll_anchor_storage_key,
                    navigation_generation,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn perform_message_synchronization(
        &self,
        scope: &RiseMessageScope,
        anchor: Option<&RiseMessageAnchor>,
        scroll_anchor_storage_key: Option<&str>,
        epoch: u64,
        request_generation: u64,
        navigation_generation: u64,
        operation_observation_generation: Option<u64>,
    ) -> anyhow::Result<()> {
        let receipt = self
            .messages
            .load_initial_receipt(scope, anchor.cloned(), RiseMessageWindowConfiguration::presentation())
            .await?;
        let is_current = self.is_current_message_synchronization(scope, epoch, request_generation, navigation_generation);
        RiseColdOpenTrace::mark(|| {
            format!(
                "a3a1 sync receipt outcome={} gen={request_generation} current={is_current} latestFallback={}",
                RiseColdOpenTrace::page_outcome(&receipt.outcome),
                receipt.used_latest_fallback
            )
        });
        if !is_current {
            return Ok(());
        }
        let fallback_configuration = self.state.with(|state| {
            let observes_older_anchor = receipt.used_latest_fallback
                && operation_observation_generation.is_some()
                && state.message_observation_generations.get(scope).copied() == operation_observation_generation
                && state.message_observation_anchors.get(scope) != Some(&RiseMessageAnchor::Latest)
                && state.message_observation_tasks.contains_key(scope);
            observes_older_anchor.then(|| {
                state
                    .message_observation_configurations
                    .get(scope)
                    .cloned()
                    .unwrap_or_else(RiseMessageWindowConfiguration::initial_viewport)
            })
        });
        if let Some(fallback_configuration) = fallback_configuration {
            let latest_generation = self
                .observe_messages_full(scope, RiseMessageAnchor::Latest, fallback_configuration.clone(), true, false)
                .await?;
            let snapshot = self.messages.snapshot(scope, RiseMessageAnchor::Latest, fallback_configuration).await?;
            self.accept_observation_generation(snapshot, latest_generation).await;
        }
        if receipt.used_latest_fallback
            && receipt.anchor_resolution_failed_authoritatively
            && let Some(key) = scroll_anchor_storage_key
        {
            let key = key.to_string();
            on_main(move |cx| ChatScrollPositionStore::clear(&key, cx)).await;
        }
        if !self.is_current_message_synchronization(scope, epoch, request_generation, navigation_generation) {
            return Ok(());
        }
        let outcome = receipt.outcome;
        if RiseMessageSynchronizationEpochPolicy::consumes_epoch(&outcome) {
            self.state.with(|state| match anchor {
                Some(anchor) => {
                    state.synchronized_message_anchors.insert(scope.clone(), (anchor.clone(), epoch));
                    if receipt.used_latest_fallback {
                        state.synchronized_message_epochs.insert(scope.clone(), epoch);
                    }
                }
                None => {
                    state.synchronized_message_epochs.insert(scope.clone(), epoch);
                    state.synchronized_message_anchors.remove(scope);
                }
            });
        }

        match outcome {
            RiseMessagePageLoadOutcome::AlreadyLoading => {
                self.schedule_message_seed_recovery(
                    scope,
                    anchor.cloned(),
                    scroll_anchor_storage_key.map(str::to_string),
                    navigation_generation,
                );
            }
            RiseMessagePageLoadOutcome::StaleResponseDropped => {}
            RiseMessagePageLoadOutcome::Loaded { .. }
            | RiseMessagePageLoadOutcome::CursorStalled
            | RiseMessagePageLoadOutcome::EndReached => {
                self.cancel_message_seed_recovery(scope);
                self.state.with(|state| state.message_seed_recovery_attempts.remove(scope));
            }
        }
        Ok(())
    }

    async fn confirm_removed_by_server(&self, chat_id: &str) -> bool {
        let this = self.this.clone();
        let account_id = self.account_id.clone();
        let expected_epoch = self.state.with(|state| state.account_epoch.clone());
        let announced_chat_id = chat_id.to_string();
        let announce = move |stored: ChatEngineRemovedChatName| {
            let this = this.clone();
            let account_id = account_id.clone();
            let expected_epoch = expected_epoch.clone();
            let chat_id = announced_chat_id.clone();
            async move {
                on_main(move |cx| {
                    if !this.upgrade().is_some_and(|this| this.is_registry_active(cx))
                        || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), expected_epoch.as_deref())
                    {
                        return;
                    }
                    if let Some(hooks) = chat_engine_hooks() {
                        (hooks.announce_removed_from_chat)(&chat_id, Some(stored), cx);
                    }
                })
                .await;
            }
        };
        matches!(self.chats.confirm_removed_by_server(chat_id, announce).await, Ok(true))
    }

    pub fn refuses_membership(error: &anyhow::Error) -> bool {
        error.downcast_ref::<RiseWireRemoteError>().is_some_and(|remote| remote.message.ends_with(MEMBERSHIP_REFUSAL_SUFFIX))
    }

    /// retries a window request that failed while the reader is still on the scope
    /// bounded by the recovery policy, at most three tries and only while the scope has an owner
    /// count resets on a good sync or reconnect or foreground
    fn schedule_message_seed_recovery(
        &self,
        scope: &RiseMessageScope,
        around: Option<RiseMessageAnchor>,
        anchor_storage_key: Option<String>,
        navigation_generation: u64,
    ) {
        let armed = self.state.with(|state| {
            if state.message_seed_recovery_tasks.contains_key(scope) {
                return None;
            }
            let expected_account_epoch = state.account_epoch.clone()?;
            let accepted = RiseMessageSeedRecoveryPolicy::accepts(
                state.is_active,
                state.is_shutdown,
                true,
                state.message_observation_owners.get(scope).is_some_and(|owners| !owners.is_empty()),
                state.message_navigation_generations.get(scope).copied().unwrap_or(0),
                navigation_generation,
            );
            if !accepted {
                return None;
            }
            let completed_attempts = state.message_seed_recovery_attempts.get(scope).copied().unwrap_or(0);
            let delay_milliseconds = RiseMessageSeedRecoveryPolicy::delay_milliseconds(completed_attempts)?;
            state.message_seed_recovery_attempts.insert(scope.clone(), completed_attempts + 1);
            let token = Uuid::new_v4();
            state.message_seed_recovery_tokens.insert(scope.clone(), token);
            Some((expected_account_epoch, delay_milliseconds, token))
        });
        let Some((expected_account_epoch, delay_milliseconds, token)) = armed else {
            return;
        };
        let owner = self.this.clone();
        let task_scope = scope.clone();
        let task = RiseAccountRuntime::current().spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_milliseconds.max(0) as u64)).await;
            if let Some(owner) = owner.upgrade() {
                owner
                    .recover_message_seed(
                        &task_scope,
                        around,
                        anchor_storage_key,
                        navigation_generation,
                        &expected_account_epoch,
                        token,
                    )
                    .await;
            }
        });
        self.state.with(|state| state.message_seed_recovery_tasks.insert(scope.clone(), task));
    }

    async fn recover_message_seed(
        &self,
        scope: &RiseMessageScope,
        around: Option<RiseMessageAnchor>,
        anchor_storage_key: Option<String>,
        navigation_generation: u64,
        expected_account_epoch: &str,
        token: Uuid,
    ) {
        if !self.is_current_message_seed_recovery(scope, navigation_generation, expected_account_epoch, token)
            || self.validate_active_account().await.is_err()
            || !self.is_current_message_seed_recovery(scope, navigation_generation, expected_account_epoch, token)
        {
            self.clear_message_seed_recovery(scope, token);
            return;
        }
        self.clear_message_seed_recovery(scope, token);
        self.synchronize_messages_if_needed(scope, around, anchor_storage_key.as_deref(), None, false).await;
    }

    fn is_current_message_seed_recovery(
        &self,
        scope: &RiseMessageScope,
        navigation_generation: u64,
        expected_account_epoch: &str,
        token: Uuid,
    ) -> bool {
        self.state.with(|state| {
            state.message_seed_recovery_tokens.get(scope) == Some(&token)
                && RiseMessageSeedRecoveryPolicy::accepts(
                    state.is_active,
                    state.is_shutdown,
                    state.account_epoch.as_deref() == Some(expected_account_epoch),
                    state.message_observation_owners.get(scope).is_some_and(|owners| !owners.is_empty()),
                    state.message_navigation_generations.get(scope).copied().unwrap_or(0),
                    navigation_generation,
                )
        })
    }

    fn clear_message_seed_recovery(&self, scope: &RiseMessageScope, token: Uuid) {
        self.state.with(|state| {
            if state.message_seed_recovery_tokens.get(scope) != Some(&token) {
                return;
            }
            state.message_seed_recovery_tasks.remove(scope);
            state.message_seed_recovery_tokens.remove(scope);
        });
    }

    fn cancel_message_seed_recovery(&self, scope: &RiseMessageScope) {
        self.state.with(|state| {
            if let Some(task) = state.message_seed_recovery_tasks.remove(scope) {
                task.abort();
            }
            state.message_seed_recovery_tokens.remove(scope);
        });
    }

    fn is_current_message_synchronization(
        &self,
        scope: &RiseMessageScope,
        epoch: u64,
        generation: u64,
        navigation_generation: u64,
    ) -> bool {
        self.state.with(|state| {
            !state.is_shutdown
                && state.is_active
                && state.message_navigation_generations.get(scope).copied().unwrap_or(0) == navigation_generation
                && RiseMessageSynchronizationFlightPolicy::accepts(
                    state.synchronizing_message_epochs.get(scope).copied(),
                    state.synchronizing_message_generations.get(scope).copied(),
                    state.synchronization_epoch,
                    epoch,
                    generation,
                )
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseConversationDomainChatPageLoad {
    Loaded,
    Busy,
    Failed,
}

impl RiseConversationDomain {
    /// busy when another load of the list was in flight
    /// its commit wakes the viewport observation so there is no failure to back off from
    pub async fn load_more_chats(&self, kind: RiseChatListKind) -> RiseConversationDomainChatPageLoad {
        let loaded = async {
            let loaded = self.chats.load_more(kind).await?;
            let _ = self.chats.take_loaded_page(kind);
            self.accept(self.chats.snapshot(kind).await?).await;
            anyhow::Ok(loaded)
        }
        .await;
        match loaded {
            Ok(true) => RiseConversationDomainChatPageLoad::Loaded,
            Ok(false) => RiseConversationDomainChatPageLoad::Busy,
            Err(_) => RiseConversationDomainChatPageLoad::Failed,
        }
    }

    pub async fn replace_message_history_warmup(&self, scopes: Vec<RiseMessageScope>) {
        let maximum_entry_count = chat_engine_hooks()
            .map_or(0, |hooks| (hooks.warm_cache_limits_production)().maximum_entry_count)
            .max(0) as usize;
        let generation = self.state.with(|state| {
            state.message_warmup_generation = state.message_warmup_generation.wrapping_add(1);
            if let Some(task) = state.message_warmup_task.take() {
                task.abort();
            }
            (!state.is_shutdown && state.is_active && !scopes.is_empty()).then_some(state.message_warmup_generation)
        });
        let Some(generation) = generation else {
            return;
        };
        let candidates: Vec<RiseMessageScope> = scopes.into_iter().take(maximum_entry_count).collect();
        let owner = self.this.clone();
        let task = RiseAccountRuntime::current().spawn(async move {
            if let Some(owner) = owner.upgrade() {
                owner.warm_message_histories(candidates, generation).await;
            }
        });
        self.state.with(|state| state.message_warmup_task = Some(task));
    }

    fn is_current_message_warmup(&self, generation: u64) -> bool {
        self.state.with(|state| !state.is_shutdown && state.is_active && state.message_warmup_generation == generation)
    }

    async fn allows_message_warmup(&self, generation: u64) -> bool {
        on_main(CleverImagePrefetchPolicy::allows_prefetch_now).await == Some(true)
            && self.is_current_message_warmup(generation)
    }

    async fn warm_message_histories(&self, scopes: Vec<RiseMessageScope>, generation: u64) {
        let configuration = RiseMessageWindowConfiguration::initial_viewport();
        let limit = (configuration.before + configuration.after + 1).max(0) as usize;
        let queries: Vec<RiseEntityWindowQuery> = scopes
            .iter()
            .map(|scope| {
                RiseEntityWindowQuery::new(
                    RiseConversationStoreLayout::messages(),
                    RiseConversationStoreLayout::message_view(scope),
                    RiseEntityWindow::Latest { limit },
                )
            })
            .collect();
        let request = RiseStoreSnapshotRequest {
            entity_windows: queries.iter().cloned().collect(),
            ..RiseStoreSnapshotRequest::default()
        };
        if !self.allows_message_warmup(generation).await {
            return;
        }
        let Ok(observation) = self.engine.observe(request.clone()).await else {
            return;
        };
        if !self.prepare_warm_message_histories(&observation.seed, &scopes, &queries, generation).await {
            return;
        }
        let mut updates = observation.updates.make_async_iterator();
        while updates.next().await.is_some() {
            if !self.allows_message_warmup(generation).await {
                return;
            }
            let Ok(snapshot) = self.engine.snapshot(request.clone()).await else {
                return;
            };
            if !self.prepare_warm_message_histories(&snapshot, &scopes, &queries, generation).await {
                return;
            }
        }
    }

    async fn prepare_warm_message_histories(
        &self,
        snapshot: &RiseStoreSnapshot,
        scopes: &[RiseMessageScope],
        queries: &[RiseEntityWindowQuery],
        generation: u64,
    ) -> bool {
        for (scope, query) in scopes.iter().zip(queries) {
            if !self.allows_message_warmup(generation).await {
                return false;
            }
            if self.state.with(|state| state.message_observation_tasks.contains_key(scope)) {
                continue;
            }
            let Ok(source) = self.messages.presentation_snapshot(snapshot, query, scope) else {
                continue;
            };
            if !self.is_current_message_warmup(generation) || source.account_id != self.account_id {
                continue;
            }
            if !self.allows_message_warmup(generation).await {
                return false;
            }
            let is_cleared = self.state.with(|state| {
                state
                    .message_history_clear_revisions_by_chat
                    .get(&scope.chat_id)
                    .is_some_and(|clear_revision| source.revision < *clear_revision)
            });
            if is_cleared {
                continue;
            }
            self.message_presentation_preparer.prepare_warm_snapshot(source).await;
        }
        true
    }

    pub async fn prefetch_message_page(&self, scope: &RiseMessageScope, direction: Option<RiseMessagePageDirection>) {
        if self.state.with(|state| state.is_shutdown || !state.is_active) {
            return;
        }
        self.messages.prefetch_page(scope, direction).await;
    }

    pub async fn load_message_page(
        &self,
        scope: &RiseMessageScope,
        direction: RiseMessagePageDirection,
    ) -> Option<RiseMessagePageLoadOutcome> {
        let loaded = async {
            self.observe_messages(scope).await?;
            self.messages.load_page_receipt(scope, direction).await
        }
        .await;
        match loaded {
            Ok(receipt) => Some(receipt.outcome),
            Err(error) => {
                let detail = format!("message={error}");
                let scope_key = scope.storage_key();
                on_main(move |cx| {
                    MessagePaginationTrace::diagnostic(
                        "domain.error",
                        &scope_key,
                        Some(if direction == RiseMessagePageDirection::Older {
                            MessageHistoryDirection::Older
                        } else {
                            MessageHistoryDirection::Newer
                        }),
                        None,
                        &detail,
                        0.0,
                        cx,
                    );
                })
                .await;
                None
            }
        }
    }

    pub async fn jump_to_message(
        &self,
        scope: &RiseMessageScope,
        server_id: &str,
        configuration: RiseMessageWindowConfiguration,
    ) -> RiseMessageJumpOutcome {
        self.jump(scope, Some(server_id), None, configuration).await
    }

    pub async fn jump_to_date(
        &self,
        scope: &RiseMessageScope,
        day_start_ms: i64,
        configuration: RiseMessageWindowConfiguration,
    ) -> RiseMessageJumpOutcome {
        self.jump(scope, None, Some(day_start_ms), configuration).await
    }

    fn has_active_message_jump(&self, scope: &RiseMessageScope) -> bool {
        self.state.with(|state| {
            state
                .active_message_jumps
                .get(scope)
                .is_some_and(|generation| state.message_navigation_generations.get(scope) == Some(generation))
        })
    }

    async fn jump(
        &self,
        scope: &RiseMessageScope,
        server_id: Option<&str>,
        at_date_ms: Option<i64>,
        configuration: RiseMessageWindowConfiguration,
    ) -> RiseMessageJumpOutcome {
        let navigation_generation = self.state.with(|state| {
            let generation = bump_generation(&mut state.message_navigation_generations, scope.clone());
            state.active_message_jumps.insert(scope.clone(), generation);
            generation
        });
        let jump_scope = scope.clone();
        let _claim = RiseConversationStateDefer {
            state: &self.state,
            cleanup: move |state: &mut RiseConversationDomainState| {
                if state.active_message_jumps.get(&jump_scope) == Some(&navigation_generation) {
                    state.active_message_jumps.remove(&jump_scope);
                }
            },
        };
        let outcome = self.perform_jump(scope, server_id, at_date_ms, configuration, navigation_generation).await;
        match outcome {
            Ok(outcome) => outcome,
            Err(_) if !self.is_message_navigation_claimed(scope, navigation_generation) => {
                RiseMessageJumpOutcome::Superseded
            }
            Err(error) => {
                let anchor_label =
                    server_id.map_or_else(|| format!("date:{}", at_date_ms.unwrap_or(0)), str::to_string);
                let line = format!("Rise jump({}) anchor={anchor_label} failed: {error:?}", scope.storage_key());
                on_main(move |cx| Sai::log(MESSAGE_LOG_NAME, move || line, SaiLogKind::Network, cx)).await;
                RiseMessageJumpOutcome::Failed
            }
        }
    }

    fn is_message_navigation_claimed(&self, scope: &RiseMessageScope, generation: u64) -> bool {
        self.state.with(|state| state.message_navigation_generations.get(scope) == Some(&generation))
    }

    async fn perform_jump(
        &self,
        scope: &RiseMessageScope,
        server_id: Option<&str>,
        at_date_ms: Option<i64>,
        configuration: RiseMessageWindowConfiguration,
        navigation_generation: u64,
    ) -> anyhow::Result<RiseMessageJumpOutcome> {
        let receipt = match at_date_ms {
            Some(at_date_ms) => self.messages.jump_receipt_at_date(scope, at_date_ms, configuration.clone()).await?,
            None => {
                let anchor = RiseMessageAnchor::ServerID(server_id.unwrap_or_default().to_string());
                self.messages.jump_receipt(scope, anchor, configuration.clone()).await?
            }
        };
        if matches!(
            receipt.outcome,
            RiseMessagePageLoadOutcome::StaleResponseDropped | RiseMessagePageLoadOutcome::AlreadyLoading
        ) || !self.is_message_navigation_claimed(scope, navigation_generation)
        {
            return Ok(RiseMessageJumpOutcome::Superseded);
        }
        let Some(server_id) = server_id.map(str::to_string).or(receipt.resolved_anchor_id) else {
            return Ok(RiseMessageJumpOutcome::Failed);
        };
        let anchor = RiseMessageAnchor::ServerID(server_id);
        let observation_generation =
            self.observe_messages_full(scope, anchor.clone(), configuration.clone(), true, false).await?;
        if !self.is_message_navigation_claimed(scope, navigation_generation) {
            return Ok(RiseMessageJumpOutcome::Superseded);
        }
        let snapshot = self.messages.snapshot(scope, anchor.clone(), configuration).await?;
        self.accept_observation_generation(snapshot, observation_generation).await;
        self.state.with(|state| {
            let epoch = state.synchronization_epoch;
            state.synchronized_message_epochs.insert(scope.clone(), epoch);
            state.synchronized_message_anchors.insert(scope.clone(), (anchor, epoch));
        });
        Ok(RiseMessageJumpOutcome::Installed)
    }

    pub async fn shift_message_window(
        &self,
        scope: &RiseMessageScope,
        local_id: &str,
        direction: RiseMessageWindowShift,
        anchor_offset_in_window: Option<i64>,
        stride: Option<i64>,
    ) {
        let stride = stride.unwrap_or(DEFAULT_WINDOW_SHIFT_STRIDE);
        let current_span = self.state.with(|state| {
            let current_window = state
                .message_observation_configurations
                .get(scope)
                .cloned()
                .unwrap_or_else(RiseMessageWindowConfiguration::presentation);
            current_window.before + current_window.after + 1
        });
        let _ = self
            .observe_messages_full(
                scope,
                RiseMessageAnchor::LocalID(local_id.to_string()),
                RiseMessageWindowConfiguration::shifted_anchor_offset_in_window(
                    direction,
                    anchor_offset_in_window,
                    stride,
                    Some(current_span + stride.max(0)),
                    (MAXIMUM_SHIFTED_WINDOW_SPAN - current_span).min(stride).max(0),
                ),
                true,
                false,
            )
            .await;
    }

    pub async fn cached_chat_calendar(
        &self,
        scope: &RiseMessageScope,
        window: &RiseChatCalendarWindow,
    ) -> Option<RiseChatCalendarPage> {
        let cached = self
            .auxiliary
            .cached_chat_calendar(&scope.chat_id, scope.topic_id.as_deref(), scope.thread_root_id.as_deref(), window)
            .await
            .ok()?;
        cached.value.as_ref().map(Self::calendar_page)
    }

    /// one month of the chat calendar for the open scope
    /// cache first, hidden refresh only when nothing cached or caller asked
    /// failed refresh keeps the cached month
    pub async fn chat_calendar(
        &self,
        scope: &RiseMessageScope,
        window: &RiseChatCalendarWindow,
        include_previews: bool,
        force_refresh: bool,
    ) -> Option<RiseChatCalendarPage> {
        let topic_id = scope.topic_id.as_deref();
        let thread_root_id = scope.thread_root_id.as_deref();
        let cached = self.auxiliary.cached_chat_calendar(&scope.chat_id, topic_id, thread_root_id, window).await.ok();
        let cached_page = cached.as_ref().and_then(|cached| cached.value.as_ref()).map(Self::calendar_page);
        if !force_refresh && cached_page.is_some() {
            return cached_page;
        }
        match self
            .auxiliary
            .refresh_chat_calendar(&scope.chat_id, topic_id, thread_root_id, window, include_previews)
            .await
        {
            Ok(refreshed) => refreshed.value.as_ref().map(Self::calendar_page).or(cached_page),
            Err(_) => cached_page,
        }
    }

    fn calendar_page(dto: &GetChatCalendarGatewayDTO) -> RiseChatCalendarPage {
        let days = dto
            .days
            .iter()
            .filter_map(|day| {
                let first_message_id = trimming_whitespaces_and_newlines(&day.first_message_id);
                if first_message_id.is_empty() {
                    return None;
                }
                let preview_url = day.preview_url.as_deref().map(trimming_whitespaces_and_newlines);
                Some(RiseChatCalendarDay {
                    day_start_ms: day.day_start_ms,
                    first_message_id: first_message_id.to_string(),
                    first_message_at_ms: day.first_message_at,
                    preview_url: preview_url.filter(|url| !url.is_empty()).map(str::to_string),
                    preview_is_video: day.preview_is_video,
                })
            })
            .collect();
        RiseChatCalendarPage { days, has_older: dto.has_older }
    }

    pub async fn cancel_message_navigation(&self, scope: &RiseMessageScope) {
        self.state.with(|state| bump_generation(&mut state.message_navigation_generations, scope.clone()));
        self.messages.cancel_seed_request(scope);
    }

    async fn present_latest_window(&self, scope: &RiseMessageScope) -> anyhow::Result<()> {
        let configuration = RiseMessageWindowConfiguration::presentation();
        let generation =
            self.observe_messages_full(scope, RiseMessageAnchor::Latest, configuration.clone(), true, false).await?;
        let snapshot = self.messages.snapshot(scope, RiseMessageAnchor::Latest, configuration).await?;
        self.accept_observation_generation(snapshot, generation).await;
        Ok(())
    }

    pub async fn present_latest_messages(&self, scope: &RiseMessageScope) -> bool {
        self.state.with(|state| bump_generation(&mut state.message_navigation_generations, scope.clone()));
        self.present_latest_window(scope).await.is_ok()
    }

    pub async fn jump_to_latest_messages(&self, scope: &RiseMessageScope) -> bool {
        self.state.with(|state| bump_generation(&mut state.message_navigation_generations, scope.clone()));
        let jumped = async {
            let is_latest = self.messages.is_latest_window(scope).await?;
            tracing::warn!(target: "riseonly::X2024", chat = %scope.chat_id, is_latest, "latest jump start");
            if !is_latest {
                let receipt = self
                    .messages
                    .load_initial_receipt(scope, None, RiseMessageWindowConfiguration::presentation())
                    .await;
                tracing::warn!(
                    target: "riseonly::X2024",
                    chat = %scope.chat_id,
                    outcome = ?receipt.as_ref().map(|receipt| receipt.outcome),
                    store_revision = ?receipt.as_ref().ok().and_then(|receipt| receipt.store_revision),
                    "latest jump receipt"
                );
                receipt?;
            }
            self.present_latest_window(scope).await
        }
        .await;
        tracing::warn!(target: "riseonly::X2024", chat = %scope.chat_id, ok = jumped.is_ok(), "latest jump end");
        jumped.is_ok()
    }

    pub async fn observe_local_messages(
        &self,
        scope: &RiseMessageScope,
        owner_id: Option<Uuid>,
        owner_generation: Option<u64>,
    ) {
        let configuration = self.state.with(|state| {
            (!state.is_shutdown).then(|| {
                state
                    .message_observation_configurations
                    .get(scope)
                    .cloned()
                    .unwrap_or_else(RiseMessageWindowConfiguration::initial_viewport)
            })
        });
        let Some(configuration) = configuration else {
            return;
        };
        if !self.register_message_observation_owner(scope, owner_id, owner_generation) {
            return;
        }
        let _ = self.observe_messages_full(scope, RiseMessageAnchor::Latest, configuration, false, false).await;
    }

    pub async fn release_messages(&self, scope: &RiseMessageScope) {
        let released = self.state.with(|state| {
            if state.is_shutdown || !state.is_active {
                return None;
            }
            let release_account_epoch = state.account_epoch.clone()?;
            bump_generation(&mut state.message_synchronization_generations, scope.clone());
            state.synchronizing_message_epochs.remove(scope);
            state.synchronizing_message_generations.remove(scope);
            state.pending_forced_message_syncs.remove(scope);
            let release_observation_generation =
                bump_generation(&mut state.message_observation_generations, scope.clone());
            if let Some(task) = state.message_observation_recovery_tasks.remove(scope) {
                task.abort();
            }
            state.message_observation_recovery_tokens.remove(scope);
            state.message_observation_recovery_attempts.remove(scope);
            if let Some(task) = state.message_seed_recovery_tasks.remove(scope) {
                task.abort();
            }
            state.message_seed_recovery_tokens.remove(scope);
            state.message_seed_recovery_attempts.remove(scope);
            bump_generation(&mut state.message_presentation_generations, scope.clone());
            bump_generation(&mut state.message_navigation_generations, scope.clone());
            state.active_message_jumps.remove(scope);
            if let Some(task) = state.message_observation_tasks.remove(scope) {
                task.abort();
            }
            state.message_observation_anchors.remove(scope);
            state.message_observation_configurations.remove(scope);
            bump_generation(&mut state.typing_observation_generations, scope.clone());
            if let Some(task) = state.typing_observation_tasks.remove(scope) {
                task.abort();
            }
            state.latest_message_snapshots.remove(scope);
            state.message_observation_owners.remove(scope);
            Some((release_account_epoch, release_observation_generation))
        });
        let Some((release_account_epoch, release_observation_generation)) = released else {
            return;
        };
        self.messages.release(scope).await;
        let still_released = || {
            self.state.with(|state| {
                !state.is_shutdown
                    && state.is_active
                    && state.account_epoch.as_ref() == Some(&release_account_epoch)
                    && state.message_observation_generations.get(scope) == Some(&release_observation_generation)
                    && !state.message_observation_tasks.contains_key(scope)
                    && state.message_observation_owners.get(scope).is_none_or(HashSet::is_empty)
            })
        };
        if !still_released() {
            return;
        }
        let this = self.this.clone();
        let account_id = self.account_id.clone();
        let epoch = release_account_epoch.clone();
        let read_scope = scope.clone();
        let presentation_read = on_main(move |cx| {
            if !this.upgrade().is_some_and(|this| this.is_registry_active(cx))
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(epoch.as_str()))
            {
                return None;
            }
            let presentation = RiseConversationPresentation::shared(cx);
            Some(presentation.read(cx).message_scope_snapshot(&read_scope).map(|snapshot| snapshot.revision))
        })
        .await
        .flatten();
        let Some(expected_presentation_revision) = presentation_read else {
            return;
        };
        if !still_released() {
            return;
        }
        let this = self.this.clone();
        let account_id = self.account_id.clone();
        let epoch = release_account_epoch.clone();
        let remove_scope = scope.clone();
        let removed_presentation = on_main(move |cx| {
            if !this.upgrade().is_some_and(|this| this.is_registry_active(cx))
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(epoch.as_str()))
            {
                return false;
            }
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.remove_message_scope_if_revision(&remove_scope, expected_presentation_revision, cx)
            })
        })
        .await;
        if removed_presentation != Some(true) {
            return;
        }
        let released_preparation =
            self.message_presentation_preparer.retire(scope, expected_presentation_revision).await;
        if !released_preparation || !still_released() {
            return;
        }
        let this = self.this.clone();
        let account_id = self.account_id.clone();
        let storage_key = scope.storage_key();
        let chat_id = scope.chat_id.clone();
        on_main(move |cx| {
            if !this.upgrade().is_some_and(|this| this.is_registry_active(cx))
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(release_account_epoch.as_str()))
            {
                return;
            }
            if let Some(hooks) = chat_engine_hooks() {
                (hooks.set_typing_peers_by_chat)(&storage_key, None, cx);
            }
        })
        .await;
    }

    pub async fn release_messages_storage_key(
        &self,
        storage_key: &str,
        owner_id: Option<Uuid>,
        owner_generation: Option<u64>,
    ) {
        let Some(scope) = RiseMessageScope::parse(storage_key) else {
            return;
        };
        if let (Some(owner_id), Some(owner_generation)) = (owner_id, owner_generation) {
            let retained = self.state.with(|state| {
                let generations = state.message_owner_generations.entry(scope.clone()).or_default();
                if owner_generation < generations.get(&owner_id).copied().unwrap_or(0) {
                    return true;
                }
                generations.insert(owner_id, owner_generation);
                let Some(owners) = state.message_observation_owners.get_mut(&scope) else {
                    return false;
                };
                owners.remove(&owner_id);
                !owners.is_empty()
            });
            if retained {
                return;
            }
        }
        self.release_messages(&scope).await;
    }

    fn owns_message_observation(
        &self,
        scope: &RiseMessageScope,
        owner_id: Option<Uuid>,
        generation: Option<u64>,
    ) -> bool {
        self.state.with(|state| {
            let (Some(owner_id), Some(generation)) = (owner_id, generation) else {
                return !state.is_shutdown;
            };
            !state.is_shutdown
                && state.message_observation_owners.get(scope).is_some_and(|owners| owners.contains(&owner_id))
                && state.message_owner_generations.get(scope).and_then(|owners| owners.get(&owner_id)) == Some(&generation)
        })
    }

    pub async fn compatibility_call(
        &self,
        service: &str,
        method: &str,
        body: RiseJSONValue,
        cache_identity: Option<&str>,
        persistence: RiseConversationResourceKeyPersistence,
        prefer_cached_value: bool,
    ) -> anyhow::Result<RiseJSONValue> {
        if self.state.with(|state| state.is_shutdown || !state.is_active) {
            return Err(RiseConversationStateError::MissingActiveSession.into());
        }
        self.auxiliary
            .compatibility_call(service, method, body, cache_identity, persistence, prefer_cached_value)
            .await
    }
}

impl RiseConversationDomain {
    fn is_unread_marks_live(&self) -> bool {
        self.state.with(|state| !state.is_shutdown && state.is_active)
    }

    pub async fn observe_unread_marks(&self, scope: &RiseMessageScope) {
        if !self.is_unread_marks_live() {
            return;
        }
        self.resend_parked_unread_mark_confirmations();
        self.state.with(|state| *state.unread_mark_observation_counts.entry(scope.clone()).or_insert(0) += 1);
        let stored = self.chats.stored_chat_id(&scope.chat_id, false).await.ok().flatten();
        let wire = stored.as_ref().map(|stored| &stored.record.wire);
        let has_marks = wire.is_some_and(|wire| {
            wire.resolved_unread_mentions_count() > 0
                || wire.resolved_unread_replies_count() > 0
                || wire.resolved_unread_reactions_count() > 0
        });
        if !has_marks && wire.is_some() {
            let snapshot = self.unread_marks.install_empty(scope).await;
            self.publish_unread_marks(vec![snapshot]).await;
            self.consume_unread_marks_on_screen(scope).await;
            return;
        }
        self.refresh_unread_marks(scope).await;
        if !self.state.with(|state| state.unread_mark_observation_counts.contains_key(scope)) {
            self.unread_marks.release(scope).await;
            Self::remove_presented_unread_marks(scope).await;
            return;
        }
        self.consume_unread_marks_on_screen(scope).await;
    }

    async fn remove_presented_unread_marks(scope: &RiseMessageScope) {
        let scope = scope.clone();
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.remove_unread_marks(&scope, cx);
            });
        })
        .await;
    }

    pub async fn release_unread_marks(&self, scope: &RiseMessageScope) {
        let retained = self.state.with(|state| {
            let remaining = state.unread_mark_observation_counts.get(scope).copied().unwrap_or(0) - 1;
            if remaining > 0 {
                state.unread_mark_observation_counts.insert(scope.clone(), remaining);
                return true;
            }
            state.unread_mark_observation_counts.remove(scope);
            state.unread_mark_viewports.remove(scope);
            state.unread_mark_candidates_awaiting_ledger.remove(scope);
            false
        });
        if retained {
            return;
        }
        self.unread_marks.release(scope).await;
        Self::remove_presented_unread_marks(scope).await;
    }

    pub async fn update_unread_mark_viewport(&self, scope: &RiseMessageScope, viewport: RiseUnreadMarkViewport) {
        self.state.with(|state| {
            if !state.is_shutdown && state.is_active {
                state.unread_mark_viewports.insert(scope.clone(), viewport);
            }
        });
    }

    /// everything the screen already showed, rows parked while the ledger loaded and rows on screen now
    /// a stationary viewport reports its rows once so this is their only turn
    fn consume_unread_marks_on_screen<'a>(&'a self, scope: &'a RiseMessageScope) -> BoxFuture<'a, ()> {
        async move {
            if !self.is_unread_marks_live() || !self.unread_marks.is_loaded(scope).await {
                return;
            }
            let (parked, viewport) = self.state.with(|state| {
                (
                    state.unread_mark_candidates_awaiting_ledger.remove(scope).unwrap_or_default(),
                    state.unread_mark_viewports.get(scope).cloned(),
                )
            });
            let mut candidates = parked;
            if let Some(viewport) = viewport {
                for candidate in viewport.current_candidates().await {
                    Self::merge_unread_mark_candidate(candidate, &mut candidates);
                }
            }
            if candidates.is_empty() {
                return;
            }
            self.consume_unread_marks(scope, candidates.into_values().collect()).await;
        }
        .boxed()
    }

    fn merge_unread_mark_candidate(
        candidate: RiseUnreadMarkCandidate,
        candidates: &mut HashMap<String, RiseUnreadMarkCandidate>,
    ) {
        let previous = candidates.get(&candidate.message_id);
        let merged = RiseUnreadMarkCandidate {
            message_id: candidate.message_id.clone(),
            is_mentioned_you: candidate.is_mentioned_you || previous.is_some_and(|previous| previous.is_mentioned_you),
            has_unread_reactions: candidate.has_unread_reactions
                || previous.is_some_and(|previous| previous.has_unread_reactions),
            is_replied_you: candidate.is_replied_you || previous.is_some_and(|previous| previous.is_replied_you),
        };
        candidates.insert(candidate.message_id, merged);
    }

    async fn revalidate_assumed_empty_unread_marks(&self) {
        let scopes: Vec<RiseMessageScope> =
            self.state.with(|state| state.unread_mark_observation_counts.keys().cloned().collect());
        for scope in &scopes {
            if !self.is_unread_marks_live() || !self.unread_marks.is_assumed_empty(scope).await {
                continue;
            }
            let Ok(Some(stored)) = self.chats.stored_chat_id(&scope.chat_id, false).await else {
                continue;
            };
            let wire = &stored.record.wire;
            if (wire.resolved_unread_mentions_count() > 0
                || wire.resolved_unread_replies_count() > 0
                || wire.resolved_unread_reactions_count() > 0)
                && self.refresh_unread_marks(scope).await.is_some()
            {
                self.consume_unread_marks_on_screen(scope).await;
            }
        }
    }

    async fn refresh_unread_marks(&self, scope: &RiseMessageScope) -> Option<RiseUnreadMarksSnapshot> {
        if !self.is_unread_marks_live() {
            return None;
        }
        let snapshot = self.unread_marks.load(scope).await.ok()??;
        self.publish_unread_marks(vec![snapshot.clone()]).await;
        if scope.topic_id.is_none() {
            self.chats
                .apply_unread_marks(
                    &scope.chat_id,
                    0,
                    0,
                    0,
                    Some((snapshot.mentions_count, snapshot.replies_count, snapshot.reactions_count)),
                )
                .await;
        }
        Some(snapshot)
    }

    pub async fn mark_channel_posts_viewed(&self, scope: &RiseMessageScope, message_ids: Vec<String>) {
        for message_id in &message_ids {
            if !self.is_unread_marks_live() {
                return;
            }
            let _ = self.messages.mark_channel_post_viewed(message_id, scope).await;
        }
    }

    /// rows on screen, ledger is trimmed at once and the server told after
    /// a refused round trip re reads the ledger and doesnt guess a rollback
    pub async fn consume_unread_marks(&self, scope: &RiseMessageScope, candidates: Vec<RiseUnreadMarkCandidate>) {
        if !self.is_unread_marks_live() || candidates.is_empty() {
            return;
        }
        let observed =
            self.state.with(|state| state.unread_mark_observation_counts.get(scope).copied().unwrap_or(0) > 0);
        if !self.unread_marks.is_loaded(scope).await {
            if !observed {
                return;
            }
            self.state.with(|state| {
                let parked = state.unread_mark_candidates_awaiting_ledger.entry(scope.clone()).or_default();
                for candidate in candidates {
                    Self::merge_unread_mark_candidate(candidate, parked);
                }
            });
            if self.unread_marks.is_loading(scope).await || self.refresh_unread_marks(scope).await.is_none() {
                return;
            }
            self.consume_unread_marks_on_screen(scope).await;
            return;
        }
        if observed
            && self.unread_marks.is_assumed_empty(scope).await
            && candidates
                .iter()
                .any(|candidate| candidate.is_mentioned_you || candidate.is_replied_you || candidate.has_unread_reactions)
            && !self.unread_marks.is_loading(scope).await
        {
            self.refresh_unread_marks(scope).await;
        }
        let Some(consumed) = self.unread_marks.consume(scope, candidates).await else {
            return;
        };
        let consumption = consumed.consumption;
        self.publish_unread_marks(consumed.snapshots).await;
        let mentions = -(consumption.mention_ids.len() as i64);
        let replies = -(consumption.reply_ids.len() as i64);
        let reactions = -consumption.reactions_cleared;
        self.chats.apply_unread_marks(&scope.chat_id, mentions, replies, reactions, None).await;
        self.apply_topic_unread_marks(&scope.chat_id, scope.topic_id.as_deref(), mentions, replies, reactions).await;
        self.enqueue_unread_mark_confirmation(consumption);
    }

    pub async fn next_unread_mark(&self, scope: &RiseMessageScope, kind: RiseUnreadMarkKind) -> Option<String> {
        if !self.is_unread_marks_live() {
            return None;
        }
        let next = match kind {
            RiseUnreadMarkKind::Mention => self.unread_marks.next_mention(scope).await,
            RiseUnreadMarkKind::Reply => self.unread_marks.next_reply(scope).await,
            RiseUnreadMarkKind::Reaction => self.unread_marks.next_reaction(scope).await,
        };
        if next.is_some() {
            return next;
        }
        let refreshed = self.refresh_unread_marks(scope).await?;
        match kind {
            RiseUnreadMarkKind::Mention => refreshed.mention_message_ids.first().cloned(),
            RiseUnreadMarkKind::Reply => refreshed.reply_message_ids.first().cloned(),
            RiseUnreadMarkKind::Reaction => refreshed.reaction_marks.first().map(|mark| mark.message_id.clone()),
        }
    }

    pub async fn clear_unread_marks(&self, scope: &RiseMessageScope, kind: RiseUnreadMarkKind) {
        if !self.is_unread_marks_live() {
            return;
        }
        let Some(cleared) = self.unread_marks.clear_all(scope, kind).await else {
            if !self.unread_marks.is_loaded(scope).await {
                let _ = self.unread_marks.confirm_clear_all(scope, kind, Vec::new()).await;
            }
            return;
        };
        let removed = cleared.removed;
        self.publish_unread_marks(cleared.snapshots).await;
        let mentions = if kind == RiseUnreadMarkKind::Mention { -removed } else { 0 };
        let replies = if kind == RiseUnreadMarkKind::Reply { -removed } else { 0 };
        let reactions = if kind == RiseUnreadMarkKind::Reaction { -removed } else { 0 };
        self.chats.apply_unread_marks(&scope.chat_id, mentions, replies, reactions, None).await;
        self.apply_topic_unread_marks(&scope.chat_id, scope.topic_id.as_deref(), mentions, replies, reactions).await;
        if self.unread_marks.confirm_clear_all(scope, kind, cleared.message_ids.clone()).await.is_err() {
            self.unread_marks
                .invalidate_consumption(RiseUnreadMarksConsumption {
                    scope: scope.clone(),
                    mention_ids: if kind == RiseUnreadMarkKind::Mention { cleared.message_ids.clone() } else { Vec::new() },
                    reply_ids: if kind == RiseUnreadMarkKind::Reply { cleared.message_ids.clone() } else { Vec::new() },
                    reaction_ids: if kind == RiseUnreadMarkKind::Reaction { cleared.message_ids } else { Vec::new() },
                    reactions_cleared: 0,
                })
                .await;
            self.refresh_unread_marks(scope).await;
        }
    }

    pub async fn drop_unread_marks(&self, chat_id: &str) {
        let snapshots = self.unread_marks.drop_all(chat_id).await;
        self.publish_unread_marks(snapshots).await;
    }

    async fn reduce_unread_marks_changed(
        &self,
        envelope: RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        if let Some(root) = envelope.data.object_value()
            && let Some(kind_raw) = RiseMessageWireCodec::string(root, &[rise_key::events::KIND])
            && RiseChatUnreadFlagKind::from_raw_value(&kind_raw).is_some()
        {
            if !self.is_unread_marks_live() {
                return Ok(None);
            }
            let chat_id = RiseMessageWireCodec::string(root, &[rise_key::message::CHAT_ID]);
            let Some(chat_id) = chat_id.filter(|chat_id| !chat_id.is_empty()) else {
                return Ok(None);
            };
            self.refresh_chat_row(&chat_id).await;
            return Ok(None);
        }
        if self.state.with(|state| state.is_shutdown) {
            return Ok(None);
        }
        let empty = BTreeMap::new();
        let Some(change) = RiseUnreadMarksRealtimeChange::new(envelope.data.object_value().unwrap_or(&empty)) else {
            tracing::warn!(target: "riseonly::X2232", "unread marks frame not decoded");
            return Ok(None);
        };
        tracing::warn!(
            target: "riseonly::X2232",
            chat = change.chat_id.as_str(),
            kind = ?change.kind,
            change = change.change.as_str(),
            delta = change.delta,
            ids = ?change.message_ids,
            live = self.is_unread_marks_live(),
            "unread marks frame"
        );
        let mut visible_scope: Option<RiseMessageScope> = None;
        let mut visible_candidates: Vec<RiseUnreadMarkCandidate> = Vec::new();
        if (change.kind == RiseUnreadMarkKind::Reaction || change.kind == RiseUnreadMarkKind::Reply)
            && change.change == REACTION_ADDED_CHANGE
        {
            let topic_scope = RiseMessageScope::new(
                change.chat_id.clone(),
                Some(change.topic_id.clone().unwrap_or_default()),
                None,
            );
            let chat_scope = RiseMessageScope::new(change.chat_id.clone(), None, None);
            let changed_ids: HashSet<&String> = change.message_ids.iter().collect();
            for scope in [topic_scope, chat_scope] {
                let Some(viewport) = self.state.with(|state| state.unread_mark_viewports.get(&scope).cloned()) else {
                    continue;
                };
                let candidates = viewport.current_candidates().await;
                if candidates.iter().any(|candidate| changed_ids.contains(&candidate.message_id)) {
                    visible_scope = Some(scope);
                    visible_candidates = candidates;
                    break;
                }
            }
        }
        if !self.is_unread_marks_live() {
            return Ok(None);
        }
        let outcome = self.unread_marks.apply_realtime(&change, visible_scope, visible_candidates).await;
        tracing::warn!(
            target: "riseonly::X2232",
            applies = outcome.applies_to_chat_row,
            snapshots = ?outcome
                .snapshots
                .iter()
                .map(|snapshot| (snapshot.scope.clone(), snapshot.reactions_count, snapshot.reaction_marks.len()))
                .collect::<Vec<_>>(),
            "unread marks applied"
        );
        self.publish_unread_marks(outcome.snapshots).await;
        let consumed_mentions = outcome.consumption.as_ref().map_or(0, |consumption| consumption.mention_ids.len() as i64);
        let consumed_replies = outcome.consumption.as_ref().map_or(0, |consumption| consumption.reply_ids.len() as i64);
        let consumed_reactions = outcome.consumption.as_ref().map_or(0, |consumption| consumption.reactions_cleared);
        if let Some(consumption) = outcome.consumption {
            self.enqueue_unread_mark_confirmation(consumption);
        }
        if !outcome.applies_to_chat_row {
            return Ok(None);
        }
        let mentions_delta =
            if change.kind == RiseUnreadMarkKind::Mention { change.delta } else { 0 } - consumed_mentions;
        let replies_delta = if change.kind == RiseUnreadMarkKind::Reply { change.delta } else { 0 } - consumed_replies;
        let reactions_delta =
            if change.kind == RiseUnreadMarkKind::Reaction { change.delta } else { 0 } - consumed_reactions;
        if mentions_delta == 0 && replies_delta == 0 && reactions_delta == 0 {
            return Ok(None);
        }
        self.apply_topic_unread_marks(
            &change.chat_id,
            Some(change.topic_id.as_deref().unwrap_or_default()),
            mentions_delta,
            replies_delta,
            reactions_delta,
        )
        .await;
        let batch = self
            .chats
            .make_unread_marks_batch(&change.chat_id, mentions_delta, replies_delta, reactions_delta, None, None)
            .await?;
        Ok(if batch.mutations.is_empty() { None } else { Some(batch) })
    }

    fn enqueue_unread_mark_confirmation(&self, consumption: RiseUnreadMarksConsumption) {
        let is_first = self.state.with(|state| {
            let merged = merged_unread_mark_consumption(
                state.pending_unread_mark_consumptions.get(&consumption.scope),
                &consumption,
            );
            state.pending_unread_mark_consumptions.insert(consumption.scope.clone(), merged);
            state.unread_mark_confirmation_task.is_none()
        });
        if !is_first {
            return;
        }
        let owner = self.this.clone();
        let task = RiseAccountRuntime::current().spawn(async move {
            tokio::time::sleep(Duration::from_millis((*UNREAD_MARK_CONFIRMATION_DELAY_MS).max(0) as u64)).await;
            if let Some(owner) = owner.upgrade() {
                owner.flush_unread_mark_confirmations().await;
            }
        });
        self.state.with(|state| state.unread_mark_confirmation_task = Some(task));
    }

    async fn flush_unread_mark_confirmations(&self) {
        loop {
            let pending = self.state.with(|state| {
                if state.pending_unread_mark_consumptions.is_empty() || state.is_shutdown || !state.is_active {
                    return None;
                }
                Some(std::mem::take(&mut state.pending_unread_mark_consumptions))
            });
            let Some(pending) = pending else {
                break;
            };
            let Some(scope) = pending.keys().next().cloned() else {
                break;
            };
            let mention_ids: HashSet<String> =
                pending.values().flat_map(|consumption| consumption.mention_ids.iter().cloned()).collect();
            let reply_ids: HashSet<String> =
                pending.values().flat_map(|consumption| consumption.reply_ids.iter().cloned()).collect();
            let reaction_ids: HashSet<String> =
                pending.values().flat_map(|consumption| consumption.reaction_ids.iter().cloned()).collect();
            let combined = RiseUnreadMarksConsumption {
                scope,
                mention_ids: mention_ids.into_iter().collect(),
                reply_ids: reply_ids.into_iter().collect(),
                reaction_ids: reaction_ids.into_iter().collect(),
                reactions_cleared: pending.values().map(|consumption| consumption.reactions_cleared).sum(),
            };
            self.clear_unread_mark_flags(&combined.mention_ids, &combined.reply_ids, &combined.reaction_ids).await;
            if !self.is_unread_marks_live() {
                break;
            }
            if let Err(error) = self.unread_marks.confirm_consumption(&combined).await {
                if !self.is_unread_marks_live() {
                    break;
                }
                if Self::is_unread_mark_rejection(&error) {
                    self.unread_marks.invalidate_consumption(combined).await;
                    let mut scopes: HashSet<RiseMessageScope> = pending.keys().cloned().collect();
                    for scope in pending.keys() {
                        scopes.insert(RiseMessageScope::new(scope.chat_id.clone(), None, None));
                    }
                    for scope in &scopes {
                        self.refresh_unread_marks(scope).await;
                    }
                } else {
                    self.park_unread_mark_consumption(&combined);
                }
            }
            if self.state.with(|state| !state.pending_unread_mark_consumptions.is_empty()) {
                RiseRuntimeTimer::sleep(Duration::from_millis((*UNREAD_MARK_CONFIRMATION_DELAY_MS).max(0) as u64)).await;
            }
        }
        self.state.with(|state| state.unread_mark_confirmation_task = None);
    }

    fn park_unread_mark_consumption(&self, consumption: &RiseUnreadMarksConsumption) {
        self.state.with(|state| {
            let merged = merged_unread_mark_consumption(
                state.parked_unread_mark_consumptions.get(&consumption.scope),
                consumption,
            );
            state.parked_unread_mark_consumptions.insert(consumption.scope.clone(), merged);
        });
    }

    fn resend_parked_unread_mark_confirmations(&self) {
        let parked = self.state.with(|state| {
            if state.is_shutdown || !state.is_active {
                return HashMap::new();
            }
            std::mem::take(&mut state.parked_unread_mark_consumptions)
        });
        for consumption in parked.into_values() {
            self.enqueue_unread_mark_confirmation(consumption);
        }
    }

    fn is_unread_mark_rejection(error: &anyhow::Error) -> bool {
        error.is::<RiseWireRemoteError>()
            || matches!(error.downcast_ref::<RiseChatRepositoryError>(), Some(RiseChatRepositoryError::Rejected(_)))
    }

    async fn publish_unread_marks(&self, snapshots: Vec<RiseUnreadMarksSnapshot>) {
        if snapshots.is_empty() {
            return;
        }
        let account_id = self.account_id.clone();
        on_main(move |cx| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                for snapshot in snapshots {
                    presentation.publish_unread_marks(snapshot, &account_id, cx);
                }
            });
        })
        .await;
    }

    /// forum topic rows still read the legacy topic cache, the chat row moves through the store batch
    async fn apply_topic_unread_marks(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        mentions: i64,
        replies: i64,
        reactions: i64,
    ) {
        let Some(topic_id) = topic_id else {
            return;
        };
        if mentions == 0 && replies == 0 && reactions == 0 {
            return;
        }
        let chat_id = chat_id.to_string();
        let topic_id = topic_id.to_string();
        on_main(move |cx| {
            if let Some(hooks) = chat_engine_hooks() {
                (hooks.apply_topic_unread_marks_delta)(&chat_id, &topic_id, mentions, replies, reactions, cx);
            }
        })
        .await;
    }

    async fn clear_unread_mark_flags(&self, mention_ids: &[String], reply_ids: &[String], reaction_ids: &[String]) {
        if mention_ids.is_empty() && reply_ids.is_empty() && reaction_ids.is_empty() {
            return;
        }
        self.messages
            .clear_unread_mark_flags(
                mention_ids.iter().cloned().collect(),
                reply_ids.iter().cloned().collect(),
                reaction_ids.iter().cloned().collect(),
            )
            .await;
    }
}

impl<Item> RiseConversationAuxiliaryPageSnapshot<Item> {
    fn retrying_failure(self) -> Self {
        if self.load_state != RiseConversationAuxiliaryLoadState::Failed {
            return self;
        }
        Self { load_state: RiseConversationAuxiliaryLoadState::Loading, error: None, ..self }
    }
}
