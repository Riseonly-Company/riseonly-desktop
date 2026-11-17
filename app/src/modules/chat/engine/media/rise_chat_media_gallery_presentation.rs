use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::chat::engine::media::rise_chat_media_gallery_repository::RiseChatMediaGalleryWindow;

pub struct RiseChatMediaGalleryPresentation {
    window: Option<RiseChatMediaGalleryWindow>,
    account_id: Option<String>,
}

impl RiseChatMediaGalleryPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { window: None, account_id: None }
    }

    pub fn window(&self) -> Option<&RiseChatMediaGalleryWindow> {
        self.window.as_ref()
    }

    /// a publish from another account replaces and doesnt refuse
    /// refusing would leave a switched account with the old window if shutdown never ran
    pub fn publish(&mut self, next: RiseChatMediaGalleryWindow, account_id: &str, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) {
            self.account_id = Some(account_id.to_owned());
            self.window = Some(next);
            cx.notify();
            return;
        }
        if let Some(current) = self.window.as_ref()
            && current.session_id == next.session_id
            && next.revision < current.revision
        {
            return;
        }
        self.window = Some(next);
        cx.notify();
    }

    pub fn clear(&mut self, session_id: Uuid, cx: &mut Context<Self>) {
        if self.window.as_ref().map(|window| window.session_id) != Some(session_id) {
            return;
        }
        self.window = None;
        cx.notify();
    }

    pub fn deactivate(&mut self, account_id: &str, cx: &mut Context<Self>) {
        if self.account_id.as_deref().is_some_and(|current| current != account_id) {
            return;
        }
        self.account_id = None;
        self.window = None;
        cx.notify();
    }
}
