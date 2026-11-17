use std::fmt;
use std::sync::{Arc, LazyLock};

use gpui::AsyncApp;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_engine::rise_upload::rise_chunked_upload::{
    RiseChunkedUploadProgressHandler, RiseChunkedUploadRequest, RiseChunkedUploadTransferProgress,
};
use rise_i18n::tr;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;

const COMPRESSION_SHARE: f64 = 0.35;
const MILLISECONDS_PER_SECOND: i64 = 1000;

static MAXIMUM_VIDEO_DURATION_MS: LazyLock<i64> = LazyLock::new(|| RiseLimits::value("bug_report_video_duration"));
static MAXIMUM_SCREENSHOTS: LazyLock<i64> = LazyLock::new(|| RiseLimits::value("bug_report_screenshots"));
static MAXIMUM_PAGES: LazyLock<i64> = LazyLock::new(|| RiseLimits::value("bug_report_surfaces"));

/// progress callback taking a fraction
pub type RiseBugReportProgress = Arc<dyn Fn(f64) + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BugReportAttachmentKind {
    Screenshot,
    Video,
}

/// one attachment as the sheet holds it before upload
/// a picker item carries editor state so the boundary to the submitter is a file on disk
/// plus metadata
#[derive(Clone, Debug, PartialEq)]
pub struct BugReportAttachment {
    pub id: String,
    pub kind: BugReportAttachmentKind,
    pub url: Option<Url>,
    pub filename: String,
    pub content_type: String,
    pub duration_ms: Option<i64>,
    pub preview_url: Option<Url>,
    pub was_trimmed_to_ceiling: bool,
    pub uploaded: Option<BugReportUploadedAttachment>,
}

impl BugReportAttachment {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        kind: BugReportAttachmentKind,
        url: Option<Url>,
        filename: String,
        content_type: String,
        duration_ms: Option<i64>,
        preview_url: Option<Url>,
        was_trimmed_to_ceiling: bool,
        uploaded: Option<BugReportUploadedAttachment>,
    ) -> Self {
        Self { id, kind, url, filename, content_type, duration_ms, preview_url, was_trimmed_to_ceiling, uploaded }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BugReportUploadedAttachment {
    pub file_url: String,
    pub file_id: String,
    pub duration_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BugReportUploadError {
    EmptyRemoteURL,
    VideoTooLong,
    NothingToUpload,
}

impl BugReportUploadError {
    pub fn error_description(&self) -> Option<String> {
        match self {
            BugReportUploadError::EmptyRemoteURL | BugReportUploadError::NothingToUpload => {
                Some(tr("bug_report_failed"))
            }
            BugReportUploadError::VideoTooLong => Some(tr("bug_report_video_too_long")),
        }
    }
}

impl fmt::Display for BugReportUploadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for BugReportUploadError {}

/// uploads bug report attachments through the app chunked client
/// no transport of its own, the conversation upload support owns auth and account checks
/// only the bug_report context is local
pub enum RiseBugReportUpload {}

impl RiseBugReportUpload {
    pub const UPLOAD_CONTEXT: &'static str = "bug_report";

    pub fn maximum_video_duration_ms() -> i64 {
        *MAXIMUM_VIDEO_DURATION_MS
    }

    pub fn maximum_screenshots() -> i64 {
        *MAXIMUM_SCREENSHOTS
    }

    pub fn maximum_pages() -> i64 {
        *MAXIMUM_PAGES
    }

    /// main thread call
    pub async fn upload(
        attachment: &BugReportAttachment,
        on_progress: RiseBugReportProgress,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<BugReportUploadedAttachment> {
        if attachment.kind == BugReportAttachmentKind::Video
            && attachment.duration_ms.is_some_and(|duration| duration > Self::maximum_video_duration_ms())
        {
            return Err(BugReportUploadError::VideoTooLong.into());
        }

        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        RiseConversationUploadSupport::prepare_authentication(account_id.clone(), account_epoch.clone(), cx).await?;
        let client =
            cx.update(|cx| RiseConversationUploadSupport::make_client(account_id, account_epoch, None, cx));

        let Some(local_url) = attachment.url.as_ref() else {
            return Err(BugReportUploadError::NothingToUpload.into());
        };
        let Some(local_path) = local_url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let mut local =
            RiseOutgoingMediaFile::new(local_path, attachment.filename.clone(), attachment.content_type.clone());
        local.file_context = Some(Self::UPLOAD_CONTEXT.to_owned());
        let compression_progress: RiseBugReportProgress = {
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

        let remote_url = trimming_whitespaces_and_newlines(&result.file_url).to_owned();
        if remote_url.is_empty() {
            return Err(BugReportUploadError::EmptyRemoteURL.into());
        }
        on_progress(1.0);

        let file_id = result.file_id.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default();
        Ok(BugReportUploadedAttachment {
            file_url: remote_url,
            file_id: if file_id.is_empty() { result.asset_id.clone() } else { file_id.to_owned() },
            duration_ms: attachment
                .duration_ms
                .or_else(|| result.duration.map(|duration| duration * MILLISECONDS_PER_SECOND)),
        })
    }
}
