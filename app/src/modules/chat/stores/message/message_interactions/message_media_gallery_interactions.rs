use gpui::Context;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::chat::engine::media::rise_chat_media_gallery_policy::RiseChatMediaGalleryPolicy;
use crate::modules::chat::engine::media::rise_chat_media_gallery_repository::{
    RiseChatMediaGalleryEdge, RiseChatMediaGalleryItem,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

impl MessageInteractionsStore {
    pub fn open_media_gallery(
        &mut self,
        session_id: Uuid,
        chat_id: &str,
        topic_id: Option<&str>,
        seed: Vec<RiseChatMediaGalleryItem>,
        anchor_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.resolved_wire_chat_id(chat_id, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.open_media_gallery_action(session_id, &chat_id, topic_id, seed, anchor_id, cx);
        });
    }

    pub fn media_gallery_did_focus(&mut self, session_id: Uuid, item_id: &str, cx: &mut Context<Self>) {
        let Some(window) = Self::service(cx).update(cx, |service, cx| service.media_gallery_window(cx)) else {
            return;
        };
        if window.session_id != session_id {
            return;
        }
        let Some(index) = window.items.iter().position(|item| item.id == item_id) else {
            return;
        };
        let index = index as i64;
        let lookahead = RiseChatMediaGalleryPolicy::prefetch_lookahead().max(0);
        let actions = Self::actions(cx);
        if window.has_more_older && !window.is_loading_older && index <= lookahead {
            actions.update(cx, |actions, cx| {
                actions.extend_media_gallery_action(session_id, RiseChatMediaGalleryEdge::Older, Some(item_id), cx);
            });
        }
        if window.has_more_newer && !window.is_loading_newer && index >= window.items.len() as i64 - 1 - lookahead {
            actions.update(cx, |actions, cx| {
                actions.extend_media_gallery_action(session_id, RiseChatMediaGalleryEdge::Newer, Some(item_id), cx);
            });
        }
    }

    pub fn media_gallery_did_reconnect(&mut self, session_id: Uuid, item_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(window) = Self::service(cx).update(cx, |service, cx| service.media_gallery_window(cx)) else {
            return;
        };
        if window.session_id != session_id {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.resume_media_gallery_action(session_id, cx));
        if let Some(item_id) = item_id {
            self.media_gallery_did_focus(session_id, item_id, cx);
        }
    }

    pub fn close_media_gallery(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.close_media_gallery_action(session_id, cx));
    }
}
