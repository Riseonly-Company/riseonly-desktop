use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_character::{Character, characters};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_file_name::{file_name_length, host_file_name};
use rise_core::foundation::foundation_file_url::{PathFlavor, file_path_string, is_local_absolute_path};
use rise_core::foundation::foundation_text::{character_boundaries, character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::core::engine_bridge::network_transport_config::NetworkClientSlot;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode, URLRequest};
use rise_i18n::tr;
use rise_platform::file_manager::{CocoaError, DirectoryEnumerationOptions, FileManager};
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::paths::temporary_directory;
use rise_platform::system_share::ShareItem;
use rise_platform::uniform_type::UTType;
use rise_widgets::overlay::blocking_loader_store::BlockingLoaderStore;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheError, RemoteMediaDiskCacheMaterializeOptions, RemoteMediaHttpResponse,
    RiseMediaRedirectSessionDelegate,
};
use crate::core::media::rise_media_transfer_hub::RiseMediaCacheIdentity;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::core::widgets::sheets::share_sheet::{SharePayload, SystemShare};
use crate::modules::chat::shared::utils::share_file_stem_hooks::ShareFileStem;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageMediaItem, MessageMediaKind, RemoteMediaURL,
};
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;

static UNCACHED_DOWNLOAD_SESSION: NetworkClientSlot = NetworkClientSlot::without_redirects();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMessageExternalShare {
    pub items: Vec<ShareItem>,
    pub temporary_directory: Option<PathBuf>,
}

impl PreparedMessageExternalShare {
    /// blocking file io, run it off the main thread
    pub fn cleanup(&self) {
        let Some(temporary_directory) = self.temporary_directory.as_ref() else {
            return;
        };
        let _ = FileManager.remove_item(temporary_directory);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMessageMediaFiles {
    pub files: Vec<PathBuf>,
    pub temporary_directory: Option<PathBuf>,
}

impl PreparedMessageMediaFiles {
    /// blocking file io, run it off the main thread
    pub fn cleanup(&self) {
        let Some(temporary_directory) = self.temporary_directory.as_ref() else {
            return;
        };
        let _ = FileManager.remove_item(temporary_directory);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MessageExternalShareError {
    #[error("invalid messages")]
    InvalidMessages,
    #[error("invalid source")]
    InvalidSource,
    #[error("invalid response")]
    InvalidResponse,
    #[error("payload too large")]
    PayloadTooLarge,
    #[error("account changed")]
    AccountChanged,
    #[error("empty payload")]
    EmptyPayload,
    #[error("presentation unavailable")]
    PresentationUnavailable,
}

struct MaterializedMedia {
    url: PathBuf,
    byte_count: i64,
}

/// where one attachment bytes come from, a local file or a remote address
enum MessageExternalShareSource {
    Local(PathBuf),
    Remote(Url),
}

/// staging dir and the account a materialization started under
struct MessageExternalShareMaterializeContext {
    directory: PathBuf,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

/// removes a path it owns on drop unless disarmed
/// a cancelled prepare drops at its next await so cleanup lives here
struct MessageExternalShareStagedPath {
    path: PathBuf,
    armed: bool,
}

impl MessageExternalShareStagedPath {
    fn new(path: PathBuf, armed: bool) -> Self {
        Self { path, armed }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for MessageExternalShareStagedPath {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let path = std::mem::take(&mut self.path);
        RiseAccountRuntime::current().spawn_blocking(move || {
            let _ = FileManager.remove_item(&path);
        });
    }
}

pub enum MessageExternalShareMaterializer {}

impl MessageExternalShareMaterializer {
    const STAGING_PREFIX: &'static str = "riseonly-message-share-";
    const DOWNLOAD_PREFIX: &'static str = "riseonly-message-share-download-";
    const DOWNLOAD_EXTENSION: &'static str = "download";
    const STALE_DIRECTORY_SECONDS: f64 = 24.0 * 60.0 * 60.0;
    const EXTENSION_CHARACTER_LIMIT: usize = 12;
    const FILE_NAME_LENGTH_LIMIT: usize = 255;
    const FALLBACK_STEM: &'static str = ShareFileStem::FALLBACK;
    const CHAT_AUDIO_TRACK_PREFIX: &'static str = "chat-audio:";
    const UNCACHED_TIMEOUT_SECONDS: u64 = 120;
    const ACCEPT_ANY: &'static str = "*/*";
    const NO_STORE: &'static str = "no-store";
    const JSON_EXTENSION: &'static str = "json";

    pub fn maximum_file_byte_count() -> i64 {
        RiseLimits::value("upload_session_total")
    }

    pub fn maximum_payload_byte_count() -> i64 {
        RiseLimits::value("upload_session_total")
    }

    pub async fn prepare(messages: &[GetMessageMessage], cx: &AsyncApp) -> anyhow::Result<PreparedMessageExternalShare> {
        if messages.is_empty()
            || !messages
                .iter()
                .all(|message| !message.is_system() && !message.protects_content_locally())
        {
            return Err(MessageExternalShareError::InvalidMessages.into());
        }

        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        Self::ensure_current_account(account_user_id.as_deref(), account_epoch.as_deref())?;

        let media_count: usize = messages.iter().map(|message| message.media_items.len()).sum();
        if media_count == 0 {
            let items = SharePayload::Messages(messages.to_vec()).share_items();
            if items.is_empty() {
                return Err(MessageExternalShareError::EmptyPayload.into());
            }
            return Ok(PreparedMessageExternalShare {
                items,
                temporary_directory: None,
            });
        }

        let mut staging = Self::create_staging_directory().await?;
        let context = MessageExternalShareMaterializeContext {
            directory: staging.path.clone(),
            account_user_id,
            account_epoch,
        };

        let mut items: Vec<ShareItem> = Vec::new();
        let mut media_index: i64 = 0;
        let mut materialized_byte_count: i64 = 0;
        for message in messages {
            Self::ensure_current_account(context.account_user_id.as_deref(), context.account_epoch.as_deref())?;

            if let Some(text) = SharePayload::message_share_text(message) {
                items.push(ShareItem::Text(text));
            }
            for media in &message.media_items {
                let remaining_byte_count = Self::maximum_payload_byte_count() - materialized_byte_count;
                if remaining_byte_count <= 0 {
                    return Err(MessageExternalShareError::PayloadTooLarge.into());
                }
                let materialized = Self::materialize(
                    media,
                    media_index,
                    message,
                    Self::maximum_file_byte_count().min(remaining_byte_count),
                    &context,
                    cx,
                )
                .await?;
                items.push(ShareItem::File(materialized.url));
                materialized_byte_count += materialized.byte_count;
                media_index += 1;
            }
        }
        if items.is_empty() {
            return Err(MessageExternalShareError::EmptyPayload.into());
        }
        staging.disarm();
        Ok(PreparedMessageExternalShare {
            items,
            temporary_directory: Some(context.directory),
        })
    }

    /// copies a subset of one message attachments into a private staging dir
    /// reuses share download and auth and cache and account guards, caller owns cleanup
    pub async fn prepare_files(
        message: &GetMessageMessage,
        items: &[MessageMediaItem],
        cx: &AsyncApp,
    ) -> anyhow::Result<PreparedMessageMediaFiles> {
        if items.is_empty() || message.is_system() || message.protects_content_locally() {
            return Err(MessageExternalShareError::InvalidMessages.into());
        }

        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        Self::ensure_current_account(account_user_id.as_deref(), account_epoch.as_deref())?;

        let mut staging = Self::create_staging_directory().await?;
        let context = MessageExternalShareMaterializeContext {
            directory: staging.path.clone(),
            account_user_id,
            account_epoch,
        };

        let mut files: Vec<PathBuf> = Vec::new();
        let mut materialized_byte_count: i64 = 0;
        for (index, media) in items.iter().enumerate() {
            Self::ensure_current_account(context.account_user_id.as_deref(), context.account_epoch.as_deref())?;
            let remaining_byte_count = Self::maximum_payload_byte_count() - materialized_byte_count;
            if remaining_byte_count <= 0 {
                return Err(MessageExternalShareError::PayloadTooLarge.into());
            }
            let materialized = Self::materialize(
                media,
                i64::try_from(index).unwrap_or(i64::MAX),
                message,
                Self::maximum_file_byte_count().min(remaining_byte_count),
                &context,
                cx,
            )
            .await?;
            files.push(materialized.url);
            materialized_byte_count += materialized.byte_count;
        }
        if files.is_empty() {
            return Err(MessageExternalShareError::EmptyPayload.into());
        }
        if files.len() == 1
            && let Some(single) = files.pop()
        {
            let stripped = RiseAccountRuntime::current()
                .spawn_blocking(move || Self::stripping_order_prefix(single))
                .await?;
            files.push(stripped);
        }
        staging.disarm();
        Ok(PreparedMessageMediaFiles {
            files,
            temporary_directory: Some(context.directory),
        })
    }

    fn is_leaked_path_segment_stem(stem: &str) -> bool {
        const KINDS: [&str; 2] = ["image-", "video-"];
        const ROOTS: [&str; 9] = ["Users", "private", "var", "tmp", "Volumes", "Applications", "opt", "home", "mnt"];
        KINDS.iter().any(|kind| stem.strip_prefix(kind).is_some_and(|rest| ROOTS.contains(&rest)))
    }

    pub fn safe_file_name(
        preferred_name: Option<&str>,
        source_url: &Url,
        response: Option<&RemoteMediaHttpResponse>,
        media: &MessageMediaItem,
        index: i64,
    ) -> String {
        let suggested_filename = response.and_then(RemoteMediaHttpResponse::suggested_filename);
        let last_path_component = source_url.last_path_component();
        let candidates = [
            preferred_name,
            suggested_filename.as_deref(),
            Some(last_path_component.as_str()),
        ];
        let raw_name = candidates
            .into_iter()
            .flatten()
            .map(trimming_whitespaces_and_newlines)
            .find(|value| !value.is_empty());

        let raw_extension = raw_name.map(Self::ns_path_extension);
        let mime_extension = UTType::from_mime_type(media.mime_type.as_deref().unwrap_or_default())
            .and_then(|uniform_type| uniform_type.preferred_filename_extension());
        let source_extension = source_url.path_extension();
        let resolved_extension = Self::matching_kind(Self::normalized_extension(raw_extension), media)
            .or_else(|| Self::matching_kind(Self::normalized_extension(mime_extension.as_deref()), media))
            .or_else(|| Self::matching_kind(Self::normalized_extension(Some(source_extension.as_str())), media))
            .unwrap_or_else(|| Self::default_extension(media.kind).to_owned());

        let raw_stem = raw_name.map_or(Self::FALLBACK_STEM, |name| {
            Self::ns_deleting_path_extension(Self::ns_last_path_component(name))
        });
        let raw_stem = if Self::is_leaked_path_segment_stem(raw_stem) { Self::FALLBACK_STEM } else { raw_stem };
        let sanitized_stem = Self::sanitize_file_stem(raw_stem);
        let prefix = format!("{:03}", index.saturating_add(1));
        Self::fitted_file_name(&prefix, &sanitized_stem, &resolved_extension)
    }

    async fn materialize(
        media: &MessageMediaItem,
        index: i64,
        message: &GetMessageMessage,
        maximum_byte_count: i64,
        context: &MessageExternalShareMaterializeContext,
        cx: &AsyncApp,
    ) -> anyhow::Result<MaterializedMedia> {
        if media
            .file_size
            .is_some_and(|declared_byte_count| declared_byte_count > maximum_byte_count)
        {
            return Err(MessageExternalShareError::PayloadTooLarge.into());
        }

        let lookup_media = media.clone();
        let chat_id = message.chat_id.clone();
        let message_id = message.id.clone();
        let lookup_user_id = context.account_user_id.clone();
        let (source, cached_data) = RiseAccountRuntime::current()
            .spawn_blocking(
                move || -> Result<(MessageExternalShareSource, Option<Vec<u8>>), MessageExternalShareError> {
                    let source = match Self::preferred_local_source(
                        &lookup_media,
                        &chat_id,
                        &message_id,
                        lookup_user_id.as_deref(),
                    ) {
                        Some(local) => MessageExternalShareSource::Local(local),
                        None => Self::source_url(&lookup_media)?,
                    };
                    let cached_data = match &source {
                        MessageExternalShareSource::Remote(_) => {
                            Self::cached_image_data(&lookup_media, lookup_user_id.as_deref())
                        }
                        MessageExternalShareSource::Local(_) => None,
                    };
                    Ok((source, cached_data))
                },
            )
            .await??;
        let source_url = match &source {
            MessageExternalShareSource::Local(path) => {
                Url::from_file_path(path).ok_or(MessageExternalShareError::InvalidSource)?
            }
            MessageExternalShareSource::Remote(url) => url.clone(),
        };

        if let Some(data) = cached_data {
            let byte_count = i64::try_from(data.len()).unwrap_or(i64::MAX);
            if byte_count > maximum_byte_count {
                return Err(MessageExternalShareError::PayloadTooLarge.into());
            }
            Self::ensure_current_account(context.account_user_id.as_deref(), context.account_epoch.as_deref())?;
            let file_name = Self::safe_file_name(media.file_name.as_deref(), &source_url, None, media, index);
            let destination = context.directory.join(file_name);
            let written_to = destination.clone();
            RiseAccountRuntime::current()
                .spawn_blocking(move || Self::write_atomically(&written_to, &data))
                .await??;
            return Ok(MaterializedMedia {
                url: destination,
                byte_count,
            });
        }

        let (downloaded, response, removes_downloaded_file) = match &source {
            MessageExternalShareSource::Local(path) => {
                let probed = path.clone();
                let exists = RiseAccountRuntime::current()
                    .spawn_blocking(move || FileManager.file_exists(&probed))
                    .await?;
                if !exists {
                    return Err(MessageExternalShareError::InvalidSource.into());
                }
                (path.clone(), None, false)
            }
            MessageExternalShareSource::Remote(remote) => {
                if !Self::is_http(remote) {
                    return Err(MessageExternalShareError::InvalidSource.into());
                }
                let options = RemoteMediaDiskCacheMaterializeOptions {
                    cache_key: Some(Self::persistent_cache_key(remote)),
                    suggested_filename: media.file_name.clone(),
                    maximum_byte_count: Some(maximum_byte_count),
                    expected_account_user_id: context.account_user_id.clone(),
                    expected_account_epoch: context.account_epoch.clone(),
                    ..RemoteMediaDiskCacheMaterializeOptions::default()
                };
                match RemoteMediaDiskCache::shared().materialize(remote, options, cx).await {
                    Ok(cached) => (cached, None, false),
                    Err(error)
                        if error.downcast_ref::<RemoteMediaDiskCacheError>()
                            == Some(&RemoteMediaDiskCacheError::PayloadTooLarge) =>
                    {
                        let (direct, direct_response) =
                            Self::download_uncached(remote, media, maximum_byte_count, context, cx).await?;
                        (direct, Some(direct_response), true)
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        let _downloaded_removal = MessageExternalShareStagedPath::new(downloaded.clone(), removes_downloaded_file);
        let cache_lease = RemoteMediaDiskCache::lease(Some(&downloaded));

        Self::ensure_current_account(context.account_user_id.as_deref(), context.account_epoch.as_deref())?;
        let measured = downloaded.clone();
        let byte_count = RiseAccountRuntime::current()
            .spawn_blocking(move || Self::byte_count(&measured))
            .await??;
        if byte_count > maximum_byte_count {
            return Err(MessageExternalShareError::PayloadTooLarge.into());
        }

        let file_name = Self::safe_file_name(
            media.file_name.as_deref(),
            &source_url,
            response.as_ref(),
            media,
            index,
        );
        let destination = context.directory.join(file_name);
        let copied_from = downloaded.clone();
        let copied_to = destination.clone();
        RiseAccountRuntime::current()
            .spawn_blocking(move || FileManager.copy_item(&copied_from, &copied_to))
            .await??;
        drop(cache_lease);
        Ok(MaterializedMedia {
            url: destination,
            byte_count,
        })
    }

    /// blocking file io, runs on the account runtime blocking pool
    fn source_url(media: &MessageMediaItem) -> Result<MessageExternalShareSource, MessageExternalShareError> {
        let candidates = [Some(media.url.as_str()), media.local_preview_url.as_deref()];
        for candidate in candidates {
            let Some(raw) = candidate
                .map(trimming_whitespaces_and_newlines)
                .filter(|raw| !raw.is_empty())
            else {
                continue;
            };

            if is_local_absolute_path(raw) {
                let local = PathBuf::from(raw);
                if FileManager.file_exists(&local) {
                    return Ok(MessageExternalShareSource::Local(local));
                }
                continue;
            }
            let normalized = RemoteMediaURL::optional_normalized(Some(raw)).unwrap_or_else(|| raw.to_owned());
            let Some(url) = Url::from_string(&normalized) else {
                continue;
            };
            if url.is_file_url() {
                let local = file_path_string(&url, PathFlavor::HOST).map(PathBuf::from);
                if let Some(local) = local.filter(|local| FileManager.file_exists(local)) {
                    return Ok(MessageExternalShareSource::Local(local));
                }
                continue;
            }
            if Self::is_http(&url) {
                return Ok(MessageExternalShareSource::Remote(url));
            }
        }
        Err(MessageExternalShareError::InvalidSource)
    }

    fn preferred_local_source(
        media: &MessageMediaItem,
        chat_id: &str,
        message_id: &str,
        account_user_id: Option<&str>,
    ) -> Option<PathBuf> {
        if media.kind == MessageMediaKind::Voice
            && let Some(cached) = VoiceMessageDiskCache::cached_url(chat_id, message_id)
        {
            return Some(cached);
        }
        if matches!(media.kind, MessageMediaKind::Audio | MessageMediaKind::Voice)
            && let Some(cached) = MusicPlaybackDiskCache::find_cached_url(
                &format!("{}{message_id}:{}", Self::CHAT_AUDIO_TRACK_PREFIX, media.id),
                account_user_id,
            )
        {
            return Some(cached);
        }
        if matches!(media.kind, MessageMediaKind::Video | MessageMediaKind::VideoNote) {
            return None;
        }
        let raw = media
            .local_preview_url
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|raw| !raw.is_empty())?;
        let local = if is_local_absolute_path(raw) {
            Some(PathBuf::from(raw))
        } else {
            Url::from_string(raw)
                .filter(Url::is_file_url)
                .and_then(|url| file_path_string(&url, PathFlavor::HOST))
                .map(PathBuf::from)
        };
        local.filter(|local| FileManager.file_exists(local))
    }

    fn cached_image_data(media: &MessageMediaItem, account_user_id: Option<&str>) -> Option<Vec<u8>> {
        match media.kind {
            MessageMediaKind::Image | MessageMediaKind::Animation | MessageMediaKind::Gif | MessageMediaKind::Sticker => {}
            MessageMediaKind::Video
            | MessageMediaKind::VideoNote
            | MessageMediaKind::Audio
            | MessageMediaKind::Voice
            | MessageMediaKind::Document => return None,
        }
        let source = RemoteMediaURL::optional_normalized(Some(media.url.as_str()))?;
        if !source.to_lowercase().starts_with("http") {
            return None;
        }
        let data = CleverImageDiskCache::image_data(&source, account_user_id, None)?;
        if data.is_empty() || image::guess_format(&data).is_err() {
            return None;
        }
        Some(data)
    }

    /// one identity per remote object, same as the bubble download chip and media viewer use
    /// keying by message id refetched a downloaded video and wrote a second copy
    fn persistent_cache_key(source: &Url) -> String {
        RiseMediaCacheIdentity::key(source)
    }

    async fn download_uncached(
        source: &Url,
        media: &MessageMediaItem,
        maximum_byte_count: i64,
        context: &MessageExternalShareMaterializeContext,
        cx: &AsyncApp,
    ) -> anyhow::Result<(PathBuf, RemoteMediaHttpResponse)> {
        let mut request = URLRequest::new(source.clone());
        request.timeout_interval = Duration::from_secs(Self::UNCACHED_TIMEOUT_SECONDS);
        request.http_method = reqwest::Method::GET.as_str().to_owned();
        request.set_value(Self::ACCEPT_ANY, reqwest::header::ACCEPT.as_str());
        request.set_value(Self::NO_STORE, reqwest::header::CACHE_CONTROL.as_str());
        if let Some(bearer) = RemoteMediaDiskCache::bearer_token(source, None, cx).await {
            request.set_value(&format!("Bearer {bearer}"), reqwest::header::AUTHORIZATION.as_str());
        }

        let temporary = temporary_directory().join(format!(
            "{}{}.{}",
            Self::DOWNLOAD_PREFIX,
            Uuid::new_v4().uuid_string().to_lowercase(),
            Self::DOWNLOAD_EXTENSION
        ));
        let mut temporary_removal = MessageExternalShareStagedPath::new(temporary.clone(), true);
        let named_candidates: Vec<String> = [media.file_name.clone(), Some(source.last_path_component())]
            .into_iter()
            .flatten()
            .collect();
        let target = temporary.clone();
        let response = RiseAccountRuntime::current()
            .spawn(async move { Self::download(request, &target, maximum_byte_count, named_candidates).await })
            .await??;

        Self::ensure_current_account(context.account_user_id.as_deref(), context.account_epoch.as_deref())?;
        temporary_removal.disarm();
        Ok((temporary, response))
    }

    async fn download(
        request: URLRequest,
        destination: &Path,
        maximum_byte_count: i64,
        named_candidates: Vec<String>,
    ) -> anyhow::Result<RemoteMediaHttpResponse> {
        let session = UNCACHED_DOWNLOAD_SESSION
            .current()
            .ok_or(URLError::new(URLErrorCode::SecureConnectionFailed))?;
        let (mut response, url) = RiseMediaRedirectSessionDelegate::send(&session, &request, None, |_, _| {}).await?;
        let head = RemoteMediaHttpResponse::from_reqwest(&response, url);
        if !(200..300).contains(&head.status_code) || matches!(head.status_code, 204..=206) {
            return Err(MessageExternalShareError::InvalidResponse.into());
        }
        if head.expected_content_length() > maximum_byte_count {
            return Err(MessageExternalShareError::PayloadTooLarge.into());
        }

        let mut writer = BufWriter::new(File::create(destination)?);
        let mut bytes: i64 = 0;
        while let Some(chunk) = response.chunk().await? {
            bytes = bytes.saturating_add(i64::try_from(chunk.len()).unwrap_or(i64::MAX));
            if bytes > maximum_byte_count {
                return Err(MessageExternalShareError::PayloadTooLarge.into());
            }
            writer.write_all(&chunk)?;
        }
        writer.flush()?;
        drop(writer);
        if bytes <= 0 {
            return Err(MessageExternalShareError::InvalidResponse.into());
        }

        let allows_json = named_candidates
            .iter()
            .map(String::as_str)
            .chain(head.suggested_filename().as_deref())
            .any(|name| Self::ns_path_extension(name).to_lowercase() == Self::JSON_EXTENSION);
        let content_type = head
            .value(reqwest::header::CONTENT_TYPE.as_str())
            .map(str::to_lowercase);
        if !RemoteMediaDiskCache::is_supported_payload(destination, content_type.as_deref(), allows_json) {
            return Err(MessageExternalShareError::InvalidResponse.into());
        }
        Ok(head)
    }

    fn byte_count(url: &Path) -> Result<i64, CocoaError> {
        FileManager
            .attributes_of_item(url)
            .map(|attributes| attributes.size.max(0))
    }

    async fn create_staging_directory() -> anyhow::Result<MessageExternalShareStagedPath> {
        let directory = RiseAccountRuntime::current()
            .spawn_blocking(|| -> Result<PathBuf, CocoaError> {
                Self::cleanup_stale_directories();
                let directory = temporary_directory().join(format!(
                    "{}{}",
                    Self::STAGING_PREFIX,
                    Uuid::new_v4().uuid_string().to_lowercase()
                ));
                FileManager.create_directory(&directory, true)?;
                Ok(directory)
            })
            .await??;
        Ok(MessageExternalShareStagedPath::new(directory, true))
    }

    fn cleanup_stale_directories() {
        let Ok(entries) = FileManager.contents_of_directory_at(
            &temporary_directory(),
            DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES,
        ) else {
            return;
        };

        let expiration = RiseClock::now().adding_time_interval(-Self::STALE_DIRECTORY_SECONDS);
        for entry in entries {
            let is_staging = entry
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(Self::STAGING_PREFIX));
            if !is_staging {
                continue;
            }
            let modified = FileManager
                .attributes_of_item(&entry)
                .ok()
                .and_then(|attributes| attributes.modification_date)
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|elapsed| Date::from_time_interval_since_1970(elapsed.as_secs_f64()));
            if modified.is_some_and(|modified| modified < expiration) {
                let _ = FileManager.remove_item(&entry);
            }
        }
    }

    fn stripping_order_prefix(url: PathBuf) -> PathBuf {
        let Some(name) = url.file_name().and_then(|name| name.to_str()).map(str::to_owned) else {
            return url;
        };
        let name_characters: Vec<Character<'_>> = characters(&name).collect();
        if name_characters.len() <= 4
            || !name_characters[..3].iter().all(|character| character.is_number())
            || name_characters[3].as_str() != "-"
        {
            return url;
        }
        let stripped: String = name_characters[4..].iter().map(|character| character.as_str()).collect();
        if stripped.is_empty() {
            return url;
        }
        let Some(parent) = url.parent() else {
            return url;
        };
        let destination = parent.join(&stripped);
        if FileManager.file_exists(&destination) || FileManager.move_item(&url, &destination).is_err() {
            return url;
        }
        destination
    }

    pub fn sanitize_file_stem(raw: &str) -> String {
        ShareFileStem::sanitize(raw)
    }

    fn normalized_extension(raw: Option<&str>) -> Option<String> {
        let raw = raw?;
        let lowered = trimming_whitespaces_and_newlines(raw).to_lowercase();
        let value: String = characters(&lowered)
            .filter(|character| character.is_letter() || character.is_number())
            .map(|character| character.as_str())
            .collect();
        if value.is_empty() {
            return None;
        }
        Some(character_prefix(&value, Self::EXTENSION_CHARACTER_LIMIT).to_owned())
    }

    /// sender file name can promise another format than the attachment is (video named IMG_0004.jpg)
    /// photo library judges a staged file by its extension
    fn matching_kind(file_extension: Option<String>, media: &MessageMediaItem) -> Option<String> {
        let file_extension = file_extension?;
        let family = match media.kind {
            MessageMediaKind::Video | MessageMediaKind::VideoNote => UTType::movie(),
            MessageMediaKind::Image => UTType::image(),
            MessageMediaKind::Animation
            | MessageMediaKind::Gif
            | MessageMediaKind::Audio
            | MessageMediaKind::Voice
            | MessageMediaKind::Document
            | MessageMediaKind::Sticker => return Some(file_extension),
        };
        let uniform_type = UTType::from_filename_extension(&file_extension)?;
        uniform_type.conforms_to(&family).then_some(file_extension)
    }

    fn default_extension(kind: MessageMediaKind) -> &'static str {
        match kind {
            MessageMediaKind::Image => "jpg",
            MessageMediaKind::Video | MessageMediaKind::VideoNote => "mp4",
            MessageMediaKind::Animation | MessageMediaKind::Gif => "gif",
            MessageMediaKind::Audio => "m4a",
            MessageMediaKind::Voice => "ogg",
            MessageMediaKind::Document => "bin",
            MessageMediaKind::Sticker => "webp",
        }
    }

    fn fitted_file_name(prefix: &str, stem: &str, file_extension: &str) -> String {
        let host_stem = host_file_name(stem, PathFlavor::HOST);
        let host_stem = if host_stem.is_empty() {
            Self::FALLBACK_STEM.to_owned()
        } else {
            host_stem
        };
        for end in character_boundaries(&host_stem).into_iter().rev() {
            if end == 0 {
                break;
            }
            let candidate = format!("{prefix}-{}.{file_extension}", &host_stem[..end]);
            if file_name_length(&candidate, PathFlavor::HOST) <= Self::FILE_NAME_LENGTH_LIMIT {
                return candidate;
            }
        }
        format!("{prefix}-{}.{file_extension}", Self::FALLBACK_STEM)
    }

    fn ns_last_path_component(path: &str) -> &str {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            return if path.is_empty() { "" } else { "/" };
        }
        trimmed.rsplit('/').next().unwrap_or(trimmed)
    }

    fn ns_split_extension(leaf: &str) -> (&str, &str) {
        match leaf.rfind('.') {
            Some(dot) if dot > 0 => (&leaf[..dot], &leaf[dot + 1..]),
            _ => (leaf, ""),
        }
    }

    fn ns_path_extension(path: &str) -> &str {
        Self::ns_split_extension(Self::ns_last_path_component(path)).1
    }

    fn ns_deleting_path_extension(leaf: &str) -> &str {
        Self::ns_split_extension(leaf).0
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut temporary = url.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, url).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }

    fn is_http(url: &Url) -> bool {
        let scheme = url.scheme().map(str::to_lowercase);
        matches!(scheme.as_deref(), Some("https" | "http"))
    }

    fn ensure_current_account(
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Result<(), MessageExternalShareError> {
        if SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            Ok(())
        } else {
            Err(MessageExternalShareError::AccountChanged)
        }
    }
}

pub struct MessageExternalShareCoordinator {
    generation: Option<Uuid>,
    preparation_task: Option<Task<()>>,
}

impl MessageExternalShareCoordinator {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            generation: None,
            preparation_task: None,
        }
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.preparation_task = None;
        self.generation = None;
        BlockingLoaderStore::shared(cx).update(cx, |loader, cx| loader.hide(cx));
    }

    pub fn present(&mut self, messages: Vec<GetMessageMessage>, cx: &mut Context<Self>) {
        if messages.is_empty()
            || !messages
                .iter()
                .all(|message| !message.is_system() && !message.protects_content_locally())
        {
            return;
        }

        let text_items = SharePayload::Messages(messages.clone()).share_items();
        if messages.iter().all(|message| message.media_items.is_empty()) {
            if text_items.is_empty() {
                return;
            }
            SystemShare::present(text_items, None, cx);
            return;
        }

        self.preparation_task = None;
        let current_generation = Uuid::new_v4();
        self.generation = Some(current_generation);
        let weak = cx.weak_entity();
        BlockingLoaderStore::shared(cx).update(cx, |loader, cx| {
            loader.show(
                move |cx| {
                    let _ = weak.update(cx, |this, _| this.cancel(current_generation));
                },
                cx,
            );
        });

        self.preparation_task = Some(cx.spawn(async move |this, cx| {
            let prepared = MessageExternalShareMaterializer::prepare(&messages, cx).await;
            let _ = this.update(cx, |this, cx| this.finish(current_generation, prepared, cx));
        }));
    }

    fn finish(
        &mut self,
        current_generation: Uuid,
        prepared: anyhow::Result<PreparedMessageExternalShare>,
        cx: &mut Context<Self>,
    ) {
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(_) => {
                self.fail(current_generation, cx);
                return;
            }
        };
        if self.generation != Some(current_generation) {
            Self::cleanup_in_background(prepared);
            return;
        }

        self.release_preparation_task();
        BlockingLoaderStore::shared(cx).update(cx, |loader, cx| loader.hide(cx));
        let completion_share = prepared.clone();
        let presented = SystemShare::present(
            prepared.items.clone(),
            Some(Box::new(move |_cx: &mut App| {
                Self::cleanup_in_background(completion_share);
            })),
            cx,
        );
        if !presented {
            Self::cleanup_in_background(prepared);
            self.fail(current_generation, cx);
            return;
        }
        self.generation = None;
    }

    fn fail(&mut self, current_generation: Uuid, cx: &mut Context<Self>) {
        if self.generation != Some(current_generation) {
            return;
        }
        self.release_preparation_task();
        self.generation = None;
        BlockingLoaderStore::shared(cx).update(cx, |loader, cx| loader.hide(cx));
        notification(NotificationKind::Error);
        Notifier::send(
            tr("share_send_handoff_failed_title"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn release_preparation_task(&mut self) {
        if let Some(task) = self.preparation_task.take() {
            task.detach();
        }
    }

    fn cleanup_in_background(prepared: PreparedMessageExternalShare) {
        RiseAccountRuntime::current().spawn_blocking(move || prepared.cleanup());
    }

    fn cancel(&mut self, generation: Uuid) {
        if self.generation != Some(generation) {
            return;
        }
        self.preparation_task = None;
        self.generation = None;
    }
}
