use gpui::{App, AsyncApp};

use super::notify_actions::NotifyActionsStore;
use crate::modules::notify::engine::rise_notify_domain::RiseNotifyDomainRegistry;

impl NotifyActionsStore {
    pub fn refresh_announcements_after_call_action(cx: &mut App) {
        let Some(domain) = RiseNotifyDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return;
        };
        cx.spawn(async move |_: &mut AsyncApp| {
            domain.refresh_announcements_after_call().await;
        })
        .detach();
    }

    pub fn mark_announcement_shown_action(announcement_id: i64, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.mark_announcement_shown(announcement_id).await;
        });
    }

    pub fn dismiss_announcement_action(announcement_id: i64, seen_slides: i64, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.dismiss_announcement(announcement_id, seen_slides).await;
        });
    }

    pub fn open_announcement_action_action(announcement_id: i64, seen_slides: i64, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.open_announcement_action(announcement_id, seen_slides).await;
        });
    }

    pub fn present_announcement_action(announcement_id: i64, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.open_announcement(announcement_id).await;
        });
    }
}
