use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::search::engine::rise_search_engine_models::{
    RiseGlobalSearchCategory, RiseGlobalSearchPagePresentation, RiseGlobalSearchScope, RiseSearchPresentationSnapshot,
};
use crate::modules::user::shared::rise_blocked_content::RiseBlockedContent;

static IDLE_PAGE: LazyLock<RiseGlobalSearchPagePresentation> =
    LazyLock::new(|| RiseGlobalSearchPagePresentation::IDLE);

pub struct RiseSearchPresentation {
    snapshot: RiseSearchPresentationSnapshot,
}

impl RiseSearchPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { snapshot: RiseSearchPresentationSnapshot::empty() }
    }

    pub fn snapshot(&self) -> &RiseSearchPresentationSnapshot {
        &self.snapshot
    }

    pub fn activate(&mut self, account_id: &str, account_epoch: &str, session_id: Uuid, cx: &mut Context<Self>) {
        self.snapshot = RiseSearchPresentationSnapshot {
            account_id: account_id.to_owned(),
            account_epoch: account_epoch.to_owned(),
            session_id,
            revision: 0,
            pages_by_scope_key: HashMap::new(),
        };
        cx.notify();
    }

    pub fn publish(&mut self, candidate: RiseSearchPresentationSnapshot, cx: &mut Context<Self>) -> bool {
        if candidate.account_id != self.snapshot.account_id
            || candidate.account_epoch != self.snapshot.account_epoch
            || candidate.session_id != self.snapshot.session_id
            || candidate.revision <= self.snapshot.revision
        {
            return false;
        }
        self.snapshot = candidate;
        cx.notify();
        true
    }

    /// posts of blocked people are left out, their profiles stay findable so you can unblock
    pub fn page<'a>(
        &'a self,
        query: &str,
        category: RiseGlobalSearchCategory,
        cx: &'a App,
    ) -> Cow<'a, RiseGlobalSearchPagePresentation> {
        let scope = RiseGlobalSearchScope { query: query.to_owned(), category };
        let page = self.snapshot.pages_by_scope_key.get(&scope.key()).unwrap_or(&IDLE_PAGE);
        RiseBlockedContent::visible_search_page(Cow::Borrowed(page), cx)
    }

    pub fn deactivate(&mut self, account_id: &str, account_epoch: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.snapshot.account_id != account_id
            || self.snapshot.account_epoch != account_epoch
            || self.snapshot.session_id != session_id
        {
            return;
        }
        self.snapshot = RiseSearchPresentationSnapshot::empty();
        cx.notify();
    }
}
