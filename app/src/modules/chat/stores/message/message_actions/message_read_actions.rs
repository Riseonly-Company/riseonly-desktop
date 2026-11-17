use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt;
use gpui::{App, Task};
use rise_bot_api::ChatAction;
use rise_canon::{rise_request, rise_response, rise_wire_methods};
use serde_json::Value;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::message_actions::MessageActionsStore;
use super::message_wire_types::MarkChatAsReadGatewayDTO;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::rise_conversation_domain::{RiseConversationDomain, RiseConversationDomainRegistry};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn active_domain(cx: &mut App) -> Option<Arc<RiseConversationDomain>> {
    RiseConversationDomainRegistry::shared(cx).read(cx).active()
}

fn profile_user_id(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx)
        .read(cx)
        .profile()
        .map(|profile| profile.id.clone())
        .filter(|user_id| !user_id.is_empty())
}

fn is_domain_current(domain: &Arc<RiseConversationDomain>, epoch: Option<&str>, cx: &mut App) -> bool {
    active_domain(cx).is_some_and(|active| Arc::ptr_eq(&active, domain))
        && SaiAccountCacheScope::is_current_account_epoch(Some(&domain.account_id), epoch)
}

fn call_and_log<Body, Reply>(method: &'static str, body: Body, failure: String, cx: &mut App) -> Task<()>
where
    Body: Serialize + 'static,
    Reply: DeserializeOwned + Send + 'static,
{
    cx.spawn(async move |cx| {
        let outcome: anyhow::Result<Reply> =
            RiseConversationActionExecutor::call(rise_wire_methods::message::SERVICE, method, &body, cx).await;
        if let Err(error) = outcome {
            let line = format!("{failure}: {error}");
            cx.update(|cx| Sai::log("Message", move || line, SaiLogKind::Network, cx));
        }
    })
}

impl MessageActionsStore {
    pub fn observe_chat_list_typing_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        shows_names: bool,
        on_update: Rc<dyn Fn(Option<String>, &mut App)>,
        cx: &mut App,
    ) -> Task<()> {
        let Some(domain) = active_domain(cx) else {
            return Task::ready(());
        };
        let epoch = SaiAccountCacheScope::active_epoch();
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        cx.spawn(async move |cx| {
            let opening = domain.clone();
            let opened = RiseAccountRuntime::current()
                .spawn(async move { opening.chat_list_typing_updates(&chat_id, topic_id.as_deref()).await })
                .await;
            let Ok(Ok(mut updates)) = opened else {
                return;
            };
            while let Some(peers) = updates.next().await {
                if !cx.update(|cx| is_domain_current(&domain, epoch.as_deref(), cx)) {
                    return;
                }
                let text = MessageServicesStore::chat_list_typing_text(&peers, shows_names);
                cx.update(|cx| on_update(text, cx));
            }
        })
    }

    pub fn mark_chat_as_read_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        clear_marks: bool,
        up_to_message_id: Option<&str>,
        cx: &mut App,
    ) -> Option<Task<()>> {
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().or_else(|| profile_user_id(cx));
        if user_id.is_none_or(|user_id| user_id.is_empty()) || chat_id.is_empty() {
            return None;
        }

        let mut body = rise_request::message::MarkChatAsRead::new(chat_id.to_owned());
        body.clear_marks = clear_marks.then_some(true);
        body.topic_id = topic_id.map(str::to_owned);
        body.up_to_message_id = up_to_message_id.map(str::to_owned);
        Some(call_and_log::<_, MarkChatAsReadGatewayDTO>(
            rise_wire_methods::message::MARK_CHAT_AS_READ,
            body,
            format!("Rise mark_chat_as_read({chat_id}) failed"),
            cx,
        ))
    }

    pub fn set_chat_unread_mark_action(
        &self,
        chat_id: &str,
        unread: bool,
        on_settled: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut App,
    ) {
        let settle_now = |on_settled: Option<Rc<dyn Fn(&mut App)>>, cx: &mut App| {
            if let Some(on_settled) = on_settled {
                cx.defer(move |cx| on_settled(cx));
            }
        };
        if chat_id.is_empty() {
            settle_now(on_settled, cx);
            return;
        }
        let Some(domain) = active_domain(cx) else {
            settle_now(on_settled, cx);
            return;
        };
        let epoch = SaiAccountCacheScope::active_epoch();
        let chat_id = chat_id.to_owned();
        let mut body = rise_request::message::SetChatUnreadMark::new(chat_id.clone());
        body.unread = Some(unread);
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<rise_response::message::SetChatUnreadMark> = RiseConversationActionExecutor::call(
                rise_wire_methods::message::SERVICE,
                rise_wire_methods::message::SET_CHAT_UNREAD_MARK,
                &body,
                cx,
            )
            .await;
            match outcome {
                Ok(reply) => {
                    if cx.update(|cx| is_domain_current(&domain, epoch.as_deref(), cx)) {
                        let unread_count = reply.unread_count;
                        let settled = RiseAccountRuntime::current().spawn(async move {
                            domain.apply_chat_unread_flag(&chat_id, unread_count).await;
                            domain.refresh_chat_row(&chat_id).await;
                            domain.refresh_unread_badge().await;
                        });
                        let _ = settled.await;
                    }
                }
                Err(error) => {
                    let line = format!("Rise set_chat_unread_mark({chat_id}) failed: {error}");
                    cx.update(|cx| Sai::log("Message", move || line, SaiLogKind::Network, cx));
                }
            }
            if let Some(on_settled) = on_settled {
                cx.update(|cx| on_settled(cx));
            }
        })
        .detach();
    }

    pub fn mark_media_played_action(&self, chat_id: &str, message_id: &str, cx: &mut App) {
        if profile_user_id(cx).is_none() || chat_id.is_empty() || message_id.is_empty() {
            return;
        }
        let body = rise_request::message::MarkMediaAsPlayed::new(chat_id.to_owned(), message_id.to_owned());
        call_and_log::<_, rise_response::message::MarkMediaAsPlayed>(
            rise_wire_methods::message::MARK_MEDIA_AS_PLAYED,
            body,
            format!("Rise mark_media_as_played({chat_id}/{message_id}) failed"),
            cx,
        )
        .detach();
    }

    pub fn mark_media_viewed_action(&self, chat_id: &str, message_id: &str, closed: bool, cx: &mut App) -> Task<bool> {
        if chat_id.is_empty() || message_id.is_empty() {
            return Task::ready(false);
        }
        let mut body = rise_request::message::MarkMediaAsPlayed::new(chat_id.to_owned(), message_id.to_owned());
        body.closed = closed.then_some(true);
        let chat_id = chat_id.to_owned();
        let message_id = message_id.to_owned();
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<Value> = RiseConversationActionExecutor::call(
                rise_wire_methods::message::SERVICE,
                rise_wire_methods::message::MARK_MEDIA_AS_PLAYED,
                &body,
                cx,
            )
            .await;
            match outcome {
                Ok(_) => true,
                Err(error) => {
                    let line = format!("Rise mark_media_viewed({chat_id}/{message_id}) failed: {error}");
                    cx.update(|cx| Sai::log("Message", move || line, SaiLogKind::Network, cx));
                    false
                }
            }
        })
    }

    pub fn mark_channel_posts_viewed_action(&self, chat_id: &str, message_ids: Vec<String>, cx: &mut App) {
        let Some(domain) = active_domain(cx) else {
            return;
        };
        if chat_id.is_empty() || message_ids.is_empty() {
            return;
        }
        let scope = RiseMessageScope::new(chat_id.to_owned(), None, None);
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.mark_channel_posts_viewed(&scope, message_ids).await;
        }));
    }

    pub fn typing_action(
        &self,
        chat_id: &str,
        is_typing: bool,
        action: ChatAction,
        topic_id: Option<&str>,
        cx: &mut App,
    ) {
        if profile_user_id(cx).is_none() || chat_id.is_empty() {
            return;
        }
        let services = ChatServicesStore::shared(cx);
        if services.read(cx).chat_info(chat_id, cx).is_some_and(|chat| chat.r#type == ChatType::Channel) {
            return;
        }

        let body = rise_request::message::MessageTyping {
            action: is_typing.then(|| action.wire().to_owned()),
            chat_id: Some(chat_id.to_owned()),
            is_typing: Some(is_typing),
            topic_id: topic_id.filter(|topic_id| !topic_id.is_empty()).map(str::to_owned),
            ..rise_request::message::MessageTyping::default()
        };
        call_and_log::<_, rise_response::message::MessageTyping>(
            rise_wire_methods::message::MESSAGE_TYPING,
            body,
            format!("Rise message_typing({chat_id}) failed"),
            cx,
        )
        .detach();
    }
}
