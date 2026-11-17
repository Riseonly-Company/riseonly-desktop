use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::push_notifications::rise_app_icon_badge_hooks::RiseAppIconBadgeHooks;

pub struct RiseNotifyUnreadPresentation {
    unread_count: i64,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseNotifyUnreadPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { unread_count: 0, account_id: None, account_epoch: None, session_id: None }
    }

    pub fn unread_count(&self) -> i64 {
        self.unread_count
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        if self.account_id.as_deref() == Some(account_id)
            && self.account_epoch.as_deref() == Some(account_epoch)
            && self.session_id == Some(session_id)
        {
            return;
        }
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.unread_count = 0;
        cx.notify();
        Self::sync_app_icon_badge(cx);
    }

    pub fn publish(
        &mut self,
        next: i64,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.account_id.as_deref() != Some(account_id)
            || self.account_epoch.as_deref() != Some(account_epoch)
            || self.session_id != Some(session_id)
        {
            return false;
        }
        let clamped = next.max(0);
        if clamped == self.unread_count {
            return true;
        }
        self.unread_count = clamped;
        cx.notify();
        Self::sync_app_icon_badge(cx);
        true
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.fence_account_transition(cx);
    }

    /// sync fence for an account switch
    /// old domain shuts down async and its counter must not stay on screen under the new account
    pub fn fence_account_transition(&mut self, cx: &mut Context<Self>) {
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.unread_count = 0;
        cx.notify();
        Self::sync_app_icon_badge(cx);
    }

    fn sync_app_icon_badge(cx: &mut Context<Self>) {
        cx.defer(RiseAppIconBadgeHooks::sync);
    }
}
