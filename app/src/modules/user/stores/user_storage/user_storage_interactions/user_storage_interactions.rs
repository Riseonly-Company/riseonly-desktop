use std::collections::{HashMap, HashSet};

use gpui::{App, Context, Entity};

use crate::core::storage::rise_storage_usage_models::{RiseStorageMediaEntry, RiseStorageSite};
use crate::modules::chat::components::chat_item::chat_list_preview_row::ChatListPreviewFactory;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::stores::user_storage::user_storage_actions::user_storage_actions::UserStorageActionsStore;
use crate::modules::user::stores::user_storage::user_storage_services::user_storage_services::UserStorageServicesStore;

pub struct UserStorageInteractionsStore {}

impl UserStorageInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn actions(cx: &mut App) -> Entity<UserStorageActionsStore> {
        UserStorageActionsStore::shared(cx)
    }

    fn services(cx: &mut App) -> Entity<UserStorageServicesStore> {
        UserStorageServicesStore::shared(cx)
    }

    pub fn open_storage_settings(&mut self, cx: &mut Context<Self>) {
        let titles = Self::chat_titles(cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.scan_action(true, cx);
            actions.scan_chats_action(titles, cx);
        });
    }

    pub fn reload_storage_usage(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.scan_action(true, cx));
    }

    pub fn open_cached_chats(&mut self, cx: &mut Context<Self>) {
        let titles = Self::chat_titles(cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.scan_action(true, cx);
            actions.scan_chats_action(titles, cx);
        });
    }

    pub fn open_cached_media(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.scan_action(true, cx);
            actions.scan_media_action(cx);
        });
    }

    pub fn open_other_data(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.scan_action(true, cx));
    }

    pub fn clear_everything(&mut self, cx: &mut Context<Self>) {
        let (has_clearable, is_clearing) = Self::services(cx)
            .update(cx, |services, cx| (services.has_clearable_bytes(cx), services.is_clearing(cx)));
        if !has_clearable || is_clearing {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.clear_all_action(cx));
    }

    pub fn clear_all_media(&mut self, cx: &mut Context<Self>) {
        if Self::is_clearing(cx) {
            return;
        }
        let sites: HashSet<RiseStorageSite> =
            [RiseStorageSite::Images, RiseStorageSite::RemoteMedia, RiseStorageSite::HttpResponses].into();
        Self::actions(cx).update(cx, |actions, cx| actions.clear_sites_action(sites, cx));
    }

    pub fn clear_site(&mut self, site: RiseStorageSite, cx: &mut Context<Self>) {
        if !site.is_clearable() || Self::is_clearing(cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.clear_sites_action(HashSet::from([site]), cx));
    }

    pub fn clear_chat(&mut self, chat_id: String, cx: &mut Context<Self>) {
        if Self::is_clearing(cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.clear_chat_action(chat_id, cx));
    }

    pub fn clear_media_entry(&mut self, entry: RiseStorageMediaEntry, cx: &mut Context<Self>) {
        if Self::is_clearing(cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.clear_media_entry_action(entry, cx));
    }

    fn is_clearing(cx: &mut App) -> bool {
        Self::services(cx).update(cx, |services, cx| services.is_clearing(cx))
    }

    fn chat_titles(cx: &mut App) -> HashMap<String, String> {
        let store = ChatServicesStore::shared(cx);
        store.update(cx, |store, cx| store.rebuild_chat_info_index_if_needed(cx));
        let index = store.read(cx).chat_info_index.borrow().clone();
        let drafts: Vec<ChatInfo> = store.read(cx).draft_direct_chat_by_id.values().cloned().collect();
        let mut titles: HashMap<String, String> = HashMap::new();
        for (chat_id, chat) in &index {
            let title = ChatListPreviewFactory::chat_title(chat, cx);
            if chat.r#type == ChatType::Direct && !ChatServicesStore::is_draft_direct_chat_id(chat_id) {
                let draft_id = ChatServicesStore::draft_direct_chat_id(&chat.participant.id);
                if !draft_id.is_empty() {
                    titles.entry(draft_id).or_insert_with(|| title.clone());
                }
            }
            titles.insert(chat_id.clone(), title);
        }
        for draft in &drafts {
            let title = ChatListPreviewFactory::chat_title(draft, cx);
            titles.insert(draft.id.clone(), title);
        }
        titles
    }
}
