use gpui::App;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::shared::try_read;
use serde_json::Value;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChatPage;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

#[derive(Clone)]
struct ChatCommonChatsProjection {
    chats: Vec<ChatInfo>,
    has_more: bool,
    relative_id: Option<String>,
}

impl ChatCommonChatsProjection {
    fn empty() -> Self {
        Self { chats: Vec::new(), has_more: false, relative_id: None }
    }
}

impl ChatServicesStore {
    pub fn common_chats_limit() -> i64 {
        RiseLimits::client_value("chat_services_store_common_chats_limit")
    }

    pub fn common_chats_cache_key_for_user(user_id: &str, peer_user_id: &str) -> String {
        SaiCache::new(["getCommonChats", user_id, peer_user_id]).key
    }

    pub fn common_chats_cache_key(&self, peer_user_id: &str, cx: &App) -> Option<String> {
        let user_id = try_read::<AuthServicesStore>(cx)
            .and_then(AuthServicesStore::effective_user_id_for_wire)
            .or_else(SaiAccountCacheScope::active_user_id)?;
        if user_id.is_empty() {
            return None;
        }
        let peer = trimming_whitespaces_and_newlines(peer_user_id);
        if peer.is_empty() {
            return None;
        }
        Some(Self::common_chats_cache_key_for_user(&user_id, peer))
    }

    fn common_chats_projection(&self, peer_user_id: &str, cx: &mut App) -> ChatCommonChatsProjection {
        let Some(key) = self.common_chats_cache_key(peer_user_id, cx) else {
            return ChatCommonChatsProjection::empty();
        };
        SaiWsResponseCache::observed_projection(
            &key,
            "chat-common-chats-projection-v1",
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            |payload: Option<&Value>| {
                let Some(dto) =
                    payload.and_then(|payload| serde_json::from_value::<RiseStoredChatPage>(payload.clone()).ok())
                else {
                    return ChatCommonChatsProjection::empty();
                };
                ChatCommonChatsProjection {
                    chats: dto.chats.unwrap_or_default().iter().map(|chat| chat.to_chat_info()).collect(),
                    has_more: dto.is_have_more.unwrap_or(false),
                    relative_id: dto.relative_id,
                }
            },
            cx,
        )
    }

    pub fn common_chats(&self, peer_user_id: &str, cx: &mut App) -> Vec<ChatInfo> {
        self.common_chats_projection(peer_user_id, cx).chats
    }

    pub fn common_chats_has_more(&self, peer_user_id: &str, cx: &mut App) -> bool {
        self.common_chats_projection(peer_user_id, cx).has_more
    }

    pub fn common_chats_relative_id(&self, peer_user_id: &str, cx: &mut App) -> Option<String> {
        self.common_chats_projection(peer_user_id, cx).relative_id
    }
}
