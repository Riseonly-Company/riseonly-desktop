use gpui::{App, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::modules::chat::shared::chat_message_jump_presenter::ChatMessageJumpPresenter;

/// go to message from any chat profile tab
/// profile sits on top of the chat so popping back keeps the loaded window and scroll
pub enum ChatProfileMessageJump {}

impl ChatProfileMessageJump {
    pub fn open(
        chat_id: &str,
        topic_id: Option<&str>,
        message_id: &str,
        nav_store: &Entity<NavigationStore>,
        cx: &mut App,
    ) {
        let chat = trimming_whitespaces_and_newlines(chat_id).to_owned();
        let message = trimming_whitespaces_and_newlines(message_id);
        if chat.is_empty() || message.is_empty() || GetSaiServerId::is_temp_id(Some(message)) {
            return;
        }

        let resolved_topic =
            topic_id.map(trimming_whitespaces_and_newlines).filter(|topic| !topic.is_empty());

        ChatMessageJumpPresenter::shared(cx).update(cx, |presenter, cx| {
            presenter.enqueue(&chat, None, resolved_topic, message, cx);
        });

        let nav_store = nav_store.clone();
        cx.defer(move |cx| {
            if Self::is_chat_below_top(&chat, &nav_store, cx) {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                return;
            }
            nav_store.update(cx, |nav_store, cx| {
                nav_store.push(RootRoute::Chat { chat_id: Some(chat) }, NavigationStoreSettlingPush::Drop, cx);
            });
        });
    }

    fn is_chat_below_top(chat_id: &str, nav_store: &Entity<NavigationStore>, cx: &App) -> bool {
        let path = &nav_store.read(cx).path;
        let Some(below_top) = path.len().checked_sub(2).and_then(|index| path.get(index)) else {
            return false;
        };
        match below_top {
            RootRoute::Chat { chat_id: id }
            | RootRoute::ChatTopic { chat_id: id, .. }
            | RootRoute::ForumAll { chat_id: id } => id.as_deref() == Some(chat_id),
            _ => false,
        }
    }
}
