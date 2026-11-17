use gpui::App;

use super::notify_actions::NotifyActionsStore;

impl NotifyActionsStore {
    pub fn mark_release_note_shown_action(release_id: i64, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.mark_release_shown(release_id).await;
        });
    }

    pub fn dismiss_release_note_action(release_id: i64, seen_slides: i64, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.dismiss_release(release_id, seen_slides).await;
        });
    }

    pub fn open_release_note_action_action(release_id: i64, seen_slides: i64, cx: &mut App) {
        Self::with_domain(cx, move |domain| async move {
            domain.open_release_action(release_id, seen_slides).await;
        });
    }
}
