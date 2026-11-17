use gpui::App;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::post::stores::post_actions::post_types::{GetPostFeedResponse, RiseStoredPostStats};
use crate::modules::post::stores::post_services::post_audio_services::PostServicesStoreAudioPostsState;
use crate::modules::post::stores::post_services::post_guide_sandbox_hooks::{
    PostGuideSandboxHooks, set_post_guide_sandbox_hooks,
};

pub(crate) fn install_post_guide_sandbox_hooks() {
    set_post_guide_sandbox_hooks(PostGuideSandboxHooks {
        owns_post: Box::new(owns_post),
        post: Box::new(post),
        post_stats: Box::new(post_stats),
        sound_posts: Box::new(sound_posts),
        intercepts_publish: Box::new(intercepts_publish),
        publish_composed_post: Box::new(publish_composed_post),
        accept_coauthor_invite: Box::new(accept_coauthor_invite),
        leave_coauthorship: Box::new(leave_coauthorship),
        set_coauthor_shows_in_grid: Box::new(set_coauthor_shows_in_grid),
    });
}

fn owns_post(post_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_post(post_id))
}

fn post(post_id: &str, cx: &mut App) -> Option<GetPostFeedResponse> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.post(post_id, cx))
}

fn post_stats(post_id: &str, cx: &App) -> Option<RiseStoredPostStats> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).post_stats(post_id))
}

fn sound_posts(track_id: &str, cx: &App) -> Option<PostServicesStoreAudioPostsState> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).sound_posts(track_id))
}

fn intercepts_publish(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).intercepts_publish())
}

fn publish_composed_post(cx: &mut App) {
    GuideSandbox::shared(cx).read(cx).publish_composed_post();
}

fn accept_coauthor_invite(cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.accept_coauthor_invite(cx));
}

fn leave_coauthorship(cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.leave_coauthorship(cx));
}

fn set_coauthor_shows_in_grid(shows: bool, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_coauthor_shows_in_grid(shows, cx));
}
