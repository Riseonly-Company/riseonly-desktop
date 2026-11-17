use std::collections::HashSet;

use gpui::{App, Task};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

pub struct ChatStoresUserHooks {
    pub restore_contact_join_dialogs_from_inbox: fn(&mut App),
    pub note_peer_refuses_viewer_messages: fn(&str, &mut App),
    pub is_contact_join_virtual_chat_id: fn(&str) -> bool,
    pub saved_contacts_list_cache_key: fn(&str) -> String,
    pub chat_mute_override_ids: fn(&App) -> HashSet<String>,
    pub mute_all_chats_enabled: fn(&App) -> bool,
    pub add_chat_mute_exception: fn(&str, &mut App),
    pub remove_chat_mute_exception: fn(&str, &mut App),
    pub hydrate_visible_chat_participants_for_contact_folders: fn(&mut App),
    pub load_saved_contacts_for_chat_folders: fn(&mut App),
}

pub struct ChatStoresPushHooks {
    pub remove_delivered_notifications: fn(&str, Option<&str>, &mut App),
}

pub struct ChatStoresShareHooks {
    pub donate_incoming_interaction: fn(&str, &str, &mut App),
    pub donate_outgoing_interaction: fn(&str, &mut App),
}

pub struct ChatStoresRtcHooks {
    pub active_room_config: fn(&mut App) -> Option<(String, Option<String>)>,
    pub group_voice_settings_load: Box<dyn Fn(String, &mut App) -> Task<bool> + Send + Sync>,
}

pub struct ChatStoresComponentsHooks {
    pub can_set: fn(&ChatInfo, &App) -> bool,
    pub can_add: fn(&str, &GetMessageMessage, &ChatInfo, &mut App) -> bool,
    pub plain_custom_emoji: fn(&str) -> String,
}

pub(crate) static CHAT_STORES_USER_HOOKS: FeatureHookSlot<ChatStoresUserHooks> = FeatureHookSlot::new("CHAT_STORES_USER_HOOKS");
pub(crate) static CHAT_STORES_PUSH_HOOKS: FeatureHookSlot<ChatStoresPushHooks> =
    FeatureHookSlot::new("CHAT_STORES_PUSH_HOOKS");
pub(crate) static CHAT_STORES_SHARE_HOOKS: FeatureHookSlot<ChatStoresShareHooks> =
    FeatureHookSlot::new("CHAT_STORES_SHARE_HOOKS");
pub(crate) static CHAT_STORES_RTC_HOOKS: FeatureHookSlot<ChatStoresRtcHooks> =
    FeatureHookSlot::new("CHAT_STORES_RTC_HOOKS");
pub(crate) static CHAT_STORES_COMPONENTS_HOOKS: FeatureHookSlot<ChatStoresComponentsHooks> =
    FeatureHookSlot::new("CHAT_STORES_COMPONENTS_HOOKS");

pub fn set_chat_stores_user_hooks(hooks: ChatStoresUserHooks) {
    let _ = CHAT_STORES_USER_HOOKS.set(hooks);
}

pub fn chat_stores_user_hooks() -> Option<&'static ChatStoresUserHooks> {
    CHAT_STORES_USER_HOOKS.get()
}

pub fn set_chat_stores_push_hooks(hooks: ChatStoresPushHooks) {
    let _ = CHAT_STORES_PUSH_HOOKS.set(hooks);
}

pub fn chat_stores_push_hooks() -> Option<&'static ChatStoresPushHooks> {
    CHAT_STORES_PUSH_HOOKS.get()
}

pub fn set_chat_stores_share_hooks(hooks: ChatStoresShareHooks) {
    let _ = CHAT_STORES_SHARE_HOOKS.set(hooks);
}

pub fn chat_stores_share_hooks() -> Option<&'static ChatStoresShareHooks> {
    CHAT_STORES_SHARE_HOOKS.get()
}

pub fn set_chat_stores_rtc_hooks(hooks: ChatStoresRtcHooks) {
    let _ = CHAT_STORES_RTC_HOOKS.set(hooks);
}

pub fn chat_stores_rtc_hooks() -> Option<&'static ChatStoresRtcHooks> {
    CHAT_STORES_RTC_HOOKS.get()
}

pub fn set_chat_stores_components_hooks(hooks: ChatStoresComponentsHooks) {
    let _ = CHAT_STORES_COMPONENTS_HOOKS.set(hooks);
}

pub fn chat_stores_components_hooks() -> Option<&'static ChatStoresComponentsHooks> {
    CHAT_STORES_COMPONENTS_HOOKS.get()
}
