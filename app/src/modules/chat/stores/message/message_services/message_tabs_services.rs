use std::collections::HashMap;
use std::fmt::Display;

use gpui::App;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::components::chat_search::chat_search_overlay_store::ChatSearchMessageKind;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::{
    ChatProfileFavEntry, ChatProfileLinkEntry, ChatProfileMediaEntry, ChatProfileSection,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationPresentation, RiseMessageTabKind, RiseMessageTabScope,
};
use crate::modules::chat::engine::core::rise_global_message_search_presentation::{
    RiseGlobalMessageSearchPresentationState, RiseGlobalMessageSearchScope,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageTabPaginationState {
    pub relative_id: Option<String>,
    pub has_more: bool,
}

impl MessageServicesStore {
    pub fn get_media_messages_error_handler(&self, error: impl Display, chat_id: &str, cx: &mut App) {
        Sai::log("Message", || format!("get_media_messages({chat_id}) error: {error}"), SaiLogKind::Network, cx);
    }

    pub fn get_link_messages_error_handler(&self, error: impl Display, chat_id: &str, cx: &mut App) {
        Sai::log("Message", || format!("get_link_messages({chat_id}) error: {error}"), SaiLogKind::Network, cx);
    }

    pub fn media_tab_entries(&self, chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        self.read_active_tab("getMediaMessages", chat_id, cx, |presentation, scope| {
            presentation.message_tab_media_entries(scope).to_vec()
        })
        .unwrap_or_default()
    }

    pub fn file_tab_entries(&self, chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        self.read_active_tab("get_file_messages", chat_id, cx, |presentation, scope| {
            presentation.message_tab_media_entries(scope).to_vec()
        })
        .unwrap_or_default()
    }

    pub fn music_tab_entries(&self, chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        self.read_active_tab("get_music_messages", chat_id, cx, |presentation, scope| {
            presentation.message_tab_media_entries(scope).to_vec()
        })
        .unwrap_or_default()
    }

    pub fn audio_tab_entries(&self, chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        self.read_active_tab("get_audio_messages", chat_id, cx, |presentation, scope| {
            presentation.message_tab_media_entries(scope).to_vec()
        })
        .unwrap_or_default()
    }

    pub fn gif_tab_entries(&self, chat_id: &str, cx: &mut App) -> Vec<ChatProfileMediaEntry> {
        self.read_active_tab("get_gif_messages", chat_id, cx, |presentation, scope| {
            presentation.message_tab_media_entries(scope).to_vec()
        })
        .unwrap_or_default()
    }

    pub fn fav_tab_entries(&self, chat_id: &str, cx: &mut App) -> Vec<ChatProfileFavEntry> {
        self.read_active_tab("get_fav_messages", chat_id, cx, |presentation, scope| {
            presentation.message_tab_favorite_entries(scope).to_vec()
        })
        .unwrap_or_default()
    }

    pub fn link_tab_entries(&self, chat_id: &str, cx: &mut App) -> Vec<ChatProfileLinkEntry> {
        self.read_active_tab("getLinkMessages", chat_id, cx, |presentation, scope| {
            presentation.message_tab_link_entries(scope).to_vec()
        })
        .unwrap_or_default()
    }

    pub fn tab_pagination(&self, cache_id: &str, chat_id: &str, cx: &mut App) -> MessageTabPaginationState {
        self.read_active_tab(cache_id, chat_id, cx, |presentation, scope| MessageTabPaginationState {
            relative_id: presentation.message_tab_cursor(scope).map(str::to_owned),
            has_more: presentation.message_tab_has_more(scope),
        })
        .unwrap_or(MessageTabPaginationState { relative_id: None, has_more: false })
    }

    pub fn message_tab_presentation_revision(&self, cache_id: &str, chat_id: &str, cx: &mut App) -> u64 {
        self.read_active_tab(cache_id, chat_id, cx, |presentation, scope| presentation.message_tab_revision(scope))
            .unwrap_or(0)
    }

    pub fn message_tab_has_resolved_initial(&self, cache_id: &str, chat_id: &str, cx: &mut App) -> bool {
        self.read_active_tab(cache_id, chat_id, cx, |presentation, scope| {
            presentation.message_tab_has_resolved_initial(scope)
        })
        .unwrap_or(false)
    }

    pub fn message_tab_error(&self, cache_id: &str, chat_id: &str, cx: &mut App) -> Option<String> {
        self.read_active_tab(cache_id, chat_id, cx, |presentation, scope| {
            presentation.message_tab_error(scope).map(str::to_owned)
        })
        .flatten()
    }

    pub fn message_tab_render_status(
        &self,
        cache_id: &str,
        chat_id: &str,
        has_entries: bool,
        cx: &mut App,
    ) -> SaiWsSliceStatus {
        if !RiseConversationPresentation::shared(cx).read(cx).is_active() {
            return SaiWsSliceStatus::Idle;
        }
        if !has_entries && GetSaiServerId::is_temp_id(Some(chat_id)) {
            let chat_services = ChatServicesStore::shared(cx);
            let app: &App = cx;
            if chat_services.read(app).real_chat_id(chat_id, app).is_none() {
                return SaiWsSliceStatus::Fulfilled;
            }
        }
        if !self.message_tab_has_resolved_initial(cache_id, chat_id, cx) {
            return SaiWsSliceStatus::Pending;
        }
        if !has_entries && self.message_tab_error(cache_id, chat_id, cx).is_some() {
            return SaiWsSliceStatus::Rejected;
        }
        SaiWsSliceStatus::Fulfilled
    }

    pub fn message_tab_media_sections(
        &self,
        cache_id: &str,
        chat_id: &str,
        _entries: &[ChatProfileMediaEntry],
        cx: &mut App,
    ) -> Vec<ChatProfileSection<ChatProfileMediaEntry>> {
        self.read_active_tab(cache_id, chat_id, cx, |presentation, scope| {
            presentation.message_tab_media_sections(scope).to_vec()
        })
        .unwrap_or_default()
    }

    pub fn message_tab_link_sections(
        &self,
        cache_id: &str,
        chat_id: &str,
        _entries: &[ChatProfileLinkEntry],
        cx: &mut App,
    ) -> Vec<ChatProfileSection<ChatProfileLinkEntry>> {
        self.read_active_tab(cache_id, chat_id, cx, |presentation, scope| {
            presentation.message_tab_link_sections(scope).to_vec()
        })
        .unwrap_or_default()
    }

    pub fn message_tab_entry_index_by_id(
        &self,
        cache_id: &str,
        chat_id: &str,
        _entry_ids: impl FnOnce() -> Vec<String>,
        cx: &mut App,
    ) -> HashMap<String, i64> {
        self.read_active_tab(cache_id, chat_id, cx, |presentation, scope| {
            presentation.message_tab_entry_index_by_id(scope).clone()
        })
        .unwrap_or_default()
    }

    pub fn global_message_search_state(
        &self,
        kind: ChatSearchMessageKind,
        query: &str,
        cx: &mut App,
    ) -> RiseGlobalMessageSearchPresentationState {
        let presentation = RiseConversationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        if !presentation.is_active() {
            return RiseGlobalMessageSearchPresentationState::empty();
        }
        let Some(scope) = RiseGlobalMessageSearchScope::new(kind, query) else {
            return RiseGlobalMessageSearchPresentationState::empty();
        };
        presentation.global_message_search_state(&scope)
    }

    fn read_active_tab<R>(
        &self,
        cache_id: &str,
        storage_scope: &str,
        cx: &mut App,
        read: impl FnOnce(&RiseConversationPresentation, &RiseMessageTabScope) -> R,
    ) -> Option<R> {
        let presentation = RiseConversationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        if !presentation.is_active() {
            return None;
        }
        let scope = self.message_tab_scope(cache_id, storage_scope)?;
        Some(read(presentation, &scope))
    }

    fn message_tab_scope(&self, cache_id: &str, storage_scope: &str) -> Option<RiseMessageTabScope> {
        let kind = RiseMessageTabKind::from_cache_id(cache_id)?;
        RiseMessageTabScope::from_storage_scope(kind, storage_scope)
    }
}
