use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Entity, Global, Subscription, Task};
use rise_canon::rise_key::{self, chat as key};
use rise_canon::{RiseRpcCanon, RiseTimingCanon, RiseUploadContextFormats, RiseValidation};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::shared::try_read;
use rise_validate::CollectionLimit;
use serde_json::{Map, Value};

use crate::core::desktop_platform::rise_app_quit_drain::{QuitDrain, QuitLease};
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_rules::RiseValidationCanonRules;
use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingPageExt};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice};
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::{SaiCacheRevisionBumped, SaiCacheRevisionTracker};
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::{
    SaiWsOptimisticError, SaiWsOptimisticFiles, SaiWsOptimisticLocalFile,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_payload_casing::ws_string;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseConversationPresentation,
};
use crate::modules::chat::engine::core::rise_undeliverable_direct_draft_loader::RiseUndeliverableDirectDraftLoaderOperation;
use crate::modules::chat::shared::utils::chat_list_utils::ChatListUtils;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_user_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_archive_wire_types::RiseStoredArchivedChatPage;
use crate::modules::chat::stores::chats::chat_actions::chat_archive_types::ChatArchiveMutationSnapshot;
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::chat::stores::chats::chat_actions::chat_message_requests_wire_types::RiseStoredMessageRequestPage;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatAnalyticsModel, ChatInfo, ChatMuteScopes, ChatPerm, ChatPermExt, ChatSettingsModel, ChatType,
    ChatWallpaperCatalogState, ChatWallpaperPlacement, JoinLinkSheetState, LinkedChatInfo,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    ChatWallpaperWireDTO, ChatWallpaperWireDTOExt, RiseStoredChatDetail, RiseStoredChatPage,
};
use crate::modules::chat::stores::chats::chat_services::chat_direct_services::DirectChatWaiter;
use crate::modules::chat::stores::chats::chat_services::chat_members_services::ChatMemberListLoadState;
use crate::modules::chat::stores::chats::chat_services::chat_topics_services::ChatRemovedTopicNotice;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageThreadOrder};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessagePayloadCodec;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::music::shared::utils::music::music_cover_upload::MusicCoverUpload;
use crate::modules::user::stores::user::user_actions::user_contact_types::UserContactLimits;

const IS_TEMP: &str = "isTemp";
const IS_ONLINE: &str = "isOnline";

const CHATS_DISK_PERSISTENCE_DEBOUNCE: Duration = Duration::from_millis(1_000);
const CHATS_DISK_PERSISTENCE_RETRY_AFTER_SUCCESS: Duration = Duration::from_millis(150);
const CHATS_DISK_PERSISTENCE_RETRY_AFTER_FAILURE: Duration = Duration::from_millis(2_000);
const CHATS_DISK_PERSISTENCE_ACTIVITY: &str = "riseonly.chats.disk_persistence";

#[derive(Clone, PartialEq)]
pub struct ChatRemovedFromChatNotice {
    pub id: Uuid,
    pub chat_id: String,
    pub uptime: f64,
}

impl ChatRemovedFromChatNotice {
    pub fn new(chat_id: String) -> Self {
        Self { id: Uuid::new_v4(), chat_id, uptime: RiseClock::monotonic().as_secs_f64() }
    }
}

#[derive(Clone, PartialEq)]
pub struct ChatServicesStoreChatInfoIndexStamp {
    pub account_id: String,
    pub active_cache_key: Option<String>,
    pub active_revision: i64,
    pub archived_cache_key: Option<String>,
    pub archived_revision: i64,
    pub requests_cache_key: Option<String>,
    pub requests_revision: i64,
}

#[derive(Clone)]
pub struct ChatServicesStoreChatNotificationCommand {
    pub id: Uuid,
    pub chat_id: String,
    pub scopes: ChatMuteScopes,
}

impl ChatServicesStoreChatNotificationCommand {
    pub fn new(chat_id: String, scopes: ChatMuteScopes) -> Self {
        Self { id: Uuid::new_v4(), chat_id, scopes }
    }
}

#[derive(Clone)]
pub struct ChatServicesStoreTopicNotificationCommand {
    pub id: Uuid,
    pub chat_id: String,
    pub topic_id: String,
    pub scopes: ChatMuteScopes,
    pub inherit: bool,
    pub on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
}

impl ChatServicesStoreTopicNotificationCommand {
    pub fn new(
        chat_id: String,
        topic_id: String,
        scopes: ChatMuteScopes,
        inherit: bool,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
    ) -> Self {
        Self { id: Uuid::new_v4(), chat_id, topic_id, scopes, inherit, on_result }
    }

    pub fn key(&self) -> String {
        format!("{}:topic:{}", self.chat_id, self.topic_id)
    }
}

struct ChatsInboxInfosMemo {
    cache_key: String,
    revision: i64,
    infos: Option<Vec<ChatInfo>>,
}

struct ChatsDiskFallbackMemo {
    revision: i64,
    payload: Option<Value>,
}

#[derive(Clone)]
pub struct ChatListSlices {
    pub get_chats_sai: Rc<SaiTypedSlice<RiseStoredChatPage>>,
    pub get_archived_chats_sai: Rc<SaiTypedSlice<RiseStoredArchivedChatPage>>,
    pub get_chat_by_id_sai: Rc<SaiTypedSlice<RiseStoredChatDetail>>,
    pub get_message_requests_sai: Rc<SaiTypedSlice<RiseStoredMessageRequestPage>>,
}

impl Global for ChatListSlices {}

impl ChatListSlices {
    pub fn shared(cx: &mut App) -> Self {
        if let Some(slices) = cx.try_global::<Self>() {
            return slices.clone();
        }
        let slices = Self {
            get_chats_sai: SaiTypedSlice::new(cx),
            get_archived_chats_sai: SaiTypedSlice::new(cx),
            get_chat_by_id_sai: SaiTypedSlice::new(cx),
            get_message_requests_sai: SaiTypedSlice::new(cx),
        };
        cx.set_global(slices.clone());
        slices
    }
}

pub struct ChatServicesStore {
    pub(crate) get_chats_sai: Rc<SaiTypedSlice<RiseStoredChatPage>>,
    pub(crate) get_archived_chats_sai: Rc<SaiTypedSlice<RiseStoredArchivedChatPage>>,
    pub(crate) get_chat_by_id_sai: Rc<SaiTypedSlice<RiseStoredChatDetail>>,
    pub(crate) get_message_requests_sai: Rc<SaiTypedSlice<RiseStoredMessageRequestPage>>,

    pub(crate) temp_chat_id_to_real: HashMap<String, String>,
    pub(crate) pending_create_chat_temp_id: Option<String>,
    pub(crate) fetched_chat_by_id_set: HashSet<String>,
    pub(crate) community_preview_by_chat_id: HashMap<String, ChatInfo>,
    opening_hint_slot: Option<ChatInfo>,
    pub(crate) draft_direct_chat_by_id: HashMap<String, ChatInfo>,
    engine_contact_join_chats: Vec<ChatInfo>,
    pub(crate) draft_direct_chat_order: Vec<String>,
    pub(crate) undeliverable_draft_direct_chat_ids: HashSet<String>,
    pub(crate) draft_direct_chat_ids_awaiting_connection: HashSet<String>,
    pub(crate) restored_undeliverable_drafts_key: Option<String>,
    pub(crate) undeliverable_drafts_restore_operation: Option<RiseUndeliverableDirectDraftLoaderOperation>,
    pub(crate) undeliverable_drafts_restore_token: Uuid,
    pub(crate) undeliverable_drafts_restore_in_flight: bool,
    pub(crate) undeliverable_drafts_restore_excluded_ids: HashSet<String>,
    pub(crate) undeliverable_drafts_persistence_pending: bool,
    pub(crate) opening_direct_chat_for: HashSet<String>,
    pub(crate) direct_chat_waiters_by_target: HashMap<String, Vec<DirectChatWaiter>>,
    pub(crate) membership_refetch_in_flight: HashSet<String>,
    pub(crate) left_chat_ids: HashSet<String>,
    pub(crate) confirmed_removed_chat_ids: HashSet<String>,
    pub(crate) pending_delete_chat_ids: HashSet<String>,
    pub(crate) archive_mutations_by_chat: HashMap<String, ChatArchiveMutationSnapshot>,
    pub folders: Vec<ChatFolder>,
    pub(crate) contact_ids: HashSet<String>,
    pub topics_enabled_override: HashSet<String>,
    pub topics_disabled_override: HashSet<String>,
    pub(crate) forced_unread_topic_keys: HashSet<String>,
    pub(crate) hide_all_topics_override: HashMap<String, bool>,
    pub(crate) dismissed_join_banner_chat_ids: HashSet<String>,
    pub(crate) join_requests_local_shift: HashMap<String, i64>,
    pub(crate) join_request_decisions_in_flight: HashMap<String, i64>,
    pub(crate) wallpaper_catalogs: HashMap<String, ChatWallpaperCatalogState>,
    pub join_link_sheet: Option<JoinLinkSheetState>,
    pub(crate) removed_topic_notice: Option<ChatRemovedTopicNotice>,
    pub(crate) deleted_topic_unread: HashMap<(String, String), i64>,
    pub removed_from_chat_notice: Option<ChatRemovedFromChatNotice>,
    pub(crate) settings_by_chat: HashMap<String, ChatSettingsModel>,
    pub(crate) slow_mode_until: HashMap<String, Date>,
    pub(crate) slow_mode_server_wait_end: HashMap<String, Date>,
    pub(crate) latest_own_server_send_at: HashMap<String, f64>,
    pub(crate) linked_chat_by_chat: HashMap<String, LinkedChatInfo>,
    pub(crate) analytics_by_chat: HashMap<String, ChatAnalyticsModel>,
    pub(crate) member_scopes_by_chat: HashMap<String, HashSet<String>>,
    pub(crate) member_list_load_states: HashMap<String, ChatMemberListLoadState>,
    pub(crate) member_list_load_started_at: HashMap<String, Duration>,
    member_mutation_chats_in_flight: HashMap<String, Duration>,
    role_mutation_chats_in_flight: HashMap<String, Duration>,
    folder_mutation_started_at: Option<Duration>,
    chat_notification_commands_in_flight: HashMap<String, ChatServicesStoreChatNotificationCommand>,
    pending_chat_notification_commands: HashMap<String, ChatServicesStoreChatNotificationCommand>,
    confirmed_chat_notification_scopes: HashMap<String, ChatMuteScopes>,
    topic_notification_commands_in_flight: HashMap<String, ChatServicesStoreTopicNotificationCommand>,
    pending_topic_notification_commands: HashMap<String, ChatServicesStoreTopicNotificationCommand>,
    chats_disk_persistence_task: Option<Task<()>>,
    chats_disk_persistence_generation: u64,
    chats_disk_persistence_completed_generation: u64,
    chats_disk_persistence_in_flight_generation: Option<u64>,
    chats_disk_persistence_token: Uuid,
    chats_disk_persistence_quit: Option<QuitLease>,
    pub(crate) chat_info_index_stamp: RefCell<Option<ChatServicesStoreChatInfoIndexStamp>>,
    pub(crate) chat_info_index: RefCell<HashMap<String, ChatInfo>>,
    chats_inbox_infos_memo: RefCell<Option<ChatsInboxInfosMemo>>,
    chats_disk_fallback_memo: RefCell<HashMap<String, ChatsDiskFallbackMemo>>,
}

fn active_presentation(cx: &App) -> Option<&RiseConversationPresentation> {
    try_read::<RiseConversationPresentation>(cx).filter(|presentation| presentation.is_active())
}

fn is_contact_join_virtual_chat_id(id: &str) -> bool {
    chat_stores_user_hooks().is_some_and(|hooks| (hooks.is_contact_join_virtual_chat_id)(id))
}

const CHAT_SERVICES_CACHE_OPERATIONS: [&str; 21] = [
    "getChats",
    "getArchivedChats",
    "getMessageRequests",
    "getChatMembers",
    "getAdminMembers",
    "getBlockedMembers",
    "getChatActions",
    "getChatSettings",
    "getLinkedChat",
    "getChatAnalytics",
    "getTopics",
    "getInviteLinks",
    "getJoinRequests",
    "getCommonChats",
    "getChatFolders",
    "folderChatSearch",
    "get_forum_structure",
    "get_effective_permissions",
    "get_topic_overwrites",
    "get_section_overwrites",
    "get_member_permissions",
];

const CHAT_SERVICES_LIST_CACHE_OPERATIONS: [&str; 5] =
    ["getChats", "getArchivedChats", "getMessageRequests", "getChatFolders", "folderChatSearch"];

fn row_from(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(name, value)| (name.to_owned(), value)).collect()
}

impl ChatServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let tracker = SaiCacheRevisionTracker::shared(cx);
        cx.subscribe(&tracker, |_, _, bumped: &SaiCacheRevisionBumped, cx| {
            if bumped.key.as_deref().is_none_or(Self::wakes_store_observers) {
                cx.notify();
            }
        })
        .detach();
        let slices = ChatListSlices::shared(cx);
        Self {
            get_chats_sai: slices.get_chats_sai,
            get_archived_chats_sai: slices.get_archived_chats_sai,
            get_chat_by_id_sai: slices.get_chat_by_id_sai,
            get_message_requests_sai: slices.get_message_requests_sai,
            temp_chat_id_to_real: HashMap::new(),
            pending_create_chat_temp_id: None,
            fetched_chat_by_id_set: HashSet::new(),
            community_preview_by_chat_id: HashMap::new(),
            opening_hint_slot: None,
            draft_direct_chat_by_id: HashMap::new(),
            engine_contact_join_chats: Vec::new(),
            draft_direct_chat_order: Vec::new(),
            undeliverable_draft_direct_chat_ids: HashSet::new(),
            draft_direct_chat_ids_awaiting_connection: HashSet::new(),
            restored_undeliverable_drafts_key: None,
            undeliverable_drafts_restore_operation: None,
            undeliverable_drafts_restore_token: Uuid::new_v4(),
            undeliverable_drafts_restore_in_flight: false,
            undeliverable_drafts_restore_excluded_ids: HashSet::new(),
            undeliverable_drafts_persistence_pending: false,
            opening_direct_chat_for: HashSet::new(),
            direct_chat_waiters_by_target: HashMap::new(),
            membership_refetch_in_flight: HashSet::new(),
            left_chat_ids: HashSet::new(),
            confirmed_removed_chat_ids: HashSet::new(),
            pending_delete_chat_ids: HashSet::new(),
            archive_mutations_by_chat: HashMap::new(),
            folders: Vec::new(),
            contact_ids: HashSet::new(),
            topics_enabled_override: HashSet::new(),
            topics_disabled_override: HashSet::new(),
            forced_unread_topic_keys: HashSet::new(),
            hide_all_topics_override: HashMap::new(),
            dismissed_join_banner_chat_ids: HashSet::new(),
            join_requests_local_shift: HashMap::new(),
            join_request_decisions_in_flight: HashMap::new(),
            wallpaper_catalogs: HashMap::new(),
            join_link_sheet: None,
            removed_topic_notice: None,
            deleted_topic_unread: HashMap::new(),
            removed_from_chat_notice: None,
            settings_by_chat: HashMap::new(),
            slow_mode_until: HashMap::new(),
            slow_mode_server_wait_end: HashMap::new(),
            latest_own_server_send_at: HashMap::new(),
            linked_chat_by_chat: HashMap::new(),
            analytics_by_chat: HashMap::new(),
            member_scopes_by_chat: HashMap::new(),
            member_list_load_states: HashMap::new(),
            member_list_load_started_at: HashMap::new(),
            member_mutation_chats_in_flight: HashMap::new(),
            role_mutation_chats_in_flight: HashMap::new(),
            folder_mutation_started_at: None,
            chat_notification_commands_in_flight: HashMap::new(),
            pending_chat_notification_commands: HashMap::new(),
            confirmed_chat_notification_scopes: HashMap::new(),
            topic_notification_commands_in_flight: HashMap::new(),
            pending_topic_notification_commands: HashMap::new(),
            chats_disk_persistence_task: None,
            chats_disk_persistence_generation: 0,
            chats_disk_persistence_completed_generation: 0,
            chats_disk_persistence_in_flight_generation: None,
            chats_disk_persistence_token: Uuid::new_v4(),
            chats_disk_persistence_quit: None,
            chat_info_index_stamp: RefCell::new(None),
            chat_info_index: RefCell::new(HashMap::new()),
            chats_inbox_infos_memo: RefCell::new(None),
            chats_disk_fallback_memo: RefCell::new(HashMap::new()),
        }
    }

    fn cache_operation(cache_key: &str) -> String {
        let logical = SaiAccountCacheScope::logical_key(cache_key);
        logical.split_once('-').map_or(logical.as_str(), |(operation, _)| operation).to_owned()
    }

    fn wakes_store_observers(cache_key: &str) -> bool {
        let operation = Self::cache_operation(cache_key);
        CHAT_SERVICES_CACHE_OPERATIONS.contains(&operation.as_str())
            && CHAT_SERVICES_LIST_CACHE_OPERATIONS.contains(&operation.as_str())
    }

    fn is_info_key_of(cache_key: &str, chat_id: Option<&str>) -> bool {
        let operation = Self::cache_operation(cache_key);
        CHAT_SERVICES_CACHE_OPERATIONS.contains(&operation.as_str())
            && !CHAT_SERVICES_LIST_CACHE_OPERATIONS.contains(&operation.as_str())
            && chat_id.filter(|id| !id.is_empty()).is_none_or(|id| cache_key.contains(id))
    }

    /// calls on_change on a bump of a chat info key like members, roles settings topics or
    /// rights of chat_id
    /// any chat for none, and on a cache reset
    pub fn subscribe_info_keys(chat_id: Option<String>, cx: &mut App, on_change: impl Fn(&mut App) + 'static) -> Subscription {
        let tracker = SaiCacheRevisionTracker::shared(cx);
        App::subscribe(cx, &tracker, move |_, bumped: &SaiCacheRevisionBumped, cx| {
            if bumped.key.as_deref().is_none_or(|key| Self::is_info_key_of(key, chat_id.as_deref())) {
                on_change(cx);
            }
        })
    }

    pub fn chats_limit() -> i64 {
        RiseTimingCanon::page("chat_list").page_size_items()
    }

    pub fn chats_cache_key_for_user(user_id: &str) -> String {
        SaiCache::new(["getChats", user_id]).key
    }

    pub fn member_scope_key(chat_id: &str, topic_id: Option<&str>) -> String {
        match topic_id.filter(|topic_id| !topic_id.is_empty()) {
            Some(topic_id) => format!("{chat_id}::t::{topic_id}"),
            None => chat_id.to_owned(),
        }
    }

    pub fn members_cache_key(scope: &str) -> String {
        SaiCache::new(["getChatMembers", scope]).key
    }

    pub fn pinned_chats_limit() -> CollectionLimit {
        RiseValidation::collection_limit("pinned_chats")
    }

    pub fn max_pinned_chats(is_premium: bool) -> i64 {
        let limit = Self::pinned_chats_limit();
        if is_premium { limit.premium_maximum.unwrap_or(limit.maximum) as i64 } else { limit.maximum as i64 }
    }

    fn abandoned_mutation_age() -> Duration {
        Duration::from_millis(RiseRpcCanon::TIMEOUTS_MAXIMUM_MS as u64)
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        let actions = ChatActionsStore::shared(cx);
        let search_cache_keys = actions.update(cx, |actions, cx| {
            let keys = std::mem::take(&mut actions.folder_chat_search_cache_order);
            actions.folder_chat_search_slices.clear();
            cx.notify();
            keys
        });
        for cache_key in search_cache_keys {
            SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Memory, cx);
        }
        self.temp_chat_id_to_real.clear();
        self.pending_create_chat_temp_id = None;
        self.fetched_chat_by_id_set.clear();
        self.community_preview_by_chat_id.clear();
        self.opening_hint_slot = None;
        self.draft_direct_chat_by_id.clear();
        self.engine_contact_join_chats.clear();
        self.draft_direct_chat_order.clear();
        self.undeliverable_draft_direct_chat_ids.clear();
        self.draft_direct_chat_ids_awaiting_connection.clear();
        self.restored_undeliverable_drafts_key = None;
        if let Some(operation) = self.undeliverable_drafts_restore_operation.take() {
            operation.cancel();
        }
        self.undeliverable_drafts_restore_token = Uuid::new_v4();
        self.undeliverable_drafts_restore_in_flight = false;
        self.undeliverable_drafts_restore_excluded_ids.clear();
        self.undeliverable_drafts_persistence_pending = false;
        self.opening_direct_chat_for.clear();
        let dropped_direct_chat_waiters: Vec<DirectChatWaiter> =
            self.direct_chat_waiters_by_target.drain().flat_map(|(_, waiters)| waiters).collect();
        cx.defer(move |cx| {
            for waiter in dropped_direct_chat_waiters {
                if let Some(on_error) = waiter.on_error {
                    on_error(String::new(), cx);
                }
            }
        });
        self.membership_refetch_in_flight.clear();
        self.left_chat_ids.clear();
        self.confirmed_removed_chat_ids.clear();
        self.pending_delete_chat_ids.clear();
        self.archive_mutations_by_chat.clear();
        self.folders.clear();
        self.contact_ids.clear();
        self.topics_enabled_override.clear();
        self.forced_unread_topic_keys.clear();
        self.hide_all_topics_override.clear();
        self.topics_disabled_override.clear();
        self.dismissed_join_banner_chat_ids.clear();
        self.join_requests_local_shift.clear();
        self.join_request_decisions_in_flight.clear();
        self.join_link_sheet = None;
        self.removed_from_chat_notice = None;
        self.removed_topic_notice = None;
        self.settings_by_chat.clear();
        self.slow_mode_until.clear();
        self.slow_mode_server_wait_end.clear();
        self.latest_own_server_send_at.clear();
        self.linked_chat_by_chat.clear();
        self.analytics_by_chat.clear();
        self.member_scopes_by_chat.clear();
        self.member_list_load_states.clear();
        self.member_list_load_started_at.clear();
        self.member_mutation_chats_in_flight.clear();
        self.role_mutation_chats_in_flight.clear();
        self.folder_mutation_started_at = None;
        self.chat_notification_commands_in_flight.clear();
        self.pending_chat_notification_commands.clear();
        self.confirmed_chat_notification_scopes.clear();
        self.topic_notification_commands_in_flight.clear();
        self.pending_topic_notification_commands.clear();
        self.chats_disk_persistence_task = None;
        self.chats_disk_persistence_generation = 0;
        self.chats_disk_persistence_completed_generation = 0;
        self.chats_disk_persistence_in_flight_generation = None;
        self.chats_disk_persistence_token = Uuid::new_v4();
        self.chats_disk_persistence_quit = None;
        *self.chat_info_index_stamp.borrow_mut() = None;
        *self.chat_info_index.borrow_mut() = HashMap::new();
        *self.chats_inbox_infos_memo.borrow_mut() = None;
        self.chats_disk_fallback_memo.borrow_mut().clear();
        cx.notify();
    }

    pub fn begin_member_mutation(&mut self, chat_id: &str, _cx: &App) -> bool {
        if chat_id.is_empty()
            || !Self::is_mutation_slot_free(self.member_mutation_chats_in_flight.get(chat_id).copied())
        {
            return false;
        }
        self.member_mutation_chats_in_flight.insert(chat_id.to_owned(), RiseClock::monotonic());
        true
    }

    pub fn finish_member_mutation(&mut self, chat_id: &str, _cx: &App) {
        self.member_mutation_chats_in_flight.remove(chat_id);
    }

    pub fn begin_role_mutation(&mut self, chat_id: &str, _cx: &App) -> bool {
        if chat_id.is_empty()
            || !Self::is_mutation_slot_free(self.role_mutation_chats_in_flight.get(chat_id).copied())
        {
            return false;
        }
        self.role_mutation_chats_in_flight.insert(chat_id.to_owned(), RiseClock::monotonic());
        true
    }

    pub fn finish_role_mutation(&mut self, chat_id: &str, _cx: &App) {
        self.role_mutation_chats_in_flight.remove(chat_id);
    }

    pub fn begin_folder_mutation(&mut self, _cx: &App) -> bool {
        if !Self::is_mutation_slot_free(self.folder_mutation_started_at) {
            return false;
        }
        self.folder_mutation_started_at = Some(RiseClock::monotonic());
        true
    }

    pub fn finish_folder_mutation(&mut self, _cx: &App) {
        self.folder_mutation_started_at = None;
    }

    fn is_mutation_slot_free(started_at: Option<Duration>) -> bool {
        let Some(started_at) = started_at else {
            return true;
        };
        RiseClock::monotonic().saturating_sub(started_at) >= Self::abandoned_mutation_age()
    }

    fn effective_or_active_user_id(cx: &App) -> Option<String> {
        try_read::<AuthServicesStore>(cx)
            .and_then(AuthServicesStore::effective_user_id_for_wire)
            .or_else(SaiAccountCacheScope::active_user_id)
    }

    pub fn current_chat_info_index_stamp(&self, cx: &App) -> ChatServicesStoreChatInfoIndexStamp {
        let account_id = Self::effective_or_active_user_id(cx).unwrap_or_default();
        let active_key = self.chats_cache_key(cx);
        let archived_key = self.archived_chats_cache_key(cx);
        let requests_key = (!account_id.is_empty()).then(|| Self::message_requests_cache_key(&account_id));
        let revision = |cache_key: &Option<String>| {
            cache_key.as_deref().map_or(0, |cache_key| SaiCacheRevisionTracker::observed_revision_in(cache_key, cx))
        };
        ChatServicesStoreChatInfoIndexStamp {
            active_revision: revision(&active_key),
            archived_revision: revision(&archived_key),
            requests_revision: revision(&requests_key),
            account_id,
            active_cache_key: active_key,
            archived_cache_key: archived_key,
            requests_cache_key: requests_key,
        }
    }

    pub fn rebuild_chat_info_index_if_needed(&self, cx: &App) {
        let stamp = self.current_chat_info_index_stamp(cx);
        if self.chat_info_index_stamp.borrow().as_ref() == Some(&stamp) {
            return;
        }

        let active = self.current_chat_infos(cx);
        let archived = self.archived_chats_memory_only(cx);
        let requests = self.message_requests_memory_only(cx);
        let mut next = HashMap::with_capacity(active.len() + archived.len() + requests.len());
        for chat in active {
            next.insert(chat.id.clone(), chat);
        }
        for chat in archived.into_iter().chain(requests) {
            next.entry(chat.id.clone()).or_insert(chat);
        }

        *self.chat_info_index.borrow_mut() = next;
        *self.chat_info_index_stamp.borrow_mut() = Some(stamp);
    }

    pub(crate) fn memory_or_disk_payload(&self, cache_key: &str, cx: &App) -> Option<Value> {
        if let Some(payload) = SaiWsResponseCache::observed_memory_only_read(cache_key, cx) {
            return Some(payload);
        }
        let revision = SaiCacheRevisionTracker::observed_revision_in(cache_key, cx);
        let mut memo = self.chats_disk_fallback_memo.borrow_mut();
        if let Some(entry) = memo.get(cache_key)
            && entry.revision == revision
        {
            return entry.payload.clone();
        }
        let payload = SaiWsDiskCache::load(cache_key);
        memo.insert(cache_key.to_owned(), ChatsDiskFallbackMemo { revision, payload: payload.clone() });
        payload
    }

    pub(crate) fn message_requests_memory_only(&self, cx: &App) -> Vec<ChatInfo> {
        if let Some(presentation) = active_presentation(cx) {
            return presentation.chats(RiseChatListKind::Requests).to_vec();
        }
        Self::effective_or_active_user_id(cx)
            .filter(|user_id| !user_id.is_empty())
            .and_then(|user_id| self.memory_or_disk_payload(&Self::message_requests_cache_key(&user_id), cx))
            .and_then(|payload| SaiJSONCodec::decode::<RiseStoredMessageRequestPage>(Some(&payload)))
            .and_then(|page| page.chats)
            .unwrap_or_default()
            .iter()
            .map(|chat| chat.to_chat_info())
            .collect()
    }

    pub fn enqueue_chat_notification_command(
        &mut self,
        command: ChatServicesStoreChatNotificationCommand,
        confirmed: ChatMuteScopes,
    ) -> bool {
        if self.chat_notification_commands_in_flight.contains_key(&command.chat_id) {
            self.pending_chat_notification_commands.insert(command.chat_id.clone(), command);
            return false;
        }
        self.confirmed_chat_notification_scopes.insert(command.chat_id.clone(), confirmed);
        self.chat_notification_commands_in_flight.insert(command.chat_id.clone(), command);
        true
    }

    pub fn complete_chat_notification_command(
        &mut self,
        command: &ChatServicesStoreChatNotificationCommand,
        success: bool,
    ) -> (bool, Option<ChatServicesStoreChatNotificationCommand>, Option<ChatMuteScopes>) {
        if self.chat_notification_commands_in_flight.get(&command.chat_id).map(|in_flight| in_flight.id) != Some(command.id) {
            return (false, None, None);
        }
        self.chat_notification_commands_in_flight.remove(&command.chat_id);
        if success {
            self.confirmed_chat_notification_scopes.insert(command.chat_id.clone(), command.scopes);
        }
        if let Some(next) = self.pending_chat_notification_commands.remove(&command.chat_id) {
            self.chat_notification_commands_in_flight.insert(command.chat_id.clone(), next.clone());
            return (true, Some(next), None);
        }
        let confirmed = self.confirmed_chat_notification_scopes.remove(&command.chat_id);
        (true, None, if success { None } else { confirmed })
    }

    pub fn enqueue_topic_notification_command(
        &mut self,
        command: ChatServicesStoreTopicNotificationCommand,
        _cx: &App,
    ) -> bool {
        let key = command.key();
        if self.topic_notification_commands_in_flight.contains_key(&key) {
            self.pending_topic_notification_commands.insert(key, command);
            return false;
        }

        self.topic_notification_commands_in_flight.insert(key, command);
        true
    }

    pub fn complete_topic_notification_command(
        &mut self,
        command: &ChatServicesStoreTopicNotificationCommand,
        _cx: &App,
    ) -> (bool, Option<ChatServicesStoreTopicNotificationCommand>) {
        let key = command.key();
        if self.topic_notification_commands_in_flight.get(&key).map(|in_flight| in_flight.id) != Some(command.id) {
            return (false, None);
        }

        self.topic_notification_commands_in_flight.remove(&key);
        let Some(next) = self.pending_topic_notification_commands.remove(&key) else {
            return (true, None);
        };

        self.topic_notification_commands_in_flight.insert(key, next.clone());
        (true, Some(next))
    }

    pub fn real_chat_id(&self, temp_id: &str, _cx: &App) -> Option<String> {
        if !GetSaiServerId::is_temp_id(Some(temp_id)) {
            return None;
        }

        self.temp_chat_id_to_real.get(temp_id).cloned()
    }

    pub fn canonical_chat_id(&self, id: &str) -> String {
        self.temp_chat_id_to_real.get(id).cloned().unwrap_or_else(|| id.to_owned())
    }

    pub fn normalize_chats_after_fetch(&mut self, cx: &mut Context<Self>) {
        let normalized: Vec<ChatInfo> = self
            .deduped_by_id(&self.current_chat_infos(cx))
            .into_iter()
            .filter(|chat| !self.left_chat_ids.contains(&chat.id))
            .collect();
        self.set_chats_payload(ChatListUtils::sort(&normalized), None, None, None, cx);
    }

    pub fn get_chats_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        let line = format!("get_chats error: {error}");
        Sai::log("Chat", move || line, SaiLogKind::Network, cx);
    }

    pub fn apply_optimistic_last_message(&mut self, chat_id: &str, message: GetMessageMessage, cx: &mut Context<Self>) {
        if active_presentation(cx).is_some() || self.left_chat_ids.contains(chat_id) {
            return;
        }
        let mut items = self.current_chat_infos(cx);
        let Some(index) = items.iter().position(|chat| chat.id == chat_id) else {
            return;
        };

        let mut updated = items.remove(index);
        updated.updated_at = message.created_at;
        updated.last_message = Some(message);
        items.insert(0, updated);
        self.set_chats_payload(items, None, None, None, cx);
    }

    pub fn unread_count(&self, chat_id: &str, cx: &App) -> i64 {
        self.chat_info(chat_id, cx).map_or(0, |chat| chat.unread_count)
    }

    pub fn mute_chat_local(&mut self, chat_id: &str, muted: bool, until: Option<f64>, cx: &mut Context<Self>) {
        if self.set_archived_mute_local(chat_id, muted, until, cx) {
            return;
        }
        self.mutate_chat(
            chat_id,
            |chat| {
                chat.is_muted = muted;
                chat.mute_until = if muted { until } else { None };
            },
            cx,
        );
    }

    pub fn apply_notif_scopes_local(&mut self, chat_id: &str, scopes: ChatMuteScopes, cx: &mut Context<Self>) {
        if self.set_archived_notification_scopes_local(chat_id, scopes, cx) {
            return;
        }
        self.mutate_chat(
            chat_id,
            |chat| {
                chat.is_muted = scopes.is_fully_muted();
                chat.notifications_enabled = !scopes.is_fully_muted();
                chat.mute_scopes = if scopes.is_all_enabled() { None } else { scopes.encoded_json() };
            },
            cx,
        );
    }

    pub fn set_allowed_reactions_local(&mut self, chat_id: &str, reactions: Vec<String>, cx: &mut Context<Self>) {
        if self.set_archived_allowed_reactions_local(chat_id, &reactions, cx) {
            return;
        }
        self.mutate_chat(chat_id, |chat| chat.allowed_reactions = reactions, cx);
    }

    pub fn pinned_count(&self, cx: &App) -> i64 {
        self.current_chat_infos(cx).iter().filter(|chat| chat.is_pinned).count() as i64
    }

    pub fn pinned_chat_ids_in_order(&self, cx: &App) -> Vec<String> {
        let pinned: Vec<ChatInfo> = self.current_chat_infos(cx).into_iter().filter(|chat| chat.is_pinned).collect();
        ChatListUtils::sort(&pinned).into_iter().map(|chat| chat.id).collect()
    }

    pub fn pinned_reorder_inserting(&self, chat_id: &str, cx: &App) -> Vec<String> {
        let mut order = vec![chat_id.to_owned()];
        order.extend(self.pinned_chat_ids_in_order(cx).into_iter().filter(|id| id != chat_id));
        order
    }

    pub fn apply_pinned_order_local(&mut self, ordered_ids: &[String], cx: &mut Context<Self>) {
        let mut items = self.current_chat_infos(cx);
        let pos_by_id: HashMap<&str, i64> =
            ordered_ids.iter().enumerate().map(|(offset, id)| (id.as_str(), offset as i64)).collect();
        for item in &mut items {
            if let Some(position) = pos_by_id.get(item.id.as_str()) {
                item.is_pinned = true;
                item.pin_position = Some(*position);
            }
        }
        self.set_chats_payload(ChatListUtils::sort(&items), None, None, None, cx);
    }

    pub fn set_chat_pinned_local(&mut self, chat_id: &str, pinned: bool, cx: &mut Context<Self>) {
        let mut items = self.current_chat_infos(cx);
        let Some(item) = items.iter_mut().find(|chat| chat.id == chat_id) else {
            return;
        };
        item.is_pinned = pinned;
        item.pin_position = None;
        self.set_chats_payload(ChatListUtils::sort(&items), None, None, None, cx);
    }

    pub fn restore_chats_snapshot(&mut self, items: Vec<ChatInfo>, cx: &mut Context<Self>) {
        self.set_chats_payload(ChatListUtils::sort(&items), None, None, None, cx);
    }

    pub fn remove_chat_local(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.left_chat_ids.insert(chat_id.to_owned());
        let items: Vec<ChatInfo> = self.current_chat_infos(cx).into_iter().filter(|chat| chat.id != chat_id).collect();

        self.set_chats_payload(items, None, None, None, cx);
    }

    pub fn restore_deleted_chat_local(&mut self, items: Vec<ChatInfo>, chat_id: &str, cx: &mut Context<Self>) {
        self.left_chat_ids.remove(chat_id);
        let alive: Vec<ChatInfo> = items.into_iter().filter(|chat| !self.left_chat_ids.contains(&chat.id)).collect();
        self.set_chats_payload(ChatListUtils::sort(&alive), None, None, None, cx);
    }

    pub fn clear_pending_delete_marks(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.left_chat_ids.remove(chat_id);
        self.confirmed_removed_chat_ids.remove(chat_id);
        self.pending_delete_chat_ids.remove(chat_id);
        cx.notify();
    }

    pub fn restore_chat_after_pending_delete(&mut self, chat: ChatInfo, cx: &mut Context<Self>) {
        self.clear_pending_delete_marks(&chat.id, cx);
        self.upsert_chat(chat, cx);
    }

    pub fn update_chat_local(
        &mut self,
        chat_id: &str,
        title: Option<&str>,
        description: Option<&str>,
        logo_url: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.mutate_chat(
            chat_id,
            |chat| {
                if let Some(title) = title.filter(|title| !title.is_empty()) {
                    chat.title = title.to_owned();
                }
                if let Some(description) = description {
                    chat.description = Some(description.to_owned());
                }
                if let Some(logo_url) = logo_url {
                    chat.logo_url = Some(logo_url.to_owned());
                }
            },
            cx,
        );
    }

    pub fn update_chat_wallpaper_local(
        &mut self,
        chat_id: &str,
        wallpaper: Option<ChatWallpaperPlacement>,
        cx: &mut Context<Self>,
    ) {
        self.mutate_chat(chat_id, |chat| chat.wallpaper = wallpaper, cx);
    }

    pub fn update_topic_wallpaper_local(
        &mut self,
        chat_id: &str,
        topic_id: &str,
        wallpaper: Option<ChatWallpaperPlacement>,
        cx: &mut Context<Self>,
    ) {
        let value = wallpaper
            .as_ref()
            .and_then(|wallpaper| SaiJSONCodec::encode(&ChatWallpaperWireDTO::from_placement(wallpaper)))
            .map_or(Value::Null, Value::Object);
        self.patch_topic(chat_id, topic_id, row_from(vec![(key::WALLPAPER, value)]), cx);
    }

    pub fn wallpaper_catalog(&self, chat_id: &str, topic_id: Option<&str>, cx: &App) -> ChatWallpaperCatalogState {
        if topic_id.is_none()
            && let Some(invented) = GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).wallpaper_catalog(chat_id))
        {
            return invented;
        }
        self.wallpaper_catalogs
            .get(&ChatWallpaperCatalogState::key(chat_id, topic_id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn mutate_wallpaper_catalog(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        change: impl FnOnce(&mut ChatWallpaperCatalogState),
        cx: &mut Context<Self>,
    ) {
        let state = self.wallpaper_catalogs.entry(ChatWallpaperCatalogState::key(chat_id, topic_id)).or_default();
        change(state);
        cx.notify();
    }

    pub fn insert_chat_at_top_if_needed(&mut self, info: ChatInfo, cx: &mut Context<Self>) {
        if self.left_chat_ids.contains(&info.id) {
            return;
        }
        let mut items = self.current_chat_infos(cx);
        if items.iter().any(|chat| chat.id == info.id) {
            return;
        }

        items.insert(0, info);
        self.set_chats_payload(ChatListUtils::sort(&items), None, None, None, cx);
    }

    pub fn is_joined_member_role(role: Option<&str>) -> bool {
        let Some(role) = role.filter(|role| !role.is_empty()) else {
            return false;
        };
        if ChatPerm::is_owner_role(Some(role)) {
            return true;
        }
        matches!(role.to_uppercase().as_str(), "ADMIN" | "MEMBER")
    }

    pub fn is_discussion_guest_card(chat: &ChatInfo) -> bool {
        !chat.is_public && chat.r#type == ChatType::Group && !Self::is_joined_member_role(chat.role.as_deref())
    }

    pub fn refetch_chat_as_member(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.left_chat_ids.remove(chat_id);
        self.confirmed_removed_chat_ids.remove(chat_id);
        cx.notify();
        if self.current_chat_infos(cx).iter().any(|chat| chat.id == chat_id) {
            return;
        }
        if active_presentation(cx).is_some() {
            cx.defer(|cx| ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_chats(cx)));
        }
        if !self.membership_refetch_in_flight.insert(chat_id.to_owned()) {
            return;
        }
        self.fetched_chat_by_id_set.remove(chat_id);
        let chat_id = chat_id.to_owned();
        cx.defer(move |cx| {
            ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_chat_by_id_action(&chat_id, false, cx));
        });
    }

    pub fn preserve_official_bot_after_history_clear(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.left_chat_ids.remove(chat_id);
        self.confirmed_removed_chat_ids.remove(chat_id);
        self.membership_refetch_in_flight.remove(chat_id);
        self.fetched_chat_by_id_set.remove(chat_id);
        cx.notify();
        let did_mutate_cached_row = self.mutate_cached_chat_row(
            chat_id,
            false,
            |row| {
                row.insert(key::LAST_MESSAGE.to_owned(), Value::Null);
                row.insert(key::UNREAD_COUNT.to_owned(), Value::from(0));
            },
            cx,
        );
        if did_mutate_cached_row {
            return;
        }
        self.mutate_chat(
            chat_id,
            |chat| {
                chat.last_message = None;
                chat.unread_count = 0;
                chat.typing_datas = Vec::new();
            },
            cx,
        );
    }

    pub fn adopt_reopened_chat(&mut self, info: ChatInfo, cx: &mut Context<Self>) {
        self.clear_pending_delete_marks(&info.id, cx);
        self.membership_refetch_in_flight.remove(&info.id);
        self.upsert_chat(info, cx);
    }

    pub fn set_opening_hint(&mut self, hint: Option<ChatInfo>, cx: &mut Context<Self>) {
        self.opening_hint_slot = hint;
        cx.notify();
    }

    pub fn opening_hint(&self, chat_id: &str) -> Option<ChatInfo> {
        self.opening_hint_slot.as_ref().filter(|hint| hint.id == chat_id).cloned()
    }

    pub fn promote_community_preview_to_member(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let Some(mut chat) = self.community_preview_by_chat_id.remove(chat_id) else {
            return;
        };
        chat.role = Some("MEMBER".to_owned());
        chat.joined_at = Some(RiseClock::now().time_interval_since_1970());
        self.left_chat_ids.remove(chat_id);
        self.confirmed_removed_chat_ids.remove(chat_id);
        self.upsert_chat(chat, cx);
    }

    pub fn upsert_chat(&mut self, info: ChatInfo, cx: &mut Context<Self>) {
        if self.left_chat_ids.contains(&info.id) {
            return;
        }
        self.community_preview_by_chat_id.remove(&info.id);
        let mut items = self.current_chat_infos(cx);
        if let Some(index) = items.iter().position(|chat| self.canonical_chat_id(&chat.id) == info.id) {
            let mut merged = info;
            merged.typing_datas = std::mem::take(&mut items[index].typing_datas);
            items[index] = merged;
            let deduped = self.deduped_by_id(&items);
            self.set_chats_payload(ChatListUtils::sort(&deduped), None, None, None, cx);
            return;
        }

        items.insert(0, info);
        let deduped = self.deduped_by_id(&items);
        self.set_chats_payload(ChatListUtils::sort(&deduped), None, None, None, cx);
    }

    pub fn finalize_created_chat(&mut self, temp_id: &str, real_id: &str, cx: &mut Context<Self>) {
        if real_id.is_empty() || !GetSaiServerId::is_temp_id(Some(temp_id)) || temp_id == real_id {
            return;
        }

        self.temp_chat_id_to_real.insert(temp_id.to_owned(), real_id.to_owned());
        if self.pending_create_chat_temp_id.as_deref() == Some(temp_id) {
            self.pending_create_chat_temp_id = None;
        }
        let draft_card = self.draft_direct_chat_by_id.get(temp_id).cloned();
        self.remove_draft_direct_chat(temp_id, cx);

        let mut items = self.current_chat_infos(cx);
        if let Some(item) = items.iter_mut().find(|chat| chat.id == temp_id) {
            item.id = real_id.to_owned();
        }
        let deduped = self.deduped_by_id(&items);
        self.set_chats_payload(ChatListUtils::sort(&deduped), None, None, None, cx);

        if let Some(mut card) = draft_card
            && self.chat_info(real_id, cx).is_none()
        {
            card.id = real_id.to_owned();
            self.set_opening_hint(Some(card), cx);
        }
    }

    pub fn deduped_by_id(&self, infos: &[ChatInfo]) -> Vec<ChatInfo> {
        let mut seen = HashSet::with_capacity(infos.len());
        let mut out = Vec::with_capacity(infos.len());
        for info in infos {
            let canonical = self.canonical_chat_id(&info.id);
            if seen.insert(canonical.clone()) {
                let mut row = info.clone();
                row.id = canonical;
                out.push(row);
            }
        }
        out
    }

    pub fn make_create_chat_temp_row(
        body: &Map<String, Value>,
        temp_id: &str,
        local_logo_url: Option<&str>,
        cx: &App,
    ) -> Map<String, Value> {
        let string_field = |name: &str| body.get(name).and_then(Value::as_str);
        let bool_field = |name: &str| body.get(name).and_then(Value::as_bool).unwrap_or(false);
        let title = string_field(key::TITLE).unwrap_or_default();
        let r#type = string_field(key::TYPE).unwrap_or("GROUP");
        let description = string_field(key::DESCRIPTION).unwrap_or_default();
        let is_public = bool_field(key::IS_PUBLIC);
        let logo_url = string_field(key::LOGO_URL).or(local_logo_url).unwrap_or_default();
        let tag = string_field(key::TAG);
        let forward_enabled = bool_field(key::FORWARD_IN_CHAT_ENABLED);
        let profile = try_read::<AuthServicesStore>(cx).and_then(|auth| auth.profile());
        let profile_id = profile.map_or("", |profile| profile.id.as_str());
        let profile_logo = profile
            .and_then(|profile| profile.avatar_url.as_deref().or(Some(profile.more.logo.as_str())))
            .unwrap_or_default();
        let now_ms = RiseClock::now().time_interval_since_1970() * 1000.0;
        let system_text = if r#type == "CHANNEL" {
            format!("Channel '{title}' was created")
        } else {
            format!("Group '{title}' was created")
        };

        let participant = row_from(vec![
            (key::ID, Value::from(profile_id)),
            (key::NAME, Value::from(profile.map_or("", |profile| profile.name.as_str()))),
            (key::TAG, Value::from(profile.map_or("", |profile| profile.tag.as_str()))),
            (key::LOGO, Value::from(profile_logo)),
            (IS_ONLINE, Value::from(true)),
        ]);
        let last_message = row_from(vec![
            (key::ID, Value::from(format!("temp_msg_{}", now_ms as i64))),
            (key::CONTENT, Value::from(system_text)),
            (key::CONTENT_TYPE, Value::from("system")),
            (key::IS_SYSTEM_MESSAGE, Value::from(true)),
            (key::TIMESTAMP, Value::from(now_ms)),
        ]);
        row_from(vec![
            (key::ID, Value::from(temp_id)),
            (IS_TEMP, Value::from(true)),
            (key::TITLE, Value::from(title)),
            (key::TYPE, Value::from(r#type)),
            (key::DESCRIPTION, Value::from(description)),
            (key::LOGO_URL, Value::from(logo_url)),
            (key::BANNER_URL, Value::from("")),
            (key::IS_PUBLIC, Value::from(is_public)),
            (key::TAG, Value::from(tag)),
            (key::USERNAME, Value::from(tag)),
            (key::MEMBER_COUNT, Value::from(1)),
            (key::UNREAD_COUNT, Value::from(0)),
            (key::IS_PINNED, Value::from(false)),
            (key::IS_MUTED, Value::from(false)),
            (key::IS_VERIFIED, Value::from(false)),
            (key::ROLE, Value::from("OWNER")),
            (key::FORWARD_IN_CHAT_ENABLED, Value::from(forward_enabled)),
            (key::CREATED_AT, Value::from(now_ms)),
            (key::UPDATED_AT, Value::from(now_ms)),
            (key::LAST_ACTIVITY, Value::from(now_ms)),
            (key::CREATOR_INFO, Value::from(profile_id)),
            (key::PARTICIPANT, Value::Object(participant)),
            (key::LAST_MESSAGE, Value::Object(last_message)),
        ])
    }

    pub fn extract_real_chat_row(any: &Value) -> Option<Map<String, Value>> {
        let root = AnyDictionaryBridge::dictionary(Some(any))?;

        let mut row = AnyDictionaryBridge::dictionary(root.get(rise_key::events::DATA)).unwrap_or(root);
        let chat_id = ws_string(&row, &[rise_key::message::CHAT_ID, key::ID])?;

        row.insert(key::ID.to_owned(), Value::from(chat_id));
        row.insert(IS_TEMP.to_owned(), Value::from(false));
        Some(row)
    }

    pub fn make_logo_optimistic_files(
        local_file: SaiWsOptimisticLocalFile,
        on_progress: Rc<dyn Fn(f64, &mut App)>,
        on_error: Rc<dyn Fn(&anyhow::Error, &mut App)>,
        reject: Rc<dyn Fn(&anyhow::Error, &mut App)>,
    ) -> SaiWsOptimisticFiles {
        let mut files = SaiWsOptimisticFiles::new(RiseUploadContextFormats::CHAT_AVATAR.context.to_owned());
        files.compression = MediaCompressionConfiguration::avatar();
        files.local_files = vec![local_file];
        files.merge_uploaded_slots_into_body_root = true;
        files.extract_uploaded_files = Some(Arc::new(
            |done: &Map<String, Value>,
             _: usize,
             _: &SaiWsOptimisticLocalFile,
             _: &mut App|
             -> anyhow::Result<Value> {
                let url = MusicCoverUpload::extract_remote_url(done)
                    .ok_or(SaiWsOptimisticError::FileUploadInvalidSlotPayload)?;
                Ok(Value::Object(row_from(vec![(key::LOGO_URL, Value::from(url))])))
            },
        ));
        let progress = on_progress.clone();
        files.on_progress = Some(Arc::new(move |_: usize, fraction: f64, cx: &mut App| {
            progress((fraction * 100.0).clamp(0.0, 100.0), cx);
        }));
        files.on_file_success = Some(Arc::new(move |_: usize, _: &Map<String, Value>, cx: &mut App| {
            on_progress(100.0, cx);
        }));
        files.on_file_error = Some(Arc::new(move |_: usize, error: &anyhow::Error, cx: &mut App| {
            reject(error, cx);
            on_error(error, cx);
        }));
        files
    }

    pub fn chats_cache_key(&self, cx: &App) -> Option<String> {
        let user_id = Self::effective_or_active_user_id(cx)?;
        if user_id.is_empty() {
            return None;
        }

        Some(Self::chats_cache_key_for_user(&user_id))
    }

    pub fn preload_chats_slice_from_disk_if_needed(&self, cx: &mut App) {
        let Some(cache_key) = self.chats_cache_key(cx) else {
            return;
        };

        SaiUpdater::hydrate_slice_from_disk(&self.get_chats_sai, &cache_key, None, cx);
    }

    pub fn chat_list_rows(&self, cx: &App) -> Vec<ChatInfo> {
        let chats = self.current_chat_infos(cx);
        if self.undeliverable_draft_direct_chat_ids.is_empty() {
            return chats;
        }

        let drafts: Vec<ChatInfo> = self
            .undeliverable_draft_direct_chat_ids
            .iter()
            .filter_map(|id| self.draft_direct_chat_by_id.get(id))
            .filter(|draft| {
                self.real_chat_id(&draft.id, cx).is_none()
                    && !chats
                        .iter()
                        .any(|chat| chat.r#type == ChatType::Direct && chat.participant.id == draft.participant.id)
            })
            .cloned()
            .collect();
        if drafts.is_empty() {
            return chats;
        }

        let mut merged = chats;
        merged.extend(drafts);
        ChatListUtils::sort(&merged)
    }

    pub fn current_chat_infos(&self, cx: &App) -> Vec<ChatInfo> {
        if let Some(presentation) = active_presentation(cx) {
            let chats = presentation.chats(RiseChatListKind::Inbox);
            if self.engine_contact_join_chats.is_empty() {
                return chats.to_vec();
            }
            let listed: HashSet<&str> = chats.iter().map(|chat| chat.id.as_str()).collect();
            let mut merged = chats.to_vec();
            merged.extend(
                self.engine_contact_join_chats.iter().filter(|chat| !listed.contains(chat.id.as_str())).cloned(),
            );
            return ChatListUtils::sort(&merged);
        }
        if let Some(cache_key) = self.chats_cache_key(cx)
            && let Some(projected) = self.projected_chat_infos(&cache_key, cx)
        {
            return projected;
        }

        self.get_chats_sai
            .data()
            .and_then(|page| page.chats)
            .unwrap_or_default()
            .iter()
            .map(|chat| chat.to_chat_info())
            .collect()
    }

    fn projected_chat_infos(&self, cache_key: &str, cx: &App) -> Option<Vec<ChatInfo>> {
        let revision = SaiCacheRevisionTracker::observed_revision_in(cache_key, cx);
        if let Some(memo) = self.chats_inbox_infos_memo.borrow().as_ref()
            && memo.revision == revision
            && memo.cache_key == cache_key
        {
            return memo.infos.clone();
        }

        let infos = SaiWsResponseCache::observed_memory_only_read(cache_key, cx)
            .or_else(|| SaiWsDiskCache::load(cache_key))
            .and_then(|payload| SaiJSONCodec::decode::<RiseStoredChatPage>(Some(&payload)))
            .map(|page| page.chats.unwrap_or_default().iter().map(|chat| chat.to_chat_info()).collect::<Vec<_>>());
        *self.chats_inbox_infos_memo.borrow_mut() =
            Some(ChatsInboxInfosMemo { cache_key: cache_key.to_owned(), revision, infos: infos.clone() });
        infos
    }

    pub fn chats_has_more(&self, cx: &App) -> bool {
        if let Some(presentation) = active_presentation(cx) {
            return presentation.chat_state(RiseChatListKind::Inbox).is_some_and(|state| state.has_more);
        }
        self.get_chats_sai.data().and_then(|page| page.is_have_more).unwrap_or(false)
    }

    pub fn chats_relative_id(&self, cx: &App) -> Option<String> {
        if let Some(presentation) = active_presentation(cx) {
            return presentation.chat_state(RiseChatListKind::Inbox).and_then(|state| state.cursor.clone());
        }
        self.get_chats_sai.data().and_then(|page| page.relative_id)
    }

    pub fn favourites_chat_id(&self, cx: &App) -> Option<String> {
        try_read::<RiseConversationPresentation>(cx)
            .and_then(|presentation| presentation.favourites_chat_id().map(str::to_owned))
            .or_else(|| {
                self.current_chat_infos(cx).into_iter().find(|chat| chat.r#type == ChatType::Favourites).map(|chat| chat.id)
            })
    }

    pub fn page_payload_dict(&self, payload: &Value) -> Option<Map<String, Value>> {
        AnyDictionaryBridge::dictionary(Some(payload))
    }

    pub fn mutate_chat(&mut self, chat_id: &str, transform: impl FnOnce(&mut ChatInfo), cx: &mut Context<Self>) {
        if active_presentation(cx).is_some() && !is_contact_join_virtual_chat_id(chat_id) {
            return;
        }
        let mut items = self.current_chat_infos(cx);
        let Some(item) = items.iter_mut().find(|chat| chat.id == chat_id) else {
            return;
        };

        transform(item);
        self.set_chats_payload(items, None, None, None, cx);
    }

    pub fn mutate_cached_chat_row(
        &mut self,
        chat_id: &str,
        reorder: bool,
        transform: impl FnOnce(&mut Map<String, Value>),
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(cache_key) = self.chats_cache_key(cx) else {
            return false;
        };
        let Some(payload) = SaiWsResponseCache::observed_memory_only(&cache_key, cx) else {
            return false;
        };
        let Some(root) = AnyDictionaryBridge::dictionary(Some(&payload)) else {
            return false;
        };
        let row_matches = |row: &Map<String, Value>| row.get(key::ID).and_then(Value::as_str) == Some(chat_id);
        if !MessagePayloadCodec::array_of_dictionaries(root.get(key::CHATS)).iter().any(row_matches) {
            return false;
        }

        let mut did_mutate = false;
        let result = SaiUpdater::update_json_value(
            &cache_key,
            |context| {
                let mut updated_root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                let mut rows = MessagePayloadCodec::array_of_dictionaries(updated_root.get(key::CHATS));
                let Some(index) = rows.iter().position(row_matches) else {
                    return Ok(Value::Object(updated_root));
                };

                let mut row = rows.remove(index);
                transform(&mut row);
                did_mutate = true;
                let insertion_index = if reorder { Self::chat_insertion_index(&row, &rows) } else { index };
                rows.insert(insertion_index, row);
                updated_root
                    .insert(key::CHATS.to_owned(), Value::Array(rows.into_iter().map(Value::Object).collect()));
                Ok(Value::Object(updated_root))
            },
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        if result.is_err() {
            return false;
        }

        if did_mutate {
            self.mark_chats_cache_for_disk_persistence(cx);
            cx.notify();
        }
        did_mutate
    }

    pub fn set_chats_payload(
        &mut self,
        items: Vec<ChatInfo>,
        relative_id: Option<String>,
        limit: Option<i64>,
        is_have_more: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if active_presentation(cx).is_some() {
            let joined: Vec<ChatInfo> = items
                .into_iter()
                .filter(|chat| is_contact_join_virtual_chat_id(&chat.id))
                .take(UserContactLimits::maximum_join_virtual_dialogs().max(0) as usize)
                .collect();
            if joined != self.engine_contact_join_chats {
                self.engine_contact_join_chats = joined;
                cx.notify();
            }
            return;
        }
        let prev = self.get_chats_sai.data();
        let existing_by_id: HashMap<&str, _> = prev
            .iter()
            .flat_map(|page| page.chats.iter().flatten())
            .map(|chat| (chat.id.as_str(), chat))
            .collect();
        let wires = items.iter().map(|item| item.to_wire_dto(existing_by_id.get(item.id.as_str()).copied())).collect();
        let dto = RiseStoredChatPage::new(
            Some(wires),
            relative_id.or_else(|| prev.as_ref().and_then(|page| page.relative_id.clone())),
            Some(limit.or_else(|| prev.as_ref().and_then(|page| page.limit)).unwrap_or_else(Self::chats_limit)),
            Some(is_have_more.or_else(|| prev.as_ref().and_then(|page| page.is_have_more)).unwrap_or(false)),
            Some(true),
            None,
        );

        let Some(cache_key) = self.chats_cache_key(cx) else {
            return;
        };
        let Some(root) = SaiJSONCodec::encode(&dto) else {
            return;
        };

        let slice: Rc<dyn SaiSliceState> = self.get_chats_sai.clone();
        SaiWsResponseCache::note_active_slice(Some(slice), Some(cache_key.as_str()), None, cx);
        let _ = SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::Cache, cx);
        self.mark_chats_cache_for_disk_persistence(cx);
        cx.notify();
        cx.defer(|cx| {
            if let Some(hooks) = chat_stores_user_hooks() {
                (hooks.hydrate_visible_chat_participants_for_contact_folders)(cx);
            }
        });
    }

    pub fn flush_chats_disk_persistence(&mut self, cx: &mut Context<Self>) {
        self.chats_disk_persistence_task = None;
        if self.chats_disk_persistence_in_flight_generation.is_some() {
            return;
        }
        if self.chats_disk_persistence_completed_generation == self.chats_disk_persistence_generation {
            self.chats_disk_persistence_quit = None;
            return;
        }
        let Some(cache_key) = self.chats_cache_key(cx) else {
            self.chats_disk_persistence_quit = None;
            return;
        };
        let Some(payload) = SaiWsResponseCache::observed_memory_only(&cache_key, cx) else {
            self.chats_disk_persistence_quit = None;
            return;
        };

        let generation = self.chats_disk_persistence_generation;
        let token = self.chats_disk_persistence_token;
        self.chats_disk_persistence_in_flight_generation = Some(generation);
        let store = cx.weak_entity();
        SaiWsDiskCache::save_asynchronously(
            cache_key,
            payload,
            move |result, cx| {
                let _ = store.update(cx, |store, cx| {
                    if store.chats_disk_persistence_token != token
                        || store.chats_disk_persistence_in_flight_generation != Some(generation)
                    {
                        return;
                    }

                    store.chats_disk_persistence_in_flight_generation = None;
                    if result.is_ok() {
                        store.chats_disk_persistence_completed_generation = generation;
                    }
                    if result.is_err()
                        || store.chats_disk_persistence_completed_generation == store.chats_disk_persistence_generation
                    {
                        store.chats_disk_persistence_quit = None;
                    }
                    if store.chats_disk_persistence_completed_generation != store.chats_disk_persistence_generation {
                        let delay = if result.is_ok() {
                            CHATS_DISK_PERSISTENCE_RETRY_AFTER_SUCCESS
                        } else {
                            CHATS_DISK_PERSISTENCE_RETRY_AFTER_FAILURE
                        };
                        store.schedule_chats_disk_persistence(delay, cx);
                    }
                });
            },
            cx,
        );
    }

    pub fn mark_chats_cache_for_disk_persistence(&mut self, cx: &mut Context<Self>) {
        self.chats_disk_persistence_generation = self.chats_disk_persistence_generation.wrapping_add(1);
        if self.chats_disk_persistence_quit.is_none() {
            self.chats_disk_persistence_quit = Some(QuitDrain::lease(CHATS_DISK_PERSISTENCE_ACTIVITY, cx));
        }
        self.schedule_chats_disk_persistence(CHATS_DISK_PERSISTENCE_DEBOUNCE, cx);
    }

    fn schedule_chats_disk_persistence(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let timer = cx.background_executor().timer(delay);
        self.chats_disk_persistence_task = Some(cx.spawn(async move |store, cx| {
            timer.await;
            let _ = store.update(cx, |store, cx| store.flush_chats_disk_persistence(cx));
        }));
    }

    fn chat_insertion_index(row: &Map<String, Value>, rows: &[Map<String, Value>]) -> usize {
        let mut lower = 0;
        let mut upper = rows.len();
        while lower < upper {
            let middle = lower + (upper - lower) / 2;
            if Self::chat_row_precedes(row, &rows[middle]) {
                upper = middle;
            } else {
                lower = middle + 1;
            }
        }
        lower
    }

    fn chat_row_precedes(lhs: &Map<String, Value>, rhs: &Map<String, Value>) -> bool {
        let pinned = |row: &Map<String, Value>| row.get(key::IS_PINNED).and_then(Value::as_bool).unwrap_or(false);
        let lhs_pinned = pinned(lhs);
        let rhs_pinned = pinned(rhs);
        if lhs_pinned != rhs_pinned {
            return lhs_pinned;
        }
        if lhs_pinned {
            let position = |row: &Map<String, Value>| {
                row.get(key::PIN_POSITION)
                    .and_then(|value| value.as_i64().or_else(|| value.as_f64().map(|number| number as i64)))
                    .unwrap_or(i64::MAX)
            };
            let lhs_position = position(lhs);
            let rhs_position = position(rhs);
            if lhs_position != rhs_position {
                return lhs_position < rhs_position;
            }
        }

        let lhs_activity = Self::chat_row_activity(lhs);
        let rhs_activity = Self::chat_row_activity(rhs);
        if lhs_activity != rhs_activity {
            return lhs_activity > rhs_activity;
        }
        let id = |row: &Map<String, Value>| -> String {
            row.get(key::ID).and_then(Value::as_str).unwrap_or_default().to_owned()
        };
        id(lhs) < id(rhs)
    }

    fn chat_row_activity(row: &Map<String, Value>) -> f64 {
        let last_message_activity = AnyDictionaryBridge::dictionary(row.get(key::LAST_MESSAGE))
            .and_then(|last_message| MessageThreadOrder::timestamp_seconds(&last_message))
            .unwrap_or(0.0);
        let mut updated = Map::new();
        if let Some(updated_at) = row.get(key::UPDATED_AT) {
            updated.insert(key::UPDATED_AT.to_owned(), updated_at.clone());
        }
        let updated_activity = MessageThreadOrder::timestamp_seconds(&updated).unwrap_or(0.0);
        last_message_activity.max(updated_activity)
    }
}
