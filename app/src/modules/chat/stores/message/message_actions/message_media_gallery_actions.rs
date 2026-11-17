use std::collections::HashSet;

use gpui::App;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use super::message_actions::{MessageActionsStore, effective_user_id_for_wire, log_message_failure};
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::media::rise_chat_media_gallery_presentation::RiseChatMediaGalleryPresentation;
use crate::modules::chat::engine::media::rise_chat_media_gallery_repository::{
    RiseChatMediaGalleryEdge, RiseChatMediaGalleryItem,
};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;

impl MessageActionsStore {
    pub fn open_media_gallery_action(
        &self,
        session_id: Uuid,
        chat_id: &str,
        topic_id: Option<&str>,
        seed: Vec<RiseChatMediaGalleryItem>,
        anchor_id: &str,
        cx: &mut App,
    ) {
        if chat_id.is_empty() || seed.is_empty() || anchor_id.is_empty() {
            return;
        }
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        let anchor_id = anchor_id.to_owned();
        cx.spawn(async move |cx| match RiseConversationActionExecutor::require_domain(cx).await {
            Ok(domain) => {
                drop(RiseAccountRuntime::current().spawn(async move {
                    domain
                        .open_media_gallery(session_id, &chat_id, topic_id.as_deref(), seed, &anchor_id, &user_id)
                        .await;
                }));
            }
            Err(error) => cx.update(|cx| {
                log_message_failure(format!("media gallery open({chat_id}) failed: {error}"), cx);
            }),
        })
        .detach();
    }

    pub fn extend_media_gallery_action(
        &self,
        session_id: Uuid,
        edge: RiseChatMediaGalleryEdge,
        focus_id: Option<&str>,
        cx: &mut App,
    ) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let focus_id = focus_id.map(str::to_owned);
        cx.spawn(async move |cx| match RiseConversationActionExecutor::require_domain(cx).await {
            Ok(domain) => {
                drop(RiseAccountRuntime::current().spawn(async move {
                    domain.extend_media_gallery(session_id, edge, focus_id.as_deref(), &user_id).await;
                }));
            }
            Err(error) => cx.update(|cx| {
                log_message_failure(format!("media gallery page failed: {error}"), cx);
            }),
        })
        .detach();
    }

    pub fn resume_media_gallery_action(&self, session_id: Uuid, cx: &mut App) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() else {
            return;
        };
        cx.spawn(async move |cx| {
            if !cx.update(|cx| domain.is_registry_active(cx)) {
                return;
            }
            drop(RiseAccountRuntime::current().spawn(async move {
                domain.resume_media_gallery(session_id, &user_id).await;
            }));
        })
        .detach();
    }

    pub fn drop_media_gallery_messages_action(
        &self,
        message_ids: &[String],
        chat_id: &str,
        session_id: Uuid,
        cx: &mut App,
    ) {
        let ids: HashSet<String> = message_ids.iter().filter(|id| !id.is_empty()).cloned().collect();
        if ids.is_empty() {
            return;
        }
        let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() else {
            return;
        };
        let chat_id = chat_id.to_owned();
        cx.spawn(async move |cx| {
            if !cx.update(|cx| domain.is_registry_active(cx)) {
                return;
            }
            drop(RiseAccountRuntime::current().spawn(async move {
                domain.drop_media_gallery_messages(&ids, Some(session_id), Some(&chat_id)).await;
            }));
        })
        .detach();
    }

    pub fn close_media_gallery_action(&self, session_id: Uuid, cx: &mut App) {
        cx.spawn(async move |cx| {
            let domain = cx.update(|cx| RiseConversationDomainRegistry::shared(cx).read(cx).active());
            let Some(domain) = domain else {
                cx.update(|cx| {
                    RiseChatMediaGalleryPresentation::shared(cx)
                        .update(cx, |presentation, cx| presentation.clear(session_id, cx));
                });
                return;
            };
            drop(RiseAccountRuntime::current().spawn(async move {
                domain.close_media_gallery(session_id).await;
            }));
        })
        .detach();
    }
}
