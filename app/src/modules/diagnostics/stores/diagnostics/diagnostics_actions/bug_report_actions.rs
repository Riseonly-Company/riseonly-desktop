use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;

use crate::modules::diagnostics::engine::core::rise_bug_report_upload::RiseBugReportProgress;
use crate::modules::diagnostics::engine::rise_bug_report_submitter::{BugReportDraftPayload, RiseBugReportSubmitter};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_types::BugReportDraftKind;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_wire_types::BugReportRecord;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_services::bug_report_services::BugReportServicesStore;

pub struct BugReportActionsStore {
    submission: Option<Task<()>>,
    submission_token: Option<Uuid>,
    listing: HashMap<BugReportDraftKind, Task<()>>,
    listing_tokens: HashMap<BugReportDraftKind, Uuid>,
}

impl BugReportActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            submission: None,
            submission_token: None,
            listing: HashMap::new(),
            listing_tokens: HashMap::new(),
        }
    }

    fn release_submission(&mut self, token: Uuid) {
        if self.submission_token != Some(token) {
            return;
        }
        self.submission_token = None;
        self.submission = None;
    }

    fn is_cancellation<T>(outcome: &anyhow::Result<T>) -> bool {
        outcome.as_ref().err().is_some_and(|error| error.is::<CancellationError>())
    }

    fn progress_forwarder(
        on_progress: impl Fn(f64, &mut App) + 'static,
        cx: &AsyncApp,
    ) -> (RiseBugReportProgress, Task<()>) {
        let (sender, mut receiver) = mpsc::unbounded::<f64>();
        let forwarding = cx.spawn(async move |cx| {
            while let Some(fraction) = receiver.next().await {
                cx.update(|cx| on_progress(fraction, cx));
            }
        });
        let progress: RiseBugReportProgress = Arc::new(move |fraction: f64| {
            let _ = sender.unbounded_send(fraction);
        });
        (progress, forwarding)
    }

    pub fn submit_action(
        &mut self,
        payload: BugReportDraftPayload,
        on_progress: impl Fn(f64, &mut App) + 'static,
        on_finished: impl FnOnce(anyhow::Result<bool>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.submission.is_some() {
            return;
        }

        let token = Uuid::new_v4();
        self.submission_token = Some(token);
        self.submission = Some(cx.spawn(async move |this, cx| {
            let (progress, _forwarding) = Self::progress_forwarder(on_progress, cx);
            let outcome = RiseBugReportSubmitter::shared().submit(payload, progress, cx).await;
            if !Self::is_cancellation(&outcome) {
                cx.update(|cx| on_finished(outcome.map(|response| response.duplicate), cx));
            }
            let _ = this.update(cx, |this, _| this.release_submission(token));
        }));
    }

    pub fn update_action(
        &mut self,
        report_id: String,
        payload: BugReportDraftPayload,
        on_progress: impl Fn(f64, &mut App) + 'static,
        on_finished: impl FnOnce(anyhow::Result<Option<BugReportRecord>>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.submission.is_some() {
            return;
        }

        let token = Uuid::new_v4();
        self.submission_token = Some(token);
        self.submission = Some(cx.spawn(async move |this, cx| {
            let (progress, _forwarding) = Self::progress_forwarder(on_progress, cx);
            let outcome = RiseBugReportSubmitter::shared().update(report_id, payload, progress, cx).await;
            if !Self::is_cancellation(&outcome) {
                cx.update(|cx| on_finished(outcome, cx));
            }
            let _ = this.update(cx, |this, _| this.release_submission(token));
        }));
    }

    pub fn cancel_submission_action(&mut self) {
        self.submission = None;
        self.submission_token = None;
    }

    pub fn load_my_reports_action(
        &mut self,
        kind: BugReportDraftKind,
        offset: i64,
        on_finished: impl FnOnce(anyhow::Result<(Vec<BugReportRecord>, bool)>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let token = Uuid::new_v4();
        self.listing_tokens.insert(kind, token);
        let task = cx.spawn(async move |this, cx| {
            let outcome = RiseBugReportSubmitter::shared()
                .my_reports(kind, BugReportServicesStore::MY_REPORTS_PAGE_SIZE, offset, cx)
                .await;
            if !Self::is_cancellation(&outcome) {
                cx.update(|cx| on_finished(outcome, cx));
            }
            let _ = this.update(cx, |this, _| {
                if this.listing_tokens.get(&kind) == Some(&token) {
                    this.listing_tokens.remove(&kind);
                    this.listing.remove(&kind);
                }
            });
        });
        self.listing.insert(kind, task);
    }

    pub fn delete_action(
        &mut self,
        report_id: String,
        on_finished: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |_, cx| {
            let outcome = RiseBugReportSubmitter::shared().delete(report_id, cx).await;
            cx.update(|cx| on_finished(outcome, cx));
        })
        .detach();
    }
}
