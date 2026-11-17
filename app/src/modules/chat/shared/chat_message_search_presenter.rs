use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

/// request to open a chat with in-chat search on
/// profile search button cant reach the chat state so it leaves the chat id here and navigates
/// one request at a time
pub struct ChatMessageSearchPresenter {
    pending_chat_id: Option<String>,
}

impl ChatMessageSearchPresenter {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { pending_chat_id: None }
    }

    pub fn pending_chat_id(&self) -> Option<&str> {
        self.pending_chat_id.as_deref()
    }

    pub fn enqueue(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let chat = trimming_whitespaces_and_newlines(chat_id);
        self.pending_chat_id = if chat.is_empty() { None } else { Some(chat.to_owned()) };
        cx.notify();
    }

    /// topic is ignored on purpose, a forum has one aggregate scope and search covers every topic
    pub fn consume(&mut self, chat_id: &str, cx: &mut Context<Self>) -> bool {
        if self.pending_chat_id.as_deref() != Some(chat_id) {
            return false;
        }
        self.pending_chat_id = None;
        cx.notify();
        true
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.pending_chat_id = None;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.cancel(cx);
    }
}
