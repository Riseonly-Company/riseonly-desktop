use std::rc::Rc;

use gpui::{App, Context};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use super::chat_interactions::ChatInteractionsStore;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseConversationPresentation,
};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

pub type ChatArchiveOnError = Rc<dyn Fn(&mut App)>;

impl ChatInteractionsStore {
    pub fn load_initial_archived_chats(&self, cx: &mut Context<Self>) {
        if RiseConversationPresentation::shared(cx).read(cx).is_active() {
            if let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() {
                drop(RiseAccountRuntime::current().spawn(async move {
                    domain.load_chats(RiseChatListKind::Archive).await;
                }));
            }
            return;
        }
        let actions = ChatActionsStore::shared(cx);
        if actions.read(cx).get_archived_chats_sai.is_pending() {
            return;
        }

        ChatServicesStore::shared(cx)
            .update(cx, |services, cx| services.preload_archived_chats_slice_from_disk_if_needed(cx));
        let reflect_pending_in_ui = actions.read(cx).get_archived_chats_sai.data().is_none();
        actions.update(cx, |actions, cx| actions.get_archived_chats_action(None, false, reflect_pending_in_ui, cx));
    }

    pub fn refresh_archived_chats(&self, cx: &mut Context<Self>) {
        if RiseConversationPresentation::shared(cx).read(cx).is_active() {
            if let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() {
                drop(RiseAccountRuntime::current().spawn(async move {
                    domain.refresh_chats(RiseChatListKind::Archive).await;
                }));
            }
            return;
        }
        let actions = ChatActionsStore::shared(cx);
        if actions.read(cx).get_archived_chats_sai.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| actions.get_archived_chats_action(None, true, false, cx));
    }

    pub fn load_more_archived_chats(&self, cx: &mut Context<Self>) {
        let services = ChatServicesStore::shared(cx);
        if RiseConversationPresentation::shared(cx).read(cx).is_active() {
            if !services.update(cx, |services, cx| services.archived_chats_has_more(cx)) {
                return;
            }
            if let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() {
                drop(RiseAccountRuntime::current().spawn(async move {
                    domain.load_more_chats(RiseChatListKind::Archive).await;
                }));
            }
            return;
        }
        let (cache_key, has_more, cursor) = services.update(cx, |services, cx| {
            (
                services.archived_chats_cache_key(cx),
                services.archived_chats_has_more(cx),
                services.archived_chats_relative_id(cx),
            )
        });
        let (Some(cache_key), true, Some(cursor)) = (cache_key, has_more, cursor.filter(|cursor| !cursor.is_empty()))
        else {
            return;
        };

        if !SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().can_request_automatically() {
            return;
        }

        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.get_archived_chats_action(Some(&cursor), false, false, cx));
    }

    pub fn archive_chat(&self, chat: &ChatInfo, on_error: Option<ChatArchiveOnError>, cx: &mut Context<Self>) {
        let refused = ChatServicesStore::shared(cx).update(cx, |services, cx| {
            services.is_archive_mutation_pending(&chat.id) || services.contains_archived_chat(&chat.id, cx)
        });
        if refused {
            return;
        }

        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.archive_chat_action(chat, on_error, cx));
    }

    pub fn unarchive_chat(&self, chat: &ChatInfo, on_error: Option<ChatArchiveOnError>, cx: &mut Context<Self>) {
        let refused = ChatServicesStore::shared(cx).update(cx, |services, cx| {
            services.is_archive_mutation_pending(&chat.id) || !services.contains_archived_chat(&chat.id, cx)
        });
        if refused {
            return;
        }

        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.unarchive_chat_action(chat, on_error, cx));
    }
}
