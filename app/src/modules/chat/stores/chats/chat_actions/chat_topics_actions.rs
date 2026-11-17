use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::chat_actions::ChatActionsStore;
use super::chat_types::{ChatMuteScopes, ChatTopic, VoiceChannelParticipant, VoiceChannelState};
use super::chat_wire_types::{CreateTopicResultDTO, RiseStoredChatTopic};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWireEmptyDTO;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiWsScreenRefresh, SaiWsScreenRefreshParams};
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::forum::forum_voice_subscription_registry::ForumVoiceSubscriptionRegistry;
use crate::modules::chat::engine::forum::rise_forum_voice_realtime::RiseForumVoiceRealtime;
use crate::modules::chat::engine::rise_conversation_domain::{RiseConversationDomain, RiseConversationDomainRegistry};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::rtc::engine::rise_rtc_domain::RiseRTCActionExecutor;
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCParticipantRole;

pub type ChatTopicSuccess = Rc<dyn Fn(RiseStoredChatTopic, &mut App)>;
pub type ChatTopicCallback = Rc<dyn Fn(&mut App)>;

fn effective_user_id(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().filter(|user_id| !user_id.is_empty())
}

fn active_domain(cx: &mut App) -> Option<std::sync::Arc<RiseConversationDomain>> {
    RiseConversationDomainRegistry::shared(cx).read(cx).active()
}

fn call(callback: Option<&ChatTopicCallback>, cx: &mut App) {
    if let Some(callback) = callback {
        callback(cx);
    }
}

fn decoded_payload<T: DeserializeOwned + 'static>(payload: Option<Box<dyn Any>>) -> Option<T> {
    match payload?.downcast::<T>() {
        Ok(decoded) => Some(*decoded),
        Err(payload) => SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>()),
    }
}

fn mutation_options(method: &str) -> SaiWsRequestOptions {
    SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: rise_canon::rise_wire_methods::chat::SERVICE.to_owned(),
        method: method.to_owned(),
        reload_if_cached: true,
        allow_concurrent_while_pending: true,
        reflect_pending_in_ui: false,
        send_immediately: true,
        store_response_in_cache: false,
        ..SaiWsRequestOptionsInit::default()
    })
}

fn on_error_logging(text: String, on_error: Option<ChatTopicCallback>) -> SaiWsOnError {
    Rc::new(move |error: &anyhow::Error, cx: &mut App| {
        let line = format!("{text} error: {error}");
        Sai::log("Chat", move || line, SaiLogKind::Network, cx);
        call(on_error.as_ref(), cx);
    })
}

fn on_success_calling(on_success: Option<ChatTopicCallback>) -> SaiWsOnSuccess {
    Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| call(on_success.as_ref(), cx))
}

impl ChatActionsStore {
    pub fn set_forum_voice_subscription_action(
        &self,
        chat_id: &str,
        topic_ids: Vec<String>,
        subscribed: bool,
        includes_group: bool,
        cx: &mut App,
    ) {
        if chat_id.is_empty() {
            return;
        }
        let registry = ForumVoiceSubscriptionRegistry::shared(cx);
        registry.update(cx, |registry, cx| {
            if subscribed {
                registry.retain(chat_id, topic_ids.clone(), includes_group, cx);
            } else {
                registry.release(chat_id, cx);
            }
        });
        let Some(domain) = active_domain(cx) else { return };

        let chat_id = chat_id.to_owned();
        drop(RiseAccountRuntime::current().spawn(async move {
            let held = RiseForumVoiceRealtime::set_subscribed(subscribed, &chat_id, &topic_ids, includes_group, &domain.engine)
                .await;
            tracing::warn!(target: "riseonly::X1906", chat = %chat_id, subscribed, includes_group, held, "voice subscription");
        }));
    }

    pub fn forget_chat_voice_presence_action(&self, chat_id: &str, cx: &mut App) {
        if chat_id.is_empty() {
            return;
        }
        let Some(domain) = active_domain(cx) else { return };
        let chat_id = chat_id.to_owned();
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.chat_voice.forget(&chat_id).await;
        }));
    }

    pub fn adopt_group_voice_state_action(&self, chat_id: &str, state: Option<VoiceChannelState>, cx: &mut App) {
        if chat_id.is_empty() {
            return;
        }
        let Some(domain) = active_domain(cx) else { return };
        let chat_id = chat_id.to_owned();
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.chat_voice.adopt_group(&chat_id, state).await;
        }));
    }

    pub fn set_forum_voice_self_presence_action(&self, chat_id: &str, topic_id: &str, joined: bool, cx: &mut App) {
        if chat_id.is_empty() {
            return;
        }
        let Some(domain) = active_domain(cx) else { return };
        let Some(user_id) = effective_user_id(cx) else { return };

        let participant = joined.then(|| Self::self_voice_participant(user_id, cx));
        let room_id = if topic_id.is_empty() {
            format!("rtc:chat:{chat_id}:group")
        } else {
            format!("rtc:chat:{chat_id}:channel:{topic_id}")
        };
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.to_owned();
        drop(RiseAccountRuntime::current().spawn(async move {
            if !topic_id.is_empty() {
                domain.forum_voice.apply_optimistic_self(&chat_id, &topic_id, joined, participant.clone()).await;
            }
            domain
                .chat_voice
                .apply_optimistic_self(&chat_id, &room_id, topic_id.is_empty(), joined, participant)
                .await;
        }));
    }

    fn self_voice_participant(user_id: String, cx: &mut App) -> VoiceChannelParticipant {
        let auth = AuthServicesStore::shared(cx);
        let profile = auth.read(cx).profile();
        VoiceChannelParticipant {
            user_id,
            name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
            tag: profile.map(|profile| profile.tag.clone()).unwrap_or_default(),
            avatar_url: [
                profile.map(|profile| profile.more.logo.as_str()),
                profile.and_then(|profile| profile.avatar_url.as_deref()),
            ]
            .into_iter()
            .flatten()
            .find(|url| !trimming_whitespaces_and_newlines(url).is_empty())
            .unwrap_or_default()
            .to_owned(),
            is_premium: profile.is_some_and(|profile| profile.more.is_premium),
            is_official: profile.is_some_and(|profile| profile.more.is_official),
            role_name: String::new(),
            role_color: 0,
            microphone_enabled: true,
            camera_enabled: false,
            screen_share_enabled: false,
            deafened: false,
            server_muted: false,
            server_deafened: false,
            connected: false,
            joined_at_ms: (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64,
            order_sequence: i64::MAX,
            role: RiseRTCParticipantRole::Unspecified,
            hand_raised: false,
            hand_expires_at_ms: None,
        }
    }

    pub fn expand_forum_voice_preview_action(&self, chat_id: &str, topic_id: &str, cx: &mut App) {
        let Some(domain) = active_domain(cx) else { return };
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.to_owned();
        cx.spawn(async move |cx| {
            let expand_chat_id = chat_id.clone();
            let expand_topic_id = topic_id.clone();
            let is_short = RiseAccountRuntime::current()
                .spawn(async move { domain.forum_voice.expand(&expand_chat_id, &expand_topic_id).await })
                .await
                .unwrap_or(false);
            if !is_short {
                return;
            }
            cx.update(|cx| {
                ChatActionsStore::shared(cx)
                    .update(cx, |actions, cx| actions.load_forum_voice_roster_action(&chat_id, &topic_id, cx));
            });
        })
        .detach();
    }

    pub fn collapse_forum_voice_preview_action(&self, chat_id: &str, topic_id: &str, cx: &mut App) {
        let Some(domain) = active_domain(cx) else { return };
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.to_owned();
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.forum_voice.collapse(&chat_id, &topic_id).await;
        }));
    }

    /// rows of an expanded channel past the server preview come from the rtc listing, forum
    /// listing has the preview only
    pub fn load_forum_voice_roster_action(&mut self, chat_id: &str, topic_id: &str, cx: &mut Context<Self>) {
        let Some(domain) = active_domain(cx) else { return };
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.to_owned();
        cx.spawn(async move |_, cx| {
            let runtime = RiseAccountRuntime::current();
            let request = {
                let domain = std::sync::Arc::clone(&domain);
                let (chat_id, topic_id) = (chat_id.clone(), topic_id.clone());
                runtime
                    .spawn(async move { domain.forum_voice.expanded_roster_request(&chat_id, &topic_id).await })
                    .await
                    .ok()
                    .flatten()
            };
            let Some(request) = request else { return };
            let rtc = cx.update(RiseRTCActionExecutor::require_domain).await.ok();
            drop(runtime.spawn(async move {
                let Some(rtc) = rtc.filter(|rtc| rtc.account_id == request.account_id) else {
                    domain.forum_voice.fail_expanded_roster(&chat_id, &topic_id).await;
                    return;
                };
                match rtc
                    .repository
                    .list_channel_participants(&request.room_id, request.cursor.clone(), request.page_limit)
                    .await
                {
                    Ok((participants, has_more)) => {
                        domain
                            .forum_voice
                            .adopt_expanded_roster(&chat_id, &topic_id, &request, &participants, has_more)
                            .await;
                    }
                    Err(_) => domain.forum_voice.fail_expanded_roster(&chat_id, &topic_id).await,
                }
            }));
        })
        .detach();
    }

    pub fn adopt_forum_voice_state_action(&self, chat_id: &str, topics: &[ChatTopic], cx: &mut App) {
        if chat_id.is_empty() {
            return;
        }
        let Some(domain) = active_domain(cx) else { return };
        let chat_id = chat_id.to_owned();
        let topics = topics.to_vec();
        drop(RiseAccountRuntime::current().spawn(async move {
            domain.forum_voice.adopt(&chat_id, &topics).await;
        }));
    }

    pub fn get_topics_action(&self, chat_id: &str, _force_refresh: bool, cx: &mut App) {
        if chat_id.is_empty() || effective_user_id(cx).is_none() {
            return;
        }

        let cache_key = ChatServicesStore::topics_cache_key(chat_id);
        let route_chat_id = || HashMap::from([(rise_key::guide_screen::camel::CHAT_ID.to_owned(), chat_id.to_owned())]);
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::chat::GET_TOPICS.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: SaiWsResponseCache::memory_only(&cache_key, cx).is_none(),
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::Persistent),
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some("topics".to_owned()),
            path_to_id: Some(rise_key::message::TOPIC_ID.to_owned()),
            screen_refresh: Some(SaiWsScreenRefresh::new(
                true,
                ["ForumHome", "ChatTopics", "Chat", "Chats"].map(str::to_owned).to_vec(),
                Some(SaiWsScreenRefreshParams::PerScreen(HashMap::from([
                    ("ForumHome".to_owned(), route_chat_id()),
                    ("ChatTopics".to_owned(), route_chat_id()),
                    ("Chat".to_owned(), route_chat_id()),
                ]))),
            )),
            ..SaiWsRequestOptionsInit::default()
        });

        let success_chat_id = chat_id.to_owned();
        let on_success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            ChatServicesStore::shared(cx).update(cx, |services, cx| {
                services.reapply_failed_topic_previews(&success_chat_id, cx);
                services.reconcile_slow_mode(&success_chat_id, cx);
            });
        });
        SaiWs::request_slice(
            &self.get_topics_sai,
            rise_canon::rise_request::chat::GetTopics::new(chat_id.to_owned()),
            options,
            Some(on_success),
            Some(on_error_logging(format!("get_topics({chat_id})"), None)),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_topic_action(
        &self,
        chat_id: &str,
        title: &str,
        icon: Option<&str>,
        section_id: Option<&str>,
        on_success: Option<ChatTopicSuccess>,
        on_error: Option<ChatTopicCallback>,
        cx: &mut App,
    ) {
        if chat_id.is_empty() || effective_user_id(cx).is_none() {
            call(on_error.as_ref(), cx);
            return;
        }

        let mut body = rise_canon::rise_request::chat::CreateTopic::new(chat_id.to_owned(), title.to_owned());
        if let Some(icon) = icon.filter(|icon| !icon.is_empty()) {
            body.icon = Some(icon.to_owned());
        }
        if let Some(section_id) = section_id.filter(|section_id| !section_id.is_empty()) {
            body.section_id = Some(section_id.to_owned());
        }

        let payload_error = on_error.clone();
        let handler: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let topic = decoded_payload::<CreateTopicResultDTO>(payload).and_then(|result| result.topic);
            let Some(wire) = topic else {
                call(payload_error.as_ref(), cx);
                return;
            };
            if let Some(on_success) = &on_success {
                on_success(RiseStoredChatTopic::from_canon(&wire), cx);
            }
        });
        SaiWs::request_slice(
            &self.create_topic_sai,
            body,
            mutation_options(rise_canon::rise_wire_methods::chat::CREATE_TOPIC),
            Some(handler),
            Some(on_error_logging(format!("create_topic({chat_id})"), on_error)),
            cx,
        );
    }

    pub fn delete_topic_action(
        &self,
        chat_id: &str,
        topic_id: &str,
        on_success: Option<ChatTopicCallback>,
        on_error: Option<ChatTopicCallback>,
        cx: &mut App,
    ) {
        if effective_user_id(cx).is_none() {
            call(on_error.as_ref(), cx);
            return;
        }

        SaiWs::request_slice(
            &self.delete_topic_sai,
            rise_canon::rise_request::chat::DeleteTopic::new(chat_id.to_owned(), topic_id.to_owned()),
            mutation_options(rise_canon::rise_wire_methods::chat::DELETE_TOPIC),
            Some(on_success_calling(on_success)),
            Some(on_error_logging(format!("delete_topic({topic_id})"), on_error)),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_topic_action(
        &self,
        chat_id: &str,
        topic_id: &str,
        is_closed: Option<bool>,
        title: Option<&str>,
        icon: Option<&str>,
        section_id: Option<&str>,
        position: Option<i64>,
        on_success: Option<ChatTopicSuccess>,
        on_error: Option<ChatTopicCallback>,
        cx: &mut App,
    ) {
        if effective_user_id(cx).is_none() {
            call(on_error.as_ref(), cx);
            return;
        }

        let mut body = rise_canon::rise_request::chat::UpdateTopic::new(chat_id.to_owned(), topic_id.to_owned());
        body.is_closed = is_closed;
        if let Some(title) = title.map(trimming_whitespaces_and_newlines).filter(|title| !title.is_empty()) {
            body.title = Some(title.to_owned());
        }
        if let Some(icon) = icon.map(trimming_whitespaces_and_newlines).filter(|icon| !icon.is_empty()) {
            body.icon = Some(icon.to_owned());
        }
        body.section_id = section_id.map(str::to_owned);
        if let Some(position) = position.filter(|position| *position >= 0) {
            body.position = Some(position);
        }

        let payload_error = on_error.clone();
        let handler: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let topic = decoded_payload::<rise_canon::rise_response::chat::UpdateTopic>(payload)
                .and_then(|result| result.topic);
            let Some(wire) = topic else {
                call(payload_error.as_ref(), cx);
                return;
            };
            if let Some(on_success) = &on_success {
                on_success(RiseStoredChatTopic::from_canon(&wire), cx);
            }
        });
        SaiWs::request_slice(
            &self.update_topic_sai,
            body,
            mutation_options(rise_canon::rise_wire_methods::chat::UPDATE_TOPIC),
            Some(handler),
            Some(on_error_logging(format!("update_topic({topic_id})"), on_error)),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_topic_notifications_action(
        &self,
        chat_id: &str,
        topic_id: &str,
        scopes: ChatMuteScopes,
        inherit: bool,
        on_success: Option<ChatTopicSuccess>,
        on_error: Option<ChatTopicCallback>,
        cx: &mut App,
    ) {
        if effective_user_id(cx).is_none() {
            call(on_error.as_ref(), cx);
            return;
        }

        let mut body =
            rise_canon::rise_request::chat::UpdateTopicNotifications::new(chat_id.to_owned(), topic_id.to_owned());
        body.inherit = Some(inherit);
        if !inherit {
            body.messages = Some(scopes.messages);
            body.mentions = Some(scopes.mentions);
            body.replies = Some(scopes.replies);
            body.reactions = Some(scopes.reactions);
            body.events = Some(scopes.events);
        }

        let payload_error = on_error.clone();
        let handler: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let topic = decoded_payload::<rise_canon::rise_response::chat::UpdateTopicNotifications>(payload)
                .and_then(|result| result.topic);
            let Some(wire) = topic else {
                call(payload_error.as_ref(), cx);
                return;
            };
            if let Some(on_success) = &on_success {
                on_success(RiseStoredChatTopic::from_canon(&wire), cx);
            }
        });
        SaiWs::request_slice(
            &self.update_topic_notifications_sai,
            body,
            mutation_options(rise_canon::rise_wire_methods::chat::UPDATE_TOPIC_NOTIFICATIONS),
            Some(handler),
            Some(on_error_logging(format!("update_topic_notifications({topic_id})"), on_error)),
            cx,
        );
    }

    pub fn set_topics_enabled_action(
        &self,
        chat_id: &str,
        enabled: bool,
        on_success: Option<ChatTopicCallback>,
        on_error: Option<ChatTopicCallback>,
        cx: &mut App,
    ) {
        if chat_id.is_empty() || effective_user_id(cx).is_none() {
            call(on_error.as_ref(), cx);
            return;
        }

        let mut body = rise_canon::rise_request::chat::UpdateChat::new(chat_id.to_owned());
        body.topics_enabled = Some(enabled);
        SaiWs::request_slice(
            &self.set_topics_enabled_sai,
            body,
            mutation_options(rise_canon::rise_wire_methods::chat::UPDATE_CHAT),
            Some(on_success_calling(on_success)),
            Some(on_error_logging(format!("set_topics_enabled({chat_id})"), on_error)),
            cx,
        );
    }

    pub fn set_hide_all_topics_action(
        &self,
        chat_id: &str,
        hidden: bool,
        on_success: impl FnOnce(&mut App) + 'static,
        on_error: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) {
        if chat_id.is_empty() || effective_user_id(cx).is_none() {
            on_error(cx);
            return;
        }

        let chat_id = chat_id.to_owned();
        let mut body = rise_canon::rise_request::chat::UpdateChat::new(chat_id.clone());
        body.hide_all_topics = Some(hidden);
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<SaiWireEmptyDTO> = RiseConversationActionExecutor::call(
                rise_canon::rise_wire_methods::chat::SERVICE,
                rise_canon::rise_wire_methods::chat::UPDATE_CHAT,
                &body,
                cx,
            )
            .await;
            cx.update(|cx| match outcome {
                Ok(_) => on_success(cx),
                Err(error) => {
                    let line = format!("Rise update_chat(hide_all_topics)({chat_id}) failed: {error}");
                    Sai::log("Chat", move || line, SaiLogKind::Network, cx);
                    on_error(cx);
                }
            });
        })
        .detach();
    }
}
