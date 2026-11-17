use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::BackgroundExecutor;
use rise_engine::CancellationError;
use rise_limits::OutgoingMediaProfile;
use rise_platform::file_manager::FileManager;
use rise_platform::uniform_type::UTType;

use crate::modules::share_extension::entry::share_extension_models::{ShareAttachment, ShareAttachmentKind};
use crate::modules::share_extension::entry::share_temporary_file_store::ShareFileName;
use crate::modules::share_extension::shared::rise_outgoing_media::{
    RiseOutgoingImageEncoder, RiseOutgoingVideoTranscoder, RiseOutgoingVideoTranscoderProgress,
    outgoing_media_profile,
};

const PROFILE_NAME: &str = "chat";
const GIF_MIME_TYPE: &str = "image/gif";
const JPEG_MIME_TYPE: &str = "image/jpeg";
const JPEG_EXTENSION: &str = "jpg";
const MP4_MIME_TYPE: &str = "video/mp4";
const MP4_EXTENSION: &str = "mp4";
const OUTPUT_PREFIX: &str = "ro-share-";
const FALLBACK_STEM: &str = "media";

/// prepares media before upload
pub enum ShareOutgoingMediaPreparer {}

impl ShareOutgoingMediaPreparer {
    fn profile() -> &'static OutgoingMediaProfile {
        outgoing_media_profile(PROFILE_NAME, false)
    }

    /// upload ready attachment, documents audio and gif pass unchanged
    /// photos and videos get a new file in the session dir, deleted on finish
    /// dropping the future cancels the work
    pub async fn prepare(
        executor: BackgroundExecutor,
        attachment: ShareAttachment,
        directory: PathBuf,
        on_progress: RiseOutgoingVideoTranscoderProgress,
    ) -> anyhow::Result<ShareAttachment> {
        let Some(source_url) = attachment.local_file_url.clone() else {
            return Ok(attachment);
        };
        match attachment.kind {
            ShareAttachmentKind::Image => {
                if attachment.mime_type.to_lowercase() == GIF_MIME_TYPE {
                    return Ok(attachment);
                }
                Self::prepare_image(executor, attachment, source_url, &directory).await
            }
            ShareAttachmentKind::Video => {
                Self::prepare_video(executor, attachment, source_url, &directory, on_progress).await
            }
            ShareAttachmentKind::Audio
            | ShareAttachmentKind::File
            | ShareAttachmentKind::Link
            | ShareAttachmentKind::Text => Ok(attachment),
        }
    }

    async fn prepare_image(
        executor: BackgroundExecutor,
        attachment: ShareAttachment,
        source_url: PathBuf,
        directory: &Path,
    ) -> anyhow::Result<ShareAttachment> {
        let profile = Self::profile();
        let output_url = directory.join(format!(
            "{OUTPUT_PREFIX}{}.{JPEG_EXTENSION}",
            attachment.id.uuid_string()
        ));
        let mut guard = ShareImageEncodeGuard {
            cancelled: Arc::new(AtomicBool::new(false)),
            output_url: Some(output_url.clone()),
        };
        let cancelled = Arc::clone(&guard.cancelled);
        let encode_output_url = output_url.clone();
        let encoded: anyhow::Result<i64> = executor
            .spawn(async move {
                let is_cancelled = || -> anyhow::Result<()> {
                    if cancelled.load(Ordering::SeqCst) {
                        return Err(CancellationError.into());
                    }
                    Ok(())
                };
                let encoded = RiseOutgoingImageEncoder::encode_jpeg::<anyhow::Error>(
                    &source_url,
                    profile,
                    &is_cancelled,
                )?;
                write_atomically(&encode_output_url, &encoded.data)?;
                Ok(i64::try_from(encoded.data.len()).unwrap_or(i64::MAX))
            })
            .await;
        let byte_count = match encoded {
            Ok(byte_count) => byte_count,
            Err(error) if error.downcast_ref::<CancellationError>().is_some() => {
                return Err(CancellationError.into());
            }
            Err(_) => return Ok(attachment),
        };
        guard.disarm();
        Ok(Self::replacement(
            &attachment,
            output_url,
            JPEG_MIME_TYPE,
            JPEG_EXTENSION,
            byte_count,
        ))
    }

    async fn prepare_video(
        executor: BackgroundExecutor,
        attachment: ShareAttachment,
        source_url: PathBuf,
        directory: &Path,
        on_progress: RiseOutgoingVideoTranscoderProgress,
    ) -> anyhow::Result<ShareAttachment> {
        let output_url = directory.join(format!(
            "{OUTPUT_PREFIX}{}.{MP4_EXTENSION}",
            attachment.id.uuid_string()
        ));
        let output = match RiseOutgoingVideoTranscoder::transcode_if_needed(
            executor,
            source_url,
            *Self::profile(),
            output_url.clone(),
            Some(on_progress),
            None,
        )
        .await
        {
            Ok(output) => output,
            Err(error) if error.downcast_ref::<CancellationError>().is_some() => {
                return Err(CancellationError.into());
            }
            Err(_) => {
                let _ = FileManager.remove_item(&output_url);
                return Ok(attachment);
            }
        };
        let Some(output) = output else {
            return Ok(attachment);
        };
        Ok(Self::replacement(
            &attachment,
            output.url,
            MP4_MIME_TYPE,
            MP4_EXTENSION,
            output.byte_count,
        ))
    }

    /// keeps id and order since upload cache and progress are keyed by id, only the extension changes
    fn replacement(
        attachment: &ShareAttachment,
        url: PathBuf,
        mime_type: &str,
        file_extension: &str,
        byte_count: i64,
    ) -> ShareAttachment {
        let source_name = attachment.original_file_name.clone().unwrap_or_else(|| {
            url.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        let stem = ShareFileName::deleting_path_extension(&source_name);
        let name = format!("{}.{file_extension}", if stem.is_empty() { FALLBACK_STEM } else { stem });
        let content_type_identifier = UTType::from_mime_type(mime_type)
            .map(|content_type| content_type.identifier())
            .unwrap_or_else(|| attachment.content_type_identifier.clone());
        ShareAttachment::new(
            attachment.id,
            attachment.kind,
            content_type_identifier,
            mime_type.to_owned(),
            Some(name),
            Some(url),
            attachment.text.clone(),
            Some(byte_count),
            attachment.source_index,
        )
    }
}

fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut temporary = url.as_os_str().to_owned();
    temporary.push(".partial");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, data)?;
    std::fs::rename(&temporary, url).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

struct ShareImageEncodeGuard {
    cancelled: Arc<AtomicBool>,
    output_url: Option<PathBuf>,
}

impl ShareImageEncodeGuard {
    fn disarm(&mut self) {
        self.output_url = None;
    }
}

impl Drop for ShareImageEncodeGuard {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(output_url) = self.output_url.take() {
            let _ = FileManager.remove_item(&output_url);
        }
    }
}
