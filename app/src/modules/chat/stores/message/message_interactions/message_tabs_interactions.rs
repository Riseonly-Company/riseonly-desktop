use gpui::Context;
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::modules::chat::components::chat_search::chat_search_overlay_store::{
    ChatSearchMessageKind, ChatSearchOverlayStore,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab::ChatProfileTab;
use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::RiseChatSearchRecentItem;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::RiseConversationAuxiliaryLoadState;
use crate::modules::chat::shared::chat_audio_queue_continuation::{
    ChatAudioQueueContinuation, ChatAudioQueueContinuationOrder,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_tabs_actions::MessageTabPageArgs;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::search::stores::search_actions::search_types::SearchEntityRow;

const MEDIA_TAB_CACHE_ID: &str = "getMediaMessages";
const LINK_TAB_CACHE_ID: &str = "getLinkMessages";

fn initial_tab_args(refresh_route_tab: Option<ChatProfileTab>, force_refresh: bool) -> MessageTabPageArgs {
    MessageTabPageArgs {
        reload_if_cached: force_refresh,
        screen_refresh_route_tab: refresh_route_tab,
        ..MessageTabPageArgs::default()
    }
}

fn next_tab_args(relative_id: String) -> MessageTabPageArgs {
    MessageTabPageArgs { relative_id: Some(relative_id), ..MessageTabPageArgs::default() }
}

impl MessageInteractionsStore {
    pub fn load_initial_media_tab(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        refresh_route_tab: Option<ChatProfileTab>,
        force_refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_media_messages_action(
                &chat_id,
                topic_id,
                initial_tab_args(refresh_route_tab, force_refresh),
                cx,
            );
        });
    }

    pub fn load_more_media_tab(&self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, None);
        self.load_next_tab_page(
            MEDIA_TAB_CACHE_ID,
            &scope,
            |relative_id, cx| {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.get_media_messages_action(chat_id, topic_id, next_tab_args(relative_id), cx);
                });
            },
            cx,
        );
    }

    pub fn load_initial_files_tab(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        refresh_route_tab: Option<ChatProfileTab>,
        force_refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_file_messages_action(
                &chat_id,
                topic_id,
                initial_tab_args(refresh_route_tab, force_refresh),
                cx,
            );
        });
    }

    pub fn load_initial_music_tab(
        &self,
        chat_id: &str,
        refresh_route_tab: Option<ChatProfileTab>,
        force_refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_music_messages_action(&chat_id, initial_tab_args(refresh_route_tab, force_refresh), cx);
        });
    }

    pub fn continue_chat_audio_queue(
        &self,
        chat_id: &str,
        order: ChatAudioQueueContinuationOrder,
        cx: &mut Context<Self>,
    ) {
        ChatAudioQueueContinuation::shared(cx).update(cx, |continuation, cx| {
            continuation.begin(chat_id, order, cx);
        });
    }

    pub fn load_initial_audio_tab(
        &self,
        chat_id: &str,
        refresh_route_tab: Option<ChatProfileTab>,
        force_refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_audio_messages_action(&chat_id, initial_tab_args(refresh_route_tab, force_refresh), cx);
        });
    }

    pub fn load_initial_gif_tab(
        &self,
        chat_id: &str,
        refresh_route_tab: Option<ChatProfileTab>,
        force_refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_gif_messages_action(&chat_id, initial_tab_args(refresh_route_tab, force_refresh), cx);
        });
    }

    pub fn load_initial_fav_tab(
        &self,
        chat_id: &str,
        refresh_route_tab: Option<ChatProfileTab>,
        force_refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_fav_messages_action(&chat_id, initial_tab_args(refresh_route_tab, force_refresh), cx);
        });
    }

    pub fn load_initial_link_tab(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        refresh_route_tab: Option<ChatProfileTab>,
        force_refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_link_messages_action(
                &chat_id,
                topic_id,
                initial_tab_args(refresh_route_tab, force_refresh),
                cx,
            );
        });
    }

    pub fn load_more_link_tab(&self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, None);
        self.load_next_tab_page(
            LINK_TAB_CACHE_ID,
            &scope,
            |relative_id, cx| {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.get_link_messages_action(chat_id, topic_id, next_tab_args(relative_id), cx);
                });
            },
            cx,
        );
    }

    pub fn load_more_files_tab(&self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, None);
        self.load_next_tab_page(
            rise_wire_methods::message::GET_FILE_MESSAGES,
            &scope,
            |relative_id, cx| {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.get_file_messages_action(chat_id, topic_id, next_tab_args(relative_id), cx);
                });
            },
            cx,
        );
    }

    pub fn load_more_music_tab(&self, chat_id: &str, cx: &mut Context<Self>) {
        self.load_next_tab_page(
            rise_wire_methods::message::GET_MUSIC_MESSAGES,
            chat_id,
            |relative_id, cx| {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.get_music_messages_action(chat_id, next_tab_args(relative_id), cx);
                });
            },
            cx,
        );
    }

    pub fn load_more_audio_tab(&self, chat_id: &str, cx: &mut Context<Self>) {
        self.load_next_tab_page(
            rise_wire_methods::message::GET_AUDIO_MESSAGES,
            chat_id,
            |relative_id, cx| {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.get_audio_messages_action(chat_id, next_tab_args(relative_id), cx);
                });
            },
            cx,
        );
    }

    pub fn load_more_gif_tab(&self, chat_id: &str, cx: &mut Context<Self>) {
        self.load_next_tab_page(
            rise_wire_methods::message::GET_GIF_MESSAGES,
            chat_id,
            |relative_id, cx| {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.get_gif_messages_action(chat_id, next_tab_args(relative_id), cx);
                });
            },
            cx,
        );
    }

    pub fn load_more_fav_tab(&self, chat_id: &str, cx: &mut Context<Self>) {
        self.load_next_tab_page(
            rise_wire_methods::message::GET_FAV_MESSAGES,
            chat_id,
            |relative_id, cx| {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.get_fav_messages_action(chat_id, next_tab_args(relative_id), cx);
                });
            },
            cx,
        );
    }

    pub fn search_global_messages(
        &self,
        kind: ChatSearchMessageKind,
        query: &str,
        relative_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let normalized = trimming_whitespaces_and_newlines(query);
        if normalized.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.search_global_messages_action(kind, normalized, relative_id, cx);
        });
    }

    pub fn search_chat_overlay_entities(&self, query: &str, relative_id: Option<&str>, cx: &mut Context<Self>) {
        let normalized = trimming_whitespaces_and_newlines(query);
        if normalized.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.search_chat_overlay_entities_action(normalized, relative_id, cx);
        });
    }

    pub fn record_chat_search_recent_chat(&self, chat: &ChatInfo, cx: &mut Context<Self>) {
        if chat.id.is_empty() || chat.title.is_empty() {
            return;
        }
        let item = RiseChatSearchRecentItem::from_chat(chat);
        Self::actions(cx).update(cx, |actions, cx| actions.record_chat_search_recent_action(item, cx));
    }

    pub fn record_chat_search_recent_entity(&self, entity: &SearchEntityRow, cx: &mut Context<Self>) {
        if entity.id.is_empty() || entity.display_name.is_empty() {
            return;
        }
        let item = RiseChatSearchRecentItem::from_entity(entity);
        Self::actions(cx).update(cx, |actions, cx| actions.record_chat_search_recent_action(item, cx));
    }

    pub fn record_chat_search_recent(&self, item: RiseChatSearchRecentItem, cx: &mut Context<Self>) {
        if item.id.is_empty() || item.title.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.record_chat_search_recent_action(item, cx));
    }

    pub fn clear_chat_search_recents(&self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.clear_chat_search_recents_action(cx));
    }

    pub fn search_chat_overlay_chats(&self, query: &str, relative_id: Option<&str>, cx: &mut Context<Self>) {
        let normalized = trimming_whitespaces_and_newlines(query);
        if normalized.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.search_chat_overlay_chats_action(normalized, relative_id, cx);
        });
    }

    pub fn search_chat_overlay_posts(&self, query: &str, relative_id: Option<&str>, cx: &mut Context<Self>) {
        let normalized = trimming_whitespaces_and_newlines(query);
        if normalized.is_empty() || normalized.starts_with('#') || normalized.starts_with('@') {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.search_chat_overlay_posts_action(normalized, relative_id, cx);
        });
    }

    pub fn retry_failed_chat_overlay_searches(&self, query: &str, cx: &mut Context<Self>) {
        let normalized = trimming_whitespaces_and_newlines(query);
        if normalized.is_empty() {
            return;
        }
        let overlay = ChatSearchOverlayStore::shared(cx);

        let entities_failed = matches!(
            overlay.update(cx, |overlay, cx| overlay.entities_page(normalized, cx)).load_state,
            RiseConversationAuxiliaryLoadState::Failed
        );
        if entities_failed {
            self.search_chat_overlay_entities(normalized, None, cx);
        }
        let chats_failed = matches!(
            overlay.update(cx, |overlay, cx| overlay.chats_page(normalized, cx)).load_state,
            RiseConversationAuxiliaryLoadState::Failed
        );
        if chats_failed {
            self.search_chat_overlay_chats(normalized, None, cx);
        }
        let posts_failed = matches!(
            overlay.update(cx, |overlay, cx| overlay.posts_page(normalized, cx)).load_state,
            RiseConversationAuxiliaryLoadState::Failed
        );
        if posts_failed {
            self.search_chat_overlay_posts(normalized, None, cx);
        }
        for kind in ChatSearchMessageKind::ALL_CASES {
            let state =
                Self::service(cx).update(cx, |service, cx| service.global_message_search_state(kind, normalized, cx));
            if matches!(state.load_state, RiseConversationAuxiliaryLoadState::Failed) {
                self.search_global_messages(kind, normalized, None, cx);
            }
        }
    }

    fn load_next_tab_page(
        &self,
        cache_id: &str,
        scope: &str,
        request: impl FnOnce(String, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) {
        let page = Self::service(cx).update(cx, |service, cx| service.tab_pagination(cache_id, scope, cx));
        if !page.has_more {
            return;
        }
        let Some(relative_id) = page.relative_id else {
            return;
        };
        request(relative_id, cx);
    }
}
