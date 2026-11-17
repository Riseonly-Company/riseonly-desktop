use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::post::stores::post_actions::post_types::{GetPostFeedResponse, RiseStoredPostStats};
use crate::modules::post::stores::post_services::post_audio_services::PostServicesStoreAudioPostsState;

pub struct PostGuideSandboxHooks {
    pub owns_post: Box<dyn Fn(&str, &App) -> bool + Send + Sync>,
    pub post: Box<dyn Fn(&str, &mut App) -> Option<GetPostFeedResponse> + Send + Sync>,
    pub post_stats: Box<dyn Fn(&str, &App) -> Option<RiseStoredPostStats> + Send + Sync>,
    pub sound_posts: Box<dyn Fn(&str, &App) -> Option<PostServicesStoreAudioPostsState> + Send + Sync>,
    pub intercepts_publish: Box<dyn Fn(&App) -> bool + Send + Sync>,
    pub publish_composed_post: Box<dyn Fn(&mut App) + Send + Sync>,
    pub accept_coauthor_invite: Box<dyn Fn(&mut App) + Send + Sync>,
    pub leave_coauthorship: Box<dyn Fn(&mut App) + Send + Sync>,
    pub set_coauthor_shows_in_grid: Box<dyn Fn(bool, &mut App) + Send + Sync>,
}

pub(crate) static POST_GUIDE_SANDBOX_HOOKS: FeatureHookSlot<PostGuideSandboxHooks> = FeatureHookSlot::new("POST_GUIDE_SANDBOX_HOOKS");

pub fn set_post_guide_sandbox_hooks(hooks: PostGuideSandboxHooks) {
    let _ = POST_GUIDE_SANDBOX_HOOKS.set(hooks);
}

pub fn post_guide_sandbox_hooks() -> Option<&'static PostGuideSandboxHooks> {
    POST_GUIDE_SANDBOX_HOOKS.get()
}

impl PostGuideSandboxHooks {
    pub fn owns_post(post_id: &str, cx: &App) -> bool {
        post_guide_sandbox_hooks().is_some_and(|hooks| (hooks.owns_post)(post_id, cx))
    }

    pub fn post(post_id: &str, cx: &mut App) -> Option<GetPostFeedResponse> {
        post_guide_sandbox_hooks().and_then(|hooks| (hooks.post)(post_id, cx))
    }

    pub fn post_stats(post_id: &str, cx: &App) -> Option<RiseStoredPostStats> {
        post_guide_sandbox_hooks().and_then(|hooks| (hooks.post_stats)(post_id, cx))
    }

    pub fn sound_posts(track_id: &str, cx: &App) -> Option<PostServicesStoreAudioPostsState> {
        post_guide_sandbox_hooks().and_then(|hooks| (hooks.sound_posts)(track_id, cx))
    }

    pub fn intercepts_publish(cx: &App) -> bool {
        post_guide_sandbox_hooks().is_some_and(|hooks| (hooks.intercepts_publish)(cx))
    }

    pub fn publish_composed_post(cx: &mut App) {
        if let Some(hooks) = post_guide_sandbox_hooks() {
            (hooks.publish_composed_post)(cx);
        }
    }

    pub fn accept_coauthor_invite(cx: &mut App) {
        if let Some(hooks) = post_guide_sandbox_hooks() {
            (hooks.accept_coauthor_invite)(cx);
        }
    }

    pub fn leave_coauthorship(cx: &mut App) {
        if let Some(hooks) = post_guide_sandbox_hooks() {
            (hooks.leave_coauthorship)(cx);
        }
    }

    pub fn set_coauthor_shows_in_grid(shows: bool, cx: &mut App) {
        if let Some(hooks) = post_guide_sandbox_hooks() {
            (hooks.set_coauthor_shows_in_grid)(shows, cx);
        }
    }
}
