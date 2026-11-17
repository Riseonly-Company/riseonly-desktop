use std::borrow::Cow;
use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::post::engine::rise_post_engine_models::{
    PostCollaboratorsSnapshot, RisePostContentPresentationSnapshot, RisePostFeedKind,
    RisePostPagePresentation, RisePostPageScope, RisePostProfileKind, RisePostSearchKind,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::user::shared::rise_blocked_content::RiseBlockedContent;

static IDLE_PAGE: LazyLock<RisePostPagePresentation> = LazyLock::new(|| RisePostPagePresentation::IDLE);
static EMPTY_COLLABORATORS: LazyLock<PostCollaboratorsSnapshot> =
    LazyLock::new(|| PostCollaboratorsSnapshot::EMPTY);

pub struct RisePostContentPresentation {
    snapshot: RisePostContentPresentationSnapshot,
}

impl RisePostContentPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { snapshot: RisePostContentPresentationSnapshot::empty() }
    }

    pub fn snapshot(&self) -> &RisePostContentPresentationSnapshot {
        &self.snapshot
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        self.snapshot = RisePostContentPresentationSnapshot {
            account_id: account_id.to_owned(),
            account_epoch: account_epoch.to_owned(),
            session_id,
            revision: 0,
            pages_by_scope_key: Default::default(),
            collaborators_by_post_id: Default::default(),
        };
        cx.notify();
    }

    pub fn publish(
        &mut self,
        candidate: RisePostContentPresentationSnapshot,
        cx: &mut Context<Self>,
    ) -> bool {
        if candidate.account_id != self.snapshot.account_id
            || candidate.account_epoch != self.snapshot.account_epoch
            || candidate.session_id != self.snapshot.session_id
            || candidate.revision <= self.snapshot.revision
        {
            x251_mark(|| {
                format!(
                    "x251 presentation publish refused revision={} current={} same_account={}",
                    candidate.revision,
                    self.snapshot.revision,
                    candidate.account_id == self.snapshot.account_id
                )
            });
            return false;
        }
        self.snapshot = candidate;
        cx.notify();
        true
    }

    pub fn deactivate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot.account_id != account_id
            || self.snapshot.account_epoch != account_epoch
            || self.snapshot.session_id != session_id
        {
            return;
        }
        self.snapshot = RisePostContentPresentationSnapshot::empty();
        cx.notify();
    }

    /// sync fence for an account switch
    /// old domain shuts down async and its feed pages must not stay on screen under the new account
    pub fn fence_account_transition(&mut self, cx: &mut Context<Self>) {
        self.snapshot = RisePostContentPresentationSnapshot::empty();
        cx.notify();
    }

    /// any scope by its own key for a caller handed a scope it didnt build
    /// every list accessor drops posts of people this account blocked
    pub fn page<'a>(&'a self, scope: &RisePostPageScope, cx: &'a App) -> Cow<'a, RisePostPagePresentation> {
        let page = self.snapshot.pages_by_scope_key.get(&scope.key()).unwrap_or(&IDLE_PAGE);
        if matches!(scope, RisePostPageScope::Detail { .. }) {
            return Cow::Borrowed(page);
        }
        RiseBlockedContent::visible_page(Cow::Borrowed(page), cx)
    }

    pub fn feed_page<'a>(&'a self, kind: RisePostFeedKind, cx: &'a App) -> Cow<'a, RisePostPagePresentation> {
        let page = self
            .snapshot
            .pages_by_scope_key
            .get(&RisePostPageScope::Feed(kind).key())
            .unwrap_or(&IDLE_PAGE);
        RiseBlockedContent::visible_page(Cow::Borrowed(page), cx)
    }

    pub fn profile_page<'a>(
        &'a self,
        user_id: &str,
        kind: RisePostProfileKind,
        cx: &'a App,
    ) -> Cow<'a, RisePostPagePresentation> {
        let prefix = format!("profile|{}:{}|", user_id.len(), user_id);
        let suffix = format!("|{}", kind.wire());
        let page = self
            .snapshot
            .pages_by_scope_key
            .iter()
            .find(|(key, _)| key.starts_with(&prefix) && key.ends_with(&suffix))
            .map(|(_, page)| page)
            .unwrap_or(&IDLE_PAGE);
        RiseBlockedContent::visible_page(Cow::Borrowed(page), cx)
    }

    pub fn profile_page_with_tag<'a>(
        &'a self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
        cx: &'a App,
    ) -> Cow<'a, RisePostPagePresentation> {
        let scope = RisePostPageScope::Profile { user_id: user_id.to_owned(), tag: tag.to_owned(), kind };
        let page = self.snapshot.pages_by_scope_key.get(&scope.key()).unwrap_or(&IDLE_PAGE);
        RiseBlockedContent::visible_page(Cow::Borrowed(page), cx)
    }

    pub fn profile_page_by_tag<'a>(
        &'a self,
        tag: &str,
        kind: RisePostProfileKind,
        cx: &'a App,
    ) -> Cow<'a, RisePostPagePresentation> {
        let lowercased = tag.to_lowercase();
        let component = format!("|{}:{}|", lowercased.len(), lowercased);
        let suffix = format!("|{}", kind.wire());
        let page = self
            .snapshot
            .pages_by_scope_key
            .iter()
            .find(|(key, _)| key.starts_with("profile|") && key.contains(&component) && key.ends_with(&suffix))
            .map(|(_, page)| page)
            .unwrap_or(&IDLE_PAGE);
        RiseBlockedContent::visible_page(Cow::Borrowed(page), cx)
    }

    pub fn search_post_page<'a>(
        &'a self,
        query: &str,
        kind: RisePostSearchKind,
        cx: &'a App,
    ) -> Cow<'a, RisePostPagePresentation> {
        let scope = RisePostPageScope::Search { query: query.to_owned(), kind };
        let page = self.snapshot.pages_by_scope_key.get(&scope.key()).unwrap_or(&IDLE_PAGE);
        RiseBlockedContent::visible_page(Cow::Borrowed(page), cx)
    }

    pub fn detail_page(&self, post_id: &str) -> &RisePostPagePresentation {
        let scope = RisePostPageScope::Detail { post_id: post_id.to_owned() };
        self.snapshot.pages_by_scope_key.get(&scope.key()).unwrap_or(&IDLE_PAGE)
    }

    pub fn collaborators(&self, post_id: &str) -> &PostCollaboratorsSnapshot {
        self.snapshot.collaborators_by_post_id.get(post_id).unwrap_or(&EMPTY_COLLABORATORS)
    }

    pub fn resolved_post(&self, post_id: &str) -> Option<&GetPostFeedResponse> {
        if let Some(detail) = self.detail_page(post_id).items.first() {
            return Some(detail);
        }
        self.snapshot
            .pages_by_scope_key
            .values()
            .find_map(|page| {
                page.items
                    .iter()
                    .find(|post| post.id == post_id || post.presentation_id() == post_id)
            })
    }
}

fn x251_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X251", "{}", stage());
}
