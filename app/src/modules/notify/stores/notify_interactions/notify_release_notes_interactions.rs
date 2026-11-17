use gpui::Context;

use super::notify_interactions::NotifyInteractionsStore;
use crate::modules::notify::engine::core::rise_release_notes_wire_models::{RiseReleaseNote, RiseReleaseSlide};
use crate::modules::notify::engine::rise_launch_interruptions::RiseLaunchInterruptions;
use crate::modules::notify::stores::notify_actions::notify_actions::NotifyActionsStore;

impl NotifyInteractionsStore {
    pub fn release_note_appeared(&mut self, note: &RiseReleaseNote, cx: &mut Context<Self>) {
        RiseLaunchInterruptions::shared(cx).update(cx, |interruptions, cx| {
            interruptions.note_release_note_shown(note.id, cx);
        });
        NotifyActionsStore::mark_release_note_shown_action(note.id, cx);
    }

    pub fn dismiss_release_note(&mut self, note: &RiseReleaseNote, seen_slides: i64, cx: &mut Context<Self>) {
        NotifyActionsStore::dismiss_release_note_action(note.id, seen_slides, cx);
    }

    pub fn open_release_note_action(
        &mut self,
        note: &RiseReleaseNote,
        slide: &RiseReleaseSlide,
        seen_slides: i64,
        cx: &mut Context<Self>,
    ) {
        NotifyActionsStore::open_release_note_action_action(note.id, seen_slides, cx);
        Self::open_slide_action_url(&slide.action_url, cx);
    }
}
