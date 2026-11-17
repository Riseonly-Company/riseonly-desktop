use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::App;

use crate::modules::comment::engine::rise_comment_engine_models::RiseCommentListSnapshot;
use crate::modules::comment::stores::comment_actions::comment_types::GetCommentsResponse;
use crate::modules::post::engine::rise_post_engine_models::RisePostPagePresentation;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::search::engine::rise_search_engine_models::{RiseGlobalSearchPagePresentation, RiseGlobalSearchRow};
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::engine::rise_user_block_presentation::RiseUserBlockPresentation;

type Scopes = HashMap<String, RiseBlockScope>;

pub enum RiseBlockedContent {}

impl RiseBlockedContent {
    fn with_scopes<R>(cx: &App, read: impl FnOnce(&Scopes) -> R) -> R {
        match rise_ui::shared::try_read::<RiseUserBlockPresentation>(cx) {
            Some(presentation) => read(&presentation.snapshot().scopes),
            None => read(&Scopes::new()),
        }
    }

    pub fn hides(author_id: Option<&str>, cx: &App) -> bool {
        let Some(author_id) = author_id.filter(|id| !id.is_empty()) else {
            return false;
        };
        Self::with_scopes(cx, |scopes| scopes.contains_key(author_id))
    }

    pub fn hides_post(post: &GetPostFeedResponse, cx: &App) -> bool {
        Self::with_scopes(cx, |scopes| Self::post_hidden_in(post, scopes))
    }

    pub fn hides_comment(comment: &GetCommentsResponse, cx: &App) -> bool {
        Self::with_scopes(cx, |scopes| Self::comment_hidden_in(comment, scopes))
    }

    pub fn visible_posts(posts: Vec<GetPostFeedResponse>, cx: &App) -> Vec<GetPostFeedResponse> {
        Self::with_scopes(cx, |scopes| {
            if scopes.is_empty() || !posts.iter().any(|post| Self::post_hidden_in(post, scopes)) {
                return posts;
            }
            posts.into_iter().filter(|post| !Self::post_hidden_in(post, scopes)).collect()
        })
    }

    /// same page when nothing in it is hidden so an untouched page keeps its identity
    pub fn visible_page<'a>(page: Cow<'a, RisePostPagePresentation>, cx: &App) -> Cow<'a, RisePostPagePresentation> {
        Self::with_scopes(cx, |scopes| {
            if scopes.is_empty() || !page.items.iter().any(|post| Self::post_hidden_in(post, scopes)) {
                return page;
            }
            let mut next = page.into_owned();
            next.items.retain(|post| !Self::post_hidden_in(post, scopes));
            Cow::Owned(next)
        })
    }

    pub fn visible_comment_snapshot(snapshot: Arc<RiseCommentListSnapshot>, cx: &App) -> Arc<RiseCommentListSnapshot> {
        Self::with_scopes(cx, |scopes| {
            if scopes.is_empty() || !snapshot.items.iter().any(|comment| Self::comment_hidden_in(comment, scopes)) {
                return snapshot;
            }
            let mut next = (*snapshot).clone();
            next.items.retain(|comment| !Self::comment_hidden_in(comment, scopes));
            tracing::warn!(
                target: "riseonly::X1903",
                "blocked filter scope={:?} before={} after={} hidden_authors={:?} scopes={}",
                snapshot.scope,
                snapshot.items.len(),
                next.items.len(),
                snapshot
                    .items
                    .iter()
                    .filter(|comment| Self::comment_hidden_in(comment, scopes))
                    .map(|comment| (comment.id.as_str(), comment.author.id.as_deref(), comment.author_id.as_str()))
                    .collect::<Vec<_>>(),
                scopes.len()
            );
            Arc::new(next)
        })
    }

    pub fn visible_search_page<'a>(
        page: Cow<'a, RiseGlobalSearchPagePresentation>,
        cx: &App,
    ) -> Cow<'a, RiseGlobalSearchPagePresentation> {
        Self::with_scopes(cx, |scopes| {
            let hidden = |row: &RiseGlobalSearchRow| match row {
                RiseGlobalSearchRow::Post(post) => Self::post_hidden_in(post, scopes),
                _ => false,
            };
            if scopes.is_empty() || !page.items.iter().any(hidden) {
                return page;
            }
            let mut next = page.into_owned();
            next.items.retain(|row| !hidden(row));
            Cow::Owned(next)
        })
    }

    fn post_hidden_in(post: &GetPostFeedResponse, scopes: &Scopes) -> bool {
        if scopes.is_empty() {
            return false;
        }
        if let Some(id) = post.author.id.as_deref()
            && !id.is_empty()
            && scopes.contains_key(id)
        {
            return true;
        }
        !post.author_id.is_empty() && scopes.contains_key(&post.author_id)
    }

    fn comment_hidden_in(comment: &GetCommentsResponse, scopes: &Scopes) -> bool {
        if scopes.is_empty() {
            return false;
        }
        if let Some(id) = comment.author.id.as_deref()
            && !id.is_empty()
            && scopes.contains_key(id)
        {
            return true;
        }
        !comment.author_id.is_empty() && scopes.contains_key(&comment.author_id)
    }
}
