use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Task};
use rise_bot_api::reply_markup as reply_markup_key;
use rise_canon::rise_key::bot as bot_key;
use rise_canon::rise_key::chat as chat_key;
use rise_canon::rise_key::comment as comment_key;
use rise_canon::rise_key::events as events_key;
use rise_canon::rise_key::file_storage_admin as file_storage_admin_key;
use rise_canon::rise_key::guide_screen as guide_screen_key;
use rise_canon::rise_key::message as key;
use rise_canon::rise_key::payload as payload_key;
use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_entities::RiseEntityViewLoadState;
use rise_platform::power::{POWER_STATE_DID_CHANGE, system_low_power_mode};
use rise_platform::process_activity::{ProcessActivityLease, begin_process_activity};
use rise_platform::process_info::{THERMAL_STATE_DID_CHANGE, ThermalState, system_uptime, thermal_state};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingPageExt};
use crate::core::lib::sai::core::utilities::get_sai_server_id::{GetSaiServerId, GetSaiServerIdOptions};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiTypedSlice, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWsOnSuccess;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_payload_casing::ws_string;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiWsScreenRefresh, SaiWsScreenRefreshParams};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::{
    ChatScrollPositionStore, ChatScrollPositionStorePosition,
};
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab::{ChatProfileTab, ChatProfileTabExt};
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationPresentation, RiseMessagePresentationContentStamp, RiseMessageScopePresentationSnapshot,
};
use crate::modules::chat::engine::messages::rise_message_read_receipt_presentation::{
    RiseMessageReadReceiptPresentation, RiseMessageReadReceiptPresentationSnapshot,
};
use crate::modules::chat::engine::messages::rise_message_scope::{RiseMessageScope, RiseMessageWindowShift};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::chat::stores::message::message_actions::message_actions::{
    MessageActionCompletion, MessageActionsStore,
};
use crate::modules::chat::stores::message::message_actions::message_types::{
    ChatTypingPeer, GetMessageMessage, MessageBotKeyboardButton, MessageBotReplyMarkup, MessageDeliveryStatus,
    MessageEditMediaSnapshot, MessageHistoryDirection, MessageHistoryLane, MessageHistoryLaneState,
    MessageHistoryPageState, MessageMediaItem, MessageMediaKind, MessageSystemKind, MessageThreadOrder,
    MessageThreadOrderKey, MessageUploadPreviewDebug,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    GetMessagesGatewayDTO, MessagePayloadCodec, MessageWireDTO, RiseStoredOutgoingMedia,
};
use crate::modules::chat::stores::message::message_services::message_calendar_services::MessageChatCalendarPresentation;
use crate::modules::chat::stores::message::message_services::message_pagination_trace::MessagePaginationTrace;
use crate::modules::chat::stores::message::message_services::message_threads_services::MessageThreadTotals;
use crate::modules::chat::stores::message::message_services::message_presentation_window_projector::{
    MessagePresentationDecodedRowCache, MessagePresentationSlice, MessagePresentationSourceOrder,
    MessagePresentationWindowProjector, MessagePresentationWindowShift, MessagePresentationWindowState,
};

const SERVER_ID_KEY: &str = "server_id";
const SAI_SERVER_ID_KEY: &str = "sai_server_id";
const IS_TEMP_KEY: &str = "isTemp";
const HISTORY_PAGINATION_KEY: &str = "history_pagination";
const SHARED_POST_CAMEL_KEY: &str = "sharedPost";
const CURRENT_TAB_KEY: &str = "currentTab";
const CHAT_PROFILE_SCREEN: &str = "ChatProfile";
const GET_MESSAGES_PREFIX: &str = "getMessages";
const GET_MESSAGES_LATEST_PREFIX: &str = "getMessagesLatest";
const GET_PINNED_MESSAGES_PREFIX: &str = "getPinnedMessages";
const MESSAGES_PERSISTENCE_ACTIVITY: &str = "riseonly.messages.persistence";

const CONFIRMED_DELETED_IDS_LIMIT: usize = 2_048;
const WARM_CANDIDATE_REFRESH_SECONDS: f64 = 0.75;
const WARM_EXPIRY_MINIMUM_SECONDS: f64 = 0.05;
const LATEST_DISK_PERSISTENCE_DELAY_NANOSECONDS: u64 = 1_000_000_000;
const FAILED_DISK_PERSISTENCE_DELAY_NANOSECONDS: u64 = 2_000_000_000;
const HISTORY_RETRY_SECONDS: f64 = 1.5;

pub trait MessagesListTransformOutput {
    fn apply_to(self, messages: &mut Vec<GetMessageMessage>);
}

impl MessagesListTransformOutput for () {
    fn apply_to(self, _messages: &mut Vec<GetMessageMessage>) {}
}

impl MessagesListTransformOutput for Vec<GetMessageMessage> {
    fn apply_to(self, messages: &mut Vec<GetMessageMessage>) {
        *messages = self;
    }
}

pub type MessagesDiskLoadCompletion = Box<dyn FnOnce(bool, &mut App)>;
pub type ProfileTabDiskLoadCompletion = Box<dyn FnOnce(bool, bool, &mut App)>;

#[derive(Clone, Debug)]
pub struct MessageSeedRefreshFence {
    pub latest_boundary: Option<MessageThreadOrderKey>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageHistoryWarmCacheLimits {
    pub maximum_entry_count: i64,
    pub maximum_estimated_cost: i64,
    pub maximum_concurrent_loads: i64,
    pub time_to_live: f64,
}

impl MessageHistoryWarmCacheLimits {
    pub fn new(
        maximum_entry_count: i64,
        maximum_estimated_cost: i64,
        maximum_concurrent_loads: i64,
        time_to_live: f64,
    ) -> Self {
        Self {
            maximum_entry_count: maximum_entry_count.max(1),
            maximum_estimated_cost: maximum_estimated_cost.max(1),
            maximum_concurrent_loads: maximum_concurrent_loads.max(1),
            time_to_live: time_to_live.max(1.0),
        }
    }

    pub fn production() -> Self {
        Self::new(
            RiseLimits::client_value("message_history_warm_cache_entries"),
            RiseLimits::client_value("message_history_warm_cache_bytes"),
            RiseLimits::client_value("message_history_warm_cache_loads"),
            RiseLimits::client_value("message_history_warm_cache_ttl_ms") as f64 / 1_000.0,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageHistoryWarmCacheRecord {
    pub cache_key: String,
    pub estimated_cost: i64,
    pub last_accessed_at: f64,
    pub expires_at: f64,
}

pub enum MessageHistoryWarmCachePolicy {}

impl MessageHistoryWarmCachePolicy {
    pub fn maximum_speculative_concurrent_loads(total_limit: i64) -> i64 {
        let normalized_limit = total_limit.max(1);
        (normalized_limit - 1).max(1)
    }

    pub fn permits_speculative_warmup(
        is_low_power_mode_enabled: bool,
        thermal_state: ThermalState,
        is_application_backgrounded: bool,
    ) -> bool {
        if is_low_power_mode_enabled || is_application_backgrounded {
            return false;
        }
        match thermal_state {
            ThermalState::Nominal | ThermalState::Fair => true,
            ThermalState::Serious | ThermalState::Critical => false,
        }
    }

    pub fn prioritized_scopes(visible: &[String], recent: &[String], limit: i64) -> Vec<String> {
        if limit <= 0 {
            return Vec::new();
        }
        let limit = limit as usize;
        let mut seen: HashSet<&str> = HashSet::new();
        let mut result = Vec::with_capacity(limit.min(visible.len() + recent.len()));
        for raw_scope in visible.iter().chain(recent.iter()) {
            let scope = trimming_whitespaces_and_newlines(raw_scope);
            if scope.is_empty() || !seen.insert(scope) {
                continue;
            }
            result.push(scope.to_owned());
            if result.len() == limit {
                break;
            }
        }
        result
    }

    pub fn estimated_resident_cost(message_count: i64) -> i64 {
        let base_cost: i64 = 192 * 1_024;
        let per_message_cost: i64 = 40 * 1_024;
        let count = message_count.max(0);
        let Some(rows_cost) = count.checked_mul(per_message_cost) else {
            return i64::MAX;
        };
        base_cost.checked_add(rows_cost).unwrap_or(i64::MAX)
    }

    pub fn eviction_keys(
        records: &[MessageHistoryWarmCacheRecord],
        now: f64,
        limits: &MessageHistoryWarmCacheLimits,
    ) -> Vec<String> {
        let mut evicted = Vec::new();
        let mut retained: Vec<&MessageHistoryWarmCacheRecord> = Vec::with_capacity(records.len());
        for record in records {
            if record.expires_at <= now {
                evicted.push(record.cache_key.clone());
            } else {
                retained.push(record);
            }
        }

        let mut retained_cost =
            retained.iter().fold(0_i64, |partial, record| partial.saturating_add(record.estimated_cost.max(0)));
        let mut retained_count = retained.len() as i64;
        if retained_count <= limits.maximum_entry_count && retained_cost <= limits.maximum_estimated_cost {
            return evicted;
        }

        retained.sort_by(|lhs, rhs| {
            lhs.last_accessed_at
                .total_cmp(&rhs.last_accessed_at)
                .then_with(|| lhs.cache_key.cmp(&rhs.cache_key))
        });
        for record in retained {
            if retained_count <= limits.maximum_entry_count && retained_cost <= limits.maximum_estimated_cost {
                break;
            }
            evicted.push(record.cache_key.clone());
            retained_count -= 1;
            retained_cost = (retained_cost - record.estimated_cost.max(0)).max(0);
        }
        evicted
    }
}

#[derive(Clone, Debug)]
pub struct MessageScopePresentationSnapshot {
    pub revision: u64,
    pub slice: MessagePresentationSlice<GetMessageMessage>,
    pub latest_message: Option<GetMessageMessage>,
    pub latest_reply_keyboard_message: Option<GetMessageMessage>,
    pub pinned_messages: Vec<GetMessageMessage>,
    pub content_stamps_by_local_id: HashMap<String, RiseMessagePresentationContentStamp>,
    pub has_more_before: bool,
    pub has_more_after: bool,
    pub request_status: SaiWsSliceStatus,
    pub error_code: Option<String>,
}

struct MessagesDiskBackgroundLease {
    #[allow(dead_code)]
    identifier: Option<ProcessActivityLease>,
}

impl MessagesDiskBackgroundLease {
    fn new() -> Self {
        Self { identifier: Some(begin_process_activity(MESSAGES_PERSISTENCE_ACTIVITY)) }
    }

    fn end(&mut self) {
        self.identifier = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MessagesDiskLoadPriority {
    Warm = 0,
    Foreground = 1,
}

#[derive(Clone, Debug)]
struct MessagesDiskLoadRequest {
    scope: String,
    cache_key: String,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    priority: MessagesDiskLoadPriority,
    candidate_rank: i64,
    enqueued_order: u64,
}

#[derive(Clone, Debug)]
struct MessagesDiskLoadInFlight {
    request: MessagesDiskLoadRequest,
    load_token: u64,
    starting_revision: i64,
}

#[derive(Clone, Debug)]
struct MessageHistoryWarmEntry {
    scope: String,
    cache_key: String,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    installed_revision: i64,
    estimated_cost: i64,
    last_accessed_at: f64,
    expires_at: f64,
}

impl MessageHistoryWarmEntry {
    fn policy_record(&self) -> MessageHistoryWarmCacheRecord {
        MessageHistoryWarmCacheRecord {
            cache_key: self.cache_key.clone(),
            estimated_cost: self.estimated_cost,
            last_accessed_at: self.last_accessed_at,
            expires_at: self.expires_at,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum MessagesWarmLifecycleSignal {
    MemoryWarning,
    DidEnterBackground,
    DidBecomeActive,
    ConstraintChange,
}

struct DecodedMessagesMemo {
    revision: i64,
    ordered: Vec<GetMessageMessage>,
}

struct PinnedMessagesMemo {
    revision: i64,
    messages: Vec<GetMessageMessage>,
}

struct LatestReplyKeyboardMemo {
    revision: i64,
    message: Option<GetMessageMessage>,
}

static MESSAGE_HISTORY_WARM_LIMITS: LazyLock<MessageHistoryWarmCacheLimits> =
    LazyLock::new(MessageHistoryWarmCacheLimits::production);

pub struct MessageServicesStore {
    pub(crate) scroll_position_keys_by_scope: HashMap<String, String>,
    pub(crate) create_message_completions: HashMap<String, MessageActionCompletion>,
    pub(crate) edit_media_snapshots: HashMap<String, MessageEditMediaSnapshot>,
    pub(crate) upload_progress_throttle: HashMap<String, (f64, bool, f64)>,
    message_history_lane_states: HashMap<MessageHistoryLane, MessageHistoryLaneState>,
    message_history_request_tokens: HashMap<MessageHistoryLane, u64>,
    next_message_history_request_token: u64,
    message_seed_request_tokens: HashMap<String, u64>,
    next_message_seed_request_token: u64,
    message_seed_refresh_fences: HashMap<String, (u64, MessageSeedRefreshFence)>,
    pub(crate) pending_latest_jump_scopes: HashSet<String>,
    pub(crate) latest_jump_completion_revisions: HashMap<String, u64>,
    pub(crate) confirmed_deleted_message_ids_by_chat: HashMap<String, HashSet<String>>,
    pub(crate) confirmed_deleted_message_revisions_by_chat: HashMap<String, u64>,
    history_eviction_revision: u64,
    pub(crate) comment_bump_target_by_temp_id: HashMap<String, String>,
    pub(crate) typing_peers_by_chat: HashMap<String, Vec<ChatTypingPeer>>,
    pub chat_calendar: MessageChatCalendarPresentation,
    decoded_messages_memo: RefCell<HashMap<String, DecodedMessagesMemo>>,
    message_presentation_slices: HashMap<String, MessagePresentationSlice<GetMessageMessage>>,
    message_presentation_decoded_rows_memo: HashMap<String, MessagePresentationDecodedRowCache<GetMessageMessage>>,
    pinned_messages_memo: HashMap<String, PinnedMessagesMemo>,
    latest_reply_keyboard_memo: HashMap<String, LatestReplyKeyboardMemo>,
    known_absent_messages_cache_keys: RefCell<HashSet<String>>,
    profile_tab_disk_load_completions: HashMap<String, Vec<ProfileTabDiskLoadCompletion>>,
    profile_tab_disk_load_generations: HashMap<String, u64>,
    pending_messages_disk_cache_keys: HashSet<String>,
    scrolling_messages_cache_keys: HashSet<String>,
    messages_disk_persistence_tasks: HashMap<String, Task<()>>,
    messages_disk_persistence_generation: HashMap<String, u64>,
    messages_disk_persistence_in_flight: HashMap<String, u64>,
    forced_messages_disk_persistence_keys: HashSet<String>,
    messages_disk_load_completions: HashMap<String, Vec<MessagesDiskLoadCompletion>>,
    messages_disk_load_tokens: HashMap<String, u64>,
    next_messages_disk_load_token: u64,
    messages_disk_load_queue: HashMap<String, MessagesDiskLoadRequest>,
    messages_disk_loads_in_flight: HashMap<String, MessagesDiskLoadInFlight>,
    next_messages_disk_load_order: u64,
    message_history_warm_entries: HashMap<String, MessageHistoryWarmEntry>,
    message_history_warm_candidate_ranks: HashMap<String, i64>,
    message_history_warm_last_candidate_refresh_at: f64,
    message_history_warm_requested_scopes: Vec<String>,
    message_history_warm_is_application_backgrounded: bool,
    messages_warm_expiry_task: Option<Task<()>>,
    #[allow(dead_code)]
    messages_warm_lifecycle_observers: Vec<RiseSubscription>,
    #[allow(dead_code)]
    messages_warm_lifecycle_delivery: Option<Task<()>>,
    initial_messages_status_by_scope: HashMap<String, SaiWsSliceStatus>,
    initial_messages_error_by_scope: HashMap<String, String>,
    initial_messages_requests_in_flight: HashSet<String>,
    message_presentation_local_revisions: HashMap<String, u64>,
}

impl MessageServicesStore {
    pub const TYPING_TTL_SECONDS: f64 = 6.0;
    pub const MESSAGES_LIMIT: i64 = 100;
    pub const PINNED_MESSAGES_LIMIT: i64 = 50;
    pub const GLOBAL_MESSAGE_SEARCH_LIMIT: i64 = 30;
    pub const TOPIC_SCOPE_MARKER: &'static str = "::t::";
    pub const THREAD_SCOPE_MARKER: &'static str = "::th::";
    pub const CHAT_PROFILE_TAB_CACHE_KEY_PREFIXES: [&'static str; 7] = [
        "getMediaMessages",
        "get_file_messages",
        "get_music_messages",
        "get_audio_messages",
        "get_gif_messages",
        "get_fav_messages",
        "getLinkMessages",
    ];
    const DECODED_MESSAGES_MEMO_MAX_KEYS: usize = 32;
    const MESSAGE_PRESENTATION_DECODED_ROWS_MEMO_MAX_KEYS: usize = 8;
    const MESSAGES_DISK_SAFETY_CHECKPOINT_DELAY_NANOSECONDS: u64 = 25_000_000_000;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut receiver) = mpsc::unbounded::<MessagesWarmLifecycleSignal>();
        let center = RiseAppEvents::default_center();
        let observers = [
            (UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION, MessagesWarmLifecycleSignal::MemoryWarning),
            (UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, MessagesWarmLifecycleSignal::DidEnterBackground),
            (UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, MessagesWarmLifecycleSignal::DidBecomeActive),
            (RiseNotificationName::new(POWER_STATE_DID_CHANGE), MessagesWarmLifecycleSignal::ConstraintChange),
            (RiseNotificationName::new(THERMAL_STATE_DID_CHANGE), MessagesWarmLifecycleSignal::ConstraintChange),
        ]
        .into_iter()
        .map(|(name, signal)| {
            let sender = sender.clone();
            center.observe(name, move |_| {
                let _ = sender.unbounded_send(signal);
            })
        })
        .collect();
        let delivery = cx.spawn(async move |this, cx| {
            while let Some(signal) = receiver.next().await {
                if this.update(cx, |store, cx| store.handle_messages_warm_lifecycle_signal(signal, cx)).is_err() {
                    break;
                }
            }
        });
        Self {
            scroll_position_keys_by_scope: HashMap::new(),
            create_message_completions: HashMap::new(),
            edit_media_snapshots: HashMap::new(),
            upload_progress_throttle: HashMap::new(),
            message_history_lane_states: HashMap::new(),
            message_history_request_tokens: HashMap::new(),
            next_message_history_request_token: 0,
            message_seed_request_tokens: HashMap::new(),
            next_message_seed_request_token: 0,
            message_seed_refresh_fences: HashMap::new(),
            pending_latest_jump_scopes: HashSet::new(),
            latest_jump_completion_revisions: HashMap::new(),
            confirmed_deleted_message_ids_by_chat: HashMap::new(),
            confirmed_deleted_message_revisions_by_chat: HashMap::new(),
            history_eviction_revision: 0,
            comment_bump_target_by_temp_id: HashMap::new(),
            typing_peers_by_chat: HashMap::new(),
            chat_calendar: MessageChatCalendarPresentation::default(),
            decoded_messages_memo: RefCell::new(HashMap::new()),
            message_presentation_slices: HashMap::new(),
            message_presentation_decoded_rows_memo: HashMap::new(),
            pinned_messages_memo: HashMap::new(),
            latest_reply_keyboard_memo: HashMap::new(),
            known_absent_messages_cache_keys: RefCell::new(HashSet::new()),
            profile_tab_disk_load_completions: HashMap::new(),
            profile_tab_disk_load_generations: HashMap::new(),
            pending_messages_disk_cache_keys: HashSet::new(),
            scrolling_messages_cache_keys: HashSet::new(),
            messages_disk_persistence_tasks: HashMap::new(),
            messages_disk_persistence_generation: HashMap::new(),
            messages_disk_persistence_in_flight: HashMap::new(),
            forced_messages_disk_persistence_keys: HashSet::new(),
            messages_disk_load_completions: HashMap::new(),
            messages_disk_load_tokens: HashMap::new(),
            next_messages_disk_load_token: 0,
            messages_disk_load_queue: HashMap::new(),
            messages_disk_loads_in_flight: HashMap::new(),
            next_messages_disk_load_order: 0,
            message_history_warm_entries: HashMap::new(),
            message_history_warm_candidate_ranks: HashMap::new(),
            message_history_warm_last_candidate_refresh_at: 0.0,
            message_history_warm_requested_scopes: Vec::new(),
            message_history_warm_is_application_backgrounded: RiseAppLifecycle::scene_phase(cx)
                == ScenePhase::Background,
            messages_warm_expiry_task: None,
            messages_warm_lifecycle_observers: observers,
            messages_warm_lifecycle_delivery: Some(delivery),
            initial_messages_status_by_scope: HashMap::new(),
            initial_messages_error_by_scope: HashMap::new(),
            initial_messages_requests_in_flight: HashSet::new(),
            message_presentation_local_revisions: HashMap::new(),
        }
    }

    fn handle_messages_warm_lifecycle_signal(&mut self, signal: MessagesWarmLifecycleSignal, cx: &mut Context<Self>) {
        match signal {
            MessagesWarmLifecycleSignal::MemoryWarning => self.purge_message_history_warm_cache(cx),
            MessagesWarmLifecycleSignal::DidEnterBackground => {
                self.message_history_warm_is_application_backgrounded = true;
                self.purge_message_history_warm_cache_preserving(true, cx);
            }
            MessagesWarmLifecycleSignal::DidBecomeActive => {
                self.message_history_warm_is_application_backgrounded = false;
                self.resume_requested_message_history_warmup_if_allowed(cx);
            }
            MessagesWarmLifecycleSignal::ConstraintChange => self.handle_message_history_warm_constraint_change(cx),
        }
    }

    pub fn tabs_limit() -> i64 {
        RiseTimingCanon::page("message_tab_links").page_size_items()
    }

    pub fn set_scroll_position_key(&mut self, key: &str, scope: &str) {
        if scope.is_empty() {
            return;
        }
        if key.is_empty() || key == scope {
            self.scroll_position_keys_by_scope.remove(scope);
        } else {
            self.scroll_position_keys_by_scope.insert(scope.to_owned(), key.to_owned());
        }
    }

    pub fn scroll_position_key(&self, scope: &str) -> String {
        self.scroll_position_keys_by_scope.get(scope).cloned().unwrap_or_else(|| scope.to_owned())
    }

    pub fn message_scope(chat_id: &str, topic_id: Option<&str>, thread_root_id: Option<&str>) -> String {
        if let Some(thread_root_id) = thread_root_id
            && !thread_root_id.is_empty()
        {
            return format!("{chat_id}{}{thread_root_id}", Self::THREAD_SCOPE_MARKER);
        }
        let Some(topic_id) = topic_id else {
            return chat_id.to_owned();
        };
        format!("{chat_id}{}{topic_id}", Self::TOPIC_SCOPE_MARKER)
    }

    pub fn message_read_receipts(
        &self,
        chat_id: &str,
        message_id: &str,
        cx: &mut App,
    ) -> RiseMessageReadReceiptPresentationSnapshot {
        RiseMessageReadReceiptPresentation::shared(cx).read(cx).snapshot(chat_id, message_id)
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        let disk_load_completions: Vec<MessagesDiskLoadCompletion> =
            self.messages_disk_load_completions.drain().flat_map(|(_, list)| list).collect();
        self.messages_disk_load_tokens.clear();
        self.messages_disk_load_queue.clear();
        self.messages_disk_loads_in_flight.clear();
        self.next_messages_disk_load_order = 0;
        fire_messages_disk_load_completions(disk_load_completions, false, cx);
        self.messages_warm_expiry_task = None;
        self.message_history_warm_entries.clear();
        self.message_history_warm_candidate_ranks.clear();
        self.message_history_warm_last_candidate_refresh_at = 0.0;
        self.message_history_warm_requested_scopes.clear();
        self.message_history_warm_is_application_backgrounded =
            RiseAppLifecycle::scene_phase(cx) == ScenePhase::Background;
        self.persist_pending_messages_before_account_reset(cx);
        self.create_message_completions.clear();
        self.edit_media_snapshots.clear();
        self.upload_progress_throttle.clear();
        self.message_history_lane_states.clear();
        self.message_history_request_tokens.clear();
        self.next_message_history_request_token = 0;
        self.message_seed_request_tokens.clear();
        self.next_message_seed_request_token = 0;
        self.message_seed_refresh_fences.clear();
        self.pending_latest_jump_scopes.clear();
        self.latest_jump_completion_revisions.clear();
        self.confirmed_deleted_message_ids_by_chat.clear();
        self.confirmed_deleted_message_revisions_by_chat.clear();
        self.comment_bump_target_by_temp_id.clear();
        self.typing_peers_by_chat.clear();
        self.decoded_messages_memo.borrow_mut().clear();
        self.message_presentation_slices.clear();
        self.message_presentation_decoded_rows_memo.clear();
        self.pinned_messages_memo.clear();
        self.latest_reply_keyboard_memo.clear();
        self.message_presentation_local_revisions.clear();
        self.known_absent_messages_cache_keys.borrow_mut().clear();
        let profile_tab_completions: Vec<ProfileTabDiskLoadCompletion> =
            self.profile_tab_disk_load_completions.drain().flat_map(|(_, list)| list).collect();
        self.profile_tab_disk_load_generations.clear();
        fire_profile_tab_disk_load_completions(profile_tab_completions, false, false, cx);
        self.messages_disk_persistence_tasks.clear();
        self.pending_messages_disk_cache_keys.clear();
        self.scrolling_messages_cache_keys.clear();
        self.messages_disk_persistence_generation.clear();
        self.messages_disk_persistence_in_flight.clear();
        self.forced_messages_disk_persistence_keys.clear();
        self.initial_messages_status_by_scope.clear();
        self.initial_messages_error_by_scope.clear();
        self.initial_messages_requests_in_flight.clear();
        self.clear_outgoing_wire_temps_for_account_transition();
        MessageThreadTotals::shared(cx).update(cx, |totals, cx| totals.reset(cx));
        cx.notify();
    }

    pub fn clear_local_history(&mut self, chat_id: &str, scopes: HashSet<RiseMessageScope>, cx: &mut Context<Self>) {
        let normalized_chat_id = trimming_whitespaces_and_newlines(chat_id);
        if normalized_chat_id.is_empty() {
            return;
        }

        let mut storage_scopes: HashSet<String> = scopes
            .iter()
            .filter(|scope| scope.chat_id == normalized_chat_id)
            .map(RiseMessageScope::storage_key)
            .collect();
        storage_scopes.insert(normalized_chat_id.to_owned());
        let mut cache_keys: HashSet<String> = HashSet::new();
        for scope in &storage_scopes {
            cache_keys.insert(Self::messages_cache_key(scope));
            cache_keys.insert(Self::latest_messages_cache_key(scope));
            for prefix in Self::CHAT_PROFILE_TAB_CACHE_KEY_PREFIXES {
                cache_keys.insert(self.tab_cache_key(prefix, scope));
            }
        }
        cache_keys.insert(self.tab_cache_key(GET_PINNED_MESSAGES_PREFIX, normalized_chat_id));

        let history_lanes: HashSet<MessageHistoryLane> = self
            .message_history_lane_states
            .keys()
            .chain(self.message_history_request_tokens.keys())
            .filter(|lane| {
                RiseMessageScope::parse(&lane.chat_id).is_some_and(|scope| scope.chat_id == normalized_chat_id)
            })
            .cloned()
            .collect();
        for lane in &history_lanes {
            self.message_history_lane_states.remove(lane);
            self.message_history_request_tokens.remove(lane);
        }
        for scope in &storage_scopes {
            self.message_seed_request_tokens.remove(scope);
            self.message_seed_refresh_fences.remove(scope);
            self.pending_latest_jump_scopes.remove(scope);
            self.latest_jump_completion_revisions.remove(scope);
            self.initial_messages_status_by_scope.remove(scope);
            self.initial_messages_error_by_scope.remove(scope);
            self.initial_messages_requests_in_flight.remove(scope);
            self.message_presentation_slices.remove(scope);
            self.bump_message_presentation_local_revision(scope);
            self.typing_peers_by_chat.remove(scope);
        }
        self.confirmed_deleted_message_ids_by_chat.remove(normalized_chat_id);
        self.confirmed_deleted_message_revisions_by_chat.remove(normalized_chat_id);

        let disk_load_keys: HashSet<String> = self
            .messages_disk_load_queue
            .iter()
            .filter(|(_, request)| storage_scopes.contains(&request.scope))
            .map(|(key, _)| key.clone())
            .chain(
                self.messages_disk_loads_in_flight
                    .iter()
                    .filter(|(_, active)| storage_scopes.contains(&active.request.scope))
                    .map(|(key, _)| key.clone()),
            )
            .collect();
        for cache_key in cache_keys.union(&disk_load_keys) {
            self.messages_disk_load_queue.remove(cache_key);
            self.messages_disk_loads_in_flight.remove(cache_key);
            self.messages_disk_load_tokens.remove(cache_key);
            if let Some(completions) = self.messages_disk_load_completions.remove(cache_key) {
                fire_messages_disk_load_completions(completions, false, cx);
            }
            self.messages_disk_persistence_tasks.remove(cache_key);
            let generation = self.messages_disk_persistence_generation.entry(cache_key.clone()).or_insert(0);
            *generation = generation.wrapping_add(1);
            self.messages_disk_persistence_in_flight.remove(cache_key);
            self.pending_messages_disk_cache_keys.remove(cache_key);
            self.scrolling_messages_cache_keys.remove(cache_key);
            self.forced_messages_disk_persistence_keys.remove(cache_key);
            self.message_history_warm_entries.remove(cache_key);
            self.message_history_warm_candidate_ranks.remove(cache_key);
            self.known_absent_messages_cache_keys.borrow_mut().remove(cache_key);
            self.decoded_messages_memo.borrow_mut().remove(cache_key);
            self.message_presentation_decoded_rows_memo.remove(cache_key);
            self.pinned_messages_memo.remove(cache_key);
            self.latest_reply_keyboard_memo.remove(cache_key);
            let profile_generation = self.profile_tab_disk_load_generations.entry(cache_key.clone()).or_insert(0);
            *profile_generation = profile_generation.wrapping_add(1);
            if let Some(completions) = self.profile_tab_disk_load_completions.remove(cache_key) {
                fire_profile_tab_disk_load_completions(completions, false, false, cx);
            }
            SaiWsResponseCache::remove(cache_key, SaiCacheCleanLayer::Everywhere, cx);
        }
        self.message_history_warm_requested_scopes.retain(|scope| !storage_scopes.contains(scope));
        RiseMessageReadReceiptPresentation::shared(cx)
            .update(cx, |presentation, cx| presentation.remove(normalized_chat_id, cx));
        cx.notify();
    }

    pub fn notify_messages_cache_did_change(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        let logical_key = SaiAccountCacheScope::logical_key(cache_key);
        let Some(chat_id) = logical_key.strip_prefix("getMessages-") else {
            return;
        };
        if chat_id.is_empty() {
            return;
        }
        self.known_absent_messages_cache_keys.borrow_mut().remove(cache_key);
        self.reconcile_outgoing_messages_cache(chat_id, cx);
    }

    pub fn record_confirmed_message_deletions(
        &mut self,
        chat_id: &str,
        message_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let normalized: HashSet<String> = message_ids
            .iter()
            .map(|raw| trimming_whitespaces_and_newlines(raw))
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect();
        if chat_id.is_empty() || normalized.is_empty() {
            return;
        }
        let voice_player = ChatVoiceMessagePlayer::shared(cx);
        let stops_voice = {
            let player = voice_player.read(cx);
            player.active_chat_id() == chat_id
                && player.playing_message_id().is_some_and(|message_id| normalized.contains(message_id))
        };
        if stops_voice {
            voice_player.update(cx, |player, cx| player.stop(cx));
        }
        let video_player = VideoNotePlayerStore::shared(cx);
        let stops_video = {
            let player = video_player.read(cx);
            player.active_chat_id() == chat_id && normalized.contains(player.message_id())
        };
        if stops_video {
            video_player.update(cx, |player, cx| player.stop(cx));
        }
        let ids = self.confirmed_deleted_message_ids_by_chat.entry(chat_id.to_owned()).or_default();
        if ids.len() + normalized.len() > CONFIRMED_DELETED_IDS_LIMIT {
            ids.clear();
        }
        ids.extend(normalized.iter().cloned());
        let revision = self.confirmed_deleted_message_revisions_by_chat.entry(chat_id.to_owned()).or_insert(0);
        *revision = revision.wrapping_add(1);
        if let Some(window) = self.media_gallery_window(cx)
            && window.chat_id == chat_id
        {
            let dropped: Vec<String> = normalized.into_iter().collect();
            let chat_id = chat_id.to_owned();
            let session_id = window.session_id;
            cx.defer(move |cx| {
                MessageActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.drop_media_gallery_messages_action(&dropped, &chat_id, session_id, cx);
                });
            });
        }
        cx.notify();
    }

    pub fn is_confirmed_deleted(&self, message_id: &str, chat_id: &str) -> bool {
        self.confirmed_deleted_message_ids_by_chat.get(chat_id).is_some_and(|ids| ids.contains(message_id))
    }

    pub fn confirmed_deletion_revision(&self, chat_id: &str) -> u64 {
        self.confirmed_deleted_message_revisions_by_chat.get(chat_id).copied().unwrap_or(0)
    }

    /// bumped after the storage purge evicted cached message history
    /// a chat still on screen in the desktop split reloads on it
    pub fn history_eviction_revision(&self) -> u64 {
        self.history_eviction_revision
    }

    pub fn note_history_evicted(&mut self, cx: &mut Context<Self>) {
        self.history_eviction_revision = self.history_eviction_revision.wrapping_add(1);
        cx.notify();
    }

    pub fn paginated_on_success(&self, is_first_page: bool, handler: SaiWsOnSuccess) -> Option<SaiWsOnSuccess> {
        if is_first_page {
            return None;
        }
        Some(handler)
    }

    pub fn profile_tab_screen_refresh(&self, chat_id: &str, route_tab: ChatProfileTab) -> SaiWsScreenRefresh {
        let params = HashMap::from([
            (guide_screen_key::camel::CHAT_ID.to_owned(), chat_id.to_owned()),
            (CURRENT_TAB_KEY.to_owned(), route_tab.tab_key().to_owned()),
        ]);
        SaiWsScreenRefresh::new(
            true,
            vec![CHAT_PROFILE_SCREEN.to_owned()],
            Some(SaiWsScreenRefreshParams::PerScreen(HashMap::from([(CHAT_PROFILE_SCREEN.to_owned(), params)]))),
        )
    }

    pub fn messages_cache_key(chat_id: &str) -> String {
        SaiCache::new([GET_MESSAGES_PREFIX, chat_id]).key
    }

    pub fn latest_messages_cache_key(chat_id: &str) -> String {
        SaiCache::new([GET_MESSAGES_LATEST_PREFIX, chat_id]).key
    }

    pub fn chat_disk_cache_keys(chat_id: &str) -> Vec<String> {
        let normalized = trimming_whitespaces_and_newlines(chat_id);
        if normalized.is_empty() {
            return Vec::new();
        }
        let mut keys = vec![
            Self::messages_cache_key(normalized),
            Self::latest_messages_cache_key(normalized),
            SaiCache::new([GET_PINNED_MESSAGES_PREFIX, normalized]).key,
        ];
        keys.extend(Self::CHAT_PROFILE_TAB_CACHE_KEY_PREFIXES.iter().map(|prefix| SaiCache::new([*prefix, normalized]).key));
        keys
    }

    pub fn preload_messages_slice_from_disk_if_needed(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }
        let _ = self.cached_messages_payload(&Self::messages_cache_key(chat_id), false, cx);
    }

    pub fn preload_messages_cache_from_disk(
        &mut self,
        chat_id: &str,
        completion: MessagesDiskLoadCompletion,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() {
            fire_messages_disk_load_completions(vec![completion], false, cx);
            return;
        }

        let key = Self::messages_cache_key(chat_id);
        self.consume_message_history_warm_entry(&key, cx);
        if SaiWsResponseCache::memory_only(&key, cx).is_some() {
            fire_messages_disk_load_completions(vec![completion], true, cx);
            return;
        }
        if self.known_absent_messages_cache_keys.borrow().contains(&key) {
            fire_messages_disk_load_completions(vec![completion], true, cx);
            return;
        }
        self.enqueue_messages_disk_load(chat_id, MessagesDiskLoadPriority::Foreground, 0, Some(completion), cx);
    }

    pub fn update_message_history_warm_candidates(&mut self, ordered_scopes: Vec<String>, cx: &mut Context<Self>) {
        let scopes = MessageHistoryWarmCachePolicy::prioritized_scopes(
            &ordered_scopes,
            &[],
            MESSAGE_HISTORY_WARM_LIMITS.maximum_entry_count,
        );
        self.message_history_warm_requested_scopes = scopes.clone();
        if !self.permits_speculative_message_history_warmup() {
            self.purge_message_history_warm_cache_preserving(true, cx);
            return;
        }
        self.apply_message_history_warm_candidates(scopes, cx);
    }

    fn apply_message_history_warm_candidates(&mut self, scopes: Vec<String>, cx: &mut Context<Self>) {
        let mut ranks: HashMap<String, i64> = HashMap::with_capacity(scopes.len());
        for (rank, scope) in scopes.iter().enumerate() {
            ranks.insert(Self::messages_cache_key(scope), rank as i64);
        }
        let now = system_uptime_seconds();
        if ranks == self.message_history_warm_candidate_ranks
            && now - self.message_history_warm_last_candidate_refresh_at < WARM_CANDIDATE_REFRESH_SECONDS
        {
            return;
        }
        self.message_history_warm_candidate_ranks = ranks.clone();
        self.message_history_warm_last_candidate_refresh_at = now;

        self.enforce_message_history_warm_limits(now, false, cx);
        self.cancel_stale_queued_message_history_warm_loads(cx);

        for scope in &scopes {
            let key = Self::messages_cache_key(scope);
            if let Some(entry) = self.message_history_warm_entries.get_mut(&key) {
                entry.last_accessed_at = now;
                entry.expires_at = now + MESSAGE_HISTORY_WARM_LIMITS.time_to_live;
                continue;
            }
            if SaiWsResponseCache::memory_only(&key, cx).is_some()
                || self.known_absent_messages_cache_keys.borrow().contains(&key)
            {
                continue;
            }
            self.enqueue_messages_disk_load(
                scope,
                MessagesDiskLoadPriority::Warm,
                ranks.get(&key).copied().unwrap_or(i64::MAX),
                None,
                cx,
            );
        }
        self.enforce_message_history_warm_limits(now, true, cx);
        self.pump_messages_disk_load_queue(cx);
    }

    pub fn purge_message_history_warm_cache(&mut self, cx: &mut Context<Self>) {
        self.purge_message_history_warm_cache_preserving(false, cx);
    }

    fn purge_message_history_warm_cache_preserving(
        &mut self,
        preserving_requested_candidates: bool,
        cx: &mut Context<Self>,
    ) {
        if !preserving_requested_candidates {
            self.message_history_warm_requested_scopes.clear();
        }
        self.message_history_warm_candidate_ranks.clear();
        self.message_history_warm_last_candidate_refresh_at = 0.0;
        self.messages_warm_expiry_task = None;
        self.cancel_stale_queued_message_history_warm_loads(cx);
        let keys: Vec<String> = self.message_history_warm_entries.keys().cloned().collect();
        for key in keys {
            self.evict_message_history_warm_entry(&key, cx);
        }
    }

    fn permits_speculative_message_history_warmup(&self) -> bool {
        MessageHistoryWarmCachePolicy::permits_speculative_warmup(
            system_low_power_mode(),
            thermal_state(),
            self.message_history_warm_is_application_backgrounded,
        )
    }

    fn handle_message_history_warm_constraint_change(&mut self, cx: &mut Context<Self>) {
        if self.permits_speculative_message_history_warmup() {
            self.resume_requested_message_history_warmup_if_allowed(cx);
        } else {
            self.purge_message_history_warm_cache_preserving(true, cx);
        }
    }

    fn resume_requested_message_history_warmup_if_allowed(&mut self, cx: &mut Context<Self>) {
        if !self.permits_speculative_message_history_warmup() || self.message_history_warm_requested_scopes.is_empty() {
            return;
        }
        self.apply_message_history_warm_candidates(self.message_history_warm_requested_scopes.clone(), cx);
    }

    fn enqueue_messages_disk_load(
        &mut self,
        scope: &str,
        priority: MessagesDiskLoadPriority,
        candidate_rank: i64,
        completion: Option<MessagesDiskLoadCompletion>,
        cx: &mut Context<Self>,
    ) {
        let key = Self::messages_cache_key(scope);
        if let Some(completion) = completion {
            self.messages_disk_load_completions.entry(key.clone()).or_default().push(completion);
        }

        if let Some(request) = self.messages_disk_load_queue.get_mut(&key) {
            promote_messages_disk_load(request, priority, candidate_rank);
            self.pump_messages_disk_load_queue(cx);
            return;
        }
        if let Some(active) = self.messages_disk_loads_in_flight.get_mut(&key) {
            promote_messages_disk_load(&mut active.request, priority, candidate_rank);
            return;
        }

        self.next_messages_disk_load_order = self.next_messages_disk_load_order.wrapping_add(1);
        self.messages_disk_load_queue.insert(
            key.clone(),
            MessagesDiskLoadRequest {
                scope: scope.to_owned(),
                cache_key: key,
                account_user_id: SaiAccountCacheScope::active_user_id(),
                account_epoch: SaiAccountCacheScope::active_epoch(),
                priority,
                candidate_rank,
                enqueued_order: self.next_messages_disk_load_order,
            },
        );
        self.pump_messages_disk_load_queue(cx);
    }

    fn pump_messages_disk_load_queue(&mut self, cx: &mut Context<Self>) {
        let concurrency = MESSAGE_HISTORY_WARM_LIMITS.maximum_concurrent_loads;
        while (self.messages_disk_loads_in_flight.len() as i64) < concurrency {
            let Some(request) = self.next_messages_disk_load_request() else {
                break;
            };
            if request.priority == MessagesDiskLoadPriority::Warm {
                let active_warm_loads = self
                    .messages_disk_loads_in_flight
                    .values()
                    .filter(|active| active.request.priority == MessagesDiskLoadPriority::Warm)
                    .count() as i64;
                let speculative_limit = MessageHistoryWarmCachePolicy::maximum_speculative_concurrent_loads(concurrency);
                if active_warm_loads >= speculative_limit {
                    break;
                }
            }
            self.messages_disk_load_queue.remove(&request.cache_key);
            let is_foreground = request.priority == MessagesDiskLoadPriority::Foreground;
            let is_warm_candidate = self.message_history_warm_candidate_ranks.contains_key(&request.cache_key);
            if !is_foreground && !is_warm_candidate {
                self.finish_messages_disk_load_completions(&request.cache_key, false, cx);
                continue;
            }
            if !SaiAccountCacheScope::is_current_account_epoch(
                request.account_user_id.as_deref(),
                request.account_epoch.as_deref(),
            ) || Self::messages_cache_key(&request.scope) != request.cache_key
            {
                self.finish_messages_disk_load_completions(&request.cache_key, false, cx);
                continue;
            }
            if SaiWsResponseCache::memory_only(&request.cache_key, cx).is_some() {
                self.known_absent_messages_cache_keys.borrow_mut().remove(&request.cache_key);
                self.finish_messages_disk_load_completions(&request.cache_key, true, cx);
                continue;
            }
            if self.known_absent_messages_cache_keys.borrow().contains(&request.cache_key) {
                self.finish_messages_disk_load_completions(&request.cache_key, true, cx);
                continue;
            }

            self.next_messages_disk_load_token = self.next_messages_disk_load_token.wrapping_add(1);
            let token = self.next_messages_disk_load_token;
            let cache_key = request.cache_key.clone();
            self.messages_disk_load_tokens.insert(cache_key.clone(), token);
            let starting_revision = cache_revision(&cache_key, cx);
            self.messages_disk_loads_in_flight.insert(
                cache_key.clone(),
                MessagesDiskLoadInFlight { request, load_token: token, starting_revision },
            );
            let this = cx.weak_entity();
            let finished_key = cache_key.clone();
            SaiWsDiskCache::load_asynchronously(
                cache_key,
                move |payload, cx| {
                    let _ = this.update(cx, |store, cx| {
                        store.finish_messages_disk_load(&finished_key, token, payload, cx);
                    });
                },
                cx,
            );
        }
    }

    fn finish_messages_disk_load_completions(&mut self, cache_key: &str, value: bool, cx: &mut Context<Self>) {
        if let Some(completions) = self.messages_disk_load_completions.remove(cache_key) {
            fire_messages_disk_load_completions(completions, value, cx);
        }
    }

    fn next_messages_disk_load_request(&self) -> Option<MessagesDiskLoadRequest> {
        self.messages_disk_load_queue
            .values()
            .min_by(|lhs, rhs| {
                rhs.priority
                    .cmp(&lhs.priority)
                    .then(lhs.candidate_rank.cmp(&rhs.candidate_rank))
                    .then(lhs.enqueued_order.cmp(&rhs.enqueued_order))
            })
            .cloned()
    }

    fn finish_messages_disk_load(
        &mut self,
        key: &str,
        load_token: u64,
        payload: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        if self.messages_disk_load_tokens.get(key) != Some(&load_token) {
            return;
        }
        let Some(active) = self.messages_disk_loads_in_flight.get(key).cloned() else {
            return;
        };
        if active.load_token != load_token {
            return;
        }
        self.messages_disk_load_tokens.remove(key);
        self.messages_disk_loads_in_flight.remove(key);
        let completions = self.messages_disk_load_completions.remove(key).unwrap_or_default();
        let request = active.request;
        let is_foreground = request.priority == MessagesDiskLoadPriority::Foreground || !completions.is_empty();
        let is_warm_candidate = self.message_history_warm_candidate_ranks.contains_key(key);
        let is_current_account = SaiAccountCacheScope::is_current_account_epoch(
            request.account_user_id.as_deref(),
            request.account_epoch.as_deref(),
        ) && Self::messages_cache_key(&request.scope) == key;
        let memory_is_empty = SaiWsResponseCache::memory_only(key, cx).is_none();
        let revision_is_current = cache_revision(key, cx) == active.starting_revision;
        let message_count = payload.as_ref().and_then(raw_message_rows).map_or(0, <[Value]>::len) as i64;
        let estimated_cost = MessageHistoryWarmCachePolicy::estimated_resident_cost(message_count);
        let can_retain_warm_payload = estimated_cost <= MESSAGE_HISTORY_WARM_LIMITS.maximum_estimated_cost;
        let request_is_relevant = is_foreground || is_warm_candidate;
        let can_hydrate_disk = is_current_account
            && request_is_relevant
            && memory_is_empty
            && revision_is_current
            && (is_foreground || can_retain_warm_payload);

        match payload {
            Some(payload) if can_hydrate_disk => {
                let _ = SaiWsResponseCache::apply_sai_updater_payload(
                    payload,
                    key,
                    SaiUpdaterWriteScope::Cache,
                    true,
                    cx,
                );
                self.known_absent_messages_cache_keys.borrow_mut().remove(key);
                if is_warm_candidate && !is_foreground {
                    let now = system_uptime_seconds();
                    let installed_revision = cache_revision(key, cx);
                    self.message_history_warm_entries.insert(
                        key.to_owned(),
                        MessageHistoryWarmEntry {
                            scope: request.scope.clone(),
                            cache_key: key.to_owned(),
                            account_user_id: request.account_user_id.clone(),
                            account_epoch: request.account_epoch.clone(),
                            installed_revision,
                            estimated_cost,
                            last_accessed_at: now,
                            expires_at: now + MESSAGE_HISTORY_WARM_LIMITS.time_to_live,
                        },
                    );
                    self.enforce_message_history_warm_limits(now, true, cx);
                }
            }
            None if is_current_account && request_is_relevant && memory_is_empty && revision_is_current => {
                self.known_absent_messages_cache_keys.borrow_mut().insert(key.to_owned());
            }
            _ => {
                if SaiWsResponseCache::memory_only(key, cx).is_some() {
                    self.known_absent_messages_cache_keys.borrow_mut().remove(key);
                }
            }
        }

        if is_foreground {
            self.consume_message_history_warm_entry(key, cx);
        }
        fire_messages_disk_load_completions(completions, is_current_account, cx);
        self.pump_messages_disk_load_queue(cx);
    }

    fn cancel_stale_queued_message_history_warm_loads(&mut self, cx: &mut Context<Self>) {
        let stale_keys: Vec<String> = self
            .messages_disk_load_queue
            .iter()
            .filter(|(key, request)| {
                request.priority == MessagesDiskLoadPriority::Warm
                    && !self.message_history_warm_candidate_ranks.contains_key(*key)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in stale_keys {
            self.messages_disk_load_queue.remove(&key);
            self.finish_messages_disk_load_completions(&key, false, cx);
        }
    }

    fn consume_message_history_warm_entry(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        if self.message_history_warm_entries.remove(cache_key).is_none() {
            return;
        }
        self.schedule_message_history_warm_expiry(cx);
    }

    fn enforce_message_history_warm_limits(&mut self, now: f64, schedule_expiry: bool, cx: &mut Context<Self>) {
        let records: Vec<MessageHistoryWarmCacheRecord> =
            self.message_history_warm_entries.values().map(MessageHistoryWarmEntry::policy_record).collect();
        let keys = MessageHistoryWarmCachePolicy::eviction_keys(&records, now, &MESSAGE_HISTORY_WARM_LIMITS);
        for key in keys {
            self.evict_message_history_warm_entry(&key, cx);
        }
        if schedule_expiry {
            self.schedule_message_history_warm_expiry(cx);
        }
    }

    fn evict_message_history_warm_entry(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(entry) = self.message_history_warm_entries.remove(key) else {
            return;
        };
        if !SaiAccountCacheScope::is_current_account_epoch(
            entry.account_user_id.as_deref(),
            entry.account_epoch.as_deref(),
        ) || SaiWsResponseCache::memory_only(key, cx).is_none()
            || cache_revision(key, cx) != entry.installed_revision
        {
            return;
        }
        SaiWsResponseCache::remove(key, SaiCacheCleanLayer::Memory, cx);
        self.decoded_messages_memo.borrow_mut().remove(key);
        self.message_presentation_decoded_rows_memo.remove(key);
        self.pinned_messages_memo.remove(key);
        self.latest_reply_keyboard_memo.remove(key);
        self.message_presentation_slices.remove(&entry.scope);
    }

    fn schedule_message_history_warm_expiry(&mut self, cx: &mut Context<Self>) {
        self.messages_warm_expiry_task = None;
        let Some(expiry) =
            self.message_history_warm_entries.values().map(|entry| entry.expires_at).min_by(f64::total_cmp)
        else {
            return;
        };
        let seconds = (expiry - system_uptime_seconds()).max(WARM_EXPIRY_MINIMUM_SECONDS);
        self.messages_warm_expiry_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(seconds)).await;
            let _ = this.update(cx, |store, cx| {
                store.enforce_message_history_warm_limits(system_uptime_seconds(), true, cx);
            });
        }));
    }

    pub fn preload_tab_slice_from_disk_if_needed<T: DeserializeOwned + Clone + 'static>(
        &self,
        slice: &Rc<SaiTypedSlice<T>>,
        cache_id: &str,
        chat_id: &str,
        cx: &mut App,
    ) {
        if chat_id.is_empty() || presentation_is_active(cx) {
            return;
        }
        SaiUpdater::hydrate_slice_from_disk(slice, &self.tab_cache_key(cache_id, chat_id), None, cx);
    }

    pub fn prepare_profile_tab_cache_from_disk(
        &mut self,
        cache_id: &str,
        chat_id: &str,
        completion: ProfileTabDiskLoadCompletion,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() {
            fire_profile_tab_disk_load_completions(vec![completion], false, false, cx);
            return;
        }

        let cache_key = self.tab_cache_key(cache_id, chat_id);
        if presentation_is_active(cx) {
            let has_memory = SaiWsResponseCache::memory_only(&cache_key, cx).is_some();
            fire_profile_tab_disk_load_completions(vec![completion], true, has_memory, cx);
            return;
        }
        if SaiWsResponseCache::memory_only(&cache_key, cx).is_some() {
            fire_profile_tab_disk_load_completions(vec![completion], true, true, cx);
            return;
        }
        if let Some(completions) = self.profile_tab_disk_load_completions.get_mut(&cache_key) {
            completions.push(completion);
            return;
        }

        self.profile_tab_disk_load_completions.insert(cache_key.clone(), vec![completion]);
        let generation = self.profile_tab_disk_load_generations.entry(cache_key.clone()).or_insert(0);
        *generation = generation.wrapping_add(1);
        let load_generation = *generation;
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let this = cx.weak_entity();
        let finished_key = cache_key.clone();
        SaiWsDiskCache::load_asynchronously(
            cache_key,
            move |payload, cx| {
                let _ = this.update(cx, |store, cx| {
                    store.finish_profile_tab_disk_load(
                        &finished_key,
                        load_generation,
                        account_user_id.as_deref(),
                        account_epoch.as_deref(),
                        payload,
                        cx,
                    );
                });
            },
            cx,
        );
    }

    fn finish_profile_tab_disk_load(
        &mut self,
        cache_key: &str,
        load_generation: u64,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        payload: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        if self.profile_tab_disk_load_generations.get(cache_key) != Some(&load_generation) {
            return;
        }
        let completions = self.profile_tab_disk_load_completions.remove(cache_key).unwrap_or_default();
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            fire_profile_tab_disk_load_completions(completions, false, false, cx);
            return;
        }
        let has_payload = payload.is_some();
        if let Some(payload) = payload {
            SaiWsResponseCache::hydrate_memory(cache_key, payload, true, cx);
        }
        fire_profile_tab_disk_load_completions(completions, true, has_payload, cx);
    }

    pub fn tab_cache_key(&self, prefix: &str, chat_id: &str) -> String {
        SaiCache::new([prefix, chat_id]).key
    }

    pub fn has_tab_cache(&self, prefix: &str, chat_id: &str, cx: &mut App) -> bool {
        let cache_key = self.tab_cache_key(prefix, chat_id);
        if presentation_is_active(cx) {
            return SaiWsResponseCache::memory_only(&cache_key, cx).is_some();
        }
        current_payload(&cache_key, cx).is_some()
    }

    pub fn get_messages_payload(&self, chat_id: &str, cx: &mut App) -> Option<GetMessagesGatewayDTO> {
        if let Some(scope) = active_message_scope(chat_id, cx) {
            let entity = RiseConversationPresentation::shared(cx);
            let presentation = entity.read(cx);
            if let Some(state) = presentation.message_state(&scope) {
                return Some(GetMessagesGatewayDTO {
                    chat_id: Some(scope.chat_id.clone()),
                    limit: Some(state.rows.len() as i64),
                    relative_id: state.before_cursor.clone(),
                    is_have_more: Some(state.has_more_before),
                    messages: Some(presentation.message_wires(&scope).to_vec()),
                    success: Some(!matches!(state.load_state, RiseEntityViewLoadState::Failed)),
                    error: state.error_code.clone(),
                });
            }
        }
        let payload = self.cached_messages_payload(&Self::messages_cache_key(chat_id), false, cx)?;
        MessagePayloadCodec::decode(Some(&payload))
    }

    pub fn cached_message_count(&self, chat_id: &str, cx: &mut App) -> i64 {
        if let Some(scope) = active_message_scope(chat_id, cx) {
            return RiseConversationPresentation::shared(cx).read(cx).messages(&scope).len() as i64;
        }
        let Some(payload) = self.cached_messages_payload(&Self::messages_cache_key(chat_id), false, cx) else {
            return 0;
        };
        message_count(&payload)
    }

    pub fn in_memory_message_count(&self, chat_id: &str, cx: &mut App) -> i64 {
        if let Some(scope) = active_message_scope(chat_id, cx) {
            return RiseConversationPresentation::shared(cx).read(cx).messages(&scope).len() as i64;
        }
        if chat_id.is_empty() {
            return 0;
        }
        SaiWsResponseCache::memory_only(&Self::messages_cache_key(chat_id), cx).map_or(0, |payload| message_count(&payload))
    }

    pub fn messages(&self, chat_id: &str, cx: &mut App) -> Vec<GetMessageMessage> {
        if chat_id.is_empty() {
            return Vec::new();
        }

        if let Some(scope) = active_message_scope(chat_id, cx) {
            return RiseConversationPresentation::shared(cx).read(cx).messages(&scope).to_vec();
        }

        let key = Self::messages_cache_key(chat_id);
        let Some(payload) = self.cached_messages_payload(&key, true, cx) else {
            return Vec::new();
        };

        if let Some(dict) = MessagePayloadCodec::dictionary(Some(&payload)) {
            let revision = cache_revision(&key, cx);
            let memoized = self
                .decoded_messages_memo
                .borrow()
                .get(&key)
                .filter(|memo| memo.revision == revision)
                .map(|memo| memo.ordered.clone());
            let ordered = match memoized {
                Some(ordered) => ordered,
                None => {
                    let rows = MessagePayloadCodec::array_of_dictionaries(dict.get(key::MESSAGES));
                    let mapped = self.map_wire_rows(&rows, chat_id, cx);
                    let ordered = MessageThreadOrder::chronological(mapped);
                    let mut memos = self.decoded_messages_memo.borrow_mut();
                    if !memos.contains_key(&key) && memos.len() >= Self::DECODED_MESSAGES_MEMO_MAX_KEYS {
                        memos.clear();
                    }
                    memos.insert(key.clone(), DecodedMessagesMemo { revision, ordered: ordered.clone() });
                    ordered
                }
            };
            return MessageUploadPreviewDebug::appending_to_thread(ordered, chat_id);
        }

        let Some(dto) = MessagePayloadCodec::decode::<GetMessagesGatewayDTO>(Some(&payload)) else {
            return Vec::new();
        };

        let mapped: Vec<GetMessageMessage> = dto
            .messages
            .unwrap_or_default()
            .iter()
            .map(|wire| {
                let mut message = wire.to_message();
                if message.chat_id.is_empty() {
                    message.chat_id = chat_id.to_owned();
                }
                message
            })
            .collect();

        let ordered = MessageThreadOrder::chronological(mapped);
        MessageUploadPreviewDebug::appending_to_thread(ordered, chat_id)
    }

    pub fn messages_in_memory(&self, chat_id: &str, cx: &mut App) -> Vec<GetMessageMessage> {
        if presentation_is_active(cx) {
            return self.messages(chat_id, cx);
        }
        if !self.has_messages_in_memory(chat_id, cx) {
            return Vec::new();
        }
        self.messages(chat_id, cx)
    }

    pub fn has_cached_message(&self, message_id: &str, chat_id: &str, cx: &mut App) -> bool {
        if let Some(scope) = active_message_scope(chat_id, cx) {
            return RiseConversationPresentation::shared(cx)
                .read(cx)
                .messages(&scope)
                .iter()
                .any(|message| message.matches_message_id(message_id));
        }
        let Some(target) = MessagePresentationWindowProjector::normalize_anchor_id(Some(message_id)) else {
            return false;
        };
        let Some(payload) = self.cached_messages_payload(&Self::messages_cache_key(chat_id), false, cx) else {
            return false;
        };
        let Some(rows) = raw_message_rows(&payload) else {
            return false;
        };
        for row in rows {
            let Some(row) = row.as_object() else {
                continue;
            };
            for row_key in [key::ID, SERVER_ID_KEY, key::TEMP_ID] {
                let Some(value) = row.get(row_key).and_then(Value::as_str) else {
                    continue;
                };
                if MessagePresentationWindowProjector::normalize_anchor_id(Some(value)).as_deref()
                    == Some(target.as_str())
                {
                    return true;
                }
            }
        }
        false
    }

    pub fn message_presentation_slice(
        &mut self,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) -> MessagePresentationSlice<GetMessageMessage> {
        if let Some(snapshot) = self.message_scope_presentation_snapshot(chat_id, cx) {
            return snapshot.slice;
        }
        let cache_key = Self::messages_cache_key(chat_id);
        self.consume_message_history_warm_entry(&cache_key, cx);
        let revision = cache_revision(&cache_key, cx);
        let payload =
            if chat_id.is_empty() { None } else { self.memory_messages_payload(&cache_key, true, cx) };
        let Some(rows) = payload.as_ref().and_then(raw_message_rows) else {
            let state = MessagePresentationWindowState {
                revision,
                generation: self.message_presentation_local_revisions.get(chat_id).copied().unwrap_or(0),
                source_order: MessagePresentationSourceOrder::Ascending,
                canonical_count: 0,
                range: 0..0,
                anchor_id: None,
                anchor_offset: 0,
            };
            return MessagePresentationSlice { state, messages: Vec::new() };
        };

        let cached = self.message_presentation_slices.get(chat_id).cloned();
        if let Some(cached) = &cached
            && cached.state.revision == revision
            && cached.state.canonical_count == rows.len() as i64
        {
            return cached.clone();
        }

        let slice = if let Some(cached) = cached {
            let anchor_id = cached.state.anchor_id.clone();
            let projected = MessagePresentationWindowProjector::reanchor(
                rows,
                revision,
                &cached.state,
                anchor_id.as_deref(),
                |row| self.decode_presentation_message(row, chat_id, &cache_key, revision, cx),
            );
            MessagePresentationWindowProjector::preserving_content_generation(cached, projected)
        } else {
            let position_key = self.scroll_position_key(chat_id);
            let saved_anchor_id =
                ChatScrollPositionStore::saved_position(&position_key, cx).and_then(|position| position.anchor_id);
            MessagePresentationWindowProjector::seed(rows, revision, saved_anchor_id.as_deref(), |row| {
                self.decode_presentation_message(row, chat_id, &cache_key, revision, cx)
            })
        };
        if !self.message_presentation_slices.contains_key(chat_id)
            && self.message_presentation_slices.len() >= Self::DECODED_MESSAGES_MEMO_MAX_KEYS
            && let Some(first_key) = self.message_presentation_slices.keys().next().cloned()
        {
            self.message_presentation_slices.remove(&first_key);
        }
        self.message_presentation_slices.insert(chat_id.to_owned(), slice.clone());
        slice
    }

    pub fn message_scope_presentation_snapshot(
        &self,
        chat_id: &str,
        cx: &mut App,
    ) -> Option<MessageScopePresentationSnapshot> {
        let scope = active_message_scope(chat_id, cx)?;
        let snapshot = RiseConversationPresentation::shared(cx).read(cx).message_scope_snapshot(&scope)?;
        let source = &snapshot.source;
        let leading: i64 = if source.has_cached_before { 1 } else { 0 };
        let trailing: i64 = if source.has_cached_after { 1 } else { 0 };
        let count = snapshot.messages.len() as i64;
        let slice = MessagePresentationSlice {
            state: MessagePresentationWindowState {
                revision: source.revision,
                generation: snapshot.revision,
                source_order: MessagePresentationSourceOrder::Ascending,
                canonical_count: leading + count + trailing,
                range: leading..(leading + count),
                anchor_id: source.anchor_local_id.clone(),
                anchor_offset: snapshot.anchor_offset,
            },
            messages: snapshot.messages.clone(),
        };
        let request_status = match source.load_state {
            RiseEntityViewLoadState::Failed => {
                if snapshot.messages.is_empty() {
                    SaiWsSliceStatus::Rejected
                } else {
                    SaiWsSliceStatus::Fulfilled
                }
            }
            RiseEntityViewLoadState::Idle | RiseEntityViewLoadState::Loading => {
                if !snapshot.messages.is_empty() {
                    SaiWsSliceStatus::Fulfilled
                } else if source.error_code.is_none() {
                    SaiWsSliceStatus::Pending
                } else {
                    SaiWsSliceStatus::Rejected
                }
            }
            RiseEntityViewLoadState::Loaded => SaiWsSliceStatus::Fulfilled,
        };
        Some(MessageScopePresentationSnapshot {
            revision: snapshot.revision,
            slice,
            latest_message: snapshot.messages.last().cloned(),
            latest_reply_keyboard_message: snapshot.latest_reply_keyboard_message.clone(),
            pinned_messages: snapshot.pinned_messages.clone(),
            content_stamps_by_local_id: snapshot.content_stamps_by_local_id.clone(),
            has_more_before: source.has_more_before,
            has_more_after: source.has_more_after,
            request_status,
            error_code: source.error_code.clone(),
        })
    }

    pub fn message_scope_presentation_source(
        &self,
        chat_id: &str,
        cx: &mut App,
    ) -> Option<std::sync::Arc<RiseMessageScopePresentationSnapshot>> {
        let scope = active_message_scope(chat_id, cx)?;
        RiseConversationPresentation::shared(cx).read(cx).message_scope_snapshot(&scope)
    }

    pub fn shift_message_presentation_window(
        &mut self,
        chat_id: &str,
        direction: MessagePresentationWindowShift,
        visible_anchor_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = self.message_presentation_slice(chat_id, cx);
        if let Some(scope) = active_message_scope(chat_id, cx) {
            let can_shift = match direction {
                MessagePresentationWindowShift::Older => current.has_cached_before(),
                MessagePresentationWindowShift::Newer => current.has_cached_after(),
            };
            if !can_shift {
                return false;
            }
            let anchor = MessagePresentationWindowProjector::normalize_anchor_id(visible_anchor_id)
                .or_else(|| current.messages.first().map(|message| message.id.clone()))
                .or_else(|| current.messages.last().map(|message| message.id.clone()));
            let Some(anchor) = anchor else {
                return false;
            };
            let anchor_offset_in_window =
                current.messages.iter().position(|message| message.id == anchor).map(|index| index as i64);
            let shift = match direction {
                MessagePresentationWindowShift::Older => RiseMessageWindowShift::Older,
                MessagePresentationWindowShift::Newer => RiseMessageWindowShift::Newer,
            };
            let stride = MessagePresentationWindowProjector::stride();
            if let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() {
                let _ = RiseAccountRuntime::current().spawn(async move {
                    domain.shift_message_window(&scope, &anchor, shift, anchor_offset_in_window, Some(stride)).await;
                });
            }
            return true;
        }
        let can_shift = match direction {
            MessagePresentationWindowShift::Older => current.has_cached_before(),
            MessagePresentationWindowShift::Newer => current.has_cached_after(),
        };
        if !can_shift {
            return false;
        }

        let cache_key = Self::messages_cache_key(chat_id);
        let revision = cache_revision(&cache_key, cx);
        let Some(payload) = self.cached_messages_payload(&cache_key, false, cx) else {
            return false;
        };
        let Some(rows) = raw_message_rows(&payload) else {
            return false;
        };

        let shifted = match direction {
            MessagePresentationWindowShift::Older => MessagePresentationWindowProjector::shift_older_slice(
                rows,
                revision,
                &current,
                visible_anchor_id,
                |row| self.decode_presentation_message(row, chat_id, &cache_key, revision, cx),
            ),
            MessagePresentationWindowShift::Newer => MessagePresentationWindowProjector::shift_newer_slice(
                rows,
                revision,
                &current,
                visible_anchor_id,
                |row| self.decode_presentation_message(row, chat_id, &cache_key, revision, cx),
            ),
        };
        if shifted.state.range == current.state.range {
            return false;
        }
        let trace_direction = match direction {
            MessagePresentationWindowShift::Older => MessageHistoryDirection::Older,
            MessagePresentationWindowShift::Newer => MessageHistoryDirection::Newer,
        };
        let detail = format!(
            "range={}..<{} generation={}",
            shifted.state.range.start, shifted.state.range.end, shifted.state.generation
        );
        let cached_count = shifted.state.canonical_count;
        let result_count = shifted.messages.len() as i64;
        self.message_presentation_slices.insert(chat_id.to_owned(), shifted);
        self.bump_message_presentation_local_revision(chat_id);
        MessagePaginationTrace::record(
            "window.shift",
            chat_id,
            Some(trace_direction),
            None,
            None,
            visible_anchor_id,
            Some(cached_count),
            None,
            Some(result_count),
            None,
            Some(&detail),
            cx,
        );
        cx.notify();
        true
    }

    pub fn latest_message(&mut self, chat_id: &str, cx: &mut Context<Self>) -> Option<GetMessageMessage> {
        if let Some(snapshot) = self.message_scope_presentation_snapshot(chat_id, cx) {
            return snapshot.latest_message;
        }
        let slice = self.message_presentation_slice(chat_id, cx);
        if !slice.has_cached_after() {
            return slice.messages.last().cloned();
        }
        let cache_key = Self::messages_cache_key(chat_id);
        let payload = self.memory_messages_payload(&cache_key, false, cx)?;
        let rows = raw_message_rows(&payload)?;
        if slice.state.range.is_empty() {
            return None;
        }
        let row = match slice.state.source_order {
            MessagePresentationSourceOrder::Ascending => rows.last(),
            MessagePresentationSourceOrder::Descending => rows.first(),
        }?
        .as_object()?;
        self.decode_presentation_message(row, chat_id, &cache_key, slice.state.revision, cx)
    }

    pub fn project_message_presentation_to_latest(&mut self, chat_id: &str, cx: &mut Context<Self>) -> bool {
        let current = self.message_presentation_slice(chat_id, cx);
        let position_key = self.scroll_position_key(chat_id);
        if let Some(scope) = active_message_scope(chat_id, cx) {
            if !current.has_cached_after() {
                ChatScrollPositionStore::clear(&position_key, cx);
                return false;
            }
            if let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() {
                let _ = RiseAccountRuntime::current().spawn(async move {
                    domain.present_latest_messages(&scope).await;
                });
            }
            ChatScrollPositionStore::clear(&position_key, cx);
            return true;
        }
        if MessagePresentationWindowProjector::is_at_latest_cached_boundary(&current.state) {
            ChatScrollPositionStore::clear(&position_key, cx);
            return false;
        }

        let cache_key = Self::messages_cache_key(chat_id);
        let revision = cache_revision(&cache_key, cx);
        let Some(payload) = self.cached_messages_payload(&cache_key, false, cx) else {
            return false;
        };
        let Some(rows) = raw_message_rows(&payload).filter(|rows| !rows.is_empty()) else {
            return false;
        };

        let latest = MessagePresentationWindowProjector::latest(rows, revision, &current.state, |row| {
            self.decode_presentation_message(row, chat_id, &cache_key, revision, cx)
        });
        ChatScrollPositionStore::clear(&position_key, cx);
        if latest.state.range == current.state.range && !current.has_cached_after() {
            return false;
        }

        let detail = format!(
            "range={}..<{} generation={}",
            latest.state.range.start, latest.state.range.end, latest.state.generation
        );
        let cached_count = latest.state.canonical_count;
        let result_count = latest.messages.len() as i64;
        self.message_presentation_slices.insert(chat_id.to_owned(), latest);
        self.bump_message_presentation_local_revision(chat_id);
        MessagePaginationTrace::record(
            "window.latest",
            chat_id,
            None,
            None,
            None,
            None,
            Some(cached_count),
            None,
            Some(result_count),
            None,
            Some(&detail),
            cx,
        );
        cx.notify();
        true
    }

    pub fn pinned_messages_in_loaded_history(
        &mut self,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) -> Vec<GetMessageMessage> {
        if let Some(snapshot) = self.message_scope_presentation_snapshot(chat_id, cx) {
            return snapshot.pinned_messages;
        }
        let cache_key = Self::messages_cache_key(chat_id);
        let revision = cache_revision(&cache_key, cx);
        if let Some(memo) = self.pinned_messages_memo.get(&cache_key)
            && memo.revision == revision
        {
            return memo.messages.clone();
        }
        let Some(payload) = self.memory_messages_payload(&cache_key, true, cx) else {
            return Vec::new();
        };
        let Some(rows) = raw_message_rows(&payload) else {
            return Vec::new();
        };
        let mut pinned = Vec::new();
        for row in rows {
            let Some(row) = row.as_object() else {
                continue;
            };
            if row.get(key::IS_PINNED).and_then(ns_number_bool_value) != Some(true) {
                continue;
            }
            if let Some(message) = self.decode_presentation_message(row, chat_id, &cache_key, revision, cx) {
                pinned.push(message);
            }
        }
        let pinned = MessageThreadOrder::chronological(pinned);
        self.pinned_messages_memo.insert(cache_key, PinnedMessagesMemo { revision, messages: pinned.clone() });
        pinned
    }

    pub fn latest_reply_keyboard_message(
        &mut self,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<GetMessageMessage> {
        if let Some(snapshot) = self.message_scope_presentation_snapshot(chat_id, cx) {
            return snapshot.latest_reply_keyboard_message;
        }
        let cache_key = Self::messages_cache_key(chat_id);
        let revision = cache_revision(&cache_key, cx);
        if let Some(memo) = self.latest_reply_keyboard_memo.get(&cache_key)
            && memo.revision == revision
        {
            return memo.message.clone();
        }
        let slice = self.message_presentation_slice(chat_id, cx);
        let payload = self.memory_messages_payload(&cache_key, true, cx)?;
        let rows = raw_message_rows(&payload)?;
        let ascending = slice.state.source_order == MessagePresentationSourceOrder::Ascending;
        let candidate = if ascending {
            rows.iter().rev().find_map(row_with_reply_markup)
        } else {
            rows.iter().find_map(row_with_reply_markup)
        };
        let result =
            candidate.and_then(|row| self.decode_presentation_message(row, chat_id, &cache_key, revision, cx));
        self.latest_reply_keyboard_memo
            .insert(cache_key, LatestReplyKeyboardMemo { revision, message: result.clone() });
        result
    }

    fn decode_presentation_message(
        &mut self,
        row: &Map<String, Value>,
        chat_id: &str,
        cache_key: &str,
        revision: i64,
        cx: &mut App,
    ) -> Option<GetMessageMessage> {
        if !self.message_presentation_decoded_rows_memo.contains_key(cache_key) {
            if self.message_presentation_decoded_rows_memo.len()
                >= Self::MESSAGE_PRESENTATION_DECODED_ROWS_MEMO_MAX_KEYS
                && let Some(first_key) = self.message_presentation_decoded_rows_memo.keys().next().cloned()
            {
                self.message_presentation_decoded_rows_memo.remove(&first_key);
            }
            self.message_presentation_decoded_rows_memo.insert(
                cache_key.to_owned(),
                MessagePresentationDecodedRowCache::new(message_presentation_decoded_rows_memo_capacity()),
            );
        }

        let mut memo = self
            .message_presentation_decoded_rows_memo
            .remove(cache_key)
            .unwrap_or_else(|| MessagePresentationDecodedRowCache::new(message_presentation_decoded_rows_memo_capacity()));
        let message = memo.value(row, revision, |row| self.map_wire_row(row, chat_id, cx));
        self.message_presentation_decoded_rows_memo.insert(cache_key.to_owned(), memo);
        message
    }

    pub fn did_load_messages(&self, chat_id: &str, cx: &mut App) -> bool {
        if chat_id.is_empty() {
            return false;
        }
        if let Some(scope) = active_message_scope(chat_id, cx) {
            return RiseConversationPresentation::shared(cx).read(cx).message_state(&scope).is_some();
        }
        self.cached_messages_payload(&Self::messages_cache_key(chat_id), false, cx).is_some()
    }

    pub fn has_messages_in_memory(&self, chat_id: &str, cx: &mut App) -> bool {
        if chat_id.is_empty() {
            return false;
        }
        if let Some(scope) = active_message_scope(chat_id, cx) {
            return RiseConversationPresentation::shared(cx).read(cx).message_state(&scope).is_some();
        }
        SaiWsResponseCache::memory_only(&Self::messages_cache_key(chat_id), cx).is_some()
    }

    pub fn begin_initial_messages_request(&mut self, chat_id: &str, has_cached_data: bool, cx: &mut Context<Self>) {
        self.initial_messages_requests_in_flight.insert(chat_id.to_owned());
        self.initial_messages_status_by_scope.insert(
            chat_id.to_owned(),
            if has_cached_data { SaiWsSliceStatus::Fulfilled } else { SaiWsSliceStatus::Pending },
        );
        self.initial_messages_error_by_scope.remove(chat_id);
        for direction in MessageHistoryDirection::ALL_CASES {
            let lane = MessageHistoryLane { chat_id: chat_id.to_owned(), direction };
            let mut state = self.message_history_lane_states.get(&lane).cloned().unwrap_or_default();
            state.is_loading = false;
            state.retry_after = 0.0;
            self.message_history_request_tokens.remove(&lane);
            self.message_history_lane_states.insert(lane, state);
        }
        cx.notify();
    }

    pub fn finish_initial_messages_request(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.initial_messages_requests_in_flight.remove(chat_id);
        self.initial_messages_status_by_scope.insert(chat_id.to_owned(), SaiWsSliceStatus::Fulfilled);
        self.initial_messages_error_by_scope.remove(chat_id);
        cx.notify();
    }

    pub fn fail_initial_messages_request(&mut self, error: &anyhow::Error, chat_id: &str, cx: &mut Context<Self>) {
        self.initial_messages_requests_in_flight.remove(chat_id);
        if self.did_load_messages(chat_id, cx) {
            self.initial_messages_status_by_scope.insert(chat_id.to_owned(), SaiWsSliceStatus::Fulfilled);
            self.initial_messages_error_by_scope.remove(chat_id);
        } else {
            self.initial_messages_status_by_scope.insert(chat_id.to_owned(), SaiWsSliceStatus::Rejected);
            let text = error
                .downcast_ref::<SaiWsResponseError>()
                .and_then(|response| CanonErrors::text(response.code.as_deref(), &response.details));
            match text {
                Some(text) => self.initial_messages_error_by_scope.insert(chat_id.to_owned(), text),
                None => self.initial_messages_error_by_scope.remove(chat_id),
            };
        }
        cx.notify();
    }

    pub fn initial_messages_status(&self, chat_id: &str, cx: &mut App) -> SaiWsSliceStatus {
        if let Some(scope) = active_message_scope(chat_id, cx) {
            let entity = RiseConversationPresentation::shared(cx);
            if let Some(state) = entity.read(cx).message_state(&scope) {
                return match state.load_state {
                    RiseEntityViewLoadState::Failed => {
                        if state.rows.is_empty() {
                            SaiWsSliceStatus::Rejected
                        } else {
                            SaiWsSliceStatus::Fulfilled
                        }
                    }
                    RiseEntityViewLoadState::Idle | RiseEntityViewLoadState::Loading => {
                        if state.rows.is_empty() {
                            SaiWsSliceStatus::Pending
                        } else {
                            SaiWsSliceStatus::Fulfilled
                        }
                    }
                    RiseEntityViewLoadState::Loaded => SaiWsSliceStatus::Fulfilled,
                };
            }
        }
        if self.has_messages_in_memory(chat_id, cx) {
            return SaiWsSliceStatus::Fulfilled;
        }
        self.initial_messages_status_by_scope.get(chat_id).copied().unwrap_or(SaiWsSliceStatus::Pending)
    }

    pub fn is_initial_messages_request_in_flight(&self, chat_id: &str) -> bool {
        self.initial_messages_requests_in_flight.contains(chat_id)
    }

    pub fn initial_messages_error(&self, chat_id: &str, cx: &mut App) -> Option<String> {
        if let Some(scope) = active_message_scope(chat_id, cx) {
            let entity = RiseConversationPresentation::shared(cx);
            let code = entity.read(cx).message_state(&scope).and_then(|state| state.error_code.clone());
            return CanonErrors::text(code.as_deref(), &HashMap::new());
        }
        self.initial_messages_error_by_scope.get(chat_id).cloned()
    }

    pub fn set_message_list_scrolling(&mut self, is_scrolling: bool, chat_id: &str, cx: &mut Context<Self>) {
        let cache_key = Self::messages_cache_key(chat_id);
        if is_scrolling {
            self.messages_disk_persistence_tasks.remove(&cache_key);
            self.scrolling_messages_cache_keys.insert(cache_key);
        } else {
            self.scrolling_messages_cache_keys.remove(&cache_key);
            self.schedule_messages_disk_persistence(&cache_key, None, false, cx);
        }
    }

    pub fn flush_messages_disk_persistence(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.flush_messages_disk_persistence_cache_key(&Self::messages_cache_key(chat_id), true, cx);
        self.flush_messages_disk_persistence_cache_key(&Self::latest_messages_cache_key(chat_id), true, cx);
    }

    pub fn mark_messages_cache_for_disk_persistence(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let cache_key = Self::messages_cache_key(chat_id);
        self.pending_messages_disk_cache_keys.insert(cache_key.clone());
        let generation = self.messages_disk_persistence_generation.entry(cache_key.clone()).or_insert(0);
        *generation = generation.wrapping_add(1);
        self.schedule_messages_disk_persistence(&cache_key, None, false, cx);
    }

    pub fn mark_latest_messages_cache_for_disk_persistence(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let cache_key = Self::latest_messages_cache_key(chat_id);
        self.pending_messages_disk_cache_keys.insert(cache_key.clone());
        let generation = self.messages_disk_persistence_generation.entry(cache_key.clone()).or_insert(0);
        *generation = generation.wrapping_add(1);
        self.schedule_messages_disk_persistence(&cache_key, Some(LATEST_DISK_PERSISTENCE_DELAY_NANOSECONDS), false, cx);
    }

    pub fn flush_pending_messages_to_disk(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<String> = self.pending_messages_disk_cache_keys.iter().cloned().collect();
        for cache_key in keys {
            self.flush_messages_disk_persistence_cache_key(&cache_key, true, cx);
        }
    }

    pub fn begin_message_history_request(
        &mut self,
        chat_id: &str,
        direction: MessageHistoryDirection,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let lane = MessageHistoryLane { chat_id: chat_id.to_owned(), direction };
        let mut state = self.message_history_lane_states.get(&lane).cloned().unwrap_or_default();
        if state.is_loading {
            MessagePaginationTrace::record(
                "request.blocked",
                chat_id,
                Some(direction),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some("reason=lane-in-flight"),
                cx,
            );
            return None;
        }
        let now = RiseClock::now().time_interval_since_1970();
        if now < state.retry_after {
            let detail = format!("reason=retry-backoff remaining={:.2}s", state.retry_after - now);
            MessagePaginationTrace::record(
                "request.blocked",
                chat_id,
                Some(direction),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(&detail),
                cx,
            );
            return None;
        }
        self.next_message_history_request_token = self.next_message_history_request_token.wrapping_add(1);
        let token = self.next_message_history_request_token;
        state.is_loading = true;
        state.retry_after = 0.0;
        self.message_history_lane_states.insert(lane.clone(), state);
        self.message_history_request_tokens.insert(lane, token);
        MessagePaginationTrace::record(
            "lane.begin",
            chat_id,
            Some(direction),
            Some(token),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            cx,
        );
        cx.notify();
        Some(token)
    }

    pub fn is_current_message_history_request(
        &self,
        chat_id: &str,
        direction: MessageHistoryDirection,
        request_token: u64,
    ) -> bool {
        self.message_history_request_tokens.get(&MessageHistoryLane { chat_id: chat_id.to_owned(), direction })
            == Some(&request_token)
    }

    pub fn begin_message_seed_request(&mut self, chat_id: &str, cx: &mut Context<Self>) -> u64 {
        self.next_message_seed_request_token = self.next_message_seed_request_token.wrapping_add(1);
        let token = self.next_message_seed_request_token;
        self.message_seed_request_tokens.insert(chat_id.to_owned(), token);
        let latest_boundary = self.current_message_seed_boundary(chat_id, cx);
        self.message_seed_refresh_fences
            .insert(chat_id.to_owned(), (token, MessageSeedRefreshFence { latest_boundary }));
        MessagePaginationTrace::record(
            "seed.begin",
            chat_id,
            None,
            Some(token),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            cx,
        );
        token
    }

    pub fn is_current_message_seed_request(&self, chat_id: &str, token: u64) -> bool {
        self.message_seed_request_tokens.get(chat_id) == Some(&token)
    }

    pub fn cancel_message_seed_request(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let Some(token) = self.message_seed_request_tokens.remove(chat_id) else {
            return;
        };
        self.message_seed_refresh_fences.remove(chat_id);
        self.initial_messages_requests_in_flight.remove(chat_id);
        MessagePaginationTrace::record(
            "seed.cancel",
            chat_id,
            None,
            Some(token),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            cx,
        );
        cx.notify();
    }

    pub fn message_seed_refresh_fence(&self, chat_id: &str, token: u64) -> Option<MessageSeedRefreshFence> {
        let (fence_token, fence) = self.message_seed_refresh_fences.get(chat_id)?;
        (*fence_token == token).then(|| fence.clone())
    }

    pub fn finish_message_seed_request(&mut self, chat_id: &str, token: u64, cx: &mut Context<Self>) {
        if self.message_seed_request_tokens.get(chat_id) != Some(&token) {
            MessagePaginationTrace::record(
                "seed.finish-ignored",
                chat_id,
                None,
                Some(token),
                None,
                None,
                None,
                None,
                None,
                None,
                Some("reason=stale-token"),
                cx,
            );
            return;
        }
        self.message_seed_request_tokens.remove(chat_id);
        self.message_seed_refresh_fences.remove(chat_id);
        MessagePaginationTrace::record(
            "seed.finish",
            chat_id,
            None,
            Some(token),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            cx,
        );
    }

    fn current_message_seed_boundary(&mut self, chat_id: &str, cx: &mut Context<Self>) -> Option<MessageThreadOrderKey> {
        let canonical_payload = self.cached_messages_payload(&Self::messages_cache_key(chat_id), false, cx);
        let latest_payload = current_payload(&Self::latest_messages_cache_key(chat_id), cx);
        let mut newest: Option<MessageThreadOrderKey> = None;
        for payload in [canonical_payload, latest_payload].into_iter().flatten() {
            let Some(candidate) = message_seed_boundary(&payload) else {
                continue;
            };
            match &newest {
                Some(newest_boundary) => {
                    if MessageThreadOrder::precedes(newest_boundary, &candidate) {
                        newest = Some(candidate);
                    }
                }
                None => newest = Some(candidate),
            }
        }
        newest
    }

    pub fn reset_message_presentation_window(&mut self, chat_id: &str, anchor_id: &str, cx: &mut Context<Self>) {
        let position_key = self.scroll_position_key(chat_id);
        let previous = ChatScrollPositionStore::saved_position(&position_key, cx);
        ChatScrollPositionStore::save_position(
            ChatScrollPositionStorePosition::new(
                Some(anchor_id.to_owned()),
                previous.as_ref().map_or(0.0, |position| position.viewport_offset),
                previous.as_ref().map_or(0.0, |position| position.distance_from_top),
            ),
            &position_key,
            false,
            cx,
        );
        self.message_presentation_slices.remove(chat_id);
        self.bump_message_presentation_local_revision(chat_id);
        cx.notify();
    }

    pub fn finish_message_history_request(
        &mut self,
        chat_id: &str,
        direction: MessageHistoryDirection,
        request_token: Option<u64>,
        failed: bool,
        cx: &mut Context<Self>,
    ) {
        let lane = MessageHistoryLane { chat_id: chat_id.to_owned(), direction };
        if let Some(request_token) = request_token {
            if self.message_history_request_tokens.get(&lane) != Some(&request_token) {
                MessagePaginationTrace::record(
                    "lane.finish-ignored",
                    chat_id,
                    Some(direction),
                    Some(request_token),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some("reason=stale-token"),
                    cx,
                );
                return;
            }
            self.message_history_request_tokens.remove(&lane);
        }
        let mut state = self.message_history_lane_states.get(&lane).cloned().unwrap_or_default();
        state.is_loading = false;
        state.retry_after =
            if failed { RiseClock::now().time_interval_since_1970() + HISTORY_RETRY_SECONDS } else { 0.0 };
        self.message_history_lane_states.insert(lane, state);
        let detail = format!("failed={failed} retry={}", if failed { "1.5s" } else { "none" });
        MessagePaginationTrace::record(
            "lane.finish",
            chat_id,
            Some(direction),
            request_token,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&detail),
            cx,
        );
        cx.notify();
    }

    pub fn is_message_history_request_loading(&self, chat_id: &str, direction: MessageHistoryDirection) -> bool {
        self.message_history_lane_states
            .get(&MessageHistoryLane { chat_id: chat_id.to_owned(), direction })
            .is_some_and(|state| state.is_loading)
    }

    pub fn message_history_page_state(
        &self,
        chat_id: &str,
        direction: MessageHistoryDirection,
        cx: &mut App,
    ) -> MessageHistoryPageState {
        if let Some(scope) = active_message_scope(chat_id, cx) {
            let entity = RiseConversationPresentation::shared(cx);
            if let Some(state) = entity.read(cx).message_state(&scope) {
                return match direction {
                    MessageHistoryDirection::Older => MessageHistoryPageState {
                        relative_id: state.before_cursor.clone(),
                        has_more: state.has_more_before,
                    },
                    MessageHistoryDirection::Newer => MessageHistoryPageState {
                        relative_id: state.after_cursor.clone(),
                        has_more: state.has_more_after,
                    },
                };
            }
        }
        let empty = MessageHistoryPageState { relative_id: None, has_more: false };
        let Some(payload) = self.cached_messages_payload(&Self::messages_cache_key(chat_id), false, cx) else {
            return empty;
        };
        if let Some(root) = MessagePayloadCodec::dictionary(Some(&payload)) {
            if let Some(pagination) = MessagePayloadCodec::dictionary(root.get(HISTORY_PAGINATION_KEY))
                && let Some(lane) = MessagePayloadCodec::dictionary(pagination.get(direction.raw_value()))
            {
                return MessageHistoryPageState {
                    relative_id: normalized_message_history_relative_id(lane.get(key::RELATIVE_ID)),
                    has_more: message_history_bool(lane.get(key::HAS_MORE)).unwrap_or(false),
                };
            }
            if direction == MessageHistoryDirection::Older {
                return MessageHistoryPageState {
                    relative_id: normalized_message_history_relative_id(root.get(key::RELATIVE_ID)),
                    has_more: message_history_bool(root.get(key::IS_HAVE_MORE)).unwrap_or(false),
                };
            }
        }
        empty
    }

    pub fn has_more_messages(
        &self,
        chat_id: &str,
        direction: MessageHistoryDirection,
        cx: &mut App,
    ) -> bool {
        self.message_history_page_state(chat_id, direction, cx).has_more
    }

    pub fn messages_relative_id(
        &self,
        chat_id: &str,
        direction: MessageHistoryDirection,
        cx: &mut App,
    ) -> Option<String> {
        self.message_history_page_state(chat_id, direction, cx).relative_id
    }

    fn cached_messages_payload(&self, cache_key: &str, observed: bool, cx: &mut App) -> Option<Value> {
        if observed {
            let _ = cache_revision(cache_key, cx);
        }
        if let Some(payload) = SaiWsResponseCache::memory_only(cache_key, cx) {
            self.known_absent_messages_cache_keys.borrow_mut().remove(cache_key);
            return Some(payload);
        }
        if self.known_absent_messages_cache_keys.borrow().contains(cache_key) {
            return None;
        }
        let payload = current_payload(cache_key, cx);
        if payload.is_none() {
            self.known_absent_messages_cache_keys.borrow_mut().insert(cache_key.to_owned());
        }
        payload
    }

    fn memory_messages_payload(&self, cache_key: &str, observed: bool, cx: &mut App) -> Option<Value> {
        if observed {
            let _ = cache_revision(cache_key, cx);
        }
        let payload = SaiWsResponseCache::memory_only(cache_key, cx)?;
        self.known_absent_messages_cache_keys.borrow_mut().remove(cache_key);
        Some(payload)
    }

    pub fn message(&self, message_id: &str, chat_id: &str, cx: &mut App) -> Option<GetMessageMessage> {
        self.messages(chat_id, cx).into_iter().find(|message| message.matches_message_id(message_id))
    }

    pub fn server_message_id(&self, message_id: &str, chat_id: &str, cx: &mut App) -> String {
        if let Some(message) = self.message(message_id, chat_id, cx) {
            let server_id = message.sai_server_id();
            if !server_id.is_empty() {
                return server_id;
            }
        }
        let mut row = Map::new();
        put(&mut row, key::ID, message_id);
        put(&mut row, SERVER_ID_KEY, "");
        GetSaiServerId::from_dictionary(&row, &GetSaiServerIdOptions::default())
    }

    pub fn map_wire_rows(&self, rows: &[Map<String, Value>], chat_id: &str, cx: &mut App) -> Vec<GetMessageMessage> {
        rows.iter().filter_map(|row| self.map_wire_row(row, chat_id, cx)).collect()
    }

    fn map_wire_row(&self, row: &Map<String, Value>, chat_id: &str, cx: &mut App) -> Option<GetMessageMessage> {
        let value = Value::Object(row.clone());
        let wire: MessageWireDTO = MessagePayloadCodec::decode_reporting(
            Some(&value),
            || {
                let id = row.get(key::ID).and_then(Value::as_str).unwrap_or("?");
                format!("message row id={id} chat={chat_id}")
            },
            cx,
        )?;
        Some(self.map_decoded_wire_row(&wire, row, chat_id))
    }

    fn map_decoded_wire_row(&self, wire: &MessageWireDTO, row: &Map<String, Value>, chat_id: &str) -> GetMessageMessage {
        let mut message = wire.to_message();
        if message.chat_id.is_empty() {
            message.chat_id = chat_id.to_owned();
        }
        if message.text.is_empty()
            && let Some(content) = row.get(key::CONTENT).and_then(Value::as_str)
            && !content.is_empty()
        {
            message.text = content.to_owned();
        }
        if message.created_at <= 0.0 {
            if let Some(timestamp) = MessageThreadOrder::timestamp_seconds(row) {
                message.created_at = timestamp;
            } else if let Some(timestamp) = self.timestamp_from_temp_message_id(&message.id) {
                message.created_at = timestamp;
            }
        }
        if let Some(server_id) = row.get(SERVER_ID_KEY).and_then(Value::as_str)
            && !server_id.is_empty()
        {
            message.server_id = Some(server_id.to_owned());
        }
        if let Some(flag) = row.get(IS_TEMP_KEY).and_then(Value::as_bool) {
            message.is_temp = flag;
        } else if GetSaiServerId::is_temp_id(Some(message.id.as_str())) {
            message.is_temp =
                message.server_id.as_deref().map_or("", trimming_whitespaces_and_newlines).is_empty();
        }
        let sai_server_id = message.sai_server_id();
        let resolved_server_id = trimming_whitespaces_and_newlines(&sai_server_id);
        if !resolved_server_id.is_empty() && !GetSaiServerId::is_temp_id(Some(resolved_server_id)) {
            message.is_temp = false;
            message.delivery_status =
                if message.is_read { MessageDeliveryStatus::Read } else { MessageDeliveryStatus::Sent };
        } else {
            message.is_temp = true;
            message.delivery_status = MessageDeliveryStatus::Sending;
        }
        message
    }

    #[allow(clippy::too_many_arguments)]
    pub fn direct_patch_message_row(
        &mut self,
        chat_id: &str,
        message_id: &str,
        write_to: SaiUpdaterWriteScope,
        persist_to_disk: bool,
        typed_mutate: Option<&mut dyn FnMut(&mut GetMessageMessage)>,
        mutate: impl FnOnce(&mut Map<String, Value>),
        cx: &mut Context<Self>,
    ) {
        let cache_key = Self::messages_cache_key(chat_id);
        let writes_cache = write_to == SaiUpdaterWriteScope::Cache;
        let mut did_patch = false;
        let result = SaiUpdater::update_json_value(
            &cache_key,
            |ctx| {
                let mut root =
                    MessagePayloadCodec::dictionary(ctx.payload.as_ref()).unwrap_or_else(empty_messages_root);
                let mut rows = MessagePayloadCodec::array_of_dictionaries(root.get(key::MESSAGES));
                let Some(index) = rows.iter().position(|row| row_matches_message(row, message_id)) else {
                    return Ok(Value::Object(root));
                };
                mutate(&mut rows[index]);
                did_patch = true;
                root.insert(key::MESSAGES.to_owned(), rows_to_value(rows));
                Ok(Value::Object(root))
            },
            write_to,
            cx,
        );
        if result.is_err() {
            return;
        }
        if did_patch && let Some(typed_mutate) = typed_mutate {
            self.patch_decoded_messages_memo(chat_id, message_id, typed_mutate, cx);
        }
        if did_patch && writes_cache && persist_to_disk {
            self.mark_messages_cache_for_disk_persistence(chat_id, cx);
        }
    }

    fn patch_decoded_messages_memo(
        &mut self,
        chat_id: &str,
        message_id: &str,
        mutate: &mut dyn FnMut(&mut GetMessageMessage),
        cx: &mut Context<Self>,
    ) {
        let cache_key = Self::messages_cache_key(chat_id);
        let revision = cache_revision(&cache_key, cx);
        let mut memos = self.decoded_messages_memo.borrow_mut();
        let Some(memo) = memos.get_mut(&cache_key) else {
            return;
        };
        let Some(index) = memo.ordered.iter().position(|message| message.matches_message_id(message_id)) else {
            return;
        };
        mutate(&mut memo.ordered[index]);
        memo.revision = revision;
    }

    pub fn upsert_realtime_message_row(
        &mut self,
        chat_id: &str,
        message: &GetMessageMessage,
        payload: &Map<String, Value>,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut incoming = payload.clone();
        incoming.remove(key::TYPE);
        incoming.remove(chat_key::CHAT);
        incoming.remove(chat_key::CHAT_INFO);
        for (row_key, value) in self.wire_to_dict_from(message) {
            incoming.insert(row_key, value);
        }
        if !incoming.contains_key(key::SENDER)
            && let Some(sender) = payload.get(events_key::SENDER_INFO)
        {
            incoming.insert(key::SENDER.to_owned(), sender.clone());
        }

        let cache_key = Self::messages_cache_key(chat_id);
        let did_mutate = self.upsert_realtime_message_row_cache_key(&cache_key, chat_id, &incoming, message, None, cx);
        if did_mutate {
            self.mark_messages_cache_for_disk_persistence(chat_id, cx);
        }

        let latest_cache_key = Self::latest_messages_cache_key(chat_id);
        let did_mutate_latest = self.upsert_realtime_message_row_cache_key(
            &latest_cache_key,
            chat_id,
            &incoming,
            message,
            Some(MessagePresentationWindowProjector::limit()),
            cx,
        );
        if did_mutate_latest {
            self.mark_latest_messages_cache_for_disk_persistence(chat_id, cx);
        }
        did_mutate || did_mutate_latest
    }

    pub fn in_memory_message_timestamp(&self, chat_id: &str, message_id: &str, cx: &mut App) -> Option<f64> {
        if message_id.is_empty() {
            return None;
        }
        for cache_key in [Self::messages_cache_key(chat_id), Self::latest_messages_cache_key(chat_id)] {
            let Some(payload) = SaiWsResponseCache::memory_only(&cache_key, cx) else {
                continue;
            };
            let Some(root) = MessagePayloadCodec::dictionary(Some(&payload)) else {
                continue;
            };
            let timestamp = MessagePayloadCodec::array_of_dictionaries(root.get(key::MESSAGES))
                .iter()
                .find(|row| row_matches_message(row, message_id))
                .and_then(MessageThreadOrder::timestamp_seconds);
            if timestamp.is_some() {
                return timestamp;
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mark_outgoing_messages_read(
        &mut self,
        chat_id: &str,
        sender_id: &str,
        topic_id: Option<&str>,
        cutoff: f64,
        same_millisecond_message_ids: Option<&HashSet<String>>,
        read_message_ids: Option<&HashSet<String>>,
        cx: &mut Context<Self>,
    ) -> bool {
        if sender_id.is_empty() {
            return false;
        }

        let did_mutate = mark_outgoing_messages_read_cache_key(
            &Self::messages_cache_key(chat_id),
            sender_id,
            topic_id,
            cutoff,
            same_millisecond_message_ids,
            read_message_ids,
            cx,
        );
        if did_mutate {
            self.mark_messages_cache_for_disk_persistence(chat_id, cx);
        }

        let did_mutate_latest = mark_outgoing_messages_read_cache_key(
            &Self::latest_messages_cache_key(chat_id),
            sender_id,
            topic_id,
            cutoff,
            same_millisecond_message_ids,
            read_message_ids,
            cx,
        );
        if did_mutate_latest {
            self.mark_latest_messages_cache_for_disk_persistence(chat_id, cx);
        }
        did_mutate || did_mutate_latest
    }

    fn upsert_realtime_message_row_cache_key(
        &self,
        cache_key: &str,
        chat_id: &str,
        incoming: &Map<String, Value>,
        message: &GetMessageMessage,
        limit: Option<i64>,
        cx: &mut App,
    ) -> bool {
        let Some(payload) = SaiWsResponseCache::memory_only(cache_key, cx) else {
            return false;
        };
        let Some(mut root) = MessagePayloadCodec::dictionary(Some(&payload)) else {
            return false;
        };

        let mut rows = MessagePayloadCodec::array_of_dictionaries(root.get(key::MESSAGES));
        let message_id = &message.id;
        let server_id = message.sai_server_id();
        if rows.iter().any(|row| {
            row_matches_message(row, message_id) || (!server_id.is_empty() && row_matches_message(row, &server_id))
        }) {
            return false;
        }

        let mut inserted = incoming.clone();
        let my_id = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .unwrap_or_default();
        let temp_index = (!my_id.is_empty() && message.sender_id == my_id)
            .then(|| {
                rows.iter().position(|row| {
                    let is_temp = row
                        .get(key::ID)
                        .and_then(Value::as_str)
                        .is_some_and(|id| GetSaiServerId::is_temp_id(Some(id)));
                    if !is_temp {
                        return false;
                    }
                    let content = row
                        .get(key::CONTENT)
                        .and_then(Value::as_str)
                        .or_else(|| row.get(bot_key::TEXT).and_then(Value::as_str))
                        .unwrap_or("");
                    let content_type = row.get(key::CONTENT_TYPE).and_then(Value::as_str).unwrap_or("text");
                    content == message.text && content_type == message.content_type.raw_value()
                })
            })
            .flatten();
        if let Some(temp_index) = temp_index {
            let temp_row = rows.remove(temp_index);
            if let Some(mut kept) = self.map_wire_row(&temp_row, chat_id, cx) {
                let temp_id = kept.id.clone();
                Self::merge_real_message_fields(message, &mut kept, &temp_id);
                inserted = temp_row;
                for (row_key, value) in self.wire_to_dict_from(&kept) {
                    if !kept.is_temp && (row_key == key::MEDIA_ITEMS || row_key == key::SENDER) {
                        continue;
                    }
                    inserted.insert(row_key, value);
                }
            } else if let Some(temp_id) = temp_row.get(key::ID).and_then(Value::as_str) {
                inserted = GetSaiServerId::apply_temp_id_preserving_server_id(incoming, Some(temp_id));
                if let Some(media_items) = temp_row.get(key::MEDIA_ITEMS) {
                    inserted.insert(key::MEDIA_ITEMS.to_owned(), media_items.clone());
                }
            }
        }

        let insertion_index = message_insertion_index(&inserted, &rows);
        rows.insert(insertion_index, inserted);
        if let Some(limit) = limit
            && rows.len() as i64 > limit
        {
            rows = rows.split_off(rows.len() - limit as usize);
        }
        root.insert(key::MESSAGES.to_owned(), rows_to_value(rows));
        SaiUpdater::update_json_value(cache_key, move |_| Ok(Value::Object(root)), SaiUpdaterWriteScope::Cache, cx)
            .is_ok()
    }

    pub fn is_within_read_boundary(
        message_id: &str,
        timestamp: f64,
        cutoff: f64,
        same_millisecond_message_ids: Option<&HashSet<String>>,
        read_message_ids: Option<&HashSet<String>>,
    ) -> bool {
        if let Some(read_message_ids) = read_message_ids
            && !read_message_ids.is_empty()
        {
            return read_message_ids.contains(message_id);
        }
        if timestamp <= 0.0 {
            return false;
        }
        let Some(same_millisecond_message_ids) =
            same_millisecond_message_ids.filter(|message_ids| !message_ids.is_empty())
        else {
            return timestamp <= cutoff;
        };
        if timestamp < cutoff {
            return true;
        }
        if timestamp != cutoff {
            return false;
        }
        same_millisecond_message_ids.contains(message_id)
    }

    pub fn decode_gateway<T: DeserializeOwned>(&self, any: Option<&Value>) -> Option<T> {
        MessagePayloadCodec::decode(any)
    }

    pub fn mutate_messages_list<Output: MessagesListTransformOutput>(
        &mut self,
        chat_id: &str,
        write_to: SaiUpdaterWriteScope,
        mut transform: impl FnMut(&mut Vec<GetMessageMessage>) -> Output,
        cx: &mut Context<Self>,
    ) {
        if presentation_is_active(cx) {
            return;
        }
        let cache_key = Self::messages_cache_key(chat_id);
        let writes_cache = write_to == SaiUpdaterWriteScope::Cache;
        let did_mutate = self.apply_messages_list_mutation(&cache_key, chat_id, write_to, None, &mut transform, cx);
        if did_mutate && writes_cache {
            self.mark_messages_cache_for_disk_persistence(chat_id, cx);
        }

        let latest_cache_key = Self::latest_messages_cache_key(chat_id);
        if current_payload(&latest_cache_key, cx).is_some() {
            let did_mutate_latest = self.apply_messages_list_mutation(
                &latest_cache_key,
                chat_id,
                SaiUpdaterWriteScope::Cache,
                Some(MessagePresentationWindowProjector::limit()),
                &mut transform,
                cx,
            );
            if did_mutate_latest {
                self.mark_latest_messages_cache_for_disk_persistence(chat_id, cx);
            }
        }
    }

    fn apply_messages_list_mutation<Output: MessagesListTransformOutput>(
        &self,
        cache_key: &str,
        chat_id: &str,
        write_to: SaiUpdaterWriteScope,
        limit: Option<i64>,
        transform: &mut impl FnMut(&mut Vec<GetMessageMessage>) -> Output,
        cx: &mut App,
    ) -> bool {
        let mut undecodable_rows: Vec<Value> = Vec::new();
        let result = SaiUpdater::update_json_value(
            cache_key,
            |ctx| {
                let mut root =
                    MessagePayloadCodec::dictionary(ctx.payload.as_ref()).unwrap_or_else(empty_messages_root);
                let raw_rows = MessagePayloadCodec::array_of_dictionaries(root.get(key::MESSAGES));

                let mut messages = Vec::with_capacity(raw_rows.len());
                for row in &raw_rows {
                    let value = Value::Object(row.clone());
                    match MessagePayloadCodec::decode::<MessageWireDTO>(Some(&value)) {
                        Some(wire) => messages.push(self.map_decoded_wire_row(&wire, row, chat_id)),
                        None => undecodable_rows.push(value),
                    }
                }
                let mut raw_by_id: HashMap<String, Map<String, Value>> = HashMap::with_capacity(raw_rows.len());
                for row in raw_rows {
                    if let Some(id) = row.get(key::ID).and_then(Value::as_str).filter(|id| !id.is_empty()) {
                        raw_by_id.insert(id.to_owned(), row);
                    }
                }
                let output = transform(&mut messages);
                output.apply_to(&mut messages);
                let did_trim = limit.is_some_and(|limit| messages.len() as i64 > limit);
                if let Some(limit) = limit
                    && did_trim
                {
                    messages = messages.split_off(messages.len() - limit as usize);
                }
                let updated_rows: Vec<Map<String, Value>> = messages
                    .iter()
                    .map(|message| {
                        let diff = self.wire_to_dict_from(message);
                        let Some(mut original) = raw_by_id.remove(&message.id) else {
                            return diff;
                        };
                        for (row_key, value) in diff {
                            if !message.is_temp && (row_key == key::MEDIA_ITEMS || row_key == key::SENDER) {
                                continue;
                            }
                            original.insert(row_key, value);
                        }
                        original
                    })
                    .collect();
                let oldest_id =
                    if did_trim { updated_rows.iter().find_map(oldest_retained_message_id) } else { None };
                root.insert(key::MESSAGES.to_owned(), rows_to_value(updated_rows));
                if let Some(oldest_id) = oldest_id {
                    put(&mut root, key::RELATIVE_ID, oldest_id.as_str());
                    put(&mut root, key::IS_HAVE_MORE, true);
                    let mut pagination =
                        MessagePayloadCodec::dictionary(root.get(HISTORY_PAGINATION_KEY)).unwrap_or_default();
                    let older_key = MessageHistoryDirection::Older.raw_value();
                    let mut older = MessagePayloadCodec::dictionary(pagination.get(older_key)).unwrap_or_default();
                    put(&mut older, key::RELATIVE_ID, oldest_id.as_str());
                    put(&mut older, key::HAS_MORE, true);
                    pagination.insert(older_key.to_owned(), Value::Object(older));
                    root.insert(HISTORY_PAGINATION_KEY.to_owned(), Value::Object(pagination));
                }
                Ok(Value::Object(root))
            },
            write_to,
            cx,
        );
        for value in undecodable_rows {
            let _ = MessagePayloadCodec::decode_reporting::<MessageWireDTO>(
                Some(&value),
                || {
                    let id = value.get(key::ID).and_then(Value::as_str).unwrap_or("?");
                    format!("message row id={id} chat={chat_id}")
                },
                cx,
            );
        }
        result.is_ok()
    }

    pub fn patch_messages_root(
        &mut self,
        chat_id: &str,
        message_rows: Vec<Map<String, Value>>,
        cx: &mut Context<Self>,
    ) {
        self.patch_messages_root_with_history(chat_id, None, None, HashMap::new(), message_rows, cx);
    }

    pub fn patch_messages_root_with_history(
        &mut self,
        chat_id: &str,
        relative_id: Option<&str>,
        is_have_more: Option<bool>,
        history_updates: HashMap<MessageHistoryDirection, MessageHistoryPageState>,
        message_rows: Vec<Map<String, Value>>,
        cx: &mut Context<Self>,
    ) {
        let cache_key = Self::messages_cache_key(chat_id);
        let result = SaiUpdater::update_json_value(
            &cache_key,
            move |ctx| {
                let mut root = MessagePayloadCodec::dictionary(ctx.payload.as_ref()).unwrap_or_default();
                root.insert(key::MESSAGES.to_owned(), rows_to_value(message_rows));
                if let Some(is_have_more) = is_have_more {
                    match relative_id {
                        Some(relative_id) => put(&mut root, key::RELATIVE_ID, relative_id),
                        None => {
                            root.remove(key::RELATIVE_ID);
                        }
                    }
                    put(&mut root, key::IS_HAVE_MORE, is_have_more);
                }
                let mut pagination =
                    MessagePayloadCodec::dictionary(root.get(HISTORY_PAGINATION_KEY)).unwrap_or_default();
                for (direction, state) in history_updates {
                    let direction_key = direction.raw_value();
                    let mut lane = MessagePayloadCodec::dictionary(pagination.get(direction_key)).unwrap_or_default();
                    match state.relative_id {
                        Some(relative_id) => put(&mut lane, key::RELATIVE_ID, relative_id),
                        None => {
                            lane.remove(key::RELATIVE_ID);
                        }
                    }
                    put(&mut lane, key::HAS_MORE, state.has_more);
                    pagination.insert(direction_key.to_owned(), Value::Object(lane));
                }
                if !pagination.is_empty() {
                    root.insert(HISTORY_PAGINATION_KEY.to_owned(), Value::Object(pagination));
                }
                put(&mut root, key::CHAT_ID, chat_id);
                Ok(Value::Object(root))
            },
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        if result.is_ok() {
            self.mark_messages_cache_for_disk_persistence(chat_id, cx);
        }
    }

    fn schedule_messages_disk_persistence(
        &mut self,
        cache_key: &str,
        delay_nanoseconds: Option<u64>,
        ignores_scrolling: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.pending_messages_disk_cache_keys.contains(cache_key)
            || (!ignores_scrolling && self.scrolling_messages_cache_keys.contains(cache_key))
        {
            return;
        }
        if ignores_scrolling {
            self.forced_messages_disk_persistence_keys.insert(cache_key.to_owned());
        }
        self.messages_disk_persistence_tasks.remove(cache_key);
        let lease = ignores_scrolling.then(MessagesDiskBackgroundLease::new);
        let resolved_delay_nanoseconds =
            delay_nanoseconds.unwrap_or(Self::MESSAGES_DISK_SAFETY_CHECKPOINT_DELAY_NANOSECONDS);
        let owned_key = cache_key.to_owned();
        let task = cx.spawn(async move |this, cx| {
            let mut lease = lease;
            cx.background_executor().timer(Duration::from_nanos(resolved_delay_nanoseconds)).await;
            let _ = this.update(cx, |store, cx| {
                store.flush_messages_disk_persistence_cache_key(&owned_key, ignores_scrolling, cx);
            });
            if let Some(lease) = lease.as_mut() {
                lease.end();
            }
        });
        self.messages_disk_persistence_tasks.insert(cache_key.to_owned(), task);
    }

    fn flush_messages_disk_persistence_cache_key(
        &mut self,
        cache_key: &str,
        ignores_scrolling: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.pending_messages_disk_cache_keys.contains(cache_key) {
            self.forced_messages_disk_persistence_keys.remove(cache_key);
            return;
        }
        if ignores_scrolling {
            self.forced_messages_disk_persistence_keys.insert(cache_key.to_owned());
        }
        let is_forced = self.forced_messages_disk_persistence_keys.contains(cache_key);
        if !is_forced && self.scrolling_messages_cache_keys.contains(cache_key) {
            return;
        }
        let Some(payload) = SaiWsResponseCache::memory_only(cache_key, cx) else {
            if self.messages_disk_persistence_in_flight.contains_key(cache_key) {
                return;
            }
            self.messages_disk_persistence_tasks.remove(cache_key);
            self.pending_messages_disk_cache_keys.remove(cache_key);
            self.messages_disk_persistence_generation.remove(cache_key);
            self.forced_messages_disk_persistence_keys.remove(cache_key);
            return;
        };
        if self.messages_disk_persistence_in_flight.contains_key(cache_key) {
            return;
        }
        self.messages_disk_persistence_tasks.remove(cache_key);
        let generation = self.messages_disk_persistence_generation.get(cache_key).copied().unwrap_or(0);
        self.messages_disk_persistence_in_flight.insert(cache_key.to_owned(), generation);
        let mut lease = MessagesDiskBackgroundLease::new();
        let own_author_ids = self.own_author_ids_for_disk_persistence(cx);
        let this = cx.weak_entity();
        let finished_key = cache_key.to_owned();
        SaiWsDiskCache::save_asynchronously_prepare(
            cache_key.to_owned(),
            payload,
            move |value| Self::sanitized_messages_payload_for_disk(&value, |id| Self::is_own_author(id, &own_author_ids)),
            move |result, cx| {
                let _ = this.update(cx, |store, cx| {
                    store.finish_messages_disk_persistence(&finished_key, generation, result.is_ok(), cx);
                });
                lease.end();
            },
            cx,
        );
    }

    fn finish_messages_disk_persistence(
        &mut self,
        cache_key: &str,
        generation: u64,
        succeeded: bool,
        cx: &mut Context<Self>,
    ) {
        if self.messages_disk_persistence_in_flight.get(cache_key) != Some(&generation) {
            if SaiWsResponseCache::memory_only(cache_key, cx).is_none()
                && !self.pending_messages_disk_cache_keys.contains(cache_key)
            {
                SaiWsDiskCache::remove(cache_key);
            }
            return;
        }
        self.messages_disk_persistence_in_flight.remove(cache_key);
        let remains_forced = self.forced_messages_disk_persistence_keys.contains(cache_key);
        if succeeded {
            if self.messages_disk_persistence_generation.get(cache_key) == Some(&generation) {
                self.pending_messages_disk_cache_keys.remove(cache_key);
                self.messages_disk_persistence_generation.remove(cache_key);
                self.forced_messages_disk_persistence_keys.remove(cache_key);
            } else {
                self.flush_messages_disk_persistence_cache_key(cache_key, remains_forced, cx);
            }
        } else {
            self.schedule_messages_disk_persistence(
                cache_key,
                Some(FAILED_DISK_PERSISTENCE_DELAY_NANOSECONDS),
                remains_forced,
                cx,
            );
        }
    }

    fn persist_pending_messages_before_account_reset(&mut self, cx: &mut Context<Self>) {
        let own_author_ids = self.own_author_ids_for_disk_persistence(cx);
        for cache_key in &self.pending_messages_disk_cache_keys {
            let Some(payload) = SaiWsResponseCache::memory_only(cache_key, cx) else {
                continue;
            };
            let own_author_ids = own_author_ids.clone();
            SaiWsDiskCache::save_asynchronously_prepare(
                cache_key.clone(),
                payload,
                move |value| {
                    Self::sanitized_messages_payload_for_disk(&value, |id| Self::is_own_author(id, &own_author_ids))
                },
                |_, _| {},
                cx,
            );
        }
    }

    fn own_author_ids_for_disk_persistence(&self, cx: &mut App) -> HashSet<String> {
        let mut ids = HashSet::new();
        let entity = AuthServicesStore::shared(cx);
        let auth = entity.read(cx);
        if let Some(effective_id) = auth.effective_user_id_for_wire() {
            let effective_id = trimming_whitespaces_and_newlines(&effective_id);
            if !effective_id.is_empty() {
                ids.insert(effective_id.to_owned());
                ids.insert("me".to_owned());
            }
        }
        if let Some(profile) = auth.profile() {
            let profile_id = trimming_whitespaces_and_newlines(&profile.id);
            if !profile_id.is_empty() {
                ids.insert(profile_id.to_owned());
            }
        }
        ids
    }

    fn is_own_author(author_id: Option<&str>, ids: &HashSet<String>) -> bool {
        let Some(author_id) = author_id.map(trimming_whitespaces_and_newlines).filter(|id| !id.is_empty()) else {
            return false;
        };
        ids.contains(author_id) || ids.contains(&author_id.to_lowercase())
    }

    pub fn sanitized_messages_payload_for_disk(payload: &Value, is_own_author: impl Fn(Option<&str>) -> bool) -> Value {
        sanitize_disk_value(payload, &is_own_author)
    }

    pub fn wire_to_dict_from(&self, message: &GetMessageMessage) -> Map<String, Value> {
        let mut row = Map::new();
        put(&mut row, key::ID, message.id.as_str());
        put(&mut row, key::CHAT_ID, message.chat_id.as_str());
        put(&mut row, key::SENDER_ID, message.sender_id.as_str());
        put(&mut row, key::SENDER_NAME, message.sender_name.as_str());
        put(&mut row, key::CONTENT, message.text.as_str());
        put(&mut row, key::ORIGINAL_CONTENT, message.original_text.as_deref().unwrap_or(&message.text));
        put(&mut row, key::CONTENT_TYPE, message.content_type.raw_value());
        put(&mut row, key::IS_READ, message.is_read);
        put(&mut row, key::PLAYED_BY, message.played_by.clone());
        put(&mut row, key::IS_PINNED, message.is_pinned);
        put(&mut row, key::HAS_PROTECTED_CONTENT, message.has_protected_content);
        put(&mut row, IS_TEMP_KEY, message.is_temp);
        if message.created_at > 0.0 {
            put(&mut row, key::CREATED_AT, message.created_at);
            put(&mut row, key::TIMESTAMP, message.created_at);
        }
        if let Some(system) = &message.system {
            put(&mut row, key::IS_SYSTEM_MESSAGE, true);
            put(&mut row, key::SYSTEM_MESSAGE_TYPE, system.kind.raw_value());
            let mut actor = Map::new();
            if let Some(actor_id) = system.actor_id.as_deref().filter(|id| !id.is_empty()) {
                put(&mut actor, key::ID, actor_id);
            }
            if let Some(actor_name) = system.actor_name.as_deref().filter(|name| !name.is_empty()) {
                put(&mut actor, key::NAME, actor_name);
            }
            if !actor.is_empty() {
                row.insert(key::SYSTEM_ACTOR_INFO.to_owned(), Value::Object(actor));
            }
        }
        if let Some(entities) = message.entities.as_deref().filter(|entities| !entities.is_empty()) {
            put(&mut row, key::ENTITIES, entities);
        }
        if let Some(server_id) = message.server_id.as_deref().filter(|id| !id.is_empty()) {
            put(&mut row, SERVER_ID_KEY, server_id);
        }
        if let Some(discussion_chat_id) = message.discussion_chat_id.as_deref().filter(|id| !id.is_empty()) {
            put(&mut row, key::DISCUSSION_CHAT_ID, discussion_chat_id);
        }
        if let Some(discussion_message_id) = message.discussion_message_id.as_deref().filter(|id| !id.is_empty()) {
            put(&mut row, key::DISCUSSION_MESSAGE_ID, discussion_message_id);
        }
        if let Some(comments_count) = message.comments_count {
            put(&mut row, key::COMMENTS_COUNT, comments_count);
        }
        if !message.recent_commenters.is_empty() {
            let commenters: Vec<Value> = message
                .recent_commenters
                .iter()
                .map(|commenter| {
                    let mut value = Map::new();
                    put(&mut value, key::LOGO, commenter.logo.as_str());
                    put(&mut value, key::NAME, commenter.name.as_str());
                    Value::Object(value)
                })
                .collect();
            row.insert(key::RECENT_COMMENTERS.to_owned(), Value::Array(commenters));
        }
        if let Some(markup) = &message.reply_markup {
            row.insert(key::REPLY_MARKUP.to_owned(), Value::Object(bot_reply_markup_json_object(markup)));
        }
        put(&mut row, key::IS_BOT_MESSAGE, message.is_bot_message);
        if let Some(via_bot) = message.via_bot.as_deref().filter(|via_bot| !via_bot.is_empty()) {
            put(&mut row, key::VIA_BOT, via_bot);
        }
        if let Some(via_bot_tag) = message.via_bot_tag.as_deref().filter(|via_bot_tag| !via_bot_tag.is_empty()) {
            put(&mut row, key::VIA_BOT_TAG, via_bot_tag);
        }
        if let Some(web_app_data) = &message.web_app_data {
            let mut value = Map::new();
            put(&mut value, bot_key::DATA, web_app_data.data.as_str());
            if let Some(button_text) = web_app_data.button_text.as_deref().filter(|text| !text.is_empty()) {
                put(&mut value, reply_markup_key::BUTTON_TEXT, button_text);
            }
            row.insert(key::WEB_APP_DATA.to_owned(), Value::Object(value));
        }

        if !message.sender_id.is_empty() || !message.sender_name.is_empty() {
            let mut sender = Map::new();
            if !message.sender_id.is_empty() {
                put(&mut sender, key::ID, message.sender_id.as_str());
            }
            if !message.sender_name.is_empty() {
                put(&mut sender, key::NAME, message.sender_name.as_str());
            }
            if !message.sender_tag.is_empty() {
                put(&mut sender, key::TAG, message.sender_tag.as_str());
            }
            if let Some(logo) = &message.sender_logo {
                let mut more = Map::new();
                put(&mut more, key::LOGO, logo.as_str());
                sender.insert(key::MORE.to_owned(), Value::Object(more));
            }
            if !sender.is_empty() {
                row.insert(key::SENDER.to_owned(), Value::Object(sender));
            }
        }
        if !message.media_items.is_empty() {
            put(&mut row, key::HAS_ATTACHMENTS, true);
            let slots: Vec<Value> = message
                .media_items
                .iter()
                .enumerate()
                .map(|(index, item)| Value::Object(media_item_to_wire_slot(item, index, &message.id)))
                .collect();
            row.insert(key::MEDIA_ITEMS.to_owned(), Value::Array(slots));
        }
        if let Some(shared_post) = message.shared_post.as_ref().filter(|post| !post.post_id.is_empty()) {
            let mut value = Map::new();
            put(&mut value, events_key::POST_ID, shared_post.post_id.parse::<i64>().unwrap_or(0));
            put(&mut value, comment_key::IS_SHORT, shared_post.is_short);
            put(&mut value, comment_key::IS_RIFF, shared_post.is_riff);
            put(&mut value, comment_key::SHOW_IN_PUBLICATIONS, shared_post.show_in_publications);
            put(&mut value, key::TITLE, shared_post.title.as_str());
            put(&mut value, key::SNIPPET, shared_post.snippet.as_str());
            put(&mut value, events_key::IMAGES, shared_post.images.clone());
            put(&mut value, events_key::VIDEO, shared_post.video.as_str());
            put(&mut value, comment_key::VIDEO_THUMBNAIL, shared_post.video_thumbnail.as_str());
            put(&mut value, comment_key::LIKES_COUNT, shared_post.likes_count);
            put(&mut value, key::COMMENTS_COUNT, shared_post.comments_count);
            let mut author = Map::new();
            put(&mut author, key::ID, shared_post.author.id.as_str());
            put(&mut author, key::TAG, shared_post.author.tag.as_str());
            put(&mut author, key::NAME, shared_post.author.name.as_str());
            put(&mut author, key::LOGO, shared_post.author.logo.as_str());
            put(&mut author, key::IS_PREMIUM, shared_post.author.is_premium);
            value.insert(events_key::AUTHOR.to_owned(), Value::Object(author));
            row.insert(payload_key::SHARED_POST.to_owned(), Value::Object(value));
        }

        put(&mut row, key::HAS_REACTIONS, !message.reactions.is_empty());
        let reactions: Vec<Value> = message
            .reactions
            .iter()
            .map(|reaction| {
                let mut value = Map::new();
                put(&mut value, key::REACTION, reaction.emoji.as_str());
                put(&mut value, key::COUNT, reaction.count);
                put(&mut value, key::REACTED_BY_YOU, reaction.reacted_by_me);
                Value::Object(value)
            })
            .collect();
        row.insert(key::REACTIONS.to_owned(), Value::Array(reactions));

        let mut reacted_by_rows: Vec<Value> = Vec::new();
        for reaction in &message.reactions {
            for reactor in reaction.reactors.iter().filter(|reactor| !reactor.user_id.is_empty()) {
                let mut sender = Map::new();
                put(&mut sender, key::ID, reactor.user_id.as_str());
                put(&mut sender, key::NAME, reactor.name.as_str());
                if let Some(url) = reactor.avatar_url.as_deref().filter(|url| !url.is_empty()) {
                    let mut more = Map::new();
                    put(&mut more, key::LOGO, url);
                    sender.insert(key::MORE.to_owned(), Value::Object(more));
                }
                let mut value = Map::new();
                put(&mut value, key::REACTION, reaction.emoji.as_str());
                put(&mut value, key::CREATED_AT, reactor.created_at);
                value.insert(key::SENDER.to_owned(), Value::Object(sender));
                put(&mut value, key::CHAT_ID, message.chat_id.as_str());
                reacted_by_rows.push(Value::Object(value));
            }
        }
        row.insert(key::REACTED_BY.to_owned(), Value::Array(reacted_by_rows));
        row
    }

    pub fn wire_to_dict(&self, wire: &MessageWireDTO) -> Map<String, Value> {
        self.wire_to_dict_from(&wire.to_message())
    }

    fn bump_message_presentation_local_revision(&mut self, chat_id: &str) {
        let revision = self.message_presentation_local_revisions.entry(chat_id.to_owned()).or_insert(0);
        *revision = revision.wrapping_add(1);
    }
}

fn system_uptime_seconds() -> f64 {
    system_uptime().map_or_else(|_| RiseClock::now().time_interval_since_1970(), |uptime| uptime.as_secs_f64())
}

fn message_presentation_decoded_rows_memo_capacity() -> i64 {
    MessagePresentationWindowProjector::limit() + MessagePresentationWindowProjector::stride() * 2
}

fn cache_revision(cache_key: &str, cx: &mut App) -> i64 {
    SaiCacheRevisionTracker::observed_revision(cache_key, cx)
}

fn current_payload(cache_key: &str, cx: &mut App) -> Option<Value> {
    SaiWsResponseCache::current_payload(
        cache_key,
        SaiWsCachePreferredLayer::MemoryFirst,
        SaiWsDiskCachePolicy::PreserveExisting,
        cx,
    )
}

fn presentation_is_active(cx: &mut App) -> bool {
    RiseConversationPresentation::shared(cx).read(cx).is_active()
}

fn active_message_scope(chat_id: &str, cx: &mut App) -> Option<RiseMessageScope> {
    if !presentation_is_active(cx) {
        return None;
    }
    RiseMessageScope::parse(chat_id)
}

fn fire_messages_disk_load_completions(completions: Vec<MessagesDiskLoadCompletion>, value: bool, cx: &mut App) {
    if completions.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        for completion in completions {
            completion(value, cx);
        }
    });
}

fn fire_profile_tab_disk_load_completions(
    completions: Vec<ProfileTabDiskLoadCompletion>,
    is_current_account: bool,
    has_payload: bool,
    cx: &mut App,
) {
    if completions.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        for completion in completions {
            completion(is_current_account, has_payload, cx);
        }
    });
}

fn promote_messages_disk_load(
    request: &mut MessagesDiskLoadRequest,
    priority: MessagesDiskLoadPriority,
    candidate_rank: i64,
) {
    if priority.cmp(&request.priority) == Ordering::Greater
        || (priority == MessagesDiskLoadPriority::Warm && request.priority == MessagesDiskLoadPriority::Warm)
    {
        request.priority = priority;
        request.candidate_rank = candidate_rank;
    }
}

fn message_count(payload: &Value) -> i64 {
    if let Value::Object(root) = payload
        && !root.is_empty()
    {
        return root.get(key::MESSAGES).and_then(Value::as_array).map_or(0, |rows| {
            rows.iter().filter(|row| row.as_object().is_some_and(|row| !row.is_empty())).count()
        }) as i64;
    }
    MessagePayloadCodec::decode::<GetMessagesGatewayDTO>(Some(payload))
        .and_then(|dto| dto.messages)
        .map_or(0, |messages| messages.len() as i64)
}

fn raw_message_rows(payload: &Value) -> Option<&[Value]> {
    let Value::Object(root) = payload else {
        return None;
    };
    if root.is_empty() {
        return None;
    }
    root.get(key::MESSAGES)?.as_array().map(Vec::as_slice)
}

fn row_with_reply_markup(row: &Value) -> Option<&Map<String, Value>> {
    let row = row.as_object()?;
    row.get(key::REPLY_MARKUP).filter(|markup| !markup.is_null())?;
    Some(row)
}

fn ns_number_bool_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(flag) => Some(*flag),
        Value::Number(number) => number.as_f64().map(|number| number != 0.0),
        _ => None,
    }
}

fn normalized_message_history_relative_id(value: Option<&Value>) -> Option<String> {
    let string = match value? {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => (if *value { "1" } else { "0" }).to_owned(),
        _ => return None,
    };
    let trimmed = trimming_whitespaces_and_newlines(&string);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn message_history_bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        Value::Number(value) => value.as_f64().map(|value| value != 0.0),
        Value::String(value) => match value.to_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn message_seed_boundary(payload: &Value) -> Option<MessageThreadOrderKey> {
    let rows = raw_message_rows(payload)?;
    let first_key = message_seed_order_key(rows.first()?.as_object()?)?;
    let last_key = message_seed_order_key(rows.last()?.as_object()?)?;
    Some(if MessageThreadOrder::precedes(&first_key, &last_key) { last_key } else { first_key })
}

fn message_seed_order_key(row: &Map<String, Value>) -> Option<MessageThreadOrderKey> {
    let timestamp = MessageThreadOrder::timestamp_seconds(row)?;
    for row_key in [SERVER_ID_KEY, key::ID, key::TEMP_ID] {
        let Some(raw) = row.get(row_key).and_then(Value::as_str) else {
            continue;
        };
        let id = trimming_whitespaces_and_newlines(raw);
        if !id.is_empty() {
            return Some(MessageThreadOrderKey { timestamp, id: id.to_owned() });
        }
    }
    None
}

fn oldest_retained_message_id(row: &Map<String, Value>) -> Option<String> {
    if row.get(key::SYSTEM_MESSAGE_TYPE).and_then(Value::as_str) == Some(MessageSystemKind::ContactJoined.raw_value()) {
        return None;
    }
    for row_key in [SERVER_ID_KEY, key::ID, key::TEMP_ID] {
        let Some(raw) = row.get(row_key).and_then(Value::as_str) else {
            continue;
        };
        let value = trimming_whitespaces_and_newlines(raw);
        if !value.is_empty() && !GetSaiServerId::is_temp_id(Some(value)) {
            return Some(value.to_owned());
        }
    }
    None
}

fn row_matches_message(row: &Map<String, Value>, id: &str) -> bool {
    row.get(key::ID).and_then(Value::as_str) == Some(id) || row.get(SERVER_ID_KEY).and_then(Value::as_str) == Some(id)
}

fn mark_outgoing_messages_read_cache_key(
    cache_key: &str,
    sender_id: &str,
    topic_id: Option<&str>,
    cutoff: f64,
    same_millisecond_message_ids: Option<&HashSet<String>>,
    read_message_ids: Option<&HashSet<String>>,
    cx: &mut App,
) -> bool {
    let Some(payload) = SaiWsResponseCache::memory_only(cache_key, cx) else {
        return false;
    };
    let Some(mut root) = MessagePayloadCodec::dictionary(Some(&payload)) else {
        return false;
    };

    let mut rows = MessagePayloadCodec::array_of_dictionaries(root.get(key::MESSAGES));
    let mut did_mutate = false;
    for row in &mut rows {
        let row_topic_id = row
            .get(key::TOPIC_ID)
            .and_then(Value::as_str)
            .or_else(|| row.get(key::camel::TOPIC_ID).and_then(Value::as_str))
            .unwrap_or("");
        let message_id = ws_string(row, &[SAI_SERVER_ID_KEY, SERVER_ID_KEY, key::ID]).unwrap_or_default();
        if row.get(key::SENDER_ID).and_then(Value::as_str) != Some(sender_id)
            || topic_id.is_some_and(|topic_id| row_topic_id != topic_id)
            || row.get(key::IS_READ).and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(timestamp) = MessageThreadOrder::timestamp_seconds(row) else {
            continue;
        };
        if !MessageServicesStore::is_within_read_boundary(
            &message_id,
            timestamp,
            cutoff,
            same_millisecond_message_ids,
            read_message_ids,
        ) {
            continue;
        }
        put(row, key::IS_READ, true);
        did_mutate = true;
    }
    if !did_mutate {
        return false;
    }

    root.insert(key::MESSAGES.to_owned(), rows_to_value(rows));
    SaiUpdater::update_json_value(cache_key, move |_| Ok(Value::Object(root)), SaiUpdaterWriteScope::Cache, cx).is_ok()
}

fn message_insertion_index(row: &Map<String, Value>, rows: &[Map<String, Value>]) -> usize {
    let order_key = message_order_key(row);
    let mut lower = 0;
    let mut upper = rows.len();
    while lower < upper {
        let middle = lower + (upper - lower) / 2;
        if MessageThreadOrder::precedes(&order_key, &message_order_key(&rows[middle])) {
            upper = middle;
        } else {
            lower = middle + 1;
        }
    }
    lower
}

fn message_order_key(row: &Map<String, Value>) -> MessageThreadOrderKey {
    let id = row
        .get(SERVER_ID_KEY)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .or_else(|| row.get(key::ID).and_then(Value::as_str))
        .unwrap_or("");
    MessageThreadOrderKey { timestamp: MessageThreadOrder::timestamp_seconds(row).unwrap_or(0.0), id: id.to_owned() }
}

fn scalar_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => {
            let trimmed = trimming_whitespaces_and_newlines(value);
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some((if *value { "1" } else { "0" }).to_owned()),
        _ => None,
    }
}

fn sanitize_disk_value(value: &Value, is_own_author: &dyn Fn(Option<&str>) -> bool) -> Value {
    let source = match value {
        Value::Array(values) => {
            return Value::Array(values.iter().map(|item| sanitize_disk_value(item, is_own_author)).collect());
        }
        Value::Object(source) => source,
        other => return other.clone(),
    };

    let mut result = Map::with_capacity(source.len());
    for (item_key, item) in source {
        let shared_post = (item_key == payload_key::SHARED_POST || item_key == SHARED_POST_CAMEL_KEY)
            .then(|| item.as_object())
            .flatten();
        let Some(shared_post) = shared_post else {
            result.insert(item_key.clone(), sanitize_disk_value(item, is_own_author));
            continue;
        };

        let author = shared_post.get(events_key::AUTHOR).and_then(Value::as_object);
        let author_id = scalar_string(
            author
                .and_then(|author| author.get(key::ID))
                .or_else(|| shared_post.get(events_key::AUTHOR_ID))
                .or_else(|| shared_post.get(events_key::camel::AUTHOR_ID)),
        );
        if is_own_author(author_id.as_deref()) {
            result.insert(item_key.clone(), sanitize_disk_value(item, is_own_author));
            continue;
        }

        let mut pointer = Map::new();
        if let Some(post_id) =
            shared_post.get(events_key::POST_ID).or_else(|| shared_post.get(events_key::camel::POST_ID))
        {
            pointer.insert(events_key::POST_ID.to_owned(), post_id.clone());
        }
        let is_short = shared_post
            .get(comment_key::IS_SHORT)
            .or_else(|| shared_post.get(events_key::camel::IS_SHORT))
            .cloned()
            .unwrap_or(Value::Bool(false));
        pointer.insert(comment_key::IS_SHORT.to_owned(), is_short);
        result.insert(item_key.clone(), Value::Object(pointer));
    }
    Value::Object(result)
}

fn bot_reply_markup_json_object(markup: &MessageBotReplyMarkup) -> Map<String, Value> {
    let mut value = Map::new();
    if markup.has_inline_keyboard() {
        value.insert(reply_markup_key::INLINE_KEYBOARD.to_owned(), bot_keyboard_rows(&markup.inline_keyboard));
    }
    if markup.has_reply_keyboard() {
        value.insert(reply_markup_key::KEYBOARD.to_owned(), bot_keyboard_rows(&markup.keyboard));
    }
    if markup.resize_keyboard {
        put(&mut value, reply_markup_key::RESIZE_KEYBOARD, true);
    }
    if markup.one_time_keyboard {
        put(&mut value, reply_markup_key::ONE_TIME_KEYBOARD, true);
    }
    if markup.selective {
        put(&mut value, reply_markup_key::SELECTIVE, true);
    }
    if let Some(placeholder) = markup.input_field_placeholder.as_deref().filter(|text| !text.is_empty()) {
        put(&mut value, reply_markup_key::INPUT_FIELD_PLACEHOLDER, placeholder);
    }
    if markup.remove_keyboard {
        put(&mut value, reply_markup_key::REMOVE_KEYBOARD, true);
    }
    if markup.force_reply {
        put(&mut value, reply_markup_key::FORCE_REPLY, true);
    }
    value
}

fn bot_keyboard_rows(rows: &[Vec<MessageBotKeyboardButton>]) -> Value {
    Value::Array(
        rows.iter()
            .map(|row| {
                Value::Array(row.iter().map(|button| Value::Object(bot_keyboard_button_json_object(button))).collect())
            })
            .collect(),
    )
}

fn bot_keyboard_button_json_object(button: &MessageBotKeyboardButton) -> Map<String, Value> {
    let mut value = Map::new();
    put(&mut value, bot_key::TEXT, button.text.as_str());
    if let Some(callback_data) = &button.callback_data {
        put(&mut value, key::CALLBACK_DATA, callback_data.as_str());
    }
    if let Some(url) = &button.url {
        put(&mut value, bot_key::URL, url.as_str());
    }
    if let Some(web_app_url) = &button.web_app_url {
        let mut web_app = Map::new();
        put(&mut web_app, bot_key::URL, web_app_url.as_str());
        if let Some(bot_id) = &button.bot_id {
            put(&mut web_app, bot_key::BOT_ID, bot_id.as_str());
        }
        if let Some(mini_app_id) = &button.mini_app_id {
            put(&mut web_app, bot_key::MINI_APP_ID, mini_app_id.as_str());
        }
        value.insert(reply_markup_key::WEB_APP.to_owned(), Value::Object(web_app));
    }
    if let Some(query) = &button.switch_inline_query {
        put(&mut value, reply_markup_key::SWITCH_INLINE_QUERY, query.as_str());
    }
    if let Some(query) = &button.switch_inline_query_current_chat {
        put(&mut value, reply_markup_key::SWITCH_INLINE_QUERY_CURRENT_CHAT, query.as_str());
    }
    if button.is_pay {
        put(&mut value, reply_markup_key::PAY, true);
    }
    if button.is_game {
        value.insert(reply_markup_key::CALLBACK_GAME.to_owned(), Value::Object(Map::new()));
    }
    value
}

fn media_item_to_wire_slot(item: &MessageMediaItem, index: usize, message_id: &str) -> Map<String, Value> {
    let mut slot = Map::new();
    if item.id.is_empty() {
        put(&mut slot, key::ID, format!("{message_id}-{index}"));
    } else {
        put(&mut slot, key::ID, item.id.as_str());
    }
    put(&mut slot, "local_slot_index", item.local_slot_index.max(0));
    put(&mut slot, key::MEDIA_TYPE, item.kind.raw_value());
    put(&mut slot, key::FILE_URL, item.url.as_str());
    if let Some(file_name) = &item.file_name {
        put(&mut slot, key::FILE_NAME, file_name.as_str());
    }
    if let Some(mime_type) = &item.mime_type {
        put(&mut slot, key::MIME_TYPE, mime_type.as_str());
    }
    put(&mut slot, "is_uploading", item.is_uploading);
    put(&mut slot, "is_compressing", item.is_compressing);
    put(&mut slot, "upload_progress", item.upload_progress);
    put(&mut slot, "compression_progress", item.compression_progress);
    if item.uploaded_bytes > 0 {
        put(&mut slot, "uploaded_bytes", item.uploaded_bytes);
    }
    if item.total_bytes > 0 {
        put(&mut slot, file_storage_admin_key::TOTAL_BYTES, item.total_bytes);
    }
    if item.is_cancelled {
        put(&mut slot, "is_cancelled", true);
    }
    if let Some(local_preview_url) = item.local_preview_url.as_deref().filter(|url| !url.is_empty()) {
        put(&mut slot, "local_preview_url", local_preview_url);
    }
    if let Some(thumbnail_url) = item.thumbnail_url.as_deref().filter(|url| !url.is_empty()) {
        put(&mut slot, key::THUMBNAIL_URL, thumbnail_url);
    }
    if item.width > 0.0 {
        put(&mut slot, key::WIDTH, item.width.round() as i64);
    }
    if item.height > 0.0 {
        put(&mut slot, key::HEIGHT, item.height.round() as i64);
    }
    if let Some(duration) = item.duration {
        put(&mut slot, key::DURATION, duration);
    }
    if let Some(file_size) = item.file_size {
        put(&mut slot, key::FILE_SIZE, file_size);
    }
    if let Some(bitrate) = item.bitrate {
        put(&mut slot, key::BITRATE, bitrate);
    }
    if let Some(fps) = item.fps {
        put(&mut slot, key::FPS, fps);
    }
    if let Some(codec) = &item.codec {
        put(&mut slot, key::CODEC, codec.as_str());
    }
    if !item.waveform.is_empty()
        && let Some(wire) = RiseStoredOutgoingMedia::waveform_wire_string(&item.waveform)
    {
        put(&mut slot, key::WAVEFORM, wire);
    }
    if item.kind == MessageMediaKind::Sticker {
        put(&mut slot, key::STICKER_ID, item.id.as_str());
        if let Some(sticker_kind) = item.sticker_kind {
            put(&mut slot, key::STICKER_TYPE, sticker_kind.raw_value());
        }
        if let Some(pack_id) = item.pack_id.as_deref().filter(|id| !id.is_empty()) {
            put(&mut slot, key::PACK_ID, pack_id);
        }
        if !item.sticker_emojis.is_empty() {
            put(&mut slot, key::ASSOCIATED_EMOJIS, item.sticker_emojis.clone());
        }
    }
    slot.retain(|_, value| !value.is_null() && value.as_str().is_none_or(|text| !text.is_empty()));
    slot
}

fn empty_messages_root() -> Map<String, Value> {
    let mut root = Map::new();
    root.insert(key::MESSAGES.to_owned(), Value::Array(Vec::new()));
    root
}

fn rows_to_value(rows: Vec<Map<String, Value>>) -> Value {
    Value::Array(rows.into_iter().map(Value::Object).collect())
}

fn put(row: &mut Map<String, Value>, row_key: &str, value: impl Into<Value>) {
    row.insert(row_key.to_owned(), value.into());
}
