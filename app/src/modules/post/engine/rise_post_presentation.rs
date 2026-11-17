use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPresentationSnapshot, RisePostStatsPresentationState,
};

pub struct RisePostPresentation {
    snapshot: RisePostPresentationSnapshot,
}

impl RisePostPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { snapshot: RisePostPresentationSnapshot::empty() }
    }

    pub fn snapshot(&self) -> &RisePostPresentationSnapshot {
        &self.snapshot
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        self.snapshot = RisePostPresentationSnapshot {
            account_id: account_id.to_owned(),
            account_epoch: account_epoch.to_owned(),
            session_id,
            revision: 0,
            stats_by_post_id: Default::default(),
            view_counts_by_post_id: Default::default(),
        };
        cx.notify();
    }

    pub fn publish(&mut self, candidate: RisePostPresentationSnapshot, cx: &mut Context<Self>) -> bool {
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
        self.snapshot = RisePostPresentationSnapshot::empty();
        cx.notify();
    }

    /// sync fence for an account switch
    pub fn fence_account_transition(&mut self, cx: &mut Context<Self>) {
        self.snapshot = RisePostPresentationSnapshot::empty();
        cx.notify();
    }

    pub fn stats(&self, post_id: &str) -> Option<&RisePostStatsPresentationState> {
        self.snapshot.stats_by_post_id.get(post_id)
    }

    pub fn view_count(&self, post_id: &str) -> Option<i64> {
        self.snapshot.view_counts_by_post_id.get(post_id).copied()
    }
}
