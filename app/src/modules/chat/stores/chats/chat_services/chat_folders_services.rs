use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::chat as key;
use rise_canon::rise_response::user::ListSavedContacts;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_user_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::{ChatActionsStore, effective_user_id_for_wire};
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::{
    ChatFolder, ChatFolderWireDTOExt, GetChatFoldersGatewayDTO,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChatPage;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

impl ChatServicesStore {
    pub fn folders_cache_key(user_id: &str) -> String {
        SaiCache::new(["getChatFolders", user_id]).key
    }

    pub fn folder_chat_search_cache_key(user_id: &str, query: &str) -> String {
        let lowered = query.to_lowercase();
        SaiCache::new(["folderChatSearch", user_id, lowered.as_str()]).key
    }

    pub fn folder_contacts_cache_key(user_id: &str) -> String {
        chat_stores_user_hooks().map(|hooks| (hooks.saved_contacts_list_cache_key)(user_id)).unwrap_or_default()
    }

    pub fn folder_chat_search_page(&self, query: &str, user_id: &str, cx: &mut App) -> Option<Rc<RiseStoredChatPage>> {
        let key = Self::folder_chat_search_cache_key(user_id, query);
        self.folder_chat_search_page_for_cache_key(&key, cx)
    }

    fn folder_chat_search_page_for_cache_key(&self, cache_key: &str, cx: &mut App) -> Option<Rc<RiseStoredChatPage>> {
        Self::observed_gateway_dto::<RiseStoredChatPage>(cache_key, cx)
    }

    pub fn folder_chat_search_results(&self, query: &str, user_id: &str, cx: &mut App) -> Vec<ChatInfo> {
        self.folder_chat_search_page(query, user_id, cx)
            .map(|page| page.chats.iter().flatten().map(|chat| chat.to_chat_info()).collect())
            .unwrap_or_default()
    }

    pub fn cached_folder_chat_search_results(&self, cx: &mut App) -> Vec<ChatInfo> {
        let order = ChatActionsStore::shared(cx).read(cx).folder_chat_search_cache_order.clone();
        let mut seen = HashSet::new();
        let mut results = Vec::new();
        for cache_key in &order {
            let Some(page) = self.folder_chat_search_page_for_cache_key(cache_key, cx) else {
                continue;
            };
            for chat in page.chats.iter().flatten() {
                let info = chat.to_chat_info();
                if seen.insert(info.id.clone()) {
                    results.push(info);
                }
            }
        }
        results
    }

    pub fn is_contact(&self, user_id: &str) -> bool {
        self.contact_ids.contains(user_id)
    }

    pub fn apply_folders(&mut self, dto: &GetChatFoldersGatewayDTO, cx: &mut Context<Self>) {
        self.folders = dto.folders.iter().map(|folder| folder.to_chat_folder()).collect();
        self.folders.sort_by_key(|folder| folder.position);
        cx.notify();
    }

    pub fn hydrate_folders_from_disk(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if !self.folders.is_empty() {
            return;
        }
        let payload = SaiWsResponseCache::observed_payload(
            &Self::folders_cache_key(user_id),
            SaiWsCachePreferredLayer::DiskFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let Some(dto) = SaiJSONCodec::decode::<GetChatFoldersGatewayDTO>(payload.as_ref()) else {
            return;
        };
        self.apply_folders(&dto, cx);
    }

    pub fn apply_folder_contacts(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        self.contact_ids = ids.into_iter().filter(|id| !id.is_empty()).collect();
        cx.notify();
    }

    pub fn refresh_folder_contacts_from_cache(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let Some(payload) = SaiWsResponseCache::observed_payload(
            &Self::folder_contacts_cache_key(user_id),
            SaiWsCachePreferredLayer::DiskFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        ) else {
            return;
        };
        let Some(dto) = SaiJSONCodec::decode::<ListSavedContacts>(Some(&payload)) else {
            return;
        };
        self.apply_folder_contacts(dto.contacts.into_iter().map(|contact| contact.target_user_id).collect(), cx);
    }

    pub fn folders_in_order(&self, cx: &App) -> Vec<ChatFolder> {
        let mut ordered = self.folders.clone();
        ordered.sort_by_key(|folder| folder.position);
        match GuideSandbox::existing(cx) {
            Some(sandbox) => sandbox.read(cx).overlay_folders(ordered),
            None => ordered,
        }
    }

    pub fn folder(&self, id: &str, cx: &App) -> Option<ChatFolder> {
        self.folders.iter().find(|folder| folder.id == id).cloned().or_else(|| {
            GuideSandbox::existing(cx).and_then(|sandbox| {
                let scene = sandbox.read(cx).folder_scene.as_ref()?;
                scene.invented.iter().find(|folder| folder.id == id).cloned()
            })
        })
    }

    pub fn chats(&self, for_folder: &ChatFolder, source: &[ChatInfo]) -> Vec<ChatInfo> {
        for_folder.filter(source, &|user_id| self.is_contact(user_id))
    }

    pub fn upsert_folder_local(&mut self, folder: ChatFolder, cx: &mut Context<Self>) {
        if let Some(index) = self.folders.iter().position(|existing| existing.id == folder.id) {
            self.folders[index] = folder;
        } else {
            self.folders.push(folder);
        }
        self.folders.sort_by_key(|folder| folder.position);
        self.persist_folders_local(cx);
        cx.notify();
    }

    pub fn remove_folder_local(&mut self, folder_id: &str, cx: &mut Context<Self>) {
        self.folders.retain(|folder| folder.id != folder_id);
        self.normalize_folder_positions();
        self.persist_folders_local(cx);
        cx.notify();
    }

    pub fn set_folders_order_local(&mut self, ordered: Vec<ChatFolder>, cx: &mut Context<Self>) {
        self.folders = ordered
            .into_iter()
            .enumerate()
            .map(|(index, mut folder)| {
                folder.position = index as i64;
                folder
            })
            .collect();
        self.persist_folders_local(cx);
        cx.notify();
    }

    pub fn folders_snapshot(&self) -> Vec<ChatFolder> {
        self.folders.clone()
    }

    pub fn set_folder_pinned_local(&mut self, folder_id: &str, ordered_ids: &[String], cx: &mut Context<Self>) {
        let Some(index) = self.folders.iter().position(|folder| folder.id == folder_id) else {
            return;
        };
        let mut seen = HashSet::new();
        self.folders[index].pinned_chat_ids =
            ordered_ids.iter().filter(|id| !id.is_empty() && seen.insert(id.as_str())).cloned().collect();
        self.persist_folders_local(cx);
        cx.notify();
    }

    pub fn restore_folders_snapshot(&mut self, snapshot: Vec<ChatFolder>, cx: &mut Context<Self>) {
        self.folders = snapshot;
        self.persist_folders_local(cx);
        cx.notify();
    }

    pub fn replace_folder_id_local(&mut self, temp_id: &str, real_id: &str, cx: &mut Context<Self>) {
        if temp_id.is_empty() || real_id.is_empty() {
            return;
        }
        let Some(index) = self.folders.iter().position(|folder| folder.id == temp_id) else {
            return;
        };
        self.folders[index].id = real_id.to_owned();
        self.persist_folders_local(cx);
        cx.notify();
    }

    fn normalize_folder_positions(&mut self) {
        self.folders.sort_by_key(|folder| folder.position);
        for (index, folder) in self.folders.iter_mut().enumerate() {
            folder.position = index as i64;
        }
    }

    fn persist_folders_local(&self, cx: &mut App) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let cache_key = Self::folders_cache_key(&user_id);
        let existing = SaiWsResponseCache::observed_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
        let mut root = AnyDictionaryBridge::dictionary(existing.as_ref()).unwrap_or_default();
        let rows: Vec<Value> =
            self.folders_in_order(cx).iter().map(|folder| Value::Object(Self::folder_cache_row(folder))).collect();
        root.insert(key::FOLDERS.to_owned(), Value::Array(rows));
        if !root.contains_key(key::IS_HAVE_MORE) {
            root.insert(key::IS_HAVE_MORE.to_owned(), Value::Bool(false));
        }
        let _ = SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::All, cx);

        let _ = SaiDebouncer::debounced_action_delay_milliseconds(
            format!("rise-folders-mirror-{user_id}"),
            Box::new(move |cx: &mut App| {
                let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() else {
                    return;
                };
                let Some(payload) = SaiWsResponseCache::observed_payload(
                    &cache_key,
                    SaiWsCachePreferredLayer::MemoryFirst,
                    SaiWsDiskCachePolicy::Persistent,
                    cx,
                ) else {
                    return;
                };
                let Ok(encoded) = serde_json::to_vec(&payload) else {
                    return;
                };
                drop(RiseAccountRuntime::current().spawn(async move {
                    let _ = domain
                        .auxiliary
                        .replace_compatibility_cached_response(
                            rise_canon::rise_wire_methods::chat::SERVICE,
                            rise_canon::rise_wire_methods::chat::GET_CHAT_FOLDERS,
                            &cache_key,
                            &encoded,
                        )
                        .await;
                }));
            }),
            50,
            "riseFolderCacheMirror",
            8,
            None,
            cx,
        );
    }

    fn folder_cache_row(folder: &ChatFolder) -> Map<String, Value> {
        let mut row = Map::new();
        row.insert(key::ID.to_owned(), Value::from(folder.id.clone()));
        row.insert(key::TITLE.to_owned(), Value::from(folder.title.clone()));
        row.insert(key::TITLE_ENTITIES.to_owned(), Value::from(folder.title_entities.clone().unwrap_or_default()));
        row.insert(key::ICON.to_owned(), Value::from(folder.icon.clone().unwrap_or_default()));
        row.insert(key::COLOR.to_owned(), Value::from(folder.color.clone().unwrap_or_default()));
        row.insert(key::INCLUDED_CHAT_IDS.to_owned(), Value::from(folder.included_chat_ids.clone()));
        row.insert(key::INCLUDED_CHAT_TYPES.to_owned(), Value::from(folder.included_chat_types.clone()));
        row.insert(key::EXCLUDED_CHAT_IDS.to_owned(), Value::from(folder.excluded_chat_ids.clone()));
        row.insert(key::EXCLUDED_CHAT_TYPES.to_owned(), Value::from(folder.excluded_chat_types.clone()));
        row.insert(key::PINNED_CHAT_IDS.to_owned(), Value::from(folder.pinned_chat_ids.clone()));
        row.insert(key::POSITION.to_owned(), Value::from(folder.position));
        row.insert(key::CREATED_AT.to_owned(), Value::from(folder.created_at));
        row.insert(key::UPDATED_AT.to_owned(), Value::from(folder.updated_at));
        row
    }
}
