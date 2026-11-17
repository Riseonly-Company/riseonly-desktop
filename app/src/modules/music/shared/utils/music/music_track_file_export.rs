use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use gpui::AsyncApp;
use rise_catalog::MusicTrackStatus;
use rise_core::foundation::foundation_character::characters;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_file_name::host_file_name;
use rise_core::foundation::foundation_file_url::PathFlavor;
use rise_core::foundation::foundation_text::{
    character_boundaries, character_prefix, trimming_whitespaces_and_newlines,
};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileManager};
use rise_platform::paths::temporary_directory;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions,
};
use crate::modules::music::shared::utils::music::music_media_url::MusicMediaURL;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;

pub enum MusicTrackSavePolicy {}

impl MusicTrackSavePolicy {
    /// only user uploaded library tracks can leave the app as a file
    /// catalogue and artist releases stream under someone elses rights
    pub fn can_save(track: &TrackModel) -> bool {
        let is_durable_device_draft = track.is_device_only_draft();
        if !(track.status == MusicTrackStatus::LocalLibrary.wire() || is_durable_device_draft)
            || track.is_local_transfer_active()
        {
            return false;
        }
        !trimming_whitespaces_and_newlines(&track.stable_audio_url()).is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedTrackFile {
    pub files: Vec<PathBuf>,
    pub temporary_directory: Option<PathBuf>,
}

impl PreparedTrackFile {
    /// blocking file io, call it off the main thread
    pub fn cleanup(&self) {
        let Some(temporary_directory) = self.temporary_directory.as_ref() else {
            return;
        };
        let _ = FileManager.remove_item(temporary_directory);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MusicTrackFileExportError {
    #[error("not savable")]
    NotSavable,
    #[error("invalid source")]
    InvalidSource,
    #[error("account changed")]
    AccountChanged,
}

pub enum MusicTrackFileMaterializer {}

impl MusicTrackFileMaterializer {
    const STAGING_PREFIX: &'static str = "riseonly-track-export-";
    const STALE_DIRECTORY_SECONDS: f64 = 24.0 * 60.0 * 60.0;
    const STEM_CHARACTER_LIMIT: usize = 96;
    const FILE_NAME_BYTE_LIMIT: usize = 255;
    const EXTENSION_CHARACTER_LIMIT: usize = 12;
    const FALLBACK_STEM: &'static str = "Riseonly";
    const FALLBACK_EXTENSION: &'static str = "mp3";
    const CACHE_KEY_PREFIX: &'static str = "music-track-";

    /// prefers bytes the player already cached so saving a played track is a local copy
    /// not a second download
    pub async fn prepare(track: &TrackModel, cx: &AsyncApp) -> anyhow::Result<PreparedTrackFile> {
        if !MusicTrackSavePolicy::can_save(track) {
            return Err(anyhow::Error::new(MusicTrackFileExportError::NotSavable));
        }

        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return Err(anyhow::Error::new(MusicTrackFileExportError::AccountChanged));
        }

        let stable_audio_url = track.stable_audio_url();
        let track_id = track.id.clone();
        let lookup_user_id = account_user_id.clone();
        let stable_for_lookup = stable_audio_url.clone();
        let local_or_cached = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let local = Url::from_string(&stable_for_lookup)
                    .filter(Url::is_file_url)
                    .and_then(|url| url.to_file_path())
                    .filter(|path| FileManager.file_exists(path));
                local.or_else(|| MusicPlaybackDiskCache::find_cached_url(&track_id, lookup_user_id.as_deref()))
            })
            .await
            .ok()
            .flatten();

        let source = match local_or_cached {
            Some(source) => source,
            None => {
                let remote = MusicMediaURL::resolved_url_string(Some(stable_audio_url.as_str()))
                    .and_then(|resolved| Url::from_string(&resolved))
                    .ok_or_else(|| anyhow::Error::new(MusicTrackFileExportError::InvalidSource))?;
                let options = RemoteMediaDiskCacheMaterializeOptions {
                    cache_key: Some(format!("{}{}", Self::CACHE_KEY_PREFIX, track.id)),
                    suggested_filename: Some(remote.last_path_component()),
                    maximum_byte_count: Some(MusicPlaybackDiskCache::MAXIMUM_FILE_BYTE_COUNT),
                    expected_account_user_id: account_user_id.clone(),
                    expected_account_epoch: account_epoch.clone(),
                    ..RemoteMediaDiskCacheMaterializeOptions::default()
                };
                RemoteMediaDiskCache::shared().materialize(&remote, options, cx).await?
            }
        };

        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return Err(anyhow::Error::new(MusicTrackFileExportError::AccountChanged));
        }

        let file_name = Self::export_file_name(track, &source);
        let copied = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<PreparedTrackFile> {
                Self::cleanup_stale_directories();
                let directory = temporary_directory().join(format!(
                    "{}{}",
                    Self::STAGING_PREFIX,
                    Uuid::new_v4().uuid_string().to_lowercase()
                ));
                FileManager.create_directory(&directory, true)?;

                let destination = directory.join(&file_name);
                let lease = RemoteMediaDiskCache::lease(Some(&source));
                let result = FileManager.copy_item(&source, &destination);
                drop(lease);
                match result {
                    Ok(()) => Ok(PreparedTrackFile {
                        files: vec![destination],
                        temporary_directory: Some(directory),
                    }),
                    Err(error) => {
                        let _ = FileManager.remove_item(&directory);
                        Err(anyhow::Error::new(error))
                    }
                }
            })
            .await;
        copied.map_err(anyhow::Error::new)?
    }

    pub fn export_file_name(track: &TrackModel, source: &Path) -> String {
        let artist = Self::sanitize(&track.artist);
        let title = Self::sanitize(&track.title);
        let stem = if !artist.is_empty() && !title.is_empty() {
            format!("{artist} - {title}")
        } else if !title.is_empty() {
            title
        } else if !artist.is_empty() {
            artist
        } else {
            Self::FALLBACK_STEM.to_owned()
        };
        let source_extension = source.extension().map(|ext| ext.to_string_lossy().into_owned());
        let stable_extension =
            Url::from_string(&track.stable_audio_url()).map(|url| url.path_extension());
        let ext = Self::normalized_extension(source_extension.as_deref())
            .or_else(|| Self::normalized_extension(stable_extension.as_deref()))
            .unwrap_or_else(|| Self::FALLBACK_EXTENSION.to_owned());
        let prefix = character_prefix(&stem, Self::STEM_CHARACTER_LIMIT);
        let byte_budget = Self::FILE_NAME_BYTE_LIMIT.saturating_sub(1 + ext.len());
        let end = character_boundaries(prefix)
            .into_iter()
            .rev()
            .find(|boundary| *boundary <= byte_budget)
            .unwrap_or(0);
        let cut_stem = if end == 0 { Self::FALLBACK_STEM } else { &prefix[..end] };
        let name = host_file_name(&format!("{cut_stem}.{ext}"), PathFlavor::HOST);
        if name.is_empty() {
            format!("{}.{ext}", Self::FALLBACK_STEM)
        } else {
            name
        }
    }

    fn sanitize(raw: &str) -> String {
        let replaced: String = raw
            .chars()
            .map(|scalar| if Self::is_disallowed(scalar) { '_' } else { scalar })
            .collect();
        let value = trimming_whitespaces_and_newlines(&replaced);
        if value == "." || value == ".." {
            return String::new();
        }
        value.to_owned()
    }

    fn is_disallowed(scalar: char) -> bool {
        if matches!(scalar, '/' | '\\' | ':') || scalar.is_control() {
            return true;
        }
        matches!(
            u32::from(scalar),
            0x00AD
                | 0x0600..=0x0605
                | 0x061C
                | 0x06DD
                | 0x070F
                | 0x0890..=0x0891
                | 0x08E2
                | 0x180E
                | 0x200B..=0x200F
                | 0x202A..=0x202E
                | 0x2060..=0x2064
                | 0x2066..=0x206F
                | 0xFEFF
                | 0xFFF9..=0xFFFB
                | 0x110BD
                | 0x110CD
                | 0x13430..=0x1343F
                | 0x1BCA0..=0x1BCA3
                | 0x1D173..=0x1D17A
                | 0xE0001
                | 0xE0020..=0xE007F
        )
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
                .map(|elapsed: Duration| Date::from_time_interval_since_1970(elapsed.as_secs_f64()));
            if modified.is_some_and(|modified| modified < expiration) {
                let _ = FileManager.remove_item(&entry);
            }
        }
    }
}
