use gpui::{App, Context, Entity, Task};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::notify::stores::notify_actions::notify_actions::NotifyActionsStore;
use crate::modules::notify::stores::notify_actions::notify_types::NotificationItem;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;

pub struct NotifyInteractionsStore {}

impl NotifyInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub(super) fn service(cx: &mut App) -> Entity<NotifyServicesStore> {
        NotifyServicesStore::shared(cx)
    }

    fn prefetch_gate_rows() -> i64 {
        RiseLimits::client_value("notification_feed_prefetch_gate_rows")
    }

    pub fn observe_notifications(&mut self, observer_id: Uuid, cx: &mut Context<Self>) -> Task<()> {
        NotifyActionsStore::observe_notifications_action(observer_id, cx)
    }

    pub fn ensure_notifications_loaded(&mut self, cx: &mut Context<Self>) {
        NotifyActionsStore::load_notifications_action(false, cx);
    }

    pub fn refresh_notifications(&mut self, cx: &mut Context<Self>) {
        NotifyActionsStore::load_notifications_action(true, cx);
    }

    pub fn notification_row_did_appear(&mut self, row_id: &str, index: i64, row_count: i64, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if service.read(cx).notifications_has_newer(cx) && self.is_near_window_top(row_id, cx) {
            NotifyActionsStore::load_newer_notifications_action(cx);
        }
        if !service.read(cx).notifications_has_more(cx) {
            return;
        }
        if index < row_count - Self::prefetch_gate_rows() {
            return;
        }
        NotifyActionsStore::load_more_notifications_action(cx);
    }

    pub fn notifications_footer_did_appear(&mut self, cx: &mut Context<Self>) {
        if !Self::service(cx).read(cx).notifications_has_more(cx) {
            return;
        }
        NotifyActionsStore::load_more_notifications_action(cx);
    }

    pub fn notifications_header_did_appear(&mut self, cx: &mut Context<Self>) {
        if !Self::service(cx).read(cx).notifications_has_newer(cx) {
            return;
        }
        NotifyActionsStore::load_newer_notifications_action(cx);
    }

    fn is_near_window_top(&self, row_id: &str, cx: &mut Context<Self>) -> bool {
        let mut remaining = Self::prefetch_gate_rows();
        let service = Self::service(cx);
        for section in &service.read(cx).notifications_snapshot(cx).sections {
            for row in &section.rows {
                if remaining <= 0 {
                    return false;
                }
                if row.id == row_id {
                    return true;
                }
                remaining -= 1;
            }
        }
        false
    }

    pub fn drain_notifications_when_filtered_empty(&mut self, filtered_row_count: i64, cx: &mut Context<Self>) {
        if filtered_row_count != 0 || !Self::service(cx).read(cx).notifications_has_more(cx) {
            return;
        }
        NotifyActionsStore::load_more_notifications_action(cx);
    }

    pub fn mark_read(&mut self, item: &NotificationItem, cx: &mut Context<Self>) {
        if item.is_read {
            return;
        }
        NotifyActionsStore::mark_notification_read_action(item.id.clone(), cx);
    }

    pub fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        if !Self::service(cx).read(cx).has_unread(cx) {
            return;
        }
        NotifyActionsStore::mark_all_notifications_read_action(cx);
    }

    pub fn delete(&mut self, item: &NotificationItem, cx: &mut Context<Self>) {
        NotifyActionsStore::delete_notification_action(item.id.clone(), cx);
    }

    pub fn register_device_token(
        &mut self,
        token: String,
        user_id: &str,
        ownership_generation: i64,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let normalized_user_id = trimming_whitespaces_and_newlines(user_id);
        if normalized_user_id.is_empty() || token.is_empty() || ownership_generation <= 0 {
            return Task::ready(Err(CancellationError.into()));
        }
        NotifyActionsStore::register_device_token_action(
            normalized_user_id.to_owned(),
            token,
            ownership_generation,
            cx,
        )
    }
}
