use std::collections::HashSet;

use futures::FutureExt;
use gpui::{App, AsyncApp};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use tokio::task::AbortHandle;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::media::rise_chat_media_gallery_presentation::RiseChatMediaGalleryPresentation;
use crate::modules::chat::engine::media::rise_chat_media_gallery_repository::{
    RiseChatMediaGalleryEdge, RiseChatMediaGalleryItem, RiseChatMediaGalleryWalkStep, RiseChatMediaGalleryWindow,
};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomain;

/// runs work on the main thread
async fn on_main<W>(work: W)
where
    W: FnOnce(&mut App) + Send + 'static,
{
    let Some(main) = RiseEngineMainHop::installed() else {
        return;
    };
    let _ = main.run(move |cx: AsyncApp| async move { Ok(cx.update(work)) }.boxed_local()).await;
}

struct RiseChatMediaGalleryPreviousWalkCancellation(AbortHandle);

impl Drop for RiseChatMediaGalleryPreviousWalkCancellation {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl RiseConversationDomain {
    pub async fn open_media_gallery(
        &self,
        session_id: Uuid,
        chat_id: &str,
        topic_id: Option<&str>,
        seed: Vec<RiseChatMediaGalleryItem>,
        anchor_id: &str,
        user_id: &str,
    ) {
        let normalized_chat_id = trimming_whitespaces_and_newlines(chat_id).to_string();
        if normalized_chat_id.is_empty() || seed.is_empty() || !self.is_active_account_session().await {
            return;
        }
        let window = self.media_gallery.open(session_id, &normalized_chat_id, topic_id, seed, anchor_id).await;
        self.publish_media_gallery(window).await;
        self.start_media_gallery_walk(session_id, user_id);
    }

    pub async fn extend_media_gallery(
        &self,
        session_id: Uuid,
        edge: RiseChatMediaGalleryEdge,
        focus_id: Option<&str>,
        user_id: &str,
    ) {
        if !self.is_active_account_session().await {
            return;
        }
        let trimmed_user_id = trimming_whitespaces_and_newlines(user_id).to_string();
        match self.media_gallery.extend(session_id, edge, focus_id, &trimmed_user_id).await {
            Ok(None) => {}
            Ok(Some(window)) => {
                self.publish_media_gallery(window).await;
                self.start_media_gallery_walk(session_id, user_id);
            }
            Err(error) => {
                let reason = error.to_string();
                on_main(move |cx| {
                    Sai::log("Message", move || format!("media gallery page failed: {reason}"), SaiLogKind::Network, cx);
                })
                .await;
                let Some(window) = self.media_gallery.current_window(session_id) else {
                    return;
                };
                self.publish_media_gallery(window).await;
            }
        }
    }

    pub async fn drop_media_gallery_messages(
        &self,
        message_ids: &HashSet<String>,
        session_id: Option<Uuid>,
        chat_id: Option<&str>,
    ) {
        if !self.is_active_account_session().await {
            return;
        }
        let Some(window) = self.media_gallery.drop_messages(message_ids, session_id, chat_id) else {
            return;
        };
        let window_session_id = window.session_id;
        self.publish_media_gallery(window).await;
        self.start_media_gallery_walk(window_session_id, &self.account_id);
    }

    pub async fn resume_media_gallery(&self, session_id: Uuid, user_id: &str) {
        if !self.is_active_account_session().await || self.media_gallery.current_window(session_id).is_none() {
            return;
        }
        self.start_media_gallery_walk(session_id, user_id);
    }

    pub async fn close_media_gallery(&self, session_id: Uuid) {
        self.with_media_gallery_walk_task(|slot| {
            if let Some(task) = slot.take() {
                task.abort();
            }
        });
        self.media_gallery.close(session_id).await;
        on_main(move |cx| {
            RiseChatMediaGalleryPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.clear(session_id, cx);
            });
        })
        .await;
    }

    /// walks the media of the conversation to both ends in the background
    /// server gives no total so 12 of 340 exists only after both ends are reached
    /// one page at a time, yields to a waiting reader
    fn start_media_gallery_walk(&self, session_id: Uuid, user_id: &str) {
        let trimmed_user_id = trimming_whitespaces_and_newlines(user_id).to_string();
        if trimmed_user_id.is_empty() {
            return;
        }
        let this = self.weak_this();
        self.with_media_gallery_walk_task(|slot| {
            let previous_walk = slot.take();
            let task = RiseAccountRuntime::current().spawn(async move {
                if let Some(previous_walk) = previous_walk {
                    let _cancellation = RiseChatMediaGalleryPreviousWalkCancellation(previous_walk.abort_handle());
                    let _ = previous_walk.await;
                }
                let Some(this) = this.upgrade() else {
                    return;
                };
                while this.step_media_gallery_walk(session_id, &trimmed_user_id).await {}
            });
            *slot = Some(task);
        });
    }

    async fn step_media_gallery_walk(&self, session_id: Uuid, user_id: &str) -> bool {
        if !self.is_active_account_session().await {
            return false;
        }
        match self.media_gallery.walk_index(session_id, user_id).await {
            Ok(RiseChatMediaGalleryWalkStep::Advanced(window)) => {
                self.publish_media_gallery(window).await;
                true
            }
            Ok(RiseChatMediaGalleryWalkStep::Finished) | Err(_) => false,
        }
    }

    async fn publish_media_gallery(&self, window: RiseChatMediaGalleryWindow) {
        if !self.is_active_account_session().await {
            return;
        }
        let owner = self.account_id.clone();
        on_main(move |cx| {
            RiseChatMediaGalleryPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(window, &owner, cx);
            });
        })
        .await;
    }
}
