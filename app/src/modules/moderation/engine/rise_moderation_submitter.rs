use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::{App, AsyncApp};
use rise_canon::{RiseRequestJSON, rise_request};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseWireError};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::{
    BugReportAttachment, BugReportUploadedAttachment,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportAttachmentFactory;
use crate::modules::moderation::engine::core::rise_report_evidence_upload::RiseReportEvidenceUpload;
use crate::modules::moderation::engine::rise_moderation_domain::{RiseModerationDomain, RiseModerationDomainRegistry};
use crate::modules::moderation::engine::rise_moderation_presentation::RiseModerationPresentation;
use crate::modules::moderation::engine::rise_moderation_rpc::{
    ModerationEvidence, RiseModerationEndpoint, RiseModerationEndpoints, RiseModerationError,
};
use crate::modules::moderation::engine::rise_moderation_wire::{AppealDTO, ReportRowDTO, RiseStoredModerationNote};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{ReportReason, ReportTarget};

const READY_POLL_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Clone, PartialEq)]
pub struct ReportDraftPayload {
    pub draft_id: String,
    pub target: ReportTarget,
    pub reason: ReportReason,
    pub description: String,
    pub attachments: Vec<BugReportAttachment>,
    pub include_context: bool,
}

#[derive(Clone, PartialEq)]
pub struct AppealDraftPayload {
    pub draft_id: String,
    pub block_id: String,
    pub report_id: Option<i64>,
    pub block_type: Option<String>,
    pub text: String,
    pub attachments: Vec<BugReportAttachment>,
}

#[derive(Clone, PartialEq)]
pub struct NoteDraftPayload {
    pub draft_id: String,
    pub report_id: i64,
    pub text: String,
    pub attachments: Vec<BugReportAttachment>,
}

/// captured before launching the task so an upload never picks a newer account or session
#[derive(Clone)]
pub struct RiseModerationSubmissionOwner {
    pub account_id: Option<String>,
    pub account_epoch: Option<String>,
    pub domain: Option<Arc<RiseModerationDomain>>,
    pub session_id: Option<Uuid>,
}

impl RiseModerationSubmissionOwner {
    /// captures the current account and session
    pub fn new(cx: &mut App) -> Self {
        let domain = RiseModerationDomainRegistry::shared(cx).read(cx).active().cloned();
        Self {
            account_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            session_id: domain.as_ref().map(|domain| domain.session_id),
            domain,
        }
    }

    fn with_domain(domain: Arc<RiseModerationDomain>) -> Self {
        Self {
            account_id: Some(domain.account_id.clone()),
            account_epoch: Some(domain.account_epoch.clone()),
            session_id: Some(domain.session_id),
            domain: Some(domain),
        }
    }

    /// enough to speak about a request the server already accepted
    /// the engine session may be rebuilt during evidence upload and the report exists anyway
    pub fn is_same_account(&self) -> bool {
        let account_id = self.account_id.as_deref().filter(|account_id| !account_id.is_empty());
        let account_epoch = self.account_epoch.as_deref().filter(|account_epoch| !account_epoch.is_empty());
        let (Some(account_id), Some(account_epoch)) = (account_id, account_epoch) else {
            return false;
        };
        SaiAccountCacheScope::is_current_account_epoch(Some(account_id), Some(account_epoch))
    }

    pub fn is_current(&self, cx: &mut App) -> bool {
        if !self.is_same_account() {
            return false;
        }
        let Some(domain) = &self.domain else {
            return true;
        };
        let registry = RiseModerationDomainRegistry::shared(cx);
        let is_installed = registry.read(cx).active().is_some_and(|active| Arc::ptr_eq(active, domain));
        let presentation = RiseModerationPresentation::shared(cx);
        let presentation = presentation.read(cx);
        self.account_id.as_deref() == Some(domain.account_id.as_str())
            && self.account_epoch.as_deref() == Some(domain.account_epoch.as_str())
            && is_installed
            && presentation.is_active()
            && presentation.account_id() == self.account_id.as_deref()
            && presentation.account_epoch() == self.account_epoch.as_deref()
            && presentation.session_id() == self.session_id
    }

    /// a cancelled task here is a dropped future which never reaches this check
    pub fn validate(&self, cx: &mut App) -> anyhow::Result<()> {
        if !self.is_current(cx) {
            return Err(CancellationError.into());
        }
        Ok(())
    }

    pub async fn prepared(self, cx: &mut AsyncApp) -> anyhow::Result<Self> {
        cx.update(|cx| self.validate(cx))?;
        if self.domain.is_some() {
            return Ok(self);
        }
        let ready = cx
            .update(|cx| RiseModerationDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx)))
            .await?;
        cx.update(|cx| self.validate(cx))?;
        if self.account_id.as_deref() != Some(ready.account_id.as_str())
            || self.account_epoch.as_deref() != Some(ready.account_epoch.as_str())
        {
            return Err(CancellationError.into());
        }
        let owner = Self::with_domain(ready);
        cx.update(|cx| owner.validate(cx))?;
        Ok(owner)
    }
}

/// defer guard, runs once when the scope ends
/// on success, error or a dropped future
struct RiseModerationDefer<F: FnOnce()> {
    body: Option<F>,
}

impl<F: FnOnce()> RiseModerationDefer<F> {
    fn new(body: F) -> Self {
        Self { body: Some(body) }
    }
}

impl<F: FnOnce()> Drop for RiseModerationDefer<F> {
    fn drop(&mut self) {
        if let Some(body) = self.body.take() {
            body();
        }
    }
}

struct RiseModerationSubmitterState {
    in_flight: HashSet<String>,
    uploaded: HashMap<String, HashMap<String, BugReportUploadedAttachment>>,
}

/// uploads the evidence then files the report or appeal, in flight set stops a double tap filing twice
/// owns the draft temp files since the sheet closes on tap
/// removed once filed or dropped
pub struct RiseModerationSubmitter {
    state: RiseIsolated<RiseModerationSubmitterState>,
}

static SHARED: LazyLock<RiseModerationSubmitter> = LazyLock::new(|| RiseModerationSubmitter {
    state: RiseIsolated::new(RiseModerationSubmitterState { in_flight: HashSet::new(), uploaded: HashMap::new() }),
});

impl RiseModerationSubmitter {
    pub fn shared() -> &'static Self {
        &SHARED
    }

    /// main thread call, owner checks read the registry and presentation between steps
    pub async fn submit_report(
        &'static self,
        draft: ReportDraftPayload,
        owner: RiseModerationSubmissionOwner,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<ReportRowDTO> {
        if !self.begin(&draft.draft_id) {
            return Err(RiseModerationError::AlreadySubmitting.into());
        }
        let draft_id = draft.draft_id.clone();
        let _in_flight = RiseModerationDefer::new(move || self.end(&draft_id));

        let owner = owner.prepared(cx).await?;
        let Some(domain) = owner.domain.clone() else {
            return Err(RiseModerationError::MissingActiveSession.into());
        };
        let evidence = self.upload_all(&draft.attachments, &draft.draft_id, &owner, cx).await?;
        Self::until_ready_for_calls(&domain, &owner, RiseModerationEndpoints::create_report(), cx).await?;
        cx.update(|cx| owner.validate(cx))?;

        let (file_ids, file_urls) = Self::evidence_columns(&evidence);
        let mut request = rise_request::report::CreateReport {
            description: Some(draft.description.clone()),
            evidence_file_ids: file_ids.moderation_evidence(),
            evidence_file_urls: file_urls.moderation_evidence(),
            include_context: (draft.include_context && draft.target.supports_context()).then_some(true),
            reason: Some(draft.reason.wire().to_owned()),
            report_type: Some(draft.target.report_type().to_owned()),
            ..rise_request::report::CreateReport::default()
        };
        draft.target.apply(&mut request);

        let report = Self::on_account_runtime({
            let domain = domain.clone();
            async move { domain.repository.create_report(request).await }
        })
        .await?;
        self.drop_files(&draft);
        if !owner.is_same_account() {
            return Err(CancellationError.into());
        }
        Ok(report)
    }

    pub async fn submit_appeal(
        &'static self,
        draft: AppealDraftPayload,
        owner: RiseModerationSubmissionOwner,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Option<AppealDTO>> {
        if !self.begin(&draft.draft_id) {
            return Err(RiseModerationError::AlreadySubmitting.into());
        }
        let draft_id = draft.draft_id.clone();
        let _in_flight = RiseModerationDefer::new(move || self.end(&draft_id));

        let owner = owner.prepared(cx).await?;
        let Some(domain) = owner.domain.clone() else {
            return Err(RiseModerationError::MissingActiveSession.into());
        };
        let evidence = self.upload_all(&draft.attachments, &draft.draft_id, &owner, cx).await?;
        Self::until_ready_for_calls(&domain, &owner, RiseModerationEndpoints::create_appeal(), cx).await?;
        cx.update(|cx| owner.validate(cx))?;

        let (file_ids, file_urls) = Self::evidence_columns(&evidence);
        let request = rise_request::report::CreateAppeal {
            block_type: draft.block_type.clone(),
            evidence_file_ids: file_ids.moderation_evidence(),
            evidence_file_urls: file_urls.moderation_evidence(),
            report_id: draft.report_id,
            text: Some(draft.text.clone()),
        };
        let appeal = Self::on_account_runtime({
            let domain = domain.clone();
            async move { domain.repository.create_appeal(request).await }
        })
        .await?;
        cx.update(|cx| owner.validate(cx))?;
        self.state.with(|state| state.uploaded.remove(&draft.draft_id));
        Ok(appeal)
    }

    pub async fn submit_note(
        &'static self,
        draft: NoteDraftPayload,
        owner: RiseModerationSubmissionOwner,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<RiseStoredModerationNote> {
        if !self.begin(&draft.draft_id) {
            return Err(RiseModerationError::AlreadySubmitting.into());
        }
        let draft_id = draft.draft_id.clone();
        let files = Self::temporary_files(&draft.attachments);
        let _in_flight = RiseModerationDefer::new(move || {
            self.state.with(|state| {
                state.in_flight.remove(&draft_id);
                state.uploaded.remove(&draft_id);
            });
            BugReportAttachmentFactory::remove_temporary_files(&files);
        });

        let owner = owner.prepared(cx).await?;
        let Some(domain) = owner.domain.clone() else {
            return Err(RiseModerationError::MissingActiveSession.into());
        };
        let evidence = self.upload_all(&draft.attachments, &draft.draft_id, &owner, cx).await?;
        Self::until_ready_for_calls(&domain, &owner, RiseModerationEndpoints::add_report_note(), cx).await?;
        cx.update(|cx| owner.validate(cx))?;

        let (file_ids, file_urls) = Self::evidence_columns(&evidence);
        let request = rise_request::report::AddReportNote {
            evidence_file_ids: Some(file_ids.into_iter().map(RiseRequestJSON::String).collect()),
            evidence_file_urls: Some(file_urls.into_iter().map(RiseRequestJSON::String).collect()),
            report_id: Some(draft.report_id),
            text: Some(draft.text.clone()),
        };
        let note = Self::on_account_runtime({
            let domain = domain.clone();
            async move { domain.repository.add_note(request).await }
        })
        .await?;
        cx.update(|cx| owner.validate(cx))?;
        Ok(note)
    }

    /// upload survives a trip to the background but the mutation cant, the wire refuses it while suspended
    /// nothing was sent yet so waiting for foreground is no replay
    async fn until_ready_for_calls(
        domain: &Arc<RiseModerationDomain>,
        owner: &RiseModerationSubmissionOwner,
        endpoint: &RiseModerationEndpoint,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        let deadline = RiseClock::monotonic() + endpoint.timeout;
        loop {
            let engine = domain.engine.clone();
            let is_ready = Self::on_account_runtime(async move { engine.is_ready_for_calls(false).await }).await?;
            if is_ready {
                return Ok(());
            }
            if !owner.is_same_account() {
                return Err(CancellationError.into());
            }
            if cx.update(|cx| RiseTransportStatus::is_network_path_down(cx)) {
                return Err(RiseWireError::NotConnected.into());
            }
            if RiseClock::monotonic() >= deadline {
                return Err(RiseWireError::NotConnected.into());
            }
            cx.background_executor().timer(READY_POLL_INTERVAL).await;
        }
    }

    /// draft was thrown away, nothing will reference its uploads
    pub fn forget(&self, draft_id: &str) {
        self.state.with(|state| state.uploaded.remove(draft_id));
    }

    /// a failed report keeps its files and finished uploads so the retry toast can resend as is
    /// this drops them once filed or when nobody will retry
    pub fn discard_report(&self, draft: &ReportDraftPayload) {
        if self.state.with(|state| state.in_flight.contains(&draft.draft_id)) {
            return;
        }
        self.drop_files(draft);
    }

    fn drop_files(&self, draft: &ReportDraftPayload) {
        self.state.with(|state| state.uploaded.remove(&draft.draft_id));
        BugReportAttachmentFactory::remove_temporary_files(&Self::temporary_files(&draft.attachments));
    }

    /// sequential on purpose, ten concurrent chunked sessions on a poor connection is how the report fails
    /// no progress reported since the sheet is already closed
    /// a fraction per chunk would invalidate the moderation screens
    async fn upload_all(
        &self,
        attachments: &[BugReportAttachment],
        draft_id: &str,
        owner: &RiseModerationSubmissionOwner,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Vec<BugReportUploadedAttachment>> {
        cx.update(|cx| owner.validate(cx))?;
        let ceiling = RiseReportEvidenceUpload::maximum_attachments();
        if i64::try_from(attachments.len()).unwrap_or(i64::MAX) > ceiling {
            return Err(RiseModerationError::TooManyAttachments.into());
        }
        let mut results = Vec::with_capacity(attachments.len());
        for attachment in attachments {
            cx.update(|cx| owner.validate(cx))?;
            let memo = self.state.with(|state| {
                state.uploaded.get(draft_id).and_then(|uploads| uploads.get(&attachment.id)).cloned()
            });
            let result = if let Some(remote) = &attachment.uploaded {
                remote.clone()
            } else if let Some(memo) = memo {
                memo
            } else {
                let result =
                    RiseReportEvidenceUpload::upload(attachment, owner, Arc::new(|_: f64| {}), cx).await?;
                self.state.with(|state| {
                    state
                        .uploaded
                        .entry(draft_id.to_owned())
                        .or_default()
                        .insert(attachment.id.clone(), result.clone());
                });
                result
            };
            cx.update(|cx| owner.validate(cx))?;
            results.push(result);
        }
        Ok(results)
    }

    fn begin(&self, draft_id: &str) -> bool {
        self.state.with(|state| state.in_flight.insert(draft_id.to_owned()))
    }

    fn end(&self, draft_id: &str) {
        self.state.with(|state| state.in_flight.remove(draft_id));
    }

    fn evidence_columns(evidence: &[BugReportUploadedAttachment]) -> (Vec<String>, Vec<String>) {
        let file_ids = evidence.iter().map(|uploaded| uploaded.file_id.clone()).collect();
        let file_urls = evidence.iter().map(|uploaded| uploaded.file_url.clone()).collect();
        (file_ids, file_urls)
    }

    fn temporary_files(attachments: &[BugReportAttachment]) -> Vec<Url> {
        attachments
            .iter()
            .flat_map(|attachment| [attachment.url.clone(), attachment.preview_url.clone()])
            .flatten()
            .collect()
    }

    /// engine and repository calls run on the account runtime where the wire lives
    /// caller stays on the main thread between them
    async fn on_account_runtime<T: Send + 'static>(
        operation: impl Future<Output = anyhow::Result<T>> + Send + 'static,
    ) -> anyhow::Result<T> {
        RiseAccountRuntime::current().spawn(operation).await.map_err(anyhow::Error::new)?
    }
}
