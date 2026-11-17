use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::chat as chat_key;
use rise_canon::{rise_request, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiWsScreenRefresh;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::{
    ChatActionsStore, chat_page_pagination, effective_user_id_for_wire, log_chat_failure,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

impl ChatActionsStore {
    pub fn get_common_chats_action(
        &mut self,
        peer_user_id: &str,
        relative_id: Option<&str>,
        reload_if_cached: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let peer = trimming_whitespaces_and_newlines(peer_user_id);
        if peer.is_empty() || peer == user_id {
            return;
        }

        let body = rise_request::chat::GetCommonChats {
            limit: Some(ChatServicesStore::common_chats_limit()),
            peer_user_id: peer.to_owned(),
            relative_id: relative_id.map(str::to_owned),
            up: Some(false),
        };
        let is_first_page = relative_id.is_none();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_wire_methods::chat::GET_COMMON_CHATS.to_owned(),
            cache_key: Some(ChatServicesStore::common_chats_cache_key_for_user(&user_id, peer)),
            reload_if_cached: !is_first_page || reload_if_cached,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: is_first_page,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
            reload_when_only_disk_cached: false,
            path_to_array: Some(chat_key::CHATS.to_owned()),
            path_to_id: Some(chat_key::ID.to_owned()),
            pagination: Some(chat_page_pagination(relative_id.map(str::to_owned))),
            screen_refresh: Some(SaiWsScreenRefresh::new(true, vec!["ChatProfile".to_owned()], None)),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_chat_failure(format!("get_common_chats error: {error}"), cx);
        });

        SaiWs::request_slice(&self.get_common_chats_sai, body, options, None, Some(on_error), cx);
    }
}
