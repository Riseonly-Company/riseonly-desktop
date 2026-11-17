use gpui::App;

use super::notify_services::NotifyServicesStore;
use crate::modules::notify::engine::core::rise_announcements_wire_models::RiseAnnouncement;

impl NotifyServicesStore {
    pub fn current_announcement(&self, cx: &App) -> Option<RiseAnnouncement> {
        self.announcements_presentation.read(cx).snapshot().current.clone()
    }

    pub fn pending_announcements(&self, cx: &App) -> Vec<RiseAnnouncement> {
        self.announcements_presentation.read(cx).snapshot().pending.clone()
    }

    pub fn has_pending_announcement(&self, cx: &App) -> bool {
        self.announcements_presentation.read(cx).snapshot().current.is_some()
    }
}
