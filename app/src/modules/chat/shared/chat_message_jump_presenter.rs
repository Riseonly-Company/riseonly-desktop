use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;

/// a jump to a message asked from a screen that is not the chat
/// they leave the target here and navigate, chat picks it up when its scope settles
/// one request at a time, a second replaces the first
pub struct ChatMessageJumpPresenter {
    pending: Option<ChatMessageJumpPresenterRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessageJumpPresenterRequest {
    pub chat_id: String,
    pub thread_root_id: Option<String>,
    pub topic_id: Option<String>,
    pub message_id: String,
}

impl ChatMessageJumpPresenter {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { pending: None }
    }

    pub fn pending(&self) -> Option<&ChatMessageJumpPresenterRequest> {
        self.pending.as_ref()
    }

    pub fn enqueue(
        &mut self,
        chat_id: &str,
        thread_root_id: Option<&str>,
        topic_id: Option<&str>,
        message_id: &str,
        cx: &mut Context<Self>,
    ) {
        let chat = trimming_whitespaces_and_newlines(chat_id);
        let message = trimming_whitespaces_and_newlines(message_id);
        if chat.is_empty() || message.is_empty() || GetSaiServerId::is_temp_id(Some(message)) {
            self.pending = None;
            cx.notify();
            return;
        }
        self.pending = Some(ChatMessageJumpPresenterRequest {
            chat_id: chat.to_owned(),
            thread_root_id: Self::normalized(thread_root_id),
            topic_id: Self::normalized(topic_id),
            message_id: message.to_owned(),
        });
        cx.notify();
    }

    /// matching ignores the topic on purpose
    /// a topic chat shows aggregate history so its scope has no topic id and the chat resolves
    /// the tab itself
    pub fn consume(
        &mut self,
        chat_id: &str,
        thread_root_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<ChatMessageJumpPresenterRequest> {
        let pending = self.pending.as_ref()?;
        if pending.chat_id != chat_id || pending.thread_root_id != Self::normalized(thread_root_id) {
            return None;
        }
        let taken = self.pending.take();
        cx.notify();
        taken
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.cancel(cx);
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        if value.is_empty() {
            return None;
        }
        Some(value.to_owned())
    }
}
