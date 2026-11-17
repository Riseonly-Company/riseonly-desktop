use gpui::App;

use super::notify_services::NotifyServicesStore;
use crate::modules::notify::engine::core::rise_release_notes_wire_models::RiseReleaseNote;

impl NotifyServicesStore {
    pub fn pending_release_note(&self, cx: &App) -> Option<RiseReleaseNote> {
        self.release_notes_presentation.read(cx).snapshot().current().cloned()
    }

    pub fn release_note_is_blocking(&self, cx: &App) -> bool {
        self.release_notes_presentation.read(cx).is_blocking()
    }
}
