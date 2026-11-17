use gpui::{App, Context, Entity};
use rise_canon::RiseGuideSignal;

use crate::core::guides::guide_facts::GuideFacts;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatMuteScopes;

/// single notifications entry point from a chat row, chat profile or user profile
/// a root page with two grouped buttons and an inner page each
/// with one side only it opens on it
pub struct NotificationsHubSheetStore {
    chat_id: Option<String>,
    chat_scopes: Option<ChatMuteScopes>,
    target_user_id: Option<String>,
    mounts: bool,
    is_presented: bool,
}

impl NotificationsHubSheetStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { chat_id: None, chat_scopes: None, target_user_id: None, mounts: false, is_presented: false }
    }

    pub fn chat_id(&self) -> Option<&str> {
        self.chat_id.as_deref()
    }

    pub fn chat_scopes(&self) -> Option<&ChatMuteScopes> {
        self.chat_scopes.as_ref()
    }

    pub fn target_user_id(&self) -> Option<&str> {
        self.target_user_id.as_deref()
    }

    pub fn mounts(&self) -> bool {
        self.mounts
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn set_is_presented(&mut self, is_presented: bool, cx: &mut Context<Self>) {
        self.is_presented = is_presented;
        GuideFacts::set(RiseGuideSignal::NotificationsHubOpen, is_presented, cx);
        cx.notify();
    }

    pub fn present(
        &mut self,
        chat_id: Option<&str>,
        chat_scopes: Option<ChatMuteScopes>,
        target_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let chat = chat_id.filter(|id| !id.is_empty()).map(str::to_owned);
        let profile = target_user_id.filter(|id| !id.is_empty()).map(str::to_owned);
        if chat.is_none() && profile.is_none() {
            return;
        }
        self.chat_id = chat;
        self.chat_scopes = chat_scopes;
        self.target_user_id = profile;
        self.mounts = true;
        cx.notify();
        self.set_is_presented(true, cx);
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.set_is_presented(false, cx);
    }

    pub fn did_dismiss(&mut self, cx: &mut Context<Self>) {
        self.mounts = false;
        self.chat_id = None;
        self.chat_scopes = None;
        self.target_user_id = None;
        cx.notify();
    }

    pub fn root_page_id(&self) -> &'static str {
        if self.chat_id.is_some() && self.target_user_id.is_some() {
            return "menu";
        }
        if self.chat_id.is_some() { "chat" } else { "profile" }
    }
}
