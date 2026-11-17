use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetStoryHooks, set_account_state_reset_story_hooks,
};
use crate::modules::story::components::story_player::story_media_view::StoryMediaPrefetcher;
use crate::modules::story::shared::story_poster_cache::StoryPosterCache;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;

pub fn install_account_state_reset_story_hooks() {
    set_account_state_reset_story_hooks(AccountStateResetStoryHooks {
        hard_reset_media_caches: |_: &mut App| {
            StoryMediaPrefetcher::reset_account_state();
            StoryPosterCache::reset_account_state();
        },
        reset_account_state: |cx: &mut App| {
            StoryServicesStore::shared(cx).update(cx, |services, cx| services.reset_account_state(cx));
            StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.reset_account_state(cx));
        },
    });
}
