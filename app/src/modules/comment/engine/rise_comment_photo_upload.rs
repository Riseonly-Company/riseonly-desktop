use std::path::Path;
use std::sync::Arc;

use gpui::AsyncApp;
use rise_canon::RiseUploadContextFormats;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::rise_upload::rise_chunked_upload::{RiseChunkedUploadRequest, RiseChunkedUploadTransferProgress};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentMedia, CommentMediaType, CommentPhotoUploadProgress,
};
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentPhotoUploadResult {
    pub file_url: String,
    pub width: i64,
    pub height: i64,
}

impl CommentMedia {
    pub(crate) fn from_photo(photo: &CommentPhotoUploadResult) -> Self {
        Self {
            width: photo.width,
            height: photo.height,
            ..Self::new(CommentMediaType::Photo, photo.file_url.clone())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CommentPhotoUploadError {
    #[error("The selected item is not a photo.")]
    UnsupportedItem,
    #[error("The upload did not return a file URL.")]
    EmptyRemoteUrl,
}

impl CommentPhotoUploadError {
    pub fn error_description(&self) -> Option<String> {
        Some(self.to_string())
    }
}

pub type CommentPhotoUploadOnProgress = Arc<dyn Fn(CommentPhotoUploadProgress) + Send + Sync>;

/// comment photo upload
/// reuses the post upload pipeline, compression from chat outgoing media 1200x800 and 200 KiB
/// transport from the engine, only the file context is its own
pub enum RiseCommentPhotoUpload {}

impl RiseCommentPhotoUpload {
    pub const UPLOAD_CONTEXT: &'static str = RiseUploadContextFormats::COMMENT.context;

    /// main thread call, the caller runs it on the foreground executor with its async app
    pub async fn upload(
        item: &MediaPickerItem,
        request_id: &str,
        on_progress: CommentPhotoUploadOnProgress,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<CommentPhotoUploadResult> {
        if !matches!(item.kind, MediaPickerItemKind::Image) || item.is_file {
            return Err(CommentPhotoUploadError::UnsupportedItem.into());
        }

        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        RiseConversationUploadSupport::prepare_authentication(account_id.clone(), account_epoch.clone(), cx).await?;
        let client = cx.update(|cx| {
            RiseConversationUploadSupport::make_client(account_id.clone(), account_epoch.clone(), None, cx)
        });

        let filename = if item.suggested_file_name.is_empty() {
            format!("comment-photo-{request_id}.jpg")
        } else {
            item.suggested_file_name.clone()
        };
        let Some(local_path) = item.url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let content_type = Self::content_type(&filename);
        let mut local = RiseOutgoingMediaFile::new(local_path, filename, content_type.to_owned());
        local.file_context = Some(Self::UPLOAD_CONTEXT.to_owned());

        let compressing = on_progress.clone();
        let compressed = OutgoingMediaCompressor::compress_if_needed(
            local.clone(),
            None,
            MediaCompressionConfiguration::chat(),
            Some(Arc::new(move |fraction: f64| compressing(CommentPhotoUploadProgress::Compressing(fraction)))),
            None,
        )
        .await?;
        let prepared = RiseConversationUploadSupport::prepared_local_file(&local, &compressed);
        let Some(compressed_path) = compressed.url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };

        let uploading = on_progress.clone();
        let result = client
            .upload(
                RiseChunkedUploadRequest::new(
                    compressed_path,
                    prepared.filename.clone(),
                    compressed.content_type.clone(),
                    Self::UPLOAD_CONTEXT.to_owned(),
                ),
                None,
                None,
                Some(Arc::new(move |progress: RiseChunkedUploadTransferProgress| {
                    uploading(CommentPhotoUploadProgress::Uploading {
                        completed_bytes: progress.completed_bytes,
                        total_bytes: progress.total_bytes,
                    });
                })),
            )
            .await?;
        let remote_url = trimming_whitespaces_and_newlines(&result.file_url).to_owned();
        if remote_url.is_empty() {
            return Err(CommentPhotoUploadError::EmptyRemoteUrl.into());
        }

        RiseConversationUploadSupport::seed_prepared_image_cache_if_possible(
            &compressed,
            &remote_url,
            account_id,
            account_epoch,
        )
        .await;
        on_progress(CommentPhotoUploadProgress::Uploading {
            completed_bytes: compressed.byte_count,
            total_bytes: compressed.byte_count,
        });

        Ok(CommentPhotoUploadResult {
            file_url: remote_url,
            width: result.width.or(compressed.pixel_width).unwrap_or(0),
            height: result.height.or(compressed.pixel_height).unwrap_or(0),
        })
    }

    fn content_type(filename: &str) -> &'static str {
        let extension = Path::new(filename)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        match extension.as_str() {
            "png" => "image/png",
            "heic" => "image/heic",
            "heif" => "image/heif",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => "image/jpeg",
        }
    }
}
