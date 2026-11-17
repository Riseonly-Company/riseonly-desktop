use std::rc::Rc;

use gpui::{App, Window};
use rise_ui::user_logo_story_hooks::{UserLogoStoryHooks, UserLogoStoryUnavailable, set_user_logo_story_hooks};

use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

pub(crate) fn install_user_logo_story_hooks() {
    set_user_logo_story_hooks(UserLogoStoryHooks {
        has_active_stories: |user_id: Option<&str>, cx: &mut App| {
            StoryServicesStore::shared(cx).read(cx).has_active_stories(user_id, cx)
        },
        has_unseen_stories: |user_id: Option<&str>, cx: &mut App| {
            StoryServicesStore::shared(cx).read(cx).has_unseen_stories(user_id, cx)
        },
        has_known_stories_state: |user_id: Option<&str>, cx: &mut App| {
            StoryServicesStore::shared(cx).read(cx).has_known_stories_state(user_id, cx)
        },
        load_stories_state: |user_ids: Vec<String>, cx: &mut App| {
            StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_stories_state(&user_ids, cx));
        },
        open_stories: |for_user_id: &str, on_unavailable: UserLogoStoryUnavailable, window: &mut Window, cx: &mut App| {
            let handle = window.window_handle();
            let on_unavailable = Rc::new(move |cx: &mut App| {
                let on_unavailable = on_unavailable.clone();
                let _ = handle.update(cx, move |_, window, cx| on_unavailable(window, cx));
            });
            StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.open_stories_for_user_id(for_user_id, Some(on_unavailable), cx);
            });
        },
    });
}
