use std::future::Future;
use std::sync::Arc;

use gpui::App;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use super::message_actions::MessageActionsStore;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat_search::chat_search_overlay_store::{
    ChatSearchMessageKind, ChatSearchOverlayStore,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab::ChatProfileTab;
use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::{
    RiseChatSearchRecentDestination, RiseChatSearchRecentItem,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::{RiseMessageTabKind, RiseMessageTabScope};
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomain;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;

pub(super) fn effective_user_id(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().filter(|user_id| !user_id.is_empty())
}

pub(super) fn run_on_required_domain<Work, WorkFuture>(failure: String, kind: SaiLogKind, work: Work, cx: &mut App)
where
    Work: FnOnce(Arc<RiseConversationDomain>) -> WorkFuture + 'static,
    WorkFuture: Future<Output = ()> + Send + 'static,
{
    cx.spawn(async move |cx| match RiseConversationActionExecutor::require_domain(cx).await {
        Ok(domain) => drop(RiseAccountRuntime::current().spawn(work(domain))),
        Err(error) => {
            let line = format!("{failure}: {error}");
            cx.update(|cx| Sai::log("Message", move || line, kind, cx));
        }
    })
    .detach();
}

#[derive(Clone, Debug)]
pub struct MessageTabPageArgs {
    pub relative_id: Option<String>,
    pub reload_if_cached: bool,
    pub reflect_pending_in_ui: Option<bool>,
    pub screen_refresh_route_tab: Option<ChatProfileTab>,
}

impl Default for MessageTabPageArgs {
    fn default() -> Self {
        Self {
            relative_id: None,
            reload_if_cached: true,
            reflect_pending_in_ui: None,
            screen_refresh_route_tab: None,
        }
    }
}

impl RiseChatSearchRecentItem {
    pub fn from_chat(chat: &ChatInfo) -> Self {
        let tag = trimming_whitespaces_and_newlines(
            chat.tag.as_deref().or(chat.username.as_deref()).unwrap_or_default(),
        );
        let participant_logo = trimming_whitespaces_and_newlines(&chat.participant.more.logo);
        let has_own_logo =
            chat.logo_url.as_deref().is_some_and(|logo_url| !trimming_whitespaces_and_newlines(logo_url).is_empty());
        let is_direct = chat.r#type == ChatType::Direct;
        Self {
            id: format!("chat:{}", chat.id),
            destination: if chat.r#type == ChatType::Channel {
                RiseChatSearchRecentDestination::Channel
            } else {
                RiseChatSearchRecentDestination::Chat
            },
            title: if is_direct { chat.direct_peer_profile_name() } else { chat.title.clone() },
            subtitle: if tag.is_empty() { String::new() } else { format!("@{tag}") },
            logo_url: if has_own_logo {
                chat.logo_url.clone().unwrap_or_default()
            } else {
                participant_logo.to_owned()
            },
            chat_id: Some(chat.id.clone()),
            user_id: is_direct.then(|| chat.participant.id.clone()),
            user_tag: is_direct.then(|| chat.participant.tag.clone()),
            is_online: chat.participant.is_online,
        }
    }

    pub fn from_entity(entity: &SearchEntityRow) -> Self {
        let chat_id = entity.chat_id.as_deref().map(trimming_whitespaces_and_newlines).filter(|id| !id.is_empty());
        let user_id = entity.user_id.as_deref().map(trimming_whitespaces_and_newlines).filter(|id| !id.is_empty());
        let identity = match chat_id {
            Some(chat_id) => format!("chat:{chat_id}"),
            None => format!("user:{}", user_id.unwrap_or(&entity.id)),
        };
        Self {
            id: identity,
            destination: if entity.is_channel() {
                RiseChatSearchRecentDestination::Channel
            } else {
                RiseChatSearchRecentDestination::User
            },
            title: entity.display_name.clone(),
            subtitle: entity.tag_line.clone(),
            logo_url: entity.logo_url.clone(),
            chat_id: chat_id.map(str::to_owned),
            user_id: user_id.map(str::to_owned),
            user_tag: entity.user_tag.clone(),
            is_online: false,
        }
    }
}

impl MessageActionsStore {
    pub fn get_media_messages_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        args: MessageTabPageArgs,
        cx: &mut App,
    ) {
        self.request_message_tab_action(RiseMessageTabKind::Media, chat_id, topic_id, args.relative_id.as_deref(), None, cx);
    }

    pub fn get_file_messages_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        args: MessageTabPageArgs,
        cx: &mut App,
    ) {
        self.request_message_tab_action(RiseMessageTabKind::Files, chat_id, topic_id, args.relative_id.as_deref(), None, cx);
    }

    pub fn get_music_messages_action(&self, chat_id: &str, args: MessageTabPageArgs, cx: &mut App) {
        self.request_message_tab_action(RiseMessageTabKind::Music, chat_id, None, args.relative_id.as_deref(), None, cx);
    }

    pub fn get_audio_messages_action(&self, chat_id: &str, args: MessageTabPageArgs, cx: &mut App) {
        self.request_message_tab_action(RiseMessageTabKind::Voice, chat_id, None, args.relative_id.as_deref(), None, cx);
    }

    pub fn get_gif_messages_action(&self, chat_id: &str, args: MessageTabPageArgs, cx: &mut App) {
        self.request_message_tab_action(RiseMessageTabKind::Gif, chat_id, None, args.relative_id.as_deref(), None, cx);
    }

    pub fn get_fav_messages_action(&self, chat_id: &str, args: MessageTabPageArgs, cx: &mut App) {
        let services = ChatServicesStore::shared(cx);
        let favorites_target_chat_id = services.read(cx).favourites_chat_id(cx);
        self.request_message_tab_action(
            RiseMessageTabKind::Favorites,
            chat_id,
            None,
            args.relative_id.as_deref(),
            favorites_target_chat_id,
            cx,
        );
    }

    pub fn get_link_messages_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        args: MessageTabPageArgs,
        cx: &mut App,
    ) {
        self.request_message_tab_action(RiseMessageTabKind::Links, chat_id, topic_id, args.relative_id.as_deref(), None, cx);
    }

    pub fn search_global_messages_action(
        &self,
        kind: ChatSearchMessageKind,
        query: &str,
        relative_id: Option<&str>,
        cx: &mut App,
    ) {
        let Some(user_id) = effective_user_id(cx) else {
            return;
        };
        let query = query.to_owned();
        let relative_id = relative_id.map(str::to_owned);
        run_on_required_domain(
            "Rise global message search failed".to_owned(),
            SaiLogKind::Network,
            move |domain| async move {
                domain
                    .load_global_message_search(
                        kind,
                        &user_id,
                        &query,
                        relative_id.as_deref(),
                        Some(MessageServicesStore::GLOBAL_MESSAGE_SEARCH_LIMIT),
                    )
                    .await;
            },
            cx,
        );
    }

    pub fn search_chat_overlay_entities_action(&self, query: &str, relative_id: Option<&str>, cx: &mut App) {
        let Some(user_id) = effective_user_id(cx) else {
            return;
        };
        let query = query.to_owned();
        let relative_id = relative_id.map(str::to_owned);
        run_on_required_domain(
            "Rise chat overlay entity search failed".to_owned(),
            SaiLogKind::Network,
            move |domain| async move {
                domain
                    .load_chat_overlay_entities(
                        &user_id,
                        &query,
                        relative_id.as_deref(),
                        Some(ChatSearchOverlayStore::users_limit()),
                    )
                    .await;
            },
            cx,
        );
    }

    pub fn record_chat_search_recent_action(&self, item: RiseChatSearchRecentItem, cx: &mut App) {
        run_on_required_domain(
            "Rise chat search recent write failed".to_owned(),
            SaiLogKind::Cache,
            move |domain| async move {
                domain.record_chat_search_recent(item).await;
            },
            cx,
        );
    }

    pub fn clear_chat_search_recents_action(&self, cx: &mut App) {
        run_on_required_domain(
            "Rise chat search recents clear failed".to_owned(),
            SaiLogKind::Cache,
            move |domain| async move {
                domain.clear_chat_search_recents().await;
            },
            cx,
        );
    }

    pub fn search_chat_overlay_chats_action(&self, query: &str, relative_id: Option<&str>, cx: &mut App) {
        let Some(user_id) = effective_user_id(cx) else {
            return;
        };
        let query = query.to_owned();
        let relative_id = relative_id.map(str::to_owned);
        run_on_required_domain(
            "Rise chat overlay chat search failed".to_owned(),
            SaiLogKind::Network,
            move |domain| async move {
                domain
                    .load_chat_overlay_chats(
                        &user_id,
                        &query,
                        relative_id.as_deref(),
                        Some(ChatSearchOverlayStore::chats_limit()),
                    )
                    .await;
            },
            cx,
        );
    }

    pub fn search_chat_overlay_posts_action(&self, query: &str, relative_id: Option<&str>, cx: &mut App) {
        let query = query.to_owned();
        let relative_id = relative_id.map(str::to_owned);
        run_on_required_domain(
            "Rise chat overlay post search failed".to_owned(),
            SaiLogKind::Network,
            move |domain| async move {
                domain
                    .load_chat_overlay_posts(
                        &query,
                        relative_id.as_deref(),
                        Some(ChatSearchOverlayStore::posts_limit()),
                    )
                    .await;
            },
            cx,
        );
    }

    fn request_message_tab_action(
        &self,
        kind: RiseMessageTabKind,
        chat_id: &str,
        topic_id: Option<&str>,
        relative_id: Option<&str>,
        favorites_target_chat_id: Option<String>,
        cx: &mut App,
    ) {
        let Some(user_id) = effective_user_id(cx) else {
            return;
        };
        if chat_id.is_empty() {
            return;
        }
        let scope = RiseMessageTabScope::new(kind, chat_id.to_owned(), topic_id.map(str::to_owned));
        let relative_id = relative_id.map(str::to_owned);
        run_on_required_domain(
            format!("Rise {} tab({chat_id}) failed", kind.raw_value()),
            SaiLogKind::Network,
            move |domain| async move {
                domain
                    .load_message_tab(
                        &user_id,
                        &scope,
                        relative_id.as_deref(),
                        Some(MessageServicesStore::tabs_limit()),
                        favorites_target_chat_id.as_deref(),
                    )
                    .await;
            },
            cx,
        );
    }
}
