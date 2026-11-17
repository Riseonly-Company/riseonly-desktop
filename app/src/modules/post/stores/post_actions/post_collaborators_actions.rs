use std::sync::Arc;

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use gpui::{App, AsyncApp, Context};

use crate::modules::post::engine::rise_post_domain::{RisePostActionExecutor, RisePostDomain};
use crate::modules::post::engine::rise_post_repository::RisePostMutationError;
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_types::{
    GetPostFeedResponse, PostCollaboratorKind, PostCollaboratorRejection, PostCollaboratorStatus,
    PostCollaboratorUser, PostTagInputDraft,
};
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

type PostCollaborationResultHandler = Box<dyn FnOnce(anyhow::Result<Vec<PostCollaboratorRejection>>, &mut App)>;

type PostCollaborationOperation =
    Box<dyn FnOnce(Arc<RisePostDomain>, i64) -> LocalBoxFuture<'static, anyhow::Result<Vec<PostCollaboratorRejection>>>>;

impl PostActionsStore {
    pub fn invite_coauthors_action(
        &mut self,
        post_id: i64,
        users: Vec<PostCollaboratorUser>,
        on_result: Option<PostCollaborationResultHandler>,
        cx: &mut Context<Self>,
    ) {
        self.collaboration_action(
            post_id,
            on_result,
            Box::new(move |domain, post_id| async move { domain.invite_coauthors(post_id, users).await }.boxed_local()),
            cx,
        );
    }

    pub fn revoke_coauthor_action(
        &mut self,
        post_id: i64,
        user_id: &str,
        on_result: Option<PostCollaborationResultHandler>,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() {
            return;
        }
        let user_id = user_id.to_owned();
        self.collaboration_action(
            post_id,
            on_result,
            Box::new(move |domain, post_id| {
                async move { domain.revoke_coauthor(post_id, &user_id).await }.boxed_local()
            }),
            cx,
        );
    }

    pub fn accept_coauthor_invite_action(
        &mut self,
        post_id: i64,
        on_result: Option<PostCollaborationResultHandler>,
        cx: &mut Context<Self>,
    ) {
        self.collaboration_action(
            post_id,
            on_result,
            Box::new(|domain, post_id| async move { domain.accept_coauthor_invite(post_id).await }.boxed_local()),
            cx,
        );
    }

    pub fn decline_coauthor_invite_action(
        &mut self,
        post_id: i64,
        on_result: Option<PostCollaborationResultHandler>,
        cx: &mut Context<Self>,
    ) {
        self.collaboration_action(
            post_id,
            on_result,
            Box::new(|domain, post_id| async move { domain.decline_coauthor_invite(post_id).await }.boxed_local()),
            cx,
        );
    }

    pub fn leave_coauthorship_action(
        &mut self,
        post_id: i64,
        on_result: Option<PostCollaborationResultHandler>,
        cx: &mut Context<Self>,
    ) {
        self.collaboration_action(
            post_id,
            on_result,
            Box::new(|domain, post_id| async move { domain.leave_coauthorship(post_id).await }.boxed_local()),
            cx,
        );
    }

    pub fn set_coauthor_grid_visibility_action(
        &mut self,
        post_id: i64,
        show_in_grid: bool,
        on_result: Option<PostCollaborationResultHandler>,
        cx: &mut Context<Self>,
    ) {
        self.collaboration_action(
            post_id,
            on_result,
            Box::new(move |domain, post_id| {
                async move { domain.set_coauthor_grid_visibility(post_id, show_in_grid).await }.boxed_local()
            }),
            cx,
        );
    }

    pub fn set_post_tags_action(
        &mut self,
        post_id: i64,
        tags: Vec<PostTagInputDraft>,
        on_result: Option<PostCollaborationResultHandler>,
        cx: &mut Context<Self>,
    ) {
        self.collaboration_action(
            post_id,
            on_result,
            Box::new(move |domain, post_id| async move { domain.set_post_tags(post_id, tags).await }.boxed_local()),
            cx,
        );
    }

    pub fn remove_post_tag_action(
        &mut self,
        post_id: i64,
        user_id: &str,
        on_result: Option<PostCollaborationResultHandler>,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() {
            return;
        }
        let user_id = user_id.to_owned();
        self.collaboration_action(
            post_id,
            on_result,
            Box::new(move |domain, post_id| {
                async move { domain.remove_post_tag(post_id, &user_id).await }.boxed_local()
            }),
            cx,
        );
    }

    pub fn remove_own_tag_action(
        &mut self,
        post_id: i64,
        on_result: Option<PostCollaborationResultHandler>,
        cx: &mut Context<Self>,
    ) {
        self.collaboration_action(
            post_id,
            on_result,
            Box::new(|domain, post_id| async move { domain.remove_own_tag(post_id).await }.boxed_local()),
            cx,
        );
    }

    pub fn load_post_collaborators_action(
        &mut self,
        post_id: i64,
        kind: Option<PostCollaboratorKind>,
        status: Option<PostCollaboratorStatus>,
        reset: bool,
        on_finished: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if post_id <= 0 {
            return;
        }
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let _ = domain.load_post_collaborators(post_id, kind, status, reset).await;
            if let Some(on_finished) = on_finished {
                cx.update(on_finished);
            }
        })
        .detach();
    }

    fn collaboration_action(
        &mut self,
        post_id: i64,
        on_result: Option<PostCollaborationResultHandler>,
        operation: PostCollaborationOperation,
        cx: &mut Context<Self>,
    ) {
        if post_id <= 0 {
            if let Some(on_result) = on_result {
                on_result(Err(RisePostMutationError::Server("invalid_post_id".to_owned()).into()), cx);
            }
            return;
        }
        let domain = RisePostActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => operation(domain, post_id).await,
                Err(error) => Err(error),
            };
            cx.update(|cx| {
                Self::mirror_collaboration_to_reels(post_id, cx);
                if let Some(on_result) = on_result {
                    on_result(result, cx);
                }
            });
        })
        .detach();
    }

    fn mirror_collaboration_to_reels(post_id: i64, cx: &mut App) {
        let id = post_id.to_string();
        let shorts = ShortsServicesStore::shared(cx);
        if shorts.update(cx, |shorts, cx| shorts.resolved_short(&id, cx)).is_none() {
            return;
        }
        let Some(live) = PostServicesStore::shared(cx).update(cx, |services, cx| services.resolved_post(&id, cx)) else {
            return;
        };
        shorts.update(cx, |shorts, cx| {
            shorts.transform_short(
                &id,
                |short: GetPostFeedResponse| GetPostFeedResponse {
                    my_collaboration: live.my_collaboration.clone(),
                    coauthors: live.coauthors.clone(),
                    coauthors_count: live.coauthors_count,
                    pending_coauthors_count: live.pending_coauthors_count,
                    tagged_users: live.tagged_users.clone(),
                    tagged_users_count: live.tagged_users_count,
                    ..short
                },
                cx,
            );
        });
    }
}
