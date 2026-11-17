use gpui::Context;

use crate::modules::auth::services::account_state_reset_coordinator::AccountResettable;
use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetPostHooks, set_account_state_reset_post_hooks,
};
use crate::modules::post::components::post::post_music_playback_controller::PostMusicPlaybackController;
use crate::modules::post::components::repost::repost_opinion_editor::RepostOpinionEditorPresenter;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_presentation::RisePostPresentation;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

impl AccountResettable for PostServicesStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        PostServicesStore::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_post_hooks() {
    set_account_state_reset_post_hooks(AccountStateResetPostHooks {
        fence_account_transition: |cx| {
            RisePostContentPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.fence_account_transition(cx);
            });
            RisePostPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.fence_account_transition(cx);
            });
        },
        reset_account_state: |cx| {
            PostServicesStore::shared(cx).update(cx, |store, cx| PostServicesStore::reset_account_state(store, cx));
            RepostOpinionEditorPresenter::shared(cx)
                .update(cx, |presenter, cx| RepostOpinionEditorPresenter::reset_account_state(presenter, cx));
            PostMusicPlaybackController::shared(cx)
                .update(cx, |controller, cx| AccountResettable::reset_account_state(controller, cx));
        },
    });
}
