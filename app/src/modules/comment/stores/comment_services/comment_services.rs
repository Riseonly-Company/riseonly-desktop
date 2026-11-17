use std::sync::Arc;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::modules::comment::engine::rise_comment_engine_models::{RiseCommentListSnapshot, RiseCommentScope};
use crate::modules::comment::engine::rise_comment_presentation::RiseCommentPresentation;
use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentSortType, GetCommentsResponse, RepliesSortType,
};
use crate::modules::post::stores::post_actions::post_types::AuthorInfo;
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;

pub struct CommentServicesStore {
    list_owner_post_id: Option<String>,
    list_sort: CommentSortType,
}

impl CommentServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { list_owner_post_id: None, list_sort: CommentSortType::Feed }
    }

    pub fn list_owner_post_id(&self) -> Option<&str> {
        self.list_owner_post_id.as_deref()
    }

    pub fn list_sort(&self) -> CommentSortType {
        self.list_sort
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.list_owner_post_id = None;
        self.list_sort = CommentSortType::Feed;
        cx.notify();
    }

    pub fn prepare_for_post(&mut self, post_id: &str, sort: CommentSortType, cx: &mut Context<Self>) {
        self.list_owner_post_id = Some(post_id.to_owned());
        self.list_sort = sort;
        cx.notify();
    }

    pub fn list_scope(&self) -> Option<RiseCommentScope> {
        let post_id = self.list_owner_post_id.as_ref().filter(|post_id| !post_id.is_empty())?;
        Some(RiseCommentScope::Post { post_id: post_id.clone(), sort: self.list_sort })
    }

    fn snapshot(scope: &RiseCommentScope, cx: &mut App) -> Arc<RiseCommentListSnapshot> {
        RiseCommentPresentation::shared(cx).update(cx, |presentation, cx| presentation.snapshot(scope, cx))
    }

    pub fn list_snapshot(&self, cx: &mut App) -> Option<Arc<RiseCommentListSnapshot>> {
        let scope = self.list_scope()?;
        Some(Self::snapshot(&scope, cx))
    }

    pub fn items(&self, cx: &mut App) -> Vec<GetCommentsResponse> {
        self.list_snapshot(cx).map(|snapshot| snapshot.items.clone()).unwrap_or_default()
    }

    pub fn has_more(&self, cx: &mut App) -> bool {
        self.list_snapshot(cx).is_some_and(|snapshot| snapshot.has_more)
    }

    pub fn replies_snapshot(
        &self,
        comment_id: &str,
        sort: RepliesSortType,
        cx: &mut App,
    ) -> Arc<RiseCommentListSnapshot> {
        Self::snapshot(&RiseCommentScope::Replies { root_id: comment_id.to_owned(), sort }, cx)
    }

    pub fn user_comments_snapshot(&self, target_user_id: &str, cx: &mut App) -> Arc<RiseCommentListSnapshot> {
        Self::snapshot(&RiseCommentScope::User { user_id: target_user_id.to_owned() }, cx)
    }

    pub fn optimistic_author(&self, profile: Option<&ProfileModel>) -> AuthorInfo {
        let mut more = profile.map(|profile| profile.more.clone()).unwrap_or_default();
        let avatar = profile
            .and_then(|profile| profile.avatar_url.as_deref())
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or_default();
        if !avatar.is_empty() {
            more.logo = avatar.to_owned();
        }
        AuthorInfo {
            id: profile.map(|profile| profile.id.clone()),
            name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
            tag: profile.map(|profile| profile.tag.clone()).unwrap_or_default(),
            more,
            ..AuthorInfo::default()
        }
    }
}
