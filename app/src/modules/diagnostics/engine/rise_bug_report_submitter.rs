use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::sync::{Arc, LazyLock};

use gpui::AsyncApp;
use rise_canon::{RiseRequestJSON, rise_request, rise_response};
use rise_core::RiseIsolated;
use rise_engine::RiseEngine;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;

use crate::core::diagnostics::rise_bug_report_envelope::RiseBugReportEnvelope;
use crate::core::diagnostics::rise_diagnostics_rpc::{
    RiseBugReportFields, RiseDiagnosticsLocale, RiseDiagnosticsRPC, SubmitDiagnosticReportDiagnostics,
};
use crate::core::lib::device_info::DeviceInfo;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::{
    BugReportAttachment, BugReportAttachmentKind, BugReportUploadedAttachment, RiseBugReportProgress,
    RiseBugReportUpload,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_types::BugReportDraftKind;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_wire_types::BugReportRecord;
use crate::modules::notify::engine::core::rise_release_notes_seen_store::RiseReleaseNotesDeviceContext;

const UPLOAD_SHARE: f64 = 0.9;

/// a finished report ready to leave the device
/// client_report_id is minted when the sheet opens not on send so a retry carries the same
/// key or the report is stored twice
#[derive(Clone, Debug, PartialEq)]
pub struct BugReportDraftPayload {
    pub client_report_id: String,
    pub kind: String,
    pub feedback_kind: Option<String>,
    pub parent_report_id: Option<String>,
    pub message: String,
    pub pages: Vec<String>,
    pub component: Option<String>,
    pub screenshots: Vec<BugReportAttachment>,
    pub video: Option<BugReportAttachment>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BugReportSubmitError {
    NoEngine,
    AlreadySubmitting,
}

impl BugReportSubmitError {
    pub fn error_description(&self) -> Option<String> {
        match self {
            BugReportSubmitError::NoEngine | BugReportSubmitError::AlreadySubmitting => {
                Some(tr("bug_report_failed"))
            }
        }
    }
}

impl fmt::Display for BugReportSubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for BugReportSubmitError {}

pub trait UpdateDiagnosticReportDraft: Sized {
    /// whole content replacement of own report, not a patch
    /// same fields as submission minus idempotency key, arrays always sent
    /// feedback_kind is required on a suggestion, absent on a fault
    fn draft(
        report_id: String,
        draft: &BugReportDraftPayload,
        screenshots: &[BugReportUploadedAttachment],
        video: Option<&BugReportUploadedAttachment>,
    ) -> Self;
}

impl UpdateDiagnosticReportDraft for rise_request::report::UpdateDiagnosticReport {
    fn draft(
        report_id: String,
        draft: &BugReportDraftPayload,
        screenshots: &[BugReportUploadedAttachment],
        video: Option<&BugReportUploadedAttachment>,
    ) -> Self {
        Self {
            message: draft.message.clone(),
            report_id,
            component: draft.component.clone(),
            feedback_kind: draft.feedback_kind.clone(),
            pages: Some(draft.pages.iter().cloned().map(RiseRequestJSON::String).collect()),
            screenshot_file_ids: Some(
                screenshots.iter().map(|uploaded| RiseRequestJSON::String(uploaded.file_id.clone())).collect(),
            ),
            screenshot_file_urls: Some(
                screenshots.iter().map(|uploaded| RiseRequestJSON::String(uploaded.file_url.clone())).collect(),
            ),
            video_duration_ms: video.and_then(|uploaded| uploaded.duration_ms),
            video_file_id: video.map(|uploaded| uploaded.file_id.clone()),
            video_file_url: video.map(|uploaded| uploaded.file_url.clone()),
        }
    }
}

struct RiseBugReportSubmitterState {
    in_flight: HashSet<String>,
    uploaded: HashMap<String, HashMap<String, BugReportUploadedAttachment>>,
}

/// dropping the send future is how a caller cancels it and that must free the key like a
/// finished send
struct RiseBugReportInFlight<'a> {
    state: &'a RiseIsolated<RiseBugReportSubmitterState>,
    client_report_id: String,
}

impl Drop for RiseBugReportInFlight<'_> {
    fn drop(&mut self) {
        self.state.with(|state| state.in_flight.remove(&self.client_report_id));
    }
}

struct SubmissionContext {
    engine: Arc<RiseEngine>,
    socket_connected: bool,
}

/// uploads the attachments, builds the payload and sends the report
/// uploads, json encoding and the rpc stay off the drawing thread
/// the in flight set lives here so a double tap on send cant start a second upload
pub struct RiseBugReportSubmitter {
    state: RiseIsolated<RiseBugReportSubmitterState>,
}

static SHARED: LazyLock<RiseBugReportSubmitter> = LazyLock::new(|| RiseBugReportSubmitter {
    state: RiseIsolated::new(RiseBugReportSubmitterState { in_flight: HashSet::new(), uploaded: HashMap::new() }),
});

impl RiseBugReportSubmitter {
    pub fn shared() -> &'static Self {
        &SHARED
    }

    /// uploads what is attached, then submits
    /// on_progress reports one fraction, uploads own the first 90 percent and the rpc the last 10
    /// so the bar never sits at 100 while the wire is open
    pub async fn submit(
        &self,
        draft: BugReportDraftPayload,
        on_progress: RiseBugReportProgress,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<rise_response::report::SubmitDiagnosticReport> {
        let _in_flight = self.begin(&draft.client_report_id)?;
        let (uploaded_screenshots, uploaded_video) = self.upload_attachments(&draft, on_progress.clone(), cx).await?;
        on_progress(UPLOAD_SHARE);

        let context = Self::submission_context(cx)?;
        let client_report_id = draft.client_report_id.clone();
        let response = Self::on_account_runtime(async move {
            let envelope = RiseBugReportEnvelope::capture(context.socket_connected);
            let payload = envelope.encoded_payload()?;
            let request = rise_request::report::SubmitDiagnosticReport::diagnostic(
                draft.kind.clone(),
                payload,
                RiseReleaseNotesDeviceContext::app_version(),
                RiseReleaseNotesDeviceContext::build_number(),
                DeviceInfo::system_version(),
                DeviceInfo::display_name(),
                RiseDiagnosticsLocale::current(),
                Some(envelope.captured_at_ms),
                Some(&RiseBugReportFields {
                    client_report_id: draft.client_report_id.clone(),
                    parent_report_id: draft.parent_report_id.clone(),
                    message: draft.message.clone(),
                    pages: draft.pages.clone(),
                    component: draft.component.clone(),
                    screenshot_file_urls: uploaded_screenshots.iter().map(|item| item.file_url.clone()).collect(),
                    screenshot_file_ids: uploaded_screenshots.iter().map(|item| item.file_id.clone()).collect(),
                    video_file_url: uploaded_video.as_ref().map(|item| item.file_url.clone()),
                    video_file_id: uploaded_video.as_ref().map(|item| item.file_id.clone()),
                    video_duration_ms: uploaded_video.as_ref().and_then(|item| item.duration_ms),
                    feedback_kind: draft.feedback_kind.clone(),
                }),
            );
            RiseDiagnosticsRPC::new(context.engine).submit_diagnostic_report(&request).await
        })
        .await?;
        self.forget(&client_report_id);
        on_progress(1.0);
        Ok(response)
    }

    /// replaces the contents of an already submitted report
    /// same upload path as submit, a new attachment is a new object and an existing one travels
    /// by its uploaded and isnt re-uploaded
    pub async fn update(
        &self,
        report_id: String,
        draft: BugReportDraftPayload,
        on_progress: RiseBugReportProgress,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Option<BugReportRecord>> {
        let _in_flight = self.begin(&draft.client_report_id)?;
        let (uploaded_screenshots, uploaded_video) = self.upload_attachments(&draft, on_progress.clone(), cx).await?;
        on_progress(UPLOAD_SHARE);

        let context = Self::submission_context(cx)?;
        let request = rise_request::report::UpdateDiagnosticReport::draft(
            report_id,
            &draft,
            &uploaded_screenshots,
            uploaded_video.as_ref(),
        );
        let response = Self::on_account_runtime(async move {
            RiseDiagnosticsRPC::new(context.engine).update_diagnostic_report(&request).await
        })
        .await?;
        self.forget(&draft.client_report_id);
        on_progress(1.0);
        Ok(response.report.as_ref().and_then(BugReportRecord::new))
    }

    /// withdraws own report, the gateway deletes the attachments itself so nothing to clean up here
    pub async fn delete(&self, report_id: String, cx: &mut AsyncApp) -> anyhow::Result<()> {
        let context = Self::submission_context(cx)?;
        let request = rise_request::report::DeleteDiagnosticReport { report_id };
        Self::on_account_runtime(async move {
            RiseDiagnosticsRPC::new(context.engine).delete_diagnostic_report(&request).await
        })
        .await?;
        Ok(())
    }

    pub async fn my_reports(
        &self,
        kind: BugReportDraftKind,
        limit: i64,
        offset: i64,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<(Vec<BugReportRecord>, bool)> {
        let context = Self::submission_context(cx)?;
        let request = rise_request::report::GetMyDiagnosticReports {
            kind: Some(kind.wire().to_owned()),
            limit: Some(limit),
            offset: Some(offset),
        };
        let response = Self::on_account_runtime(async move {
            RiseDiagnosticsRPC::new(context.engine).get_my_diagnostic_reports(&request).await
        })
        .await?;
        let records = response.reports.iter().filter_map(BugReportRecord::new).collect();
        Ok((records, response.has_more))
    }

    /// draft abandoned, its upload memo can never be referenced again
    pub fn forget(&self, client_report_id: &str) {
        self.state.with(|state| state.uploaded.remove(client_report_id));
    }

    fn begin(&self, client_report_id: &str) -> anyhow::Result<RiseBugReportInFlight<'_>> {
        if !self.state.with(|state| state.in_flight.insert(client_report_id.to_owned())) {
            return Err(BugReportSubmitError::AlreadySubmitting.into());
        }
        Ok(RiseBugReportInFlight { state: &self.state, client_report_id: client_report_id.to_owned() })
    }

    /// sequential not concurrent, ten chunked sessions at once over a bad connection is how the
    /// report itself starts failing
    async fn upload_attachments(
        &self,
        draft: &BugReportDraftPayload,
        on_progress: RiseBugReportProgress,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<(Vec<BugReportUploadedAttachment>, Option<BugReportUploadedAttachment>)> {
        let attachments: Vec<&BugReportAttachment> = draft.screenshots.iter().chain(draft.video.iter()).collect();
        let total = attachments.len().max(1) as f64;
        let mut uploaded_screenshots = Vec::new();
        let mut uploaded_video = None;

        for (index, attachment) in attachments.into_iter().enumerate() {
            let base = index as f64;
            let memo = self.state.with(|state| {
                state.uploaded.get(&draft.client_report_id).and_then(|uploads| uploads.get(&attachment.id)).cloned()
            });
            let result = if let Some(remote) = attachment.uploaded.clone() {
                on_progress(((base + 1.0) / total) * UPLOAD_SHARE);
                remote
            } else if let Some(memo) = memo {
                on_progress(((base + 1.0) / total) * UPLOAD_SHARE);
                memo
            } else {
                let attachment_progress: RiseBugReportProgress = {
                    let on_progress = on_progress.clone();
                    Arc::new(move |fraction: f64| on_progress(((base + fraction) / total) * UPLOAD_SHARE))
                };
                let result = RiseBugReportUpload::upload(attachment, attachment_progress, cx).await?;
                self.state.with(|state| {
                    state
                        .uploaded
                        .entry(draft.client_report_id.clone())
                        .or_default()
                        .insert(attachment.id.clone(), result.clone());
                });
                result
            };
            match attachment.kind {
                BugReportAttachmentKind::Screenshot => uploaded_screenshots.push(result),
                BugReportAttachmentKind::Video => uploaded_video = Some(result),
            }
        }
        Ok((uploaded_screenshots, uploaded_video))
    }

    fn submission_context(cx: &mut AsyncApp) -> anyhow::Result<SubmissionContext> {
        cx.update(|cx| -> anyhow::Result<SubmissionContext> {
            let engine = RiseEngineAppRuntime::shared(cx).read(cx).engine().ok_or(BugReportSubmitError::NoEngine)?;
            let socket_connected = Sai::ws(cx).read(cx).is_connected();
            Ok(SubmissionContext { engine, socket_connected })
        })
    }

    /// engine calls run on the account runtime where the wire lives, the caller stays on the
    /// main thread between them
    async fn on_account_runtime<T: Send + 'static>(
        operation: impl Future<Output = anyhow::Result<T>> + Send + 'static,
    ) -> anyhow::Result<T> {
        RiseAccountRuntime::current().spawn(operation).await.map_err(anyhow::Error::new)?
    }
}
