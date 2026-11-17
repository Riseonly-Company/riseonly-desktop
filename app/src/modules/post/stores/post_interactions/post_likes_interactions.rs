use gpui::{App, Context};

use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::{SaiDebouncerOptions, sai_debounced_action};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::modules::comment::stores::comment_actions::comment_types::GetCommentsResponse;
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_likes_services::PostLikesTarget;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

impl PostInteractionsStore {
    const POST_LIKES_SEARCH_DELAY_MILLISECONDS: i64 = 350;
    const POST_LIKES_SEARCH_MAX_CACHE_DATA: i64 = 10;

    pub fn open_post_likes(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        if post.likes_count <= 0 {
            return;
        }
        self.open_likes(&PostLikesTarget::Post(post.id.clone()), cx);
    }

    pub fn open_comment_likes(&mut self, comment: &GetCommentsResponse, cx: &mut Context<Self>) {
        if comment.likes_count <= 0 || comment.is_temp {
            return;
        }
        self.open_likes(&PostLikesTarget::Comment(comment.id.clone()), cx);
    }

    fn open_likes(&mut self, target: &PostLikesTarget, cx: &mut Context<Self>) {
        PostServicesStore::shared(cx).update(cx, |services, cx| services.present_post_likes(target, cx));
        let cache_key = PostServicesStore::post_likes_cache_key(target, None);
        if SaiWsPendingRegistry::shared(cx).read(cx).in_flight_request_id(&cache_key).is_some() {
            return;
        }

        PostActionsStore::shared(cx).update(cx, |actions, cx| actions.get_post_likes_action(target, None, None, cx));
    }

    pub fn close_post_likes(&mut self, cx: &mut Context<Self>) {
        PostServicesStore::shared(cx).update(cx, |services, cx| services.dismiss_post_likes(cx));
    }

    pub fn refresh_post_likes(&mut self, target: &PostLikesTarget, search: Option<&str>, cx: &mut Context<Self>) {
        PostActionsStore::shared(cx).update(cx, |actions, cx| actions.get_post_likes_action(target, search, None, cx));
    }

    pub fn search_post_likes(&mut self, target: &PostLikesTarget, query: &str, cx: &mut Context<Self>) {
        let normalized = PostServicesStore::normalized_post_likes_search(Some(query));
        let action_target = target.clone();
        sai_debounced_action(
            &format!("postLikesSearch_{}", target.id()),
            Box::new(move |cx: &mut App| {
                PostActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.get_post_likes_action(&action_target, normalized.as_deref(), None, cx);
                });
            }),
            SaiDebouncerOptions {
                delay_milliseconds: Self::POST_LIKES_SEARCH_DELAY_MILLISECONDS,
                group_key: "postLikesSearch".to_owned(),
                max_cache_data: Self::POST_LIKES_SEARCH_MAX_CACHE_DATA,
            },
            cx,
        );
    }

    pub fn load_more_post_likes(&mut self, target: &PostLikesTarget, search: Option<&str>, cx: &mut Context<Self>) {
        let services = PostServicesStore::shared(cx);
        if !services.update(cx, |services, cx| services.post_likes_has_more(target, search, cx)) {
            return;
        }
        let Some(relative_id) = services.update(cx, |services, cx| services.post_likes_relative_id(target, search, cx))
        else {
            return;
        };

        let cache_key = PostServicesStore::post_likes_cache_key(target, search);
        if !SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().can_request_automatically() {
            return;
        }

        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_post_likes_action(target, search, Some(relative_id), cx);
        });
    }
}
