use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Task};
use rise_core::foundation::foundation_date::TimeInterval;
use serde_json::{Map, Value};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileFavEntry, ChatProfileLinkEntry, ChatProfileMediaEntry,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_search_session::{
    ChatSearchChatKindFilter, RiseMessageSearchPresentationState,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatMember, ChatSettingsModel};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::EffectivePermissionsDTO;
use crate::modules::chat::stores::message::message_actions::message_types::{ChatTypingPeer, GetMessageMessage};

/// main thread continuation given to a store action, shared and not consumed
pub type ChatEngineMainCallback = Rc<dyn Fn(&mut App)>;

/// fields of the removed chat name, the type stays with the interactions store
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatEngineRemovedChatName {
    pub title: String,
    pub is_channel: bool,
    pub announces: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChatEngineWarmCacheLimits {
    pub maximum_entry_count: i64,
    pub maximum_estimated_cost: i64,
    pub time_to_live: TimeInterval,
}

pub struct ChatEngineHooks {
    pub get_chat_by_id_action: fn(chat_id: &str, force: bool, cx: &mut App),
    pub get_chat_settings_action: fn(chat_id: &str, cx: &mut App),
    pub get_forum_structure_action: fn(
        chat_id: &str,
        force: bool,
        on_loaded: Option<ChatEngineMainCallback>,
        on_failed: Option<ChatEngineMainCallback>,
        cx: &mut App,
    ),
    pub get_topics_action: fn(chat_id: &str, force_refresh: bool, cx: &mut App),
    pub load_forum_voice_roster_action: fn(chat_id: &str, topic_id: &str, cx: &mut App),

    pub announce_removed_from_chat: fn(chat_id: &str, stored: Option<ChatEngineRemovedChatName>, cx: &mut App),
    pub load_effective_permissions: fn(chat_id: &str, topic_id: Option<&str>, force: bool, cx: &mut App),

    pub adopt_known_chats_for_draft_direct_chats: fn(cx: &mut App),
    pub chat_info: fn(chat_id: &str, cx: &mut App) -> Option<ChatInfo>,
    pub known_direct_chat_id: fn(peer_user_id: &str, cx: &mut App) -> String,
    pub member: fn(scope: &str, user_id: &str, cx: &mut App) -> Option<ChatMember>,
    pub apply_member_fields_local: fn(chat_id: &str, user_id: &str, fields: Map<String, Value>, cx: &mut App),
    pub refresh_loaded_member_lists: fn(chat_id: &str, cx: &mut App),
    pub remove_member_local: fn(chat_id: &str, user_id: &str, reason: Option<&str>, add_to_blocked: bool, cx: &mut App),
    pub effective_permissions: fn(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> Option<EffectivePermissionsDTO>,
    pub invalidate_effective_permissions: fn(chat_id: &str, cx: &mut App),
    pub invalidate_overwrite_caches: fn(chat_id: &str, cx: &mut App),
    pub preserve_official_bot_after_history_clear: fn(chat_id: &str, cx: &mut App),
    pub record_own_server_send: fn(chat_id: &str, sent_at: TimeInterval, cx: &mut App),
    pub settings_by_chat: fn(chat_id: &str, cx: &mut App) -> Option<ChatSettingsModel>,
    pub live_voice_change: fn(payload: &Map<String, Value>) -> Option<bool>,
    pub apply_topic_deleted_event: fn(any: Option<Value>, cx: &mut App) -> i64,
    pub apply_topic_message_created: fn(
        chat_id: &str,
        topic_id: &str,
        last_message: Map<String, Value>,
        updated_at: TimeInterval,
        increment_unread: bool,
        cx: &mut App,
    ) -> bool,
    pub apply_topic_message_deleted: fn(chat_id: &str, topic_id: Option<&str>, message_id: &str, cx: &mut App) -> bool,
    pub apply_topic_message_edited:
        fn(chat_id: &str, topic_id: Option<&str>, message_id: &str, last_message: Map<String, Value>, cx: &mut App),
    pub apply_topic_read_local: fn(chat_id: &str, topic_id: &str, cx: &mut App),
    pub apply_topic_receipt: fn(chat_id: &str, server_ids: &[String], cx: &mut App),
    pub apply_topic_message_send_failed:
        fn(chat_id: &str, local_id: &str, failed_row: Option<(String, Map<String, Value>, f64)>, cx: &mut App),
    pub apply_topic_unread_marks_delta:
        fn(chat_id: &str, topic_id: &str, mentions: i64, replies: i64, reactions: i64, cx: &mut App) -> bool,
    pub forum_structure_cache_key: fn(chat_id: &str) -> String,
    pub member_scope_key: fn(chat_id: &str, topic_id: Option<&str>) -> String,
    pub members_cache_key: fn(scope: &str) -> String,
    pub topics_cache_key: fn(chat_id: &str) -> String,
    pub is_joined_member_role: fn(role: Option<&str>) -> bool,
    pub effective_perms_cache_key: fn(chat_id: &str, topic_id: Option<&str>) -> String,

    pub apply_thread_total: fn(discussion_message_id: &str, count: i64, cx: &mut App),
    pub clear_local_history: fn(chat_id: &str, scopes: HashSet<RiseMessageScope>, cx: &mut App),
    pub record_confirmed_message_deletions: fn(chat_id: &str, message_ids: Vec<String>, cx: &mut App),
    pub typing_peers_by_chat: fn(cx: &mut App) -> HashMap<String, Vec<ChatTypingPeer>>,
    pub set_typing_peers_by_chat: fn(key: &str, value: Option<Vec<ChatTypingPeer>>, cx: &mut App),
    pub remove_all_typing_peers_by_chat: fn(cx: &mut App),
    pub chat_overlay_message_search_state:
        fn(query: &str, chat_kind: ChatSearchChatKindFilter, cx: &mut App) -> RiseMessageSearchPresentationState,
    pub message_scope: fn(chat_id: &str, topic_id: Option<&str>, thread_root_id: Option<&str>) -> String,
    pub audio_tab_entries: fn(chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry>,
    pub fav_tab_entries: fn(chat_id: &str, cx: &mut App) -> Vec<ChatProfileFavEntry>,
    pub file_tab_entries: fn(chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry>,
    pub gif_tab_entries: fn(chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry>,
    pub link_tab_entries: fn(chat_id: &str, cx: &mut App) -> Vec<ChatProfileLinkEntry>,
    pub media_tab_entries: fn(chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry>,
    pub music_tab_entries: fn(chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry>,
    pub message: fn(message_id: &str, chat_id: &str, cx: &mut App) -> Option<GetMessageMessage>,
    pub messages: fn(chat_id: &str, cx: &mut App) -> Vec<GetMessageMessage>,
    pub warm_cache_limits_production: fn() -> ChatEngineWarmCacheLimits,
    pub estimated_resident_cost: fn(message_count: i64) -> i64,

    pub can_repark_draft_send: fn(message: &GetMessageMessage, chat_id: &str, cx: &mut App) -> bool,
    pub is_parked_draft_send: fn(local_id: &str, chat_id: &str, cx: &mut App) -> bool,
}

pub(crate) static CHAT_ENGINE_HOOKS: FeatureHookSlot<ChatEngineHooks> = FeatureHookSlot::new("CHAT_ENGINE_HOOKS");

pub(crate) fn set_chat_engine_hooks(hooks: ChatEngineHooks) {
    let _ = CHAT_ENGINE_HOOKS.set(hooks);
}

pub(crate) fn chat_engine_hooks() -> Option<&'static ChatEngineHooks> {
    CHAT_ENGINE_HOOKS.get()
}

/// the two fields of the active room config that chat reads
pub struct ChatEngineRtcHooks {
    pub active_room_config: fn(&mut App) -> Option<(String, Option<String>)>,
    pub group_voice_settings_load: fn(chat_id: &str, cx: &mut App) -> Task<bool>,
}

pub(crate) static CHAT_ENGINE_RTC_HOOKS: FeatureHookSlot<ChatEngineRtcHooks> = FeatureHookSlot::new("CHAT_ENGINE_RTC_HOOKS");

pub fn set_chat_engine_rtc_hooks(hooks: ChatEngineRtcHooks) {
    let _ = CHAT_ENGINE_RTC_HOOKS.set(hooks);
}

pub(crate) fn chat_engine_rtc_hooks() -> Option<&'static ChatEngineRtcHooks> {
    CHAT_ENGINE_RTC_HOOKS.get()
}

/// app icon badge sync hook
pub struct ChatEngineBadgeHooks {
    pub sync: fn(force: bool, cx: &mut App),
}

pub(crate) static CHAT_ENGINE_BADGE_HOOKS: FeatureHookSlot<ChatEngineBadgeHooks> = FeatureHookSlot::new("CHAT_ENGINE_BADGE_HOOKS");

pub fn set_chat_engine_badge_hooks(hooks: ChatEngineBadgeHooks) {
    let _ = CHAT_ENGINE_BADGE_HOOKS.set(hooks);
}

pub(crate) fn chat_engine_badge_hooks() -> Option<&'static ChatEngineBadgeHooks> {
    CHAT_ENGINE_BADGE_HOOKS.get()
}

/// hook that tells if a bubble is own
pub struct ChatEngineComponentsHooks {
    pub is_own_bubble: fn(message: &GetMessageMessage, chat: &ChatInfo, cx: &mut App) -> bool,
}

pub(crate) static CHAT_ENGINE_COMPONENTS_HOOKS: FeatureHookSlot<ChatEngineComponentsHooks> = FeatureHookSlot::new("CHAT_ENGINE_COMPONENTS_HOOKS");

pub fn set_chat_engine_components_hooks(hooks: ChatEngineComponentsHooks) {
    let _ = CHAT_ENGINE_COMPONENTS_HOOKS.set(hooks);
}

pub(crate) fn chat_engine_components_hooks() -> Option<&'static ChatEngineComponentsHooks> {
    CHAT_ENGINE_COMPONENTS_HOOKS.get()
}
