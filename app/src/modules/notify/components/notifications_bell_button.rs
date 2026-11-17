use gpui::{App, Context, ElementId, RenderOnce, Subscription, Window};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::prelude::*;

use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;

/// feed header bell
/// separate element so the feed body doesnt rebuild on every badge count change
#[derive(IntoElement)]
pub struct NotificationsBellButton {
    id: ElementId,
    action: Box<dyn Fn(&mut Window, &mut App) + 'static>,
}

impl NotificationsBellButton {
    pub fn new(id: impl Into<ElementId>, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), action: Box::new(action) }
    }
}

struct NotificationsBellBadge {
    count: i64,
    _store: Subscription,
}

impl NotificationsBellBadge {
    fn new(cx: &mut Context<Self>) -> Self {
        let store = NotifyServicesStore::shared(cx);
        let count = store.read(cx).unread_badge_count(cx);
        let subscription = cx.observe(&store, |badge, store, cx| {
            let count = store.read(cx).unread_badge_count(cx);
            if count != badge.count {
                badge.count = count;
                cx.notify();
            }
        });
        Self { count, _store: subscription }
    }
}

impl RenderOnce for NotificationsBellButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let badge = window.use_keyed_state(self.id.clone(), cx, |_, cx| NotificationsBellBadge::new(cx));
        let badge_count = badge.read(cx).count;
        let action = self.action;
        PageHeaderIconButton::new(self.id, "bell", move |window, cx| action(window, cx)).badge_count(badge_count)
    }
}
