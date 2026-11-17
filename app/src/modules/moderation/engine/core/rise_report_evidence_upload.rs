use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use gpui::AsyncApp;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::rise_upload::rise_chunked_upload::{
    RiseChunkedUploadProgressHandler, RiseChunkedUploadRequest, RiseChunkedUploadStep,
    RiseChunkedUploadTransferProgress,
};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::{
    BugReportAttachment, BugReportAttachmentKind, BugReportUploadError, BugReportUploadedAttachment,
};
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::moderation::engine::rise_moderation_rpc::RiseModerationError;
use crate::modules::moderation::engine::rise_moderation_submitter::RiseModerationSubmissionOwner;

const COMPRESSION_SHARE: f64 = 0.35;
const MILLISECONDS_PER_SECOND: i64 = 1000;

static MAXIMUM_ATTACHMENTS: LazyLock<i64> = LazyLock::new(|| RiseLimits::value("report_evidence_attachments"));
static MAXIMUM_VIDEO_DURATION_MS: LazyLock<i64> = LazyLock::new(|| RiseLimits::value("bug_report_video_duration"));

/// progress callback for evidence upload, fraction as f64
pub type RiseReportEvidenceProgress = Arc<dyn Fn(f64) + Send + Sync>;

/// uploads report and appeal evidence through the chunked client
/// context is report_evidence so objects land under the reporters prefix
/// report-service can then check the reporter owns them
pub enum RiseReportEvidenceUpload {}

impl RiseReportEvidenceUpload {
    pub const UPLOAD_CONTEXT: &'static str = "report_evidence";

    pub fn maximum_attachments() -> i64 {
        *MAXIMUM_ATTACHMENTS
    }

    pub fn maximum_video_duration_ms() -> i64 {
        *MAXIMUM_VIDEO_DURATION_MS
    }

    /// main thread call
    pub async fn upload(
        attachment: &BugReportAttachment,
        owner: &RiseModerationSubmissionOwner,
        on_progress: RiseReportEvidenceProgress,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<BugReportUploadedAttachment> {
        cx.update(|cx| owner.validate(cx))?;
        if attachment.kind == BugReportAttachmentKind::Video
            && attachment.duration_ms.is_some_and(|duration| duration > Self::maximum_video_duration_ms())
        {
            return Err(RiseModerationError::VideoTooLong.into());
        }

        let account_id = owner.account_id.clone();
        let account_epoch = owner.account_epoch.clone();
        RiseConversationUploadSupport::prepare_authentication(account_id.clone(), account_epoch.clone(), cx).await?;
        cx.update(|cx| owner.validate(cx))?;
        let additional_validation = Self::additional_validation(owner.clone(), cx);
        let client = cx.update(|cx| {
            RiseConversationUploadSupport::make_client(account_id, account_epoch, Some(additional_validation), cx)
        });

        let Some(local_url) = attachment.url.as_ref() else {
            return Err(BugReportUploadError::NothingToUpload.into());
        };
        let Some(local_path) = local_url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let mut local =
            RiseOutgoingMediaFile::new(local_path, attachment.filename.clone(), attachment.content_type.clone());
        local.file_context = Some(Self::UPLOAD_CONTEXT.to_owned());
        let compression_progress: RiseReportEvidenceProgress = {
            let on_progress = on_progress.clone();
            Arc::new(move |fraction: f64| on_progress(COMPRESSION_SHARE.min(fraction * COMPRESSION_SHARE)))
        };
        let compressed = OutgoingMediaCompressor::compress_if_needed(
            local.clone(),
            None,
            MediaCompressionConfiguration::evidence(),
            Some(compression_progress),
            None,
        )
        .await?;
        cx.update(|cx| owner.validate(cx))?;
        let prepared = RiseConversationUploadSupport::prepared_local_file(&local, &compressed);
        let Some(compressed_path) = compressed.url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let request = RiseChunkedUploadRequest::new(
            compressed_path,
            prepared.filename.clone(),
            compressed.content_type.clone(),
            Self::UPLOAD_CONTEXT.to_owned(),
        );
        let transfer_progress: RiseChunkedUploadProgressHandler = {
            let on_progress = on_progress.clone();
            Arc::new(move |progress: RiseChunkedUploadTransferProgress| {
                on_progress(COMPRESSION_SHARE + progress.fraction * (1.0 - COMPRESSION_SHARE));
            })
        };
        let result = client.upload(request, None, None, Some(transfer_progress)).await?;

        cx.update(|cx| owner.validate(cx))?;
        let remote_url = trimming_whitespaces_and_newlines(&result.file_url).to_owned();
        if remote_url.is_empty() {
            return Err(BugReportUploadError::EmptyRemoteURL.into());
        }
        on_progress(1.0);

        let file_id = result
            .file_id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or_default()
            .to_owned();
        Ok(BugReportUploadedAttachment {
            file_url: remote_url,
            file_id: if file_id.is_empty() { result.asset_id.clone() } else { file_id },
            duration_ms: attachment
                .duration_ms
                .or_else(|| result.duration.map(|duration| duration * MILLISECONDS_PER_SECOND)),
        })
    }

    /// chunked client asks from its own tasks
    /// so each check hops to the main thread where the owner reads the registry
    fn additional_validation(owner: RiseModerationSubmissionOwner, cx: &mut AsyncApp) -> RiseChunkedUploadStep {
        let hop = cx.update(RiseEngineMainHop::shared);
        Arc::new(move || {
            let owner = owner.clone();
            hop.run(move |cx: AsyncApp| async move { cx.update(|cx| owner.validate(cx)) }.boxed_local())
        })
    }
}
