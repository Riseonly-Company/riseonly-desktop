use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_platform::AppPaths;
use rise_platform::file_manager::{CocoaError, CocoaErrorCode, FileManager};
use rise_platform::uniform_type::UTType;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{MEDIA_PICKER_RECENT_FILES, MEDIA_PICKER_RECENT_FILES_ACCOUNT_SCOPE};
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerAudioPayload, MediaPickerItem, MediaPickerItemKind,
};

static APPLICATION_SUPPORT_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::compiled().canon())
        .ok()
        .map(|paths| paths.data_dir().to_path_buf())
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaPickerRecentKind {
    File,
    Audio,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPickerRecentFile {
    pub id: String,
    pub kind: MediaPickerRecentKind,
    pub local_path: String,
    pub display_name: String,
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<TimeInterval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_path: Option<String>,
    pub last_used_at: TimeInterval,
}

impl Eq for MediaPickerRecentFile {}

impl Hash for MediaPickerRecentFile {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.kind.hash(state);
        self.local_path.hash(state);
        self.display_name.hash(state);
        self.mime_type.hash(state);
        self.title.hash(state);
        self.artist.hash(state);
        self.duration.map(f64::to_bits).hash(state);
        self.cover_path.hash(state);
        self.last_used_at.to_bits().hash(state);
    }
}

impl MediaPickerRecentFile {
    pub fn local_url(&self) -> PathBuf {
        PathBuf::from(&self.local_path)
    }

    pub fn cover_url(&self) -> Option<PathBuf> {
        self.cover_path.as_ref().map(PathBuf::from)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaPickerRecentAccountStamp {
    pub user_id: Option<String>,
    pub epoch: Option<String>,
}

impl MediaPickerRecentAccountStamp {
    pub fn current() -> Self {
        Self {
            user_id: SaiAccountCacheScope::active_user_id(),
            epoch: SaiAccountCacheScope::active_epoch(),
        }
    }

    pub fn is_current(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(self.user_id.as_deref(), self.epoch.as_deref())
    }
}

/// what import_file fails with, cancellation or the file error of the copy
#[derive(Clone, Debug)]
pub enum MediaPickerRecentFilesError {
    Cancelled,
    File(CocoaError),
}

struct MediaPickerRecentFilesPreparedImport {
    entry: MediaPickerRecentFile,
    copied_urls: Vec<PathBuf>,
}

struct MediaPickerRecentFilesImportCancellation(Arc<AtomicBool>);

impl Drop for MediaPickerRecentFilesImportCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// files and audio picked from disk, kept per account so the picker can offer them again
pub struct MediaPickerRecentFiles {
    entries: Vec<MediaPickerRecentFile>,
    loaded_defaults_key: Option<String>,
}

impl MediaPickerRecentFiles {
    const LOGICAL_DEFAULTS_KEY: &'static str = MEDIA_PICKER_RECENT_FILES;
    const MAXIMUM_ENTRIES: usize = 40;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        let mut files = Self {
            entries: Vec::new(),
            loaded_defaults_key: None,
        };
        files.activate();
        files
    }

    pub fn entries(&self) -> &[MediaPickerRecentFile] {
        &self.entries
    }

    pub fn files(
        &mut self,
        matching: &str,
        kind: MediaPickerRecentKind,
        cx: &mut Context<Self>,
    ) -> Vec<MediaPickerRecentFile> {
        self.activate_account_state(cx);
        let needle = trimming_whitespaces_and_newlines(matching).to_lowercase();
        let file_manager = FileManager;
        self.entries
            .iter()
            .filter(|entry| {
                if entry.kind != kind || !file_manager.file_exists(Path::new(&entry.local_path)) {
                    return false;
                }
                if needle.is_empty() {
                    return true;
                }
                [Some(&entry.display_name), entry.title.as_ref(), entry.artist.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|value| value.to_lowercase().contains(&needle))
            })
            .cloned()
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn import_file(
        &mut self,
        source_url: &Path,
        kind: MediaPickerRecentKind,
        display_name: &str,
        title: Option<String>,
        artist: Option<String>,
        duration: Option<TimeInterval>,
        cover_url: Option<&Path>,
        cx: &mut Context<Self>,
    ) -> Result<MediaPickerRecentFile, MediaPickerRecentFilesError> {
        self.activate_account_state(cx);
        let prepared = Self::prepare_import(
            source_url,
            kind,
            display_name,
            title,
            artist,
            duration,
            cover_url,
            SaiAccountCacheScope::active_user_id().as_deref(),
            &|| false,
        )?;
        Ok(self.commit(prepared, cx))
    }

    /// async import, copies off the main thread and only the state commit comes back
    /// dropping the returned task cancels the copy
    #[allow(clippy::too_many_arguments)]
    pub fn import_file_async(
        &mut self,
        source_url: PathBuf,
        kind: MediaPickerRecentKind,
        display_name: String,
        title: Option<String>,
        artist: Option<String>,
        duration: Option<TimeInterval>,
        cover_url: Option<PathBuf>,
        expected_account_stamp: Option<MediaPickerRecentAccountStamp>,
        cx: &mut Context<Self>,
    ) -> Task<Result<MediaPickerRecentFile, MediaPickerRecentFilesError>> {
        let account_stamp = expected_account_stamp.unwrap_or_else(MediaPickerRecentAccountStamp::current);
        if !account_stamp.is_current() {
            return Task::ready(Err(MediaPickerRecentFilesError::Cancelled));
        }
        self.activate_account_state(cx);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = MediaPickerRecentFilesImportCancellation(cancelled.clone());
        let account_user_id = account_stamp.user_id.clone();
        let operation = RiseAccountRuntime::current().spawn_blocking(move || {
            let is_cancelled = || cancelled.load(Ordering::Acquire);
            let prepared = Self::prepare_import(
                &source_url,
                kind,
                &display_name,
                title,
                artist,
                duration,
                cover_url.as_deref(),
                account_user_id.as_deref(),
                &is_cancelled,
            )?;
            if is_cancelled() {
                Self::remove_copied_urls(&prepared.copied_urls);
                return Err(MediaPickerRecentFilesError::Cancelled);
            }
            Ok(prepared)
        });
        cx.spawn(async move |this, cx| {
            let _cancellation = cancellation;
            let prepared = match operation.await {
                Ok(result) => result?,
                Err(_) => return Err(MediaPickerRecentFilesError::Cancelled),
            };
            let copied_urls = prepared.copied_urls.clone();
            let committed = if account_stamp.is_current() {
                this.update(cx, |this, cx| {
                    this.activate_account_state(cx);
                    this.commit(prepared, cx)
                })
                .ok()
            } else {
                None
            };
            match committed {
                Some(entry) => Ok(entry),
                None => {
                    Self::remove_prepared_import(copied_urls).await;
                    Err(MediaPickerRecentFilesError::Cancelled)
                }
            }
        })
    }

    fn commit(
        &mut self,
        prepared: MediaPickerRecentFilesPreparedImport,
        cx: &mut Context<Self>,
    ) -> MediaPickerRecentFile {
        let entry = prepared.entry;
        self.entries.retain(|existing| {
            !(existing.kind == entry.kind && existing.display_name.to_lowercase() == entry.display_name.to_lowercase())
        });
        self.entries.insert(0, entry.clone());
        self.trim_and_persist();
        cx.notify();
        entry
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_import(
        source_url: &Path,
        kind: MediaPickerRecentKind,
        display_name: &str,
        title: Option<String>,
        artist: Option<String>,
        duration: Option<TimeInterval>,
        cover_url: Option<&Path>,
        account_user_id: Option<&str>,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<MediaPickerRecentFilesPreparedImport, MediaPickerRecentFilesError> {
        if is_cancelled() {
            return Err(MediaPickerRecentFilesError::Cancelled);
        }
        let directory = Self::cache_directory(account_user_id).map_err(MediaPickerRecentFilesError::File)?;
        let mut copied_urls: Vec<PathBuf> = Vec::new();
        let result = (|| {
            let file_url = Self::persistent_copy(source_url, &directory, display_name, is_cancelled)?;
            copied_urls.push(file_url.clone());
            if is_cancelled() {
                return Err(MediaPickerRecentFilesError::Cancelled);
            }

            let stored_cover = match cover_url {
                Some(cover_url) => {
                    let preferred_name = format!("cover-{}.jpg", Uuid::new_v4().uuid_string());
                    let stored = Self::persistent_copy(cover_url, &directory, &preferred_name, is_cancelled)?;
                    copied_urls.push(stored.clone());
                    if is_cancelled() {
                        return Err(MediaPickerRecentFilesError::Cancelled);
                    }
                    Some(stored)
                }
                None => None,
            };

            let mime = file_url
                .extension()
                .and_then(|extension| extension.to_str())
                .and_then(UTType::from_filename_extension)
                .and_then(|content_type| content_type.preferred_mime_type())
                .unwrap_or_else(|| {
                    if kind == MediaPickerRecentKind::Audio {
                        "audio/mpeg".to_string()
                    } else {
                        "application/octet-stream".to_string()
                    }
                });
            Ok(MediaPickerRecentFilesPreparedImport {
                entry: MediaPickerRecentFile {
                    id: Uuid::new_v4().uuid_string(),
                    kind,
                    local_path: file_url.to_string_lossy().into_owned(),
                    display_name: display_name.to_string(),
                    mime_type: mime,
                    title,
                    artist,
                    duration,
                    cover_path: stored_cover.map(|path| path.to_string_lossy().into_owned()),
                    last_used_at: RiseClock::now().time_interval_since_1970(),
                },
                copied_urls: Vec::new(),
            })
        })();
        match result {
            Ok(mut prepared) => {
                prepared.copied_urls = copied_urls;
                Ok(prepared)
            }
            Err(error) => {
                Self::remove_copied_urls(&copied_urls);
                Err(error)
            }
        }
    }

    async fn remove_prepared_import(copied_urls: Vec<PathBuf>) {
        let _ = RiseAccountRuntime::current()
            .spawn_blocking(move || Self::remove_copied_urls(&copied_urls))
            .await;
    }

    fn remove_copied_urls(copied_urls: &[PathBuf]) {
        let file_manager = FileManager;
        for url in copied_urls {
            let _ = file_manager.remove_item(url);
        }
    }

    pub fn item(&mut self, from: &MediaPickerRecentFile, cx: &mut Context<Self>) -> Option<MediaPickerItem> {
        self.activate_account_state(cx);
        if !FileManager.file_exists(Path::new(&from.local_path)) {
            self.remove(from, cx);
            return None;
        }
        let audio_payload = if from.kind == MediaPickerRecentKind::Audio {
            Some(MediaPickerAudioPayload {
                title: from.title.clone().unwrap_or_else(|| {
                    Path::new(&from.local_path)
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_default()
                }),
                artist: from.artist.clone().unwrap_or_else(|| tr("music_unknown_artist")),
                duration: from.duration.unwrap_or(0.0),
                cover_url: from.cover_url().as_deref().and_then(Self::file_url),
            })
        } else {
            None
        };
        Some(MediaPickerItem::new(
            Uuid::new_v4().uuid_string(),
            MediaPickerItemKind::File,
            Self::file_url(&from.local_url())?,
            from.display_name.clone(),
            None,
            None,
            None,
            audio_payload,
            true,
        ))
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.entries = Vec::new();
        self.loaded_defaults_key = None;
        cx.notify();
    }

    pub fn activate_account_state(&mut self, cx: &mut Context<Self>) {
        if self.activate() {
            cx.notify();
        }
    }

    pub fn remove_persisted_account(&self, user_id: &str) {
        UserDefaults::standard().remove_object(&Self::scoped_defaults_key(user_id));
        if let Ok(directory) = Self::cache_directory(Some(user_id)) {
            let _ = FileManager.remove_item(&directory);
        }
    }

    /// activates the account state without the observation
    /// true when the entries were reloaded
    fn activate(&mut self) -> bool {
        self.migrate_legacy_state_if_needed();
        let key = Self::defaults_key();
        if self.loaded_defaults_key.as_deref() == Some(key.as_str()) {
            return false;
        }
        self.load(&key);
        self.loaded_defaults_key = Some(key);
        true
    }

    fn load(&mut self, defaults_key: &str) {
        self.entries = Vec::new();
        let Some(data) = UserDefaults::standard().data(defaults_key) else {
            return;
        };
        let Ok(decoded) = serde_json::from_slice::<Vec<MediaPickerRecentFile>>(&data) else {
            return;
        };
        let file_manager = FileManager;
        self.entries = decoded
            .into_iter()
            .filter(|entry| file_manager.file_exists(Path::new(&entry.local_path)))
            .collect();
        self.persist();
    }

    fn remove(&mut self, entry: &MediaPickerRecentFile, cx: &mut Context<Self>) {
        self.entries.retain(|existing| existing.id != entry.id);
        self.persist();
        cx.notify();
    }

    fn trim_and_persist(&mut self) {
        let mut removed_urls: Vec<PathBuf> = Vec::new();
        if self.entries.len() > Self::MAXIMUM_ENTRIES {
            for entry in self.entries.drain(Self::MAXIMUM_ENTRIES..) {
                removed_urls.push(entry.local_url());
                if let Some(cover_path) = entry.cover_path {
                    removed_urls.push(PathBuf::from(cover_path));
                }
            }
        }
        self.persist();
        if removed_urls.is_empty() {
            return;
        }
        RiseAccountRuntime::current().spawn_blocking(move || Self::remove_copied_urls(&removed_urls));
    }

    fn persist(&self) {
        let Ok(data) = serde_json::to_vec(&self.entries) else {
            return;
        };
        UserDefaults::standard().set(&Self::defaults_key(), data);
    }

    pub fn storage_directory_url(user_id: Option<&str>) -> Option<PathBuf> {
        Self::cache_directory(user_id).ok()
    }

    fn cache_directory(user_id: Option<&str>) -> Result<PathBuf, CocoaError> {
        let Some(root) = APPLICATION_SUPPORT_ROOT.as_ref() else {
            return Err(CocoaError::new(CocoaErrorCode::FileNoSuchFile));
        };
        let directory = root
            .join("RecentAttachments")
            .join("accounts")
            .join(Self::account_directory_name(user_id));
        FileManager.create_directory(&directory, true)?;
        Ok(directory)
    }

    fn defaults_key() -> String {
        SaiAccountCacheScope::scoped_key(Self::LOGICAL_DEFAULTS_KEY)
    }

    fn scoped_defaults_key(user_id: &str) -> String {
        SaiAccountCacheScope::prefix(user_id) + Self::LOGICAL_DEFAULTS_KEY
    }

    fn account_directory_name(user_id: Option<&str>) -> String {
        let value = trimming_whitespaces_and_newlines(user_id.unwrap_or(""));
        if value.is_empty() {
            return "anonymous".to_string();
        }
        sha256_hex(value.as_bytes())
    }

    fn migrate_legacy_state_if_needed(&self) {
        let defaults = UserDefaults::standard();
        if SaiAccountCacheScope::active_user_id().is_none()
            || defaults.bool(MEDIA_PICKER_RECENT_FILES_ACCOUNT_SCOPE)
        {
            return;
        }
        let defaults_key = Self::defaults_key();
        let legacy = if defaults.object(&defaults_key).is_none() {
            defaults
                .data(Self::LOGICAL_DEFAULTS_KEY)
                .and_then(|data| serde_json::from_slice::<Vec<MediaPickerRecentFile>>(&data).ok())
                .and_then(|decoded| {
                    Self::cache_directory(SaiAccountCacheScope::active_user_id().as_deref())
                        .ok()
                        .map(|destination| (decoded, destination))
                })
        } else {
            None
        };
        if let Some((decoded, destination)) = legacy {
            let migrated: Vec<MediaPickerRecentFile> = decoded
                .into_iter()
                .filter_map(|entry| {
                    let file_path = Self::move_legacy_file(&entry.local_path, &destination)?;
                    let cover_path = entry
                        .cover_path
                        .as_deref()
                        .and_then(|path| Self::move_legacy_file(path, &destination));
                    Some(MediaPickerRecentFile {
                        local_path: file_path,
                        cover_path,
                        ..entry
                    })
                })
                .collect();
            if let Ok(migrated_data) = serde_json::to_vec(&migrated) {
                defaults.set(&defaults_key, migrated_data);
            }
        }
        defaults.remove_object(Self::LOGICAL_DEFAULTS_KEY);
        defaults.set(MEDIA_PICKER_RECENT_FILES_ACCOUNT_SCOPE, true);
    }

    fn move_legacy_file(path: &str, directory: &Path) -> Option<String> {
        let source = PathBuf::from(path);
        let file_manager = FileManager;
        if !file_manager.file_exists(&source) {
            return None;
        }
        if source.parent() == Some(directory) {
            return Some(source.to_string_lossy().into_owned());
        }
        let file_name = source.file_name()?.to_string_lossy().into_owned();
        let destination = directory.join(format!("{}-{file_name}", Uuid::new_v4().uuid_string()));
        file_manager.move_item(&source, &destination).ok()?;
        Some(destination.to_string_lossy().into_owned())
    }

    fn persistent_copy(
        source: &Path,
        into: &Path,
        preferred_name: &str,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<PathBuf, MediaPickerRecentFilesError> {
        let ext = source
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_default();
        let raw_stem = Self::deleting_path_extension(preferred_name);
        let stem: String = raw_stem
            .chars()
            .map(|scalar| {
                if scalar.is_alphanumeric() || scalar == '-' || scalar == '_' || scalar == ' ' {
                    scalar
                } else {
                    '-'
                }
            })
            .collect();
        let safe_stem = trimming_whitespaces_and_newlines(&stem);
        let base = if safe_stem.is_empty() { "attachment" } else { safe_stem };
        let mut file_name = format!("{}-{base}", Uuid::new_v4().uuid_string());
        if !ext.is_empty() {
            file_name.push('.');
            file_name.push_str(&ext);
        }
        let destination = into.join(file_name);
        let file_manager = FileManager;
        let copied = (|| {
            if is_cancelled() {
                return Err(MediaPickerRecentFilesError::Cancelled);
            }
            file_manager
                .copy_item(source, &destination)
                .map_err(MediaPickerRecentFilesError::File)?;
            if is_cancelled() {
                return Err(MediaPickerRecentFilesError::Cancelled);
            }
            Ok(())
        })();
        match copied {
            Ok(()) => Ok(destination),
            Err(error) => {
                let _ = file_manager.remove_item(&destination);
                Err(error)
            }
        }
    }

    fn deleting_path_extension(name: &str) -> &str {
        match name.rfind('.') {
            Some(dot) if dot > 0 && !name[dot + 1..].contains('/') && !name[..dot].ends_with('/') => &name[..dot],
            _ => name,
        }
    }

    fn file_url(path: &Path) -> Option<Url> {
        Url::from_file_path(path)
    }
}
