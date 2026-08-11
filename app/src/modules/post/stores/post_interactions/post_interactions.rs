use gpui::{App, Entity};

use crate::modules::post::engine::rise_post_engine_models::{PostEntry, PostToggle};
use crate::modules::post::engine::rise_post_rpc::PostScope;

use super::super::post_actions::PostActionsStore;
use super::super::post_services::PostServicesStore;

#[derive(Clone)]
pub struct PostInteractionsStore {
    actions: PostActionsStore,
    services: Entity<PostServicesStore>,
}

impl PostInteractionsStore {
    pub fn new(actions: PostActionsStore, services: Entity<PostServicesStore>) -> Self {
        Self { actions, services }
    }

    pub fn services(&self) -> &Entity<PostServicesStore> {
        &self.services
    }

    pub fn activate_feed(&self, scope: PostScope) {
        self.actions.activate_feed_action(scope);
    }

    pub fn refresh_feed(&self, scope: PostScope) {
        self.actions.refresh_feed_action(scope);
    }

    pub fn load_more(&self, scope: PostScope, cx: &App) {
        let services = self.services.read(cx);
        if !services.has_more(&scope) || services.is_loading_more(&scope) {
            return;
        }
        self.actions.load_more_feed_action(scope);
    }

    pub fn toggle_like(&self, post: &PostEntry) {
        self.actions
            .toggle_action(post.id.clone(), PostToggle::Like, !post.is_liked);
    }

    pub fn like_from_double_tap(&self, post: &PostEntry) {
        if post.is_liked {
            return;
        }
        self.actions
            .toggle_action(post.id.clone(), PostToggle::Like, true);
    }

    pub fn toggle_favorite(&self, post: &PostEntry) {
        self.actions
            .toggle_action(post.id.clone(), PostToggle::Favorite, !post.is_favorited);
    }

    pub fn toggle_repost(&self, post: &PostEntry) {
        self.actions
            .toggle_action(post.id.clone(), PostToggle::Repost, !post.is_reposted);
    }

    pub fn open_comments(&self, post: &PostEntry, cx: &mut App) {
        let post = post.clone();
        self.services
            .update(cx, |services, cx| services.open_comments(&post, cx));
    }

    pub fn open_profile(&self, user_id: String, cx: &mut App) {
        self.services
            .update(cx, |services, cx| services.open_profile(user_id, cx));
    }

    pub fn set_identity(&self, user_id: Option<String>) {
        self.actions.set_identity_action(user_id);
    }
}
