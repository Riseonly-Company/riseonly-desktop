use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, UNIX_EPOCH};

use futures::FutureExt;
use futures::future::{LocalBoxFuture, Shared};
use gpui::AsyncApp;
use parking_lot::Mutex;
use rise_core::foundation::foundation_compare::localized_case_insensitive_compare;
use rise_i18n::LanguageStore;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::URLRequest;
use rise_platform::AppPaths;
use rise_platform::file_manager::{
    CocoaError, CocoaErrorCode, DirectoryEnumerationOptions, FileAttributeType, FileManager,
};
use rise_platform::paths::temporary_directory;
use rise_platform::url_resource_values::{URLResourceValues, set_resource_values};
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheError, RemoteMediaDownloadProgress, RemoteMediaHttpResponse,
    RemoteMediaTransferError,
};
use crate::core::media::rise_media_transfer_hub::{RiseMediaCacheIdentity, RiseMediaTransferHub};
use crate::core::media::rise_progressive_data_fetcher::{
    RiseProgressiveDataFetcher, RiseProgressiveDataFetcherLane, RiseProgressiveDataFetcherOnChunk,
    RiseProgressiveDataFetcherOnProgress, RiseProgressiveDataFetcherOnResponse,
};

const SUBFOLDER: &str = "VoiceMessageCache";
const INDEX_FILE_NAME: &str = "_voice_index.json";
const ACCOUNTS_FOLDER: &str = "accounts";
const ANONYMOUS_ACCOUNT_DIRECTORY: &str = "anonymous";
const MINIMUM_PAYLOAD_BYTE_COUNT: i64 = 1_024;
const MAXIMUM_FILE_BYTE_COUNT: i64 = 256 * 1_024 * 1_024;
const MAXIMUM_IN_MEMORY_DOWNLOAD_BYTE_COUNT: i64 = 16 * 1_024 * 1_024;
const MAXIMUM_TOTAL_BYTE_COUNT: i64 = 2 * 1_024 * 1_024 * 1_024;
const MAXIMUM_ENTRY_COUNT: usize = 1_200;
const CHAT_DIRECTORY_PREFIX: &str = "chat-";
const MESSAGE_FILE_PREFIX: &str = "voice-";
const MESSAGE_FILE_EXTENSION: &str = ".m4a";
const STAGING_FILE_EXTENSION: &str = "part";
const DOWNLOAD_RETRY_DELAYS_NANOSECONDS: [u64; 6] = [
    250_000_000,
    750_000_000,
    1_500_000_000,
    3_000_000_000,
    5_000_000_000,
    8_000_000_000,
];
const DOWNLOAD_REQUEST_TIMEOUT_SECONDS: u64 = 90;
const HTTP_METHOD_GET: &str = "GET";
const ACCEPT_ENCODING_HEADER: &str = "Accept-Encoding";
const ACCEPT_ENCODING_IDENTITY: &str = "identity";
const AUTHORIZATION_HEADER: &str = "Authorization";
const CONTENT_RANGE_HEADER: &str = "Content-Range";
const CONTENT_RANGE_BYTES_UNIT: &str = "bytes ";
const SHA256_HEX_LENGTH: usize = 64;
const LOG_IDENTIFIER_PREFIX_LENGTH: usize = 8;
const LOG_NAME: &str = "🎵 Voice cache";
const LOG_CHANNEL: &str = "player";
const AUDIO_HEAD_BYTE_COUNT: usize = 512;
const AUDIO_SIGNATURES: [&[u8]; 7] = [
    &[0x49, 0x44, 0x33],
    &[0x4F, 0x67, 0x67, 0x53],
    &[0x66, 0x4C, 0x61, 0x43],
    &[0x52, 0x49, 0x46, 0x46],
    &[0x63, 0x61, 0x66, 0x66],
    &[0x23, 0x21, 0x41, 0x4D, 0x52],
    &[0x1A, 0x45, 0xDF, 0xA3],
];
const ISO_MEDIA_FILE_TYPE_BOX: [u8; 4] = [0x66, 0x74, 0x79, 0x70];

static DATA_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    let paths = AppPaths::resolve(AppEnvironment::compiled().canon()).ok()?;
    paths.create_all().ok()?;
    Some(paths.data_dir().to_path_buf())
});
static INDEX_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

thread_local! {
    static DOWNLOAD_COORDINATOR: VoiceMessageDiskCacheDownloadCoordinator =
        VoiceMessageDiskCacheDownloadCoordinator {
            tasks: RefCell::new(HashMap::new()),
        };
}

/// account scoped disk cache of chat voice notes and audio attachments
/// sync members do blocking file io so call them off the main thread
pub enum VoiceMessageDiskCache {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMessageDiskCacheIndexEntry {
    pub message_id: String,
    pub chat_id: String,
    pub chat_name: String,
    pub file_name: String,
    pub remote_url: Option<String>,
    pub byte_count: i64,
    pub cached_at: TimeInterval,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoiceMessageDiskCacheListedEntry {
    pub message_id: String,
    pub chat_id: String,
    pub chat_name: String,
    pub file_name: String,
    pub file_bytes: i64,
    pub cached_at: Date,
    pub remote_url: Option<String>,
}

impl VoiceMessageDiskCacheListedEntry {
    pub fn id(&self) -> &str {
        &self.message_id
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoiceMessageDiskCacheChatGroup {
    pub chat_id: String,
    pub chat_name: String,
    pub entries: Vec<VoiceMessageDiskCacheListedEntry>,
}

impl VoiceMessageDiskCacheChatGroup {
    pub fn id(&self) -> &str {
        &self.chat_id
    }

    pub fn total_bytes(&self) -> i64 {
        self.entries.iter().map(|entry| entry.file_bytes).sum()
    }
}

/// per chat usage of the voice disk cache
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceMessageDiskCacheChatUsage {
    pub chat_id: String,
    pub chat_name: String,
    pub entry_count: i64,
    pub byte_count: i64,
}

impl VoiceMessageDiskCacheChatUsage {
    pub fn id(&self) -> &str {
        &self.chat_id
    }
}

/// downloaded payload, the temp file goes with the payload
struct VoiceMessageDiskCacheDownloadedPayload {
    response: RemoteMediaHttpResponse,
    data: Option<Vec<u8>>,
    temporary_url: Option<PathBuf>,
    byte_count: i64,
}

impl VoiceMessageDiskCacheDownloadedPayload {
    fn remove_temporary_file(&self) {
        if let Some(temporary_url) = &self.temporary_url {
            FileManager.remove_item(temporary_url).ok();
        }
    }
}

impl Drop for VoiceMessageDiskCacheDownloadedPayload {
    fn drop(&mut self) {
        self.remove_temporary_file();
    }
}

struct VoiceMessageDiskCacheDownloadSink {
    file: Option<std::fs::File>,
    byte_count: i64,
    failed: bool,
}

impl VoiceMessageDiskCacheDownloadSink {
    fn write(&mut self, chunk: &[u8]) {
        if self.failed {
            return;
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };
        self.byte_count = self
            .byte_count
            .saturating_add(i64::try_from(chunk.len()).unwrap_or(i64::MAX));
        if self.byte_count > MAXIMUM_FILE_BYTE_COUNT || file.write_all(chunk).is_err() {
            self.failed = true;
            self.file = None;
        }
    }
}

/// how one checked download left the cache
enum VoiceMessageDiskCacheCommit {
    Rejected { status_code: i64, valid_payload: bool },
    AccountChanged,
    Stored { byte_count: i64 },
}

/// how a local seed left the cache
enum VoiceMessageDiskCacheSeed {
    Stored(PathBuf),
    Skipped,
    Vanished,
}

/// one transfer per key, every caller awaits it
struct VoiceMessageDiskCacheDownloadCoordinator {
    tasks: RefCell<HashMap<String, Shared<LocalBoxFuture<'static, Option<PathBuf>>>>>,
}

impl VoiceMessageDiskCacheDownloadCoordinator {
    async fn run(key: String, operation: LocalBoxFuture<'static, Option<PathBuf>>) -> Option<PathBuf> {
        let task = DOWNLOAD_COORDINATOR.with(|coordinator| {
            coordinator
                .tasks
                .borrow_mut()
                .entry(key.clone())
                .or_insert_with(|| operation.shared())
                .clone()
        });
        let value = task.clone().await;
        DOWNLOAD_COORDINATOR.with(|coordinator| {
            let mut tasks = coordinator.tasks.borrow_mut();
            if tasks.get(&key).is_some_and(|current| current.ptr_eq(&task)) {
                tasks.remove(&key);
            }
        });
        value
    }
}

impl VoiceMessageDiskCache {
    fn root_directory() -> Result<PathBuf, CocoaError> {
        let root = DATA_ROOT
            .as_ref()
            .ok_or_else(|| CocoaError::new(CocoaErrorCode::FileNoSuchFile))?;
        let dir = root.join(SUBFOLDER);
        Self::create_protected_directory(&dir)?;
        Ok(dir)
    }

    fn base_directory(user_id: Option<&str>) -> Result<PathBuf, CocoaError> {
        let root = Self::root_directory()?;
        let accounts = root.join(ACCOUNTS_FOLDER);
        Self::create_protected_directory(&accounts)?;
        let dir = accounts.join(Self::account_directory_name(user_id));
        Self::create_protected_directory(&dir)?;
        Ok(dir)
    }

    fn active_base_directory() -> Result<PathBuf, CocoaError> {
        Self::base_directory(SaiAccountCacheScope::active_user_id().as_deref())
    }

    fn account_directory_name(user_id: Option<&str>) -> String {
        match Self::normalized_user_id(user_id) {
            Some(value) => Self::digest_hex(&value),
            None => ANONYMOUS_ACCOUNT_DIRECTORY.to_owned(),
        }
    }

    fn normalized_user_id(user_id: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(user_id.unwrap_or_default());
        (!value.is_empty()).then(|| value.to_owned())
    }

    fn chat_directory(chat_id: &str, base: &Path) -> Result<PathBuf, CocoaError> {
        let dir = base.join(Self::chat_directory_name(chat_id));
        Self::create_protected_directory(&dir)?;
        Ok(dir)
    }

    fn chat_directory_name(chat_id: &str) -> String {
        format!("{CHAT_DIRECTORY_PREFIX}{}", Self::digest_hex(chat_id))
    }

    fn message_file_name(message_id: &str) -> String {
        format!("{MESSAGE_FILE_PREFIX}{}{MESSAGE_FILE_EXTENSION}", Self::digest_hex(message_id))
    }

    fn digest_hex(value: &str) -> String {
        sha256_hex(value.as_bytes())
    }

    fn index_url(base: &Path) -> PathBuf {
        base.join(INDEX_FILE_NAME)
    }

    fn decode_index_unlocked(base: &Path) -> Option<HashMap<String, VoiceMessageDiskCacheIndexEntry>> {
        let url = Self::index_url(base);
        if !FileManager.file_exists(&url) {
            return None;
        }
        let data = std::fs::read(&url).ok()?;
        serde_json::from_slice(&data).ok()
    }

    fn write_index_unlocked(
        dict: &HashMap<String, VoiceMessageDiskCacheIndexEntry>,
        base: &Path,
    ) -> Result<(), CocoaError> {
        let url = Self::index_url(base);
        let data = serde_json::to_vec(dict).map_err(|_| CocoaError::new(CocoaErrorCode::FileWriteUnknown))?;
        Self::write_atomically(&url, &data)?;
        Self::apply_storage_attributes(&url);
        Ok(())
    }

    fn write_atomically(url: &Path, data: &[u8]) -> Result<(), CocoaError> {
        let parent = url
            .parent()
            .ok_or_else(|| CocoaError::new(CocoaErrorCode::FileWriteInvalidFileName))?;
        let staging = parent.join(format!(
            ".{}.{}.tmp",
            Self::last_path_component(url),
            Uuid::new_v4().uuid_string()
        ));
        if std::fs::write(&staging, data).is_err() {
            FileManager.remove_item(&staging).ok();
            return Err(CocoaError::new(CocoaErrorCode::FileWriteUnknown));
        }
        if std::fs::rename(&staging, url).is_err() {
            FileManager.remove_item(&staging).ok();
            return Err(CocoaError::new(CocoaErrorCode::FileWriteUnknown));
        }
        Ok(())
    }

    pub fn file_url(chat_id: &str, message_id: &str) -> Result<PathBuf, CocoaError> {
        let base = Self::active_base_directory()?;
        Self::file_url_in_base(chat_id, message_id, &base)
    }

    fn file_url_in_base(chat_id: &str, message_id: &str, base: &Path) -> Result<PathBuf, CocoaError> {
        Ok(Self::chat_directory(chat_id, base)?.join(Self::message_file_name(message_id)))
    }

    pub fn seeded_local_file(chat_id: &str, message_id: &str) -> Option<String> {
        let base = Self::active_base_directory().ok()?;
        let path = base.join(Self::chat_directory_name(chat_id)).join(Self::message_file_name(message_id));
        if Self::valid_file_byte_count(&path).is_none() {
            return None;
        }
        path.to_str().map(str::to_owned)
    }

    pub fn cached_url(chat_id: &str, message_id: &str) -> Option<PathBuf> {
        let base = Self::active_base_directory().ok()?;
        Self::cached_url_in_base(chat_id, message_id, &base)
    }

    fn cached_url_in_base(chat_id: &str, message_id: &str, base: &Path) -> Option<PathBuf> {
        Self::with_index_lock(|| {
            let mut should_write = false;
            let mut index = match Self::decode_index_unlocked(base) {
                Some(decoded) => decoded,
                None => {
                    let reconciled = Self::reconcile_index_unlocked(HashMap::new(), base);
                    should_write = true;
                    Self::prune_index_unlocked(reconciled, base, Some(message_id))
                }
            };

            if let Some(entry) = index.get(message_id).cloned() {
                let url = Self::resolved_entry_url_unlocked(&entry, message_id, base);
                if let Some(bytes) = Self::valid_file_byte_count(&url) {
                    if let Some(parent) = url.parent() {
                        Self::apply_storage_attributes(parent);
                    }
                    Self::apply_storage_attributes(&url);
                    let file_name = Self::last_path_component(&url);
                    if entry.byte_count != bytes || entry.file_name != file_name {
                        let mut updated = entry;
                        updated.byte_count = bytes;
                        updated.file_name = file_name;
                        index.insert(message_id.to_owned(), updated);
                        should_write = true;
                    }
                    if should_write {
                        Self::write_index_unlocked(&index, base).ok();
                    }
                    return Some(url);
                }
                FileManager.remove_item(&url).ok();
                index.remove(message_id);
                should_write = true;
            }

            let fallback = Self::file_url_in_base(chat_id, message_id, base)
                .ok()
                .and_then(|fallback| Self::valid_file_byte_count(&fallback).map(|bytes| (fallback, bytes)));
            let Some((fallback, bytes)) = fallback else {
                if should_write {
                    Self::write_index_unlocked(&index, base).ok();
                }
                return None;
            };

            Self::apply_storage_attributes(&fallback);
            index.insert(
                message_id.to_owned(),
                VoiceMessageDiskCacheIndexEntry {
                    message_id: message_id.to_owned(),
                    chat_id: chat_id.to_owned(),
                    chat_name: chat_id.to_owned(),
                    file_name: Self::last_path_component(&fallback),
                    remote_url: None,
                    byte_count: bytes,
                    cached_at: Self::file_access_time(&fallback),
                },
            );
            let index = Self::prune_index_unlocked(index, base, Some(message_id));
            Self::write_index_unlocked(&index, base).ok();
            index.contains_key(message_id).then_some(fallback)
        })
    }

    pub async fn ensure_cached(
        remote: &Url,
        message_id: &str,
        chat_id: &str,
        chat_name: &str,
        expected_byte_count: Option<i64>,
        cx: &AsyncApp,
    ) -> Option<PathBuf> {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let key = Self::coordination_key(
            account_user_id.as_deref(),
            account_epoch.as_deref(),
            chat_id,
            message_id,
        );
        let operation = Self::ensure_cached_uncoordinated(
            remote.clone(),
            message_id.to_owned(),
            chat_id.to_owned(),
            chat_name.to_owned(),
            expected_byte_count,
            account_user_id,
            account_epoch,
            cx.clone(),
        )
        .boxed_local();
        VoiceMessageDiskCacheDownloadCoordinator::run(key, operation).await
    }

    pub async fn store_local(
        source: &Path,
        message_id: &str,
        chat_id: &str,
        chat_name: &str,
        cx: &AsyncApp,
    ) -> Option<PathBuf> {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let key = Self::coordination_key(
            account_user_id.as_deref(),
            account_epoch.as_deref(),
            chat_id,
            message_id,
        );
        let source = source.to_path_buf();
        let message_id = message_id.to_owned();
        let chat_id = chat_id.to_owned();
        let chat_name = chat_name.to_owned();
        let cx = cx.clone();
        let operation = async move {
            let log_id = Self::log_identifier(&message_id);
            let seeded = {
                let message_id = message_id.clone();
                cx.background_executor()
                    .spawn(async move {
                        Self::store_local_blocking(
                            &source,
                            &message_id,
                            &chat_id,
                            &chat_name,
                            account_user_id.as_deref(),
                            account_epoch.as_deref(),
                        )
                    })
                    .await
            };
            match seeded {
                Ok(VoiceMessageDiskCacheSeed::Stored(destination)) => Some(destination),
                Ok(VoiceMessageDiskCacheSeed::Skipped) => None,
                Ok(VoiceMessageDiskCacheSeed::Vanished) => {
                    Self::log_download_failure(format!("local seed vanished id={log_id}"), &cx);
                    None
                }
                Err(error) => {
                    Self::log_download_failure(
                        format!(
                            "local seed failed id={log_id} error={}",
                            Self::download_error_label(&RemoteMediaTransferError::from(error))
                        ),
                        &cx,
                    );
                    None
                }
            }
        }
        .boxed_local();
        VoiceMessageDiskCacheDownloadCoordinator::run(key, operation).await
    }

    fn store_local_blocking(
        source: &Path,
        message_id: &str,
        chat_id: &str,
        chat_name: &str,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Result<VoiceMessageDiskCacheSeed, CocoaError> {
        if Self::valid_file_byte_count(source).is_none() {
            return Ok(VoiceMessageDiskCacheSeed::Skipped);
        }
        let Ok(base) = Self::base_directory(account_user_id) else {
            return Ok(VoiceMessageDiskCacheSeed::Skipped);
        };
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            return Ok(VoiceMessageDiskCacheSeed::Skipped);
        }
        let Ok(destination) = Self::file_url_in_base(chat_id, message_id, &base) else {
            return Ok(VoiceMessageDiskCacheSeed::Skipped);
        };

        if let Some(existing) = Self::cached_url_in_base(chat_id, message_id, &base) {
            return Ok(VoiceMessageDiskCacheSeed::Stored(existing));
        }

        let staging = Self::staging_url(&destination, message_id);
        let seeded = Self::store_local_staged(
            source,
            &staging,
            &destination,
            message_id,
            chat_id,
            chat_name,
            &base,
            account_user_id,
            account_epoch,
        );
        if seeded.is_err() {
            FileManager.remove_item(&staging).ok();
        }
        seeded
    }

    #[allow(clippy::too_many_arguments)]
    fn store_local_staged(
        source: &Path,
        staging: &Path,
        destination: &Path,
        message_id: &str,
        chat_id: &str,
        chat_name: &str,
        base: &Path,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Result<VoiceMessageDiskCacheSeed, CocoaError> {
        if let Some(parent) = destination.parent() {
            Self::create_protected_directory(parent)?;
        }
        FileManager.remove_item(staging).ok();
        FileManager.copy_item(source, staging)?;
        Self::apply_storage_attributes(staging);
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch)
            || Self::valid_file_byte_count(staging).is_none()
        {
            FileManager.remove_item(staging).ok();
            return Ok(VoiceMessageDiskCacheSeed::Skipped);
        }

        FileManager.remove_item(destination).ok();
        FileManager.move_item(staging, destination)?;
        Self::apply_storage_attributes(destination);
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            FileManager.remove_item(destination).ok();
            return Ok(VoiceMessageDiskCacheSeed::Skipped);
        }
        Self::register(message_id, chat_id, chat_name, destination, None, base);
        if Self::valid_file_byte_count(destination).is_none() {
            return Ok(VoiceMessageDiskCacheSeed::Vanished);
        }
        Ok(VoiceMessageDiskCacheSeed::Stored(destination.to_path_buf()))
    }

    fn staging_url(destination: &Path, message_id: &str) -> PathBuf {
        let name = format!(
            ".{MESSAGE_FILE_PREFIX}{}-{}.{STAGING_FILE_EXTENSION}",
            Self::digest_hex(message_id),
            Uuid::new_v4().uuid_string()
        );
        destination
            .parent()
            .map_or_else(|| PathBuf::from(&name), |parent| parent.join(&name))
    }

    fn coordination_key(
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        chat_id: &str,
        message_id: &str,
    ) -> String {
        [
            Self::account_directory_name(account_user_id),
            account_epoch.unwrap_or_default().to_owned(),
            Self::chat_directory_name(chat_id),
            message_id.to_owned(),
        ]
        .join("|")
    }

    #[allow(clippy::too_many_arguments)]
    async fn ensure_cached_uncoordinated(
        remote: Url,
        message_id: String,
        chat_id: String,
        chat_name: String,
        expected_byte_count: Option<i64>,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        mut cx: AsyncApp,
    ) -> Option<PathBuf> {
        let background = cx.background_executor().clone();
        let log_id = Self::log_identifier(&message_id);
        let is_current = |user_id: &Option<String>, epoch: &Option<String>| {
            SaiAccountCacheScope::is_current_account_epoch(user_id.as_deref(), epoch.as_deref())
        };

        let base = {
            let user_id = account_user_id.clone();
            background
                .spawn(async move { Self::base_directory(user_id.as_deref()).ok() })
                .await
        };
        let Some(base) = base.filter(|_| is_current(&account_user_id, &account_epoch)) else {
            Self::log_download_failure(format!("no cache directory id={log_id}"), &cx);
            return None;
        };

        let existing = {
            let (chat_id, message_id, base) = (chat_id.clone(), message_id.clone(), base.clone());
            background
                .spawn(async move { Self::cached_url_in_base(&chat_id, &message_id, &base) })
                .await
        };
        if let Some(existing) = existing {
            Self::ingest_transfer(RiseMediaCacheIdentity::key(&remote), 0, 0, true).await;
            {
                let (message_id, chat_id, chat_name) = (message_id.clone(), chat_id.clone(), chat_name.clone());
                let (existing, remote, base) = (existing.clone(), remote.clone(), base.clone());
                background
                    .spawn(async move {
                        Self::touch_index(&message_id, &chat_id, &chat_name, &existing, &remote, &base);
                    })
                    .await;
            }
            return is_current(&account_user_id, &account_epoch).then_some(existing);
        }

        let prepared = {
            let (chat_id, message_id, base) = (chat_id.clone(), message_id.clone(), base.clone());
            background
                .spawn(async move {
                    let destination = Self::file_url_in_base(&chat_id, &message_id, &base).map_err(|_| None::<CocoaError>)?;
                    let staging = Self::staging_url(&destination, &message_id);
                    if let Some(parent) = destination.parent() {
                        Self::create_protected_directory(parent).map_err(Some)?;
                    }
                    Ok::<(PathBuf, PathBuf), Option<CocoaError>>((destination, staging))
                })
                .await
        };
        let (destination, staging) = match prepared {
            Ok(prepared) => prepared,
            Err(None) => {
                Self::log_download_failure(
                    format!(
                        "no destination path id={log_id} chat={}",
                        Self::log_identifier(&chat_id)
                    ),
                    &cx,
                );
                return None;
            }
            Err(Some(error)) => {
                Self::log_download_failure(
                    format!(
                        "prepare failed id={log_id} error={}",
                        Self::download_error_label(&RemoteMediaTransferError::from(error))
                    ),
                    &cx,
                );
                return None;
            }
        };

        let transfer_key = RiseMediaCacheIdentity::key(&remote);
        let retry_count = DOWNLOAD_RETRY_DELAYS_NANOSECONDS.len();
        let maximum_attempts = retry_count + 1;
        for attempt in 0..maximum_attempts {
            if !is_current(&account_user_id, &account_epoch) {
                return None;
            }

            let downloaded = Self::download_attempt(
                &remote,
                expected_byte_count,
                &transfer_key,
                account_user_id.clone(),
                account_epoch.clone(),
                &mut cx,
            )
            .await;
            let committed = match downloaded {
                Ok(payload) => {
                    let (staging, destination, base) = (staging.clone(), destination.clone(), base.clone());
                    let (message_id, chat_id, chat_name) = (message_id.clone(), chat_id.clone(), chat_name.clone());
                    let (remote, user_id, epoch) = (remote.clone(), account_user_id.clone(), account_epoch.clone());
                    background
                        .spawn(async move {
                            Self::commit_download(
                                payload,
                                expected_byte_count,
                                &staging,
                                &destination,
                                &message_id,
                                &chat_id,
                                &chat_name,
                                &remote,
                                &base,
                                user_id.as_deref(),
                                epoch.as_deref(),
                            )
                        })
                        .await
                        .map_err(RemoteMediaTransferError::from)
                }
                Err(error) => Err(error),
            };

            match committed {
                Ok(VoiceMessageDiskCacheCommit::Rejected {
                    status_code,
                    valid_payload,
                }) => {
                    let retryable =
                        Self::should_retry_download(status_code) || (status_code == 200 && !valid_payload);
                    if retryable && attempt < retry_count {
                        Self::log_download_failure(
                            format!(
                                "download retry id={log_id} status={status_code} valid={valid_payload} attempt={}/{maximum_attempts}",
                                attempt + 1
                            ),
                            &cx,
                        );
                        background
                            .timer(Duration::from_nanos(DOWNLOAD_RETRY_DELAYS_NANOSECONDS[attempt]))
                            .await;
                        continue;
                    }
                    Self::log_download_failure(
                        format!(
                            "download rejected id={log_id} status={status_code} valid={valid_payload} attempts={}",
                            attempt + 1
                        ),
                        &cx,
                    );
                    return None;
                }
                Ok(VoiceMessageDiskCacheCommit::AccountChanged) => return None,
                Ok(VoiceMessageDiskCacheCommit::Stored { byte_count }) => {
                    Self::ingest_transfer(transfer_key.clone(), byte_count, byte_count, true).await;
                    if !is_current(&account_user_id, &account_epoch) {
                        return None;
                    }
                    let present = {
                        let destination = destination.clone();
                        background
                            .spawn(async move { Self::valid_file_byte_count(&destination).is_some() })
                            .await
                    };
                    if !present {
                        Self::log_download_failure(
                            format!("cache write vanished id={log_id} bytes={byte_count}"),
                            &cx,
                        );
                        return None;
                    }
                    return Some(destination);
                }
                Err(error) => {
                    {
                        let staging = staging.clone();
                        background
                            .spawn(async move {
                                FileManager.remove_item(&staging).ok();
                            })
                            .await;
                    }
                    if matches!(error, RemoteMediaTransferError::Cancelled(_)) {
                        return None;
                    }
                    let label = Self::download_error_label(&error);
                    if attempt < retry_count {
                        Self::log_download_failure(
                            format!(
                                "download retry id={log_id} error={label} attempt={}/{maximum_attempts}",
                                attempt + 1
                            ),
                            &cx,
                        );
                        background
                            .timer(Duration::from_nanos(DOWNLOAD_RETRY_DELAYS_NANOSECONDS[attempt]))
                            .await;
                        continue;
                    }
                    Self::log_download_failure(
                        format!(
                            "download failed id={log_id} error={label} attempts={}",
                            attempt + 1
                        ),
                        &cx,
                    );
                    return None;
                }
            }
        }
        None
    }

    async fn download_attempt(
        remote: &Url,
        expected_byte_count: Option<i64>,
        transfer_key: &str,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) -> Result<VoiceMessageDiskCacheDownloadedPayload, RemoteMediaTransferError> {
        let mut request = URLRequest::new(remote.clone());
        request.timeout_interval = Duration::from_secs(DOWNLOAD_REQUEST_TIMEOUT_SECONDS);
        request.http_method = HTTP_METHOD_GET.to_owned();
        request.set_value(ACCEPT_ENCODING_IDENTITY, ACCEPT_ENCODING_HEADER);
        if let Some(token) = RemoteMediaDiskCache::bearer_token(remote, None, cx).await {
            request.set_value(&format!("Bearer {token}"), AUTHORIZATION_HEADER);
        }
        Self::download_payload(
            request,
            expected_byte_count,
            transfer_key,
            account_user_id,
            account_epoch,
            cx,
        )
        .await
    }

    async fn download_payload(
        request: URLRequest,
        expected_byte_count: Option<i64>,
        transfer_key: &str,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) -> Result<VoiceMessageDiskCacheDownloadedPayload, RemoteMediaTransferError> {
        let expected = Self::normalized_expected_byte_count(expected_byte_count);
        if let Some(declared) = expected.filter(|expected| *expected <= MAXIMUM_IN_MEMORY_DOWNLOAD_BYTE_COUNT) {
            let progress_key = transfer_key.to_owned();
            let on_progress: RiseProgressiveDataFetcherOnProgress =
                Arc::new(move |progress: RemoteMediaDownloadProgress| {
                    let cache_key = progress_key.clone();
                    RiseAccountRuntime::current().spawn(async move {
                        RiseMediaTransferHub::shared()
                            .ingest(
                                cache_key,
                                progress.downloaded_bytes,
                                progress.total_bytes.unwrap_or(declared),
                                false,
                            )
                            .await;
                    });
                });
            let (data, response) = RiseProgressiveDataFetcher::data(
                request,
                MAXIMUM_FILE_BYTE_COUNT,
                RiseProgressiveDataFetcherLane::PrivateEphemeral,
                account_user_id,
                account_epoch,
                RiseProgressiveDataFetcher::DEFAULT_PRIORITY,
                on_progress,
                cx,
            )
            .await?;
            let byte_count = i64::try_from(data.len()).unwrap_or(i64::MAX);
            return Ok(VoiceMessageDiskCacheDownloadedPayload {
                response,
                data: Some(data),
                temporary_url: None,
                byte_count,
            });
        }

        let request_url = request.url.clone();
        let temporary_url = temporary_directory().join(format!(
            "riseonly-voice-{}.download",
            Uuid::new_v4().uuid_string().to_lowercase()
        ));
        let file = {
            let temporary_url = temporary_url.clone();
            cx.background_executor()
                .spawn(async move {
                    if let Some(parent) = temporary_url.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    std::fs::File::create(&temporary_url).ok()
                })
                .await
        };
        let Some(file) = file else {
            return Err(CocoaError::new(CocoaErrorCode::FileWriteUnknown).into());
        };
        let sink = Arc::new(Mutex::new(VoiceMessageDiskCacheDownloadSink {
            file: Some(file),
            byte_count: 0,
            failed: false,
        }));
        let captured: Arc<Mutex<Option<RemoteMediaHttpResponse>>> = Arc::new(Mutex::new(None));
        let on_response: RiseProgressiveDataFetcherOnResponse = {
            let captured = Arc::clone(&captured);
            Arc::new(move |response: &RemoteMediaHttpResponse| {
                *captured.lock() = Some(response.clone());
                true
            })
        };
        let on_chunk: RiseProgressiveDataFetcherOnChunk = {
            let sink = Arc::clone(&sink);
            Arc::new(move |chunk: &[u8]| sink.lock().write(chunk))
        };
        let streamed = RiseProgressiveDataFetcher::stream(
            request,
            RiseProgressiveDataFetcherLane::PrivateEphemeral,
            account_user_id,
            account_epoch,
            on_response,
            on_chunk,
            cx,
        )
        .await;
        let write_failed = {
            let mut sink = sink.lock();
            sink.file = None;
            sink.failed
        };
        let response = match streamed {
            Ok(()) => captured.lock().take(),
            Err(RemoteMediaTransferError::Cache(RemoteMediaDiskCacheError::InvalidResponse {
                status_code: Some(status_code),
            })) => Some(RemoteMediaHttpResponse::new(request_url, status_code, Vec::new())),
            Err(error) => {
                FileManager.remove_item(&temporary_url).ok();
                return Err(error);
            }
        };
        if write_failed {
            FileManager.remove_item(&temporary_url).ok();
            return Err(CocoaError::new(CocoaErrorCode::FileWriteUnknown).into());
        }
        let Some(response) = response else {
            FileManager.remove_item(&temporary_url).ok();
            return Err(RemoteMediaDiskCacheError::InvalidResponse { status_code: None }.into());
        };
        let byte_count = Self::regular_file_byte_count(&temporary_url).unwrap_or(0);
        Ok(VoiceMessageDiskCacheDownloadedPayload {
            response,
            data: None,
            temporary_url: Some(temporary_url),
            byte_count,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_download(
        payload: VoiceMessageDiskCacheDownloadedPayload,
        expected_byte_count: Option<i64>,
        staging: &Path,
        destination: &Path,
        message_id: &str,
        chat_id: &str,
        chat_name: &str,
        remote: &Url,
        base: &Path,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Result<VoiceMessageDiskCacheCommit, CocoaError> {
        let status_code = payload.response.status_code;
        let complete_response =
            Self::is_complete_download_response_with_byte_count(&payload.response, payload.byte_count);
        let valid_payload = Self::is_valid_downloaded_payload(&payload, expected_byte_count);
        if !complete_response || !valid_payload {
            return Ok(VoiceMessageDiskCacheCommit::Rejected {
                status_code,
                valid_payload,
            });
        }

        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            return Ok(VoiceMessageDiskCacheCommit::AccountChanged);
        }

        FileManager.remove_item(staging).ok();
        let staged = Self::materialize(&payload, staging).and_then(|()| {
            Self::apply_storage_attributes(staging);
            if Self::valid_file_byte_count(staging) == Some(payload.byte_count) {
                Ok(())
            } else {
                Err(CocoaError::new(CocoaErrorCode::FileReadCorruptFile))
            }
        });
        if let Err(error) = staged {
            FileManager.remove_item(staging).ok();
            return Err(error);
        }

        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            FileManager.remove_item(staging).ok();
            return Ok(VoiceMessageDiskCacheCommit::AccountChanged);
        }

        FileManager.remove_item(destination).ok();
        if let Err(error) = FileManager.move_item(staging, destination) {
            FileManager.remove_item(staging).ok();
            return Err(error);
        }
        Self::apply_storage_attributes(destination);

        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            FileManager.remove_item(destination).ok();
            return Ok(VoiceMessageDiskCacheCommit::AccountChanged);
        }

        Self::register(message_id, chat_id, chat_name, destination, Some(remote), base);
        Ok(VoiceMessageDiskCacheCommit::Stored {
            byte_count: payload.byte_count,
        })
    }

    fn is_valid_downloaded_payload(
        payload: &VoiceMessageDiskCacheDownloadedPayload,
        expected_byte_count: Option<i64>,
    ) -> bool {
        if payload.byte_count <= MINIMUM_PAYLOAD_BYTE_COUNT
            || payload.byte_count > MAXIMUM_FILE_BYTE_COUNT
            || !Self::expected_download_byte_count_matches(payload.byte_count, expected_byte_count)
        {
            return false;
        }
        if let Some(data) = &payload.data {
            return Self::is_likely_audio_data(data);
        }
        let Some(temporary_url) = &payload.temporary_url else {
            return false;
        };
        Self::valid_file_byte_count(temporary_url) == Some(payload.byte_count)
    }

    fn materialize(payload: &VoiceMessageDiskCacheDownloadedPayload, destination: &Path) -> Result<(), CocoaError> {
        if let Some(data) = &payload.data {
            return std::fs::write(destination, data).map_err(|_| CocoaError::new(CocoaErrorCode::FileWriteUnknown));
        }
        let Some(temporary_url) = &payload.temporary_url else {
            return Err(CocoaError::new(CocoaErrorCode::FileNoSuchFile));
        };
        FileManager
            .move_item(temporary_url, destination)
            .or_else(|_| FileManager.copy_item(temporary_url, destination))
    }

    fn normalized_expected_byte_count(value: Option<i64>) -> Option<i64> {
        value.filter(|value| *value > 0 && *value <= MAXIMUM_FILE_BYTE_COUNT)
    }

    pub fn expected_download_byte_count_matches(downloaded_byte_count: i64, expected_byte_count: Option<i64>) -> bool {
        match Self::normalized_expected_byte_count(expected_byte_count) {
            Some(expected) => downloaded_byte_count == expected,
            None => downloaded_byte_count > 0,
        }
    }

    pub fn is_complete_download_response(response: &RemoteMediaHttpResponse) -> bool {
        response.status_code == 200
    }

    pub fn is_complete_download_response_with_byte_count(
        response: &RemoteMediaHttpResponse,
        downloaded_byte_count: i64,
    ) -> bool {
        if response.status_code == 200 {
            let declared = response.expected_content_length();
            return downloaded_byte_count > 0 && (declared <= 0 || downloaded_byte_count == declared);
        }
        if response.status_code != 206 || downloaded_byte_count <= 0 {
            return false;
        }
        let Some((start, end, total)) = response
            .value(CONTENT_RANGE_HEADER)
            .and_then(Self::complete_content_range)
        else {
            return false;
        };
        start == 0 && end + 1 == total && downloaded_byte_count == total
    }

    fn complete_content_range(raw_value: &str) -> Option<(i64, i64, i64)> {
        let value = trimming_whitespaces_and_newlines(raw_value);
        if !value.to_lowercase().starts_with(CONTENT_RANGE_BYTES_UNIT) {
            return None;
        }
        let rest = value.get(CONTENT_RANGE_BYTES_UNIT.len()..)?;
        let components: Vec<&str> = rest.split('/').collect();
        let [bounds, total] = components.as_slice() else {
            return None;
        };
        let total = total.parse::<i64>().ok().filter(|total| *total > 0)?;
        let bounds: Vec<&str> = bounds.split('-').collect();
        let [start, end] = bounds.as_slice() else {
            return None;
        };
        let start = start.parse::<i64>().ok()?;
        let end = end.parse::<i64>().ok()?;
        if start < 0 || end < start || end >= total {
            return None;
        }
        Some((start, end, total))
    }

    pub fn should_retry_download(status_code: i64) -> bool {
        matches!(status_code, -1 | 206 | 404 | 408 | 425 | 429 | 500..=599)
    }

    fn download_error_label(error: &RemoteMediaTransferError) -> String {
        match error {
            RemoteMediaTransferError::Url(error) => format!("url_error_{:?}", error.code),
            RemoteMediaTransferError::Cache(_) => "RemoteMediaDiskCacheError".to_owned(),
            RemoteMediaTransferError::File(_) => "CocoaError".to_owned(),
            RemoteMediaTransferError::Cancelled(_) => "CancellationError".to_owned(),
            RemoteMediaTransferError::ProtectedRefusal(_) => "RemoteMediaRefusalReport".to_owned(),
            RemoteMediaTransferError::Other(_) => "Error".to_owned(),
        }
    }

    /// not debug only on purpose, a failed download is the only explanation for a loader blink
    /// with no sound
    /// same log channel as the player so the cause sits next to its play
    fn log_download_failure(message: String, cx: &AsyncApp) {
        cx.update(|cx| {
            Sai::log(LOG_NAME, move || message, SaiLogKind::Custom(LOG_CHANNEL.to_owned()), cx);
        });
    }

    fn log_identifier(value: &str) -> String {
        character_prefix(value, LOG_IDENTIFIER_PREFIX_LENGTH).to_owned()
    }

    async fn ingest_transfer(cache_key: String, downloaded_bytes: i64, total_bytes: i64, is_finished: bool) {
        RiseAccountRuntime::current()
            .spawn(async move {
                RiseMediaTransferHub::shared()
                    .ingest(cache_key, downloaded_bytes, total_bytes, is_finished)
                    .await;
            })
            .await
            .ok();
    }

    fn register(
        message_id: &str,
        chat_id: &str,
        chat_name: &str,
        file_url: &Path,
        remote: Option<&Url>,
        base: &Path,
    ) {
        Self::with_index_lock(|| {
            let Some(bytes) = Self::valid_file_byte_count(file_url) else {
                FileManager.remove_item(file_url).ok();
                return;
            };
            let now = RiseClock::now().time_interval_since_1970();
            let mut index = Self::decode_index_unlocked(base).unwrap_or_default();
            index.insert(
                message_id.to_owned(),
                VoiceMessageDiskCacheIndexEntry {
                    message_id: message_id.to_owned(),
                    chat_id: chat_id.to_owned(),
                    chat_name: (if chat_name.is_empty() { chat_id } else { chat_name }).to_owned(),
                    file_name: Self::last_path_component(file_url),
                    remote_url: remote.map(|remote| remote.absolute_string().to_owned()),
                    byte_count: bytes,
                    cached_at: now,
                },
            );
            Self::touch_file(file_url, now);
            let index = Self::reconcile_index_unlocked(index, base);
            let index = Self::prune_index_unlocked(index, base, Some(message_id));
            Self::write_index_unlocked(&index, base).ok();
        });
    }

    fn touch_index(message_id: &str, chat_id: &str, chat_name: &str, file_url: &Path, remote: &Url, base: &Path) {
        Self::with_index_lock(|| {
            let Some(bytes) = Self::valid_file_byte_count(file_url) else {
                FileManager.remove_item(file_url).ok();
                let mut index = Self::decode_index_unlocked(base).unwrap_or_default();
                index.remove(message_id);
                Self::write_index_unlocked(&index, base).ok();
                return;
            };

            let mut index = Self::decode_index_unlocked(base)
                .unwrap_or_else(|| Self::reconcile_index_unlocked(HashMap::new(), base));
            let prior_chat_name = index.get(message_id).map(|prior| prior.chat_name.clone());
            let now = RiseClock::now().time_interval_since_1970();
            let resolved_chat_name = if chat_name.is_empty() {
                prior_chat_name.unwrap_or_else(|| chat_id.to_owned())
            } else {
                chat_name.to_owned()
            };
            index.insert(
                message_id.to_owned(),
                VoiceMessageDiskCacheIndexEntry {
                    message_id: message_id.to_owned(),
                    chat_id: chat_id.to_owned(),
                    chat_name: resolved_chat_name,
                    file_name: Self::last_path_component(file_url),
                    remote_url: Some(remote.absolute_string().to_owned()),
                    byte_count: bytes,
                    cached_at: now,
                },
            );
            Self::touch_file(file_url, now);

            if index.len() > MAXIMUM_ENTRY_COUNT || Self::total_byte_count(&index) > MAXIMUM_TOTAL_BYTE_COUNT {
                index = Self::reconcile_index_unlocked(index, base);
                index = Self::prune_index_unlocked(index, base, Some(message_id));
            }
            Self::write_index_unlocked(&index, base).ok();
        });
    }

    pub fn clear_message(message_id: &str) {
        let Ok(base) = Self::active_base_directory() else {
            return;
        };
        Self::with_index_lock(|| {
            let mut index = Self::decode_index_unlocked(&base)
                .unwrap_or_else(|| Self::reconcile_index_unlocked(HashMap::new(), &base));
            let Some(entry) = index.remove(message_id) else {
                return;
            };
            Self::remove_entry_files_unlocked(&entry, message_id, &base);
            Self::write_index_unlocked(&index, &base).ok();
        });
    }

    pub fn clear_chat(chat_id: &str) {
        let Ok(base) = Self::active_base_directory() else {
            return;
        };
        Self::with_index_lock(|| {
            let mut index = Self::decode_index_unlocked(&base)
                .unwrap_or_else(|| Self::reconcile_index_unlocked(HashMap::new(), &base));
            index.retain(|_, entry| entry.chat_id != chat_id);
            FileManager.remove_item(&base.join(Self::chat_directory_name(chat_id))).ok();
            if let Some(legacy_name) = Self::legacy_chat_directory_name(chat_id) {
                FileManager.remove_item(&base.join(legacy_name)).ok();
            }
            Self::write_index_unlocked(&index, &base).ok();
        });
    }

    pub fn clear_all(user_id: Option<&str>) {
        let Ok(dir) = Self::base_directory(user_id) else {
            return;
        };
        Self::with_index_lock(|| {
            if let Ok(items) = FileManager.contents_of_directory_at(&dir, DirectoryEnumerationOptions::default()) {
                for url in items {
                    FileManager.remove_item(&url).ok();
                }
            }
            Self::create_protected_directory(&dir).ok();
            Self::write_index_unlocked(&HashMap::new(), &dir).ok();
        });
    }

    pub fn remove_persisted_account(user_id: &str) {
        let Ok(root) = Self::root_directory() else {
            return;
        };
        let dir = root
            .join(ACCOUNTS_FOLDER)
            .join(Self::account_directory_name(Some(user_id)));
        Self::with_index_lock(|| {
            FileManager.remove_item(&dir).ok();
        });
    }

    pub fn folder_path_for_inspector() -> String {
        Self::active_base_directory()
            .map(|base| base.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "(unavailable)".to_owned())
    }

    pub fn storage_directory_url(user_id: Option<&str>) -> Option<PathBuf> {
        Self::base_directory(user_id).ok()
    }

    pub fn chat_usage(user_id: Option<&str>) -> Vec<VoiceMessageDiskCacheChatUsage> {
        let Ok(base) = Self::base_directory(user_id) else {
            return Vec::new();
        };
        let index = Self::with_index_lock(|| Self::decode_index_unlocked(&base).unwrap_or_default());
        let mut grouped: HashMap<String, Vec<VoiceMessageDiskCacheIndexEntry>> = HashMap::new();
        for entry in index.into_values() {
            grouped.entry(entry.chat_id.clone()).or_default().push(entry);
        }
        let mut usage: Vec<VoiceMessageDiskCacheChatUsage> = grouped
            .into_iter()
            .map(|(chat_id, entries)| VoiceMessageDiskCacheChatUsage {
                chat_name: entries
                    .first()
                    .map_or_else(|| chat_id.clone(), |entry| entry.chat_name.clone()),
                entry_count: i64::try_from(entries.len()).unwrap_or(i64::MAX),
                byte_count: entries.iter().map(|entry| entry.byte_count.max(0)).sum(),
                chat_id,
            })
            .collect();
        usage.sort_by(|lhs, rhs| rhs.byte_count.cmp(&lhs.byte_count));
        usage
    }

    pub fn list_groups() -> Vec<VoiceMessageDiskCacheChatGroup> {
        let Ok(base) = Self::active_base_directory() else {
            return Vec::new();
        };
        let index = Self::with_index_lock(|| {
            let value = Self::reconcile_index_unlocked(Self::decode_index_unlocked(&base).unwrap_or_default(), &base);
            let value = Self::prune_index_unlocked(value, &base, None);
            Self::write_index_unlocked(&value, &base).ok();
            value
        });

        let mut grouped: HashMap<String, Vec<VoiceMessageDiskCacheListedEntry>> = HashMap::new();
        for entry in index.into_values() {
            let url = Self::entry_url_for_entry(&entry, &base);
            let Some(bytes) = Self::valid_file_byte_count(&url) else {
                continue;
            };
            grouped
                .entry(entry.chat_id.clone())
                .or_default()
                .push(VoiceMessageDiskCacheListedEntry {
                    message_id: entry.message_id,
                    chat_id: entry.chat_id,
                    chat_name: entry.chat_name,
                    file_name: entry.file_name,
                    file_bytes: bytes,
                    cached_at: Date::from_time_interval_since_1970(entry.cached_at),
                    remote_url: entry.remote_url,
                });
        }
        let mut groups: Vec<VoiceMessageDiskCacheChatGroup> = grouped
            .into_iter()
            .map(|(chat_id, mut entries)| {
                let chat_name = entries
                    .first()
                    .map_or_else(|| chat_id.clone(), |entry| entry.chat_name.clone());
                entries.sort_by(|lhs, rhs| {
                    rhs.cached_at
                        .time_interval_since_1970()
                        .total_cmp(&lhs.cached_at.time_interval_since_1970())
                });
                VoiceMessageDiskCacheChatGroup {
                    chat_id,
                    chat_name,
                    entries,
                }
            })
            .collect();
        let locale = LanguageStore::current_locale();
        groups.sort_by(|lhs, rhs| localized_case_insensitive_compare(&lhs.chat_name, &rhs.chat_name, locale.identifier()));
        groups
    }

    fn reconcile_index_unlocked(
        raw: HashMap<String, VoiceMessageDiskCacheIndexEntry>,
        base: &Path,
    ) -> HashMap<String, VoiceMessageDiskCacheIndexEntry> {
        let mut reconciled: HashMap<String, VoiceMessageDiskCacheIndexEntry> = HashMap::new();

        for (message_id, entry) in raw {
            let url = Self::resolved_entry_url_unlocked(&entry, &message_id, base);
            let Some(bytes) = Self::valid_file_byte_count(&url) else {
                Self::remove_entry_files_unlocked(&entry, &message_id, base);
                continue;
            };
            Self::apply_storage_attributes(&url);
            let mut updated = entry;
            updated.message_id = message_id.clone();
            updated.file_name = Self::last_path_component(&url);
            updated.byte_count = bytes;
            if updated.cached_at <= 0.0 {
                updated.cached_at = Self::file_access_time(&url);
            }
            reconciled.insert(message_id, updated);
        }

        let chat_directories = FileManager
            .contents_of_directory_at(base, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
            .unwrap_or_default();

        let indexed_paths: HashSet<PathBuf> = reconciled
            .iter()
            .map(|(message_id, entry)| Self::entry_url(message_id, &entry.chat_id, base))
            .collect();

        for chat_directory in chat_directories {
            let chat_directory_name = Self::last_path_component(&chat_directory);
            let mut is_directory = false;
            if chat_directory_name == INDEX_FILE_NAME
                || !FileManager.file_exists_is_directory(&chat_directory, &mut is_directory)
                || !is_directory
            {
                continue;
            }
            Self::apply_storage_attributes(&chat_directory);
            let files = FileManager
                .contents_of_directory_at(&chat_directory, DirectoryEnumerationOptions::default())
                .unwrap_or_default();

            for file in files {
                if !Self::is_regular_file(&file) {
                    continue;
                }
                if Self::path_extension(&file).to_lowercase() == STAGING_FILE_EXTENSION {
                    FileManager.remove_item(&file).ok();
                    continue;
                }
                let Some(bytes) = Self::valid_file_byte_count(&file) else {
                    FileManager.remove_item(&file).ok();
                    continue;
                };
                Self::apply_storage_attributes(&file);
                if indexed_paths.contains(&file) {
                    continue;
                }
                let file_name = Self::last_path_component(&file);
                if Self::is_hashed_chat_directory_name(&chat_directory_name)
                    || Self::is_hashed_message_file_name(&file_name)
                {
                    FileManager.remove_item(&file).ok();
                    continue;
                }
                let message_id = file
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if message_id.is_empty() || reconciled.contains_key(&message_id) {
                    continue;
                }
                let mut recovered = VoiceMessageDiskCacheIndexEntry {
                    message_id: message_id.clone(),
                    chat_id: chat_directory_name.clone(),
                    chat_name: chat_directory_name.clone(),
                    file_name,
                    remote_url: None,
                    byte_count: bytes,
                    cached_at: Self::file_access_time(&file),
                };
                let migrated = Self::resolved_entry_url_unlocked(&recovered, &message_id, base);
                if let Some(migrated_bytes) = Self::valid_file_byte_count(&migrated) {
                    recovered.file_name = Self::last_path_component(&migrated);
                    recovered.byte_count = migrated_bytes;
                    reconciled.insert(message_id, recovered);
                }
            }
        }
        reconciled
    }

    fn prune_index_unlocked(
        raw: HashMap<String, VoiceMessageDiskCacheIndexEntry>,
        base: &Path,
        preserving: Option<&str>,
    ) -> HashMap<String, VoiceMessageDiskCacheIndexEntry> {
        let mut index = raw;
        let mut total = Self::total_byte_count(&index);
        if index.len() <= MAXIMUM_ENTRY_COUNT && total <= MAXIMUM_TOTAL_BYTE_COUNT {
            return index;
        }

        let mut candidates: Vec<VoiceMessageDiskCacheIndexEntry> = index
            .values()
            .filter(|entry| Some(entry.message_id.as_str()) != preserving)
            .cloned()
            .collect();
        candidates.sort_by(|lhs, rhs| {
            lhs.cached_at
                .total_cmp(&rhs.cached_at)
                .then_with(|| lhs.message_id.cmp(&rhs.message_id))
        });

        for entry in candidates {
            if index.len() <= MAXIMUM_ENTRY_COUNT && total <= MAXIMUM_TOTAL_BYTE_COUNT {
                break;
            }
            Self::remove_entry_files_unlocked(&entry, &entry.message_id, base);
            index.remove(&entry.message_id);
            total -= entry.byte_count.max(0);
        }

        if (index.len() > MAXIMUM_ENTRY_COUNT || total > MAXIMUM_TOTAL_BYTE_COUNT)
            && let Some(message_id) = preserving
            && let Some(preserved) = index.get(message_id).cloned()
            && preserved.byte_count > MAXIMUM_TOTAL_BYTE_COUNT
        {
            Self::remove_entry_files_unlocked(&preserved, message_id, base);
            index.remove(message_id);
        }
        index
    }

    fn total_byte_count(index: &HashMap<String, VoiceMessageDiskCacheIndexEntry>) -> i64 {
        index.values().map(|entry| entry.byte_count.max(0)).sum()
    }

    fn entry_url(message_id: &str, chat_id: &str, base: &Path) -> PathBuf {
        base.join(Self::chat_directory_name(chat_id))
            .join(Self::message_file_name(message_id))
    }

    fn entry_url_for_entry(entry: &VoiceMessageDiskCacheIndexEntry, base: &Path) -> PathBuf {
        Self::entry_url(&entry.message_id, &entry.chat_id, base)
    }

    fn resolved_entry_url_unlocked(entry: &VoiceMessageDiskCacheIndexEntry, message_id: &str, base: &Path) -> PathBuf {
        let destination = Self::entry_url(message_id, &entry.chat_id, base);
        if Self::valid_file_byte_count(&destination).is_some() {
            return destination;
        }

        for legacy in Self::legacy_entry_urls(entry, message_id, base) {
            if legacy == destination || Self::valid_file_byte_count(&legacy).is_none() {
                continue;
            }
            return match Self::move_legacy_file(&legacy, &destination) {
                Ok(()) => {
                    Self::apply_storage_attributes(&destination);
                    destination
                }
                Err(_) => legacy,
            };
        }
        destination
    }

    fn move_legacy_file(legacy: &Path, destination: &Path) -> Result<(), CocoaError> {
        if let Some(parent) = destination.parent() {
            Self::create_protected_directory(parent)?;
        }
        FileManager.remove_item(destination).ok();
        FileManager.move_item(legacy, destination)
    }

    fn remove_entry_files_unlocked(entry: &VoiceMessageDiskCacheIndexEntry, message_id: &str, base: &Path) {
        let current = Self::entry_url(message_id, &entry.chat_id, base);
        FileManager.remove_item(&current).ok();
        for legacy in Self::legacy_entry_urls(entry, message_id, base) {
            if legacy != current {
                FileManager.remove_item(&legacy).ok();
            }
        }
    }

    fn legacy_entry_urls(entry: &VoiceMessageDiskCacheIndexEntry, message_id: &str, base: &Path) -> Vec<PathBuf> {
        let Some(directory_name) = Self::legacy_chat_directory_name(&entry.chat_id) else {
            return Vec::new();
        };
        let directory = base.join(directory_name);
        let mut names: Vec<String> = Vec::new();
        if let Some(indexed) = Self::legacy_file_name(&entry.file_name) {
            names.push(indexed);
        }
        if let Some(calculated) = Self::legacy_file_name(&format!("{message_id}{MESSAGE_FILE_EXTENSION}"))
            && !names.contains(&calculated)
        {
            names.push(calculated);
        }
        names.into_iter().map(|name| directory.join(name)).collect()
    }

    fn legacy_chat_directory_name(chat_id: &str) -> Option<String> {
        let value = chat_id.replace('/', "_");
        if value.is_empty() || value == "." || value == ".." || !Self::is_single_path_component(&value) {
            return None;
        }
        Some(value)
    }

    fn legacy_file_name(value: &str) -> Option<String> {
        if value.is_empty() || !Self::is_single_path_component(value) || value.contains('/') || value.contains('\\') {
            return None;
        }
        Some(value.to_owned())
    }

    fn is_single_path_component(value: &str) -> bool {
        Path::new(value).file_name().is_some_and(|name| name == value)
    }

    fn is_hashed_chat_directory_name(value: &str) -> bool {
        value
            .strip_prefix(CHAT_DIRECTORY_PREFIX)
            .is_some_and(Self::is_sha256_hex)
    }

    fn is_hashed_message_file_name(value: &str) -> bool {
        value
            .strip_prefix(MESSAGE_FILE_PREFIX)
            .and_then(|rest| rest.strip_suffix(MESSAGE_FILE_EXTENSION))
            .is_some_and(Self::is_sha256_hex)
    }

    fn is_sha256_hex(digest: &str) -> bool {
        digest.chars().count() == SHA256_HEX_LENGTH && digest.chars().all(|character| character.is_ascii_hexdigit())
    }

    fn valid_file_byte_count(url: &Path) -> Option<i64> {
        let bytes = Self::regular_file_byte_count(url)?;
        if bytes <= MINIMUM_PAYLOAD_BYTE_COUNT || bytes > MAXIMUM_FILE_BYTE_COUNT || !Self::is_likely_audio_file(url) {
            return None;
        }
        Some(bytes)
    }

    fn regular_file_byte_count(url: &Path) -> Option<i64> {
        let attributes = FileManager.attributes_of_item(url).ok()?;
        (attributes.r#type == FileAttributeType::TypeRegular).then_some(attributes.size)
    }

    fn is_regular_file(url: &Path) -> bool {
        FileManager
            .attributes_of_item(url)
            .is_ok_and(|attributes| attributes.r#type == FileAttributeType::TypeRegular)
    }

    fn is_likely_audio_file(url: &Path) -> bool {
        let Ok(handle) = std::fs::File::open(url) else {
            return false;
        };
        let mut head = Vec::with_capacity(AUDIO_HEAD_BYTE_COUNT);
        let limit = u64::try_from(AUDIO_HEAD_BYTE_COUNT).unwrap_or(u64::MAX);
        if handle.take(limit).read_to_end(&mut head).is_err() || head.len() < 2 {
            return false;
        }
        Self::is_likely_audio_data(&head)
    }

    fn is_likely_audio_data(data: &[u8]) -> bool {
        let head = &data[..data.len().min(AUDIO_HEAD_BYTE_COUNT)];
        if head.len() < 2 {
            return false;
        }
        if AUDIO_SIGNATURES.iter().any(|signature| head.starts_with(signature)) {
            return true;
        }
        if head.len() >= 8 && head[4..8] == ISO_MEDIA_FILE_TYPE_BOX {
            return true;
        }
        head[0] == 0xFF && ((head[1] & 0xE0) == 0xE0 || (head[1] & 0xF6) == 0xF0)
    }

    fn file_access_time(url: &Path) -> TimeInterval {
        FileManager
            .attributes_of_item(url)
            .ok()
            .and_then(|attributes| attributes.modification_date)
            .and_then(|date| date.duration_since(UNIX_EPOCH).ok())
            .map_or_else(
                || RiseClock::now().time_interval_since_1970(),
                |elapsed| elapsed.as_secs_f64(),
            )
    }

    fn touch_file(url: &Path, timestamp: TimeInterval) {
        let modified = UNIX_EPOCH + Duration::from_secs_f64(timestamp.max(0.0));
        if let Ok(file) = std::fs::File::options().write(true).open(url) {
            file.set_modified(modified).ok();
        }
        Self::apply_storage_attributes(url);
    }

    fn create_protected_directory(url: &Path) -> Result<(), CocoaError> {
        FileManager.create_directory(url, true)?;
        Self::apply_storage_attributes(url);
        Ok(())
    }

    fn apply_storage_attributes(url: &Path) {
        let values = URLResourceValues {
            is_excluded_from_backup: Some(true),
        };
        let _ = set_resource_values(url, &values);
    }

    fn with_index_lock<T>(operation: impl FnOnce() -> T) -> T {
        let _guard = INDEX_LOCK.lock();
        operation()
    }

    fn last_path_component(url: &Path) -> String {
        url.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn path_extension(url: &Path) -> String {
        url.extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}
