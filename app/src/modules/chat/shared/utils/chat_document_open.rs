use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, AsyncApp};
use rise_core::foundation::foundation_file_name::host_file_name;
use rise_core::foundation::foundation_file_url::PathFlavor;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::download_mark::{self, DownloadOrigin};
use rise_platform::open_document;
use rise_platform::open_url::open_url;
use rise_platform::system_share::ShareItem;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaFileLease};
use crate::core::media::rise_media_transfer_hub::{
    RiseMediaCacheIdentity, RiseMediaTransferHub, RiseMediaTransferRequest, RiseMediaTransferRequestArgs,
};
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::chat::components::chat::bubble::chat_audio_track_adapter::ChatAudioTrackAdapter;
use crate::modules::chat::shared::chat_audio_queue_continuation::ChatAudioQueueContinuationOrder;
use crate::modules::chat::shared::utils::share_file_stem_hooks::ShareFileStem;
use crate::modules::chat::shared::utils::system_share_hooks::SystemShare;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, RemoteMediaURL};
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::{MusicPlayOptions, MusicPlayerState};
use crate::modules::music::shared::utils::music::music_track_file_upload::MusicTrackFileUpload;

const PLAYABLE_AUDIO_MIME_TYPES: &[&str] =
    &["audio/mpeg", "audio/mp3", "audio/x-mp3", "audio/mp4", "audio/x-m4a", "audio/aac", "audio/aacp"];
const OPEN_DIRECTORY_PREFIX: &str = "riseonly-open-document-";
const OPEN_DIRECTORY_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// opens a document from a chat
/// url and transfer request are resolved here since two places open documents
/// the bubble keeps its own cancellable task, the tab just calls open
pub enum ChatDocumentOpen {}

impl ChatDocumentOpen {
    pub fn source_url(raw_value: &str) -> Option<Url> {
        let value = trimming_whitespaces_and_newlines(raw_value);
        if value.starts_with('/') || Path::new(value).is_absolute() {
            return Url::from_file_path(Path::new(value));
        }
        Url::from_string(value).or_else(|| Url::from_file_path(Path::new(value)))
    }

    pub fn transfer_request(item: &MessageMediaItem) -> Option<RiseMediaTransferRequest> {
        if item.resolved_local_preview_url().is_some() {
            return None;
        }
        let normalized = RemoteMediaURL::optional_normalized(Some(item.url.as_str()))?;
        let remote = Url::from_string(&normalized)?;
        let scheme = remote.scheme().map(str::to_lowercase);
        if !matches!(scheme.as_deref(), Some("https" | "http")) {
            return None;
        }
        let args = RiseMediaTransferRequestArgs {
            cache_key: Some(RiseMediaCacheIdentity::key(&remote)),
            suggested_filename: Some(item.file_name.clone().unwrap_or_else(|| remote.last_path_component())),
            declared_byte_count: item.effective_total_bytes(),
            ..RiseMediaTransferRequestArgs::default()
        };
        Some(RiseMediaTransferRequest::new(remote, args))
    }

    /// audio sent as a file plays in the in-app player like telegram
    /// the system viewer doesnt take audio
    pub fn plays_in_app_player(item: &MessageMediaItem) -> bool {
        let mime = item
            .mime_type
            .as_deref()
            .and_then(|mime| mime.split(';').next())
            .map(|mime| trimming_whitespaces_and_newlines(mime).to_lowercase())
            .unwrap_or_default();
        if PLAYABLE_AUDIO_MIME_TYPES.contains(&mime.as_str()) {
            return true;
        }
        let name = trimming_whitespaces_and_newlines(item.file_name.as_deref().unwrap_or(""));
        let file_extension = if name.is_empty() {
            Self::source_url(&item.url).map(|url| url.path_extension()).unwrap_or_default()
        } else {
            Path::new(name).extension().and_then(OsStr::to_str).unwrap_or("").to_owned()
        };
        MusicTrackFileUpload::CHAT_AUDIO_EXTENSIONS.contains(&file_extension.to_lowercase().as_str())
    }

    pub fn toggle_in_app_playback(
        item: &MessageMediaItem,
        message_id: &str,
        cache_message_id: Option<&str>,
        chat_id: &str,
        chat_queue: Vec<TrackModel>,
        cx: &mut App,
    ) {
        let track = ChatAudioTrackAdapter::track(message_id, cache_message_id, chat_id, item, "");
        let player = MusicPlayerState::shared(cx);
        if ChatAudioTrackAdapter::is_same_audio(player.read(cx).current_track.as_ref(), &track) {
            player.update(cx, |player, cx| player.toggle_play_pause(cx));
            return;
        }
        let continues_chat = chat_queue.iter().any(|queued| ChatAudioTrackAdapter::is_same_audio(Some(queued), &track));
        let options = MusicPlayOptions {
            queue: if continues_chat { chat_queue } else { vec![track.clone()] },
            scope_id: Some(format!("chat:{chat_id}")),
            record_play_on_success: false,
            ..MusicPlayOptions::default()
        };
        player.update(cx, |player, cx| player.play(track, options, cx));
        if continues_chat {
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.continue_chat_audio_queue(chat_id, ChatAudioQueueContinuationOrder::Chronological, cx);
            });
        }
    }

    /// downloads if needed and hands the file to the system
    /// the account is rechecked after the await so a foreign file isnt opened after an account switch
    pub fn open(item: &MessageMediaItem, message_id: Option<&str>, chat_id: Option<&str>, cx: &mut App) {
        if let (Some(message_id), Some(chat_id)) = (message_id, chat_id) {
            if Self::plays_in_app_player(item) {
                Self::toggle_in_app_playback(item, message_id, None, chat_id, Vec::new(), cx);
                return;
            }
        }
        let raw_source = item.resolved_local_preview_url().unwrap_or_else(|| item.url.clone());
        let Some(source) = Self::source_url(&raw_source) else {
            return;
        };
        let file_name = item.file_name.clone();
        if source.is_file_url() {
            let Some(path) = source.to_file_path() else {
                return;
            };
            cx.spawn(async move |cx: &mut AsyncApp| {
                let named = Self::named_for_opening(path, file_name, cx).await;
                cx.update(|cx| {
                    Self::present(&named, None, cx);
                });
            })
            .detach();
            return;
        }
        let Some(request) = Self::transfer_request(item) else {
            open_url(&source, cx);
            return;
        };
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let hub = RiseMediaTransferHub::shared();
            let fetched = hub.fetch(request, cx).await;
            if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                return;
            }
            match fetched {
                Ok(local) => {
                    let lease = RemoteMediaDiskCache::lease(Some(local.as_path()));
                    Self::mark_as_downloaded(&local, &source, cx).await;
                    let named = Self::named_for_opening(local, file_name, cx).await;
                    cx.update(|cx| {
                        Self::present(&named, lease, cx);
                    });
                }
                Err(_) => cx.update(|cx| Self::open_after_failed_download(&source, cx)),
            }
        })
        .detach();
    }

    /// a secure gateway file needs the bearer token which the browser doesnt have, so only
    /// public links go out
    pub fn open_after_failed_download(source: &Url, cx: &mut App) {
        if source.path().to_lowercase().contains("/api/file/secure/") {
            Notifier::send(
                tr("chat_media_download_failed"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    icon: NotifierIcon::System("doc".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
            return;
        }
        open_url(source, cx);
    }

    /// shows the file in the system viewer, quick look on mac
    /// if the system wont open it, file manager select then share as last resort
    /// lease keeps the cached file alive while shown
    pub fn present(local: &Path, lease: Option<RemoteMediaFileLease>, cx: &mut App) -> bool {
        if open_document::preview(cx, local).is_ok() || open_document::reveal(cx, local).is_ok() {
            ChatDocumentPreviewSource::retain(local, lease);
            return true;
        }
        SystemShare::present(
            vec![ShareItem::File(local.to_path_buf())],
            Some(Box::new(move |_: &mut App| drop(lease))),
            cx,
        )
    }

    /// file came from another person so the system must warn like for a browser download
    /// runs before named_for_opening since its hard link shares the inode and the mark
    /// a refused mark only costs the warning
    pub async fn mark_as_downloaded(local: &Path, source: &Url, cx: &AsyncApp) {
        let path = local.to_path_buf();
        let origin =
            DownloadOrigin { host_url: download_mark::recorded_url(source.absolute_string()), referrer_url: None };
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = download_mark::mark_downloaded(&path, &origin) {
                    tracing::warn!(target: "riseonly", %error, "opened document stays without the download mark");
                }
            })
            .await;
    }

    /// the cache names files by digest but the viewer titles a document by file name
    /// so the file goes over as a hard link named after the message
    pub async fn named_for_opening(local: PathBuf, file_name: Option<String>, cx: &AsyncApp) -> PathBuf {
        let trimmed = trimming_whitespaces_and_newlines(file_name.as_deref().unwrap_or(""));
        let leaf = Path::new(trimmed).file_name().and_then(OsStr::to_str).unwrap_or("");
        let raw_stem =
            trimming_whitespaces_and_newlines(Path::new(leaf).file_stem().and_then(OsStr::to_str).unwrap_or(""));
        if raw_stem.is_empty() {
            return local;
        }
        let stem = ShareFileStem::sanitize(raw_stem);
        let leaf_extension = Path::new(leaf).extension().and_then(OsStr::to_str).unwrap_or("");
        let local_extension = local.extension().and_then(OsStr::to_str).unwrap_or("");
        let cache_extension = if local_extension.eq_ignore_ascii_case("bin") { "" } else { local_extension };
        let file_extension = if leaf_extension.is_empty() { cache_extension } else { leaf_extension };
        let name = if file_extension.is_empty() { stem } else { format!("{stem}.{file_extension}") };
        let name = host_file_name(&name, PathFlavor::HOST);
        if name.is_empty() {
            return local;
        }
        if local.file_name().and_then(OsStr::to_str) == Some(name.as_str()) {
            return local;
        }
        cx.background_executor()
            .spawn(async move {
                let root = rise_platform::paths::temporary_directory();
                Self::remove_expired_open_directories(&root);
                let directory = root.join(format!("{OPEN_DIRECTORY_PREFIX}{}", Uuid::new_v4().uuid_string()));
                let destination = directory.join(&name);
                let named = std::fs::create_dir_all(&directory).and_then(|()| {
                    std::fs::hard_link(&local, &destination)
                        .or_else(|_| std::fs::copy(&local, &destination).map(|_| ()))
                });
                if named.is_ok() {
                    return destination;
                }
                let _ = std::fs::remove_dir_all(&directory);
                local
            })
            .await
    }

    fn remove_expired_open_directories(root: &Path) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().starts_with(OPEN_DIRECTORY_PREFIX) {
                continue;
            }
            let modified = entry.metadata().and_then(|metadata| metadata.modified());
            let expired = modified
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > OPEN_DIRECTORY_LIFETIME);
            if expired {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// the system viewer runs outside the app and shows one document at a time
/// so the last document source stays here until the next replaces it
struct ChatDocumentPreviewSource {
    _item: PathBuf,
    _lease: Option<RemoteMediaFileLease>,
}

thread_local! {
    static CURRENT_PREVIEW_SOURCE: RefCell<Option<ChatDocumentPreviewSource>> = const { RefCell::new(None) };
}

impl ChatDocumentPreviewSource {
    fn retain(url: &Path, lease: Option<RemoteMediaFileLease>) {
        let source = Self { _item: url.to_path_buf(), _lease: lease };
        CURRENT_PREVIEW_SOURCE.with(|current| *current.borrow_mut() = Some(source));
    }
}
