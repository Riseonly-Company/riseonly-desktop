use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::chat as key;
use rise_canon::rise_key::message as message_key;
use rise_canon::rise_request;
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use super::chat_actions::{ChatActionsStore, effective_user_id_for_wire};
use super::chat_folder_types::{
    ChatFolder, ChatFolderWireDTOExt, CreateChatFolderResultDTO, EditChatFolderResultDTO, GetChatFoldersGatewayDTO,
};
use super::chat_wire_types::RiseStoredChatPage;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationCursorFallback, SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_compatibility_bridge::RiseConversationCompatibilityBridge;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_user_hooks;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const GET_CHAT_FOLDERS_LIMIT: i64 = 50;
const FOLDER_CHAT_SEARCH_SLICE_LIMIT: usize = 12;

pub(super) fn chat_call_options(
    method: &str,
    allow_concurrent_while_pending: bool,
    reflect_pending_in_ui: bool,
) -> SaiWsRequestOptions {
    SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: rise_wire_methods::chat::SERVICE.to_owned(),
        method: method.to_owned(),
        reload_if_cached: true,
        allow_concurrent_while_pending,
        reflect_pending_in_ui,
        send_immediately: true,
        store_response_in_cache: false,
        ..SaiWsRequestOptionsInit::default()
    })
}

impl ChatActionsStore {
    pub fn get_chat_folders_action(&self, cx: &mut App) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let services = ChatServicesStore::shared(cx);
        services.update(cx, |store, cx| store.hydrate_folders_from_disk(&user_id, cx));

        let cache_key = ChatServicesStore::folders_cache_key(&user_id);
        let request = rise_request::chat::GetChatFolders { limit: Some(GET_CHAT_FOLDERS_LIMIT), ..rise_request::chat::GetChatFolders::default() };
        let folders_are_empty = services.read(cx).folders.is_empty();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_wire_methods::chat::GET_CHAT_FOLDERS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: folders_are_empty,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(key::FOLDERS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });
        let network_answer = Rc::new(FoldersNetworkAnswer::default());

        if folders_are_empty {
            let request = request.clone();
            let options = options.clone();
            let network_answer = Rc::clone(&network_answer);
            let user_id = user_id.clone();
            cx.spawn(async move |cx| {
                let cached = RiseConversationCompatibilityBridge::cached_response::<_, GetChatFoldersGatewayDTO>(
                    &request, &options, cx,
                )
                .await;
                cx.update(|cx| {
                    if network_answer.received.get() {
                        return;
                    }
                    let services = ChatServicesStore::shared(cx);
                    if !services.read(cx).folders.is_empty() {
                        return;
                    }
                    let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
                    if current_user_id.as_deref() != Some(user_id.as_str()) {
                        return;
                    }
                    let Some(dto) = cached else {
                        return;
                    };
                    services.update(cx, |store, cx| store.apply_folders(&dto, cx));
                });
            })
            .detach();
        }

        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            network_answer.received.set(true);
            let Some(dto) = payload.and_then(|payload| payload.downcast::<GetChatFoldersGatewayDTO>().ok()) else {
                return;
            };
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.apply_folders(&dto, cx));
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Sai::log("Chat", || format!("get_chat_folders error: {error}"), SaiLogKind::Network, cx);
        });
        SaiWs::request_slice(&self.get_chat_folders_sai, request, options, Some(on_success), Some(on_error), cx);
    }

    pub fn load_folder_contacts_action(&self, _relative_id: Option<i64>, cx: &mut App) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.refresh_folder_contacts_from_cache(&user_id, cx));
        if let Some(hooks) = chat_stores_user_hooks() {
            (hooks.load_saved_contacts_for_chat_folders)(cx);
        }
    }

    pub fn search_folder_chats_action(&mut self, q: &str, relative_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };

        let trimmed = trimming_whitespaces_and_newlines(q);
        if trimmed.is_empty() {
            return;
        }

        let body = rise_request::chat::GetChats {
            limit: Some(ChatServicesStore::chats_limit()),
            q: Some(trimmed.to_owned()),
            relative_id: relative_id.map(str::to_owned),
            up: Some(false),
        };
        let cache_key = ChatServicesStore::folder_chat_search_cache_key(&user_id, trimmed);
        let request_slice = if let Some(existing) = self.folder_chat_search_slices.get(&cache_key).cloned() {
            self.folder_chat_search_cache_order.retain(|key| *key != cache_key);
            self.folder_chat_search_cache_order.push(cache_key.clone());
            existing
        } else {
            if self.folder_chat_search_slices.len() >= FOLDER_CHAT_SEARCH_SLICE_LIMIT {
                if self.folder_chat_search_cache_order.is_empty() {
                    self.folder_chat_search_slices.clear();
                } else {
                    let evicted_key = self.folder_chat_search_cache_order.remove(0);
                    self.folder_chat_search_slices.remove(&evicted_key);
                    SaiWsResponseCache::remove(&evicted_key, SaiCacheCleanLayer::Memory, cx);
                }
            }
            let created = SaiTypedSlice::<RiseStoredChatPage>::new(cx);
            self.folder_chat_search_slices.insert(cache_key.clone(), Rc::clone(&created));
            self.folder_chat_search_cache_order.push(cache_key.clone());
            created
        };
        cx.notify();

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_wire_methods::chat::GET_CHATS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::Persistent),
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(key::CHATS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: relative_id.map(str::to_owned),
                response_relative_id_fallback: Some(SaiWsPaginationCursorFallback::LastItemId),
                has_more_path: Some(message_key::IS_HAVE_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        });
        let started = rise_core::foundation::foundation_date::RiseClock::monotonic();
        let sent_query = trimmed.to_owned();
        let sent_relative = relative_id.map(str::to_owned);
        tracing::warn!(target: "riseonly::X903", query = %sent_query, relative = ?sent_relative, "folder search sent");
        let on_success: SaiWsOnSuccess = {
            let slice = Rc::clone(&request_slice);
            let query = sent_query.clone();
            Rc::new(move |_payload: Option<Box<dyn Any>>, _cx: &mut App| {
                let page = slice.data();
                let titles: Vec<String> = page
                    .as_ref()
                    .map(|page| page.chats.iter().flatten().take(5).map(|chat| chat.to_chat_info().title).collect())
                    .unwrap_or_default();
                tracing::warn!(
                    target: "riseonly::X903",
                    query = %query,
                    ms = (rise_core::foundation::foundation_date::RiseClock::monotonic().saturating_sub(started)).as_millis() as u64,
                    count = page.as_ref().map_or(0, |page| page.chats.iter().flatten().count()),
                    more = ?page.as_ref().and_then(|page| page.is_have_more),
                    titles = ?titles,
                    "folder search reply"
                );
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            tracing::warn!(target: "riseonly::X903", query = %sent_query, ms = (rise_core::foundation::foundation_date::RiseClock::monotonic().saturating_sub(started)).as_millis() as u64, %error, "folder search error");
            Sai::log("Chat", || format!("folder search get_chats error: {error}"), SaiLogKind::Network, cx);
        });
        SaiWs::request_slice(&request_slice, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn create_chat_folder_action(
        &self,
        folder: &ChatFolder,
        on_result: Rc<dyn Fn(Option<String>, &mut App)>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() {
            on_result(None, cx);
            return;
        }

        let mut body = rise_request::chat::CreateChatFolder::new(folder.title.clone());
        body.excluded_chat_ids = Some(folder.excluded_chat_ids.clone());
        body.excluded_chat_types = Some(folder.excluded_chat_types.clone());
        body.included_chat_ids = Some(folder.included_chat_ids.clone());
        body.included_chat_types = Some(folder.included_chat_types.clone());
        if let Some(icon) = folder.icon.as_ref().filter(|icon| !icon.is_empty()) {
            body.icon = Some(icon.clone());
        }
        if let Some(entities) = folder.title_entities.as_ref().filter(|entities| !entities.is_empty()) {
            body.title_entities = Some(entities.clone());
        }
        if let Some(color) = folder.color.as_ref().filter(|color| !color.is_empty()) {
            body.color = Some(color.clone());
        }

        let slice = Rc::clone(&self.create_chat_folder_sai);
        let on_success: SaiWsOnSuccess = {
            let on_result = Rc::clone(&on_result);
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let response = payload
                    .and_then(|payload| payload.downcast::<CreateChatFolderResultDTO>().ok())
                    .map(|response| *response)
                    .or_else(|| slice.data());
                on_result(response.map(|response| response.folder_id), cx);
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Sai::log("Chat", || format!("create_chat_folder error: {error}"), SaiLogKind::Network, cx);
            on_result(None, cx);
        });
        SaiWs::request_slice(
            &self.create_chat_folder_sai,
            body,
            chat_call_options(rise_wire_methods::chat::CREATE_CHAT_FOLDER, false, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn edit_chat_folder_action(
        &self,
        folder: &ChatFolder,
        on_result: Rc<dyn Fn(bool, Option<ChatFolder>, &mut App)>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || folder.id.is_empty() {
            on_result(false, None, cx);
            return;
        }

        let mut body = rise_request::chat::EditChatFolder::new(folder.id.clone());
        body.color = Some(folder.color.clone().unwrap_or_default());
        body.excluded_chat_ids = Some(rise_request::chat::edit_chat_folder::ExcludedChatIds { values: Some(folder.excluded_chat_ids.clone()) });
        body.excluded_chat_types =
            Some(rise_request::chat::edit_chat_folder::ExcludedChatTypes { values: Some(folder.excluded_chat_types.clone()) });
        body.icon = Some(folder.icon.clone().unwrap_or_default());
        body.included_chat_ids = Some(rise_request::chat::edit_chat_folder::IncludedChatIds { values: Some(folder.included_chat_ids.clone()) });
        body.included_chat_types =
            Some(rise_request::chat::edit_chat_folder::IncludedChatTypes { values: Some(folder.included_chat_types.clone()) });
        body.title = Some(folder.title.clone());
        body.title_entities = Some(folder.title_entities.clone().unwrap_or_default());

        let slice = Rc::clone(&self.edit_chat_folder_sai);
        let on_success: SaiWsOnSuccess = {
            let on_result = Rc::clone(&on_result);
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let response = payload
                    .and_then(|payload| payload.downcast::<EditChatFolderResultDTO>().ok())
                    .map(|response| *response)
                    .or_else(|| slice.data());
                let folder = response.and_then(|response| response.folder).map(|folder| folder.to_chat_folder());
                on_result(true, folder, cx);
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Sai::log("Chat", || format!("edit_chat_folder error: {error}"), SaiLogKind::Network, cx);
            on_result(false, None, cx);
        });
        SaiWs::request_slice(
            &self.edit_chat_folder_sai,
            body,
            chat_call_options(rise_wire_methods::chat::EDIT_CHAT_FOLDER, false, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn delete_chat_folder_action(
        &self,
        folder_id: &str,
        on_success: Rc<dyn Fn(&mut App)>,
        on_error: Rc<dyn Fn(&mut App)>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || folder_id.is_empty() {
            on_error(cx);
            return;
        }

        let success: SaiWsOnSuccess =
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| on_success(cx));
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Sai::log("Chat", || format!("delete_chat_folder error: {error}"), SaiLogKind::Network, cx);
            on_error(cx);
        });
        SaiWs::request_slice(
            &self.delete_chat_folder_sai,
            rise_request::chat::DeleteChatFolder::new(folder_id.to_owned()),
            chat_call_options(rise_wire_methods::chat::DELETE_CHAT_FOLDER, false, false),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn reorder_chat_folders_action(
        &self,
        folder_ids: Vec<String>,
        on_success: Rc<dyn Fn(&mut App)>,
        on_error: Rc<dyn Fn(&mut App)>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() {
            on_error(cx);
            return;
        }

        let success: SaiWsOnSuccess =
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| on_success(cx));
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Sai::log("Chat", || format!("reorder_chat_folders error: {error}"), SaiLogKind::Network, cx);
            on_error(cx);
        });
        SaiWs::request_slice(
            &self.reorder_chat_folders_sai,
            rise_request::chat::ReorderChatFolders { folder_ids: Some(folder_ids) },
            chat_call_options(rise_wire_methods::chat::REORDER_CHAT_FOLDERS, false, false),
            Some(success),
            Some(failure),
            cx,
        );
    }
}

#[derive(Default)]
struct FoldersNetworkAnswer {
    received: Cell<bool>,
}
