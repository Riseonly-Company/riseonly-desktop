use std::path::{Path, PathBuf};

use gpui::AsyncApp;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_engine::rise_upload::rise_chunked_upload::RiseChunkedUploadRequest;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;

/// a picked photo on its way to a chat wallpaper, bytes on disk plus wire metadata
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseChatWallpaperDraft {
    pub local_url: Url,
    pub filename: String,
    pub content_type: String,
}

impl RiseChatWallpaperDraft {
    pub fn new(local_url: Url, filename: String, content_type: String) -> Self {
        Self { local_url, filename, content_type }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RiseChatWallpaperUploadError {
    #[error("The wallpaper upload did not return a file URL.")]
    EmptyRemoteUrl,
}

impl RiseChatWallpaperUploadError {
    pub fn error_description(&self) -> Option<String> {
        Some(self.to_string())
    }
}

/// wallpaper image upload over the chunked client, same support as album photos and post media
pub enum RiseChatWallpaperUpload {}

impl RiseChatWallpaperUpload {
    pub const UPLOAD_CONTEXT: &'static str = "theme_wallpaper";

    /// main thread call, the caller hops here through RiseEngineMainHop
    pub async fn upload(draft: &RiseChatWallpaperDraft, cx: &mut AsyncApp) -> anyhow::Result<String> {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        RiseConversationUploadSupport::prepare_authentication(
            account_user_id.clone(),
            account_epoch.clone(),
            cx,
        )
        .await?;
        let client = cx.update(|cx| {
            RiseConversationUploadSupport::make_client(
                account_user_id.clone(),
                account_epoch.clone(),
                None,
                cx,
            )
        });

        let Some(local_path) = draft.local_url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let mut original =
            RiseOutgoingMediaFile::new(local_path, draft.filename.clone(), draft.content_type.clone());
        original.file_context = Some(Self::UPLOAD_CONTEXT.to_owned());
        let compressed = OutgoingMediaCompressor::compress_if_needed(
            original.clone(),
            None,
            MediaCompressionConfiguration::wallpaper(),
            None,
            None,
        )
        .await?;
        let _staged = RiseChatWallpaperStagedCopy {
            url: (compressed.url.standardized() != draft.local_url.standardized())
                .then(|| compressed.url.clone()),
        };
        let prepared = RiseConversationUploadSupport::prepared_local_file(&original, &compressed);
        let Some(compressed_path) = compressed.url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let mut request = RiseChunkedUploadRequest::new(
            compressed_path,
            prepared.filename.clone(),
            compressed.content_type.clone(),
            Self::UPLOAD_CONTEXT.to_owned(),
        );
        request.resource_id = None;
        let result = client.upload(request, None, None, None).await?;
        let remote_url = trimming_whitespaces_and_newlines(&result.file_url).to_owned();
        if remote_url.is_empty() {
            return Err(RiseChatWallpaperUploadError::EmptyRemoteUrl.into());
        }

        RiseConversationUploadSupport::seed_prepared_image_cache_if_possible(
            &compressed,
            &remote_url,
            account_user_id,
            account_epoch,
        )
        .await;

        Ok(remote_url)
    }

    /// picker exports into the temp dir and nothing else holds those bytes
    pub fn discard_staged_file(url: &Url) {
        if !url.is_file_url() {
            return;
        }
        let Some(path) = url.to_file_path() else {
            return;
        };
        let temporary = Self::resolving_symlinks_in_path(&temporary_directory());
        if !Self::resolving_symlinks_in_path(&path).starts_with(&temporary) {
            return;
        }
        let _ = FileManager.remove_item(&path);
    }

    /// resolves symlinks in a path, a path that doesnt exist comes back unchanged
    fn resolving_symlinks_in_path(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }
}

struct RiseChatWallpaperStagedCopy {
    url: Option<Url>,
}

impl Drop for RiseChatWallpaperStagedCopy {
    fn drop(&mut self) {
        if let Some(url) = self.url.take() {
            RiseChatWallpaperUpload::discard_staged_file(&url);
        }
    }
}
