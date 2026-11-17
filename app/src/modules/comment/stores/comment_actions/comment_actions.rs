use std::future::Future;
use std::sync::Arc;

use gpui::{App, AsyncApp, Context, Entity, Task};

use crate::modules::comment::engine::rise_comment_domain::{RiseCommentActionExecutor, RiseCommentDomain};
use crate::modules::comment::engine::rise_comment_engine_models::{
    RiseCommentCreateDraft, RiseCommentCreateOutcome, RiseCommentScope,
};
use crate::modules::comment::stores::comment_actions::comment_types::{CommentMedia, CommentReaction};

pub type CommentActionsStoreOnFocused = Box<dyn FnOnce(Option<String>, &mut App) + 'static>;

pub struct CommentActionsStore {}

impl CommentActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn on_domain<Operation, Work>(cx: &mut Context<Self>, operation: Operation)
    where
        Operation: FnOnce(Arc<RiseCommentDomain>) -> Work + 'static,
        Work: Future<Output = ()> + 'static,
    {
        let domain = RiseCommentActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            operation(domain).await;
        })
        .detach();
    }

    pub fn open_list_action(
        &mut self,
        scope: RiseCommentScope,
        force: bool,
        focus_comment_id: Option<String>,
        on_focused: Option<CommentActionsStoreOnFocused>,
        cx: &mut Context<Self>,
    ) {
        let domain = RiseCommentActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let focused_id = domain.open(&scope, force, focus_comment_id.as_deref()).await;
            if focus_comment_id.is_none() {
                return;
            }
            if let Some(on_focused) = on_focused {
                cx.update(|cx| on_focused(focused_id, cx));
            }
        })
        .detach();
    }

    pub fn load_more_action(&mut self, scope: RiseCommentScope, cx: &mut Context<Self>) {
        Self::on_domain(cx, move |domain| async move { domain.load_more(&scope).await });
    }

    pub fn begin_observing_action(&mut self, scope: RiseCommentScope, cx: &mut Context<Self>) {
        Self::on_domain(cx, move |domain| async move { domain.begin_observing(&scope).await });
    }

    pub fn end_observing_action(&mut self, scope: RiseCommentScope, cx: &mut Context<Self>) {
        Self::on_domain(cx, move |domain| async move { domain.end_observing(&scope).await });
    }

    pub fn create_action(
        &mut self,
        draft: RiseCommentCreateDraft,
        send_now: bool,
        cx: &mut Context<Self>,
    ) -> Task<RiseCommentCreateOutcome> {
        let domain = RiseCommentActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseCommentCreateOutcome::Failed { draft, refusal: None };
            };
            domain.create(draft, send_now).await
        })
    }

    pub fn complete_create_action(
        &mut self,
        temp_id: &str,
        media: CommentMedia,
        cx: &mut Context<Self>,
    ) -> Task<RiseCommentCreateOutcome> {
        let temp_id = temp_id.to_owned();
        let domain = RiseCommentActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseCommentCreateOutcome::Withdrawn;
            };
            domain.complete_create(&temp_id, media).await
        })
    }

    pub fn discard_create_action(&mut self, temp_id: &str, cx: &mut Context<Self>) {
        let temp_id = temp_id.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.discard_create(&temp_id).await });
    }

    pub fn delete_comment_action(&mut self, comment_id: &str, cx: &mut Context<Self>) {
        let comment_id = comment_id.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.delete(&comment_id).await });
    }

    pub fn set_desired_reaction_action(
        &mut self,
        comment_id: &str,
        desired: CommentReaction,
        cx: &mut Context<Self>,
    ) {
        let comment_id = comment_id.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.set_desired_reaction(&comment_id, desired).await });
    }

    pub fn flush_reaction_action(&mut self, comment_id: &str, cx: &mut Context<Self>) {
        let comment_id = comment_id.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.flush_reaction(&comment_id).await });
    }

    pub fn apply_moderation_status_action(&mut self, comment_id: &str, status: &str, cx: &mut Context<Self>) {
        let comment_id = comment_id.to_owned();
        let status = status.to_owned();
        Self::on_domain(cx, move |domain| async move { domain.apply_moderation_status(&comment_id, &status).await });
    }
}
