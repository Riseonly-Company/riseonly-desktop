use std::path::{Path, PathBuf};
use std::time::Duration;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::{URLRequest, URLSession};
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileManager};
use rise_platform::paths::AppPaths;
use rise_platform::url_resource_values::{URLResourceValues, set_resource_values};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::modules::notify::engine::core::rise_release_notes_wire_models::{
    RiseReleaseNote, RiseReleaseSlide,
};

const VIDEO_DIRECTORY_NAME: &str = "rise-release-video";
const VIDEO_FILE_EXTENSION: &str = "mp4";
const INCOMING_PREFIX: &str = ".incoming-";
const MAXIMUM_VIDEO_BYTES: usize = 5 * 1_024 * 1_024;
const SESSION_RESOURCE_TIMEOUT: Duration = Duration::from_secs(12);
const HTTP_STATUS_OK: i64 = 200;
const MEDIA_KIND_VIDEO: &str = "video";
const MEDIA_KIND_NONE: &str = "none";

/// puts a releases media on disk before the modal appears
/// first slide fetched before the snapshot, deadline so a dead url never blocks it
/// images go to the image cache, video is downloaded whole
pub enum RiseReleaseMediaWarmer {}

impl RiseReleaseMediaWarmer {
    pub const FIRST_SLIDE_DEADLINE: Duration = Duration::from_secs(4);
    pub const TAP_OPEN_DEADLINE: Duration = Duration::from_millis(600);

    /// fetches what the first slide needs or gives up at the deadline
    pub async fn warm_first_slide(note: &RiseReleaseNote, deadline: Duration) {
        let Some(slide) = note.slides.first().cloned() else {
            return;
        };
        Self::with_deadline(deadline, async move {
            Self::warm(&slide).await;
        })
        .await;
    }

    /// everything after the first slide, with the modal already on screen
    /// sequential on purpose, these compete with a running animation
    pub async fn warm_remaining_slides(note: &RiseReleaseNote) {
        for slide in note.slides.iter().skip(1) {
            Self::warm(slide).await;
        }
    }

    /// local file for a warmed clip or none if it never arrived
    /// player then falls back to the remote url
    pub fn local_video_url(remote_url: &str) -> Option<PathBuf> {
        let destination = Self::video_file_url(remote_url)?;
        let size = FileManager.attributes_of_item(&destination).ok()?.size;
        if size > 0 { Some(destination) } else { None }
    }

    pub fn storage_directory_url() -> Option<PathBuf> {
        Self::caches_directory().map(|caches| caches.join(VIDEO_DIRECTORY_NAME))
    }

    pub fn clear_disk_cache() {
        let Some(directory) = Self::storage_directory_url() else {
            return;
        };
        let Ok(items) = FileManager.contents_of_directory_at(&directory, DirectoryEnumerationOptions::default())
        else {
            return;
        };
        for url in items {
            FileManager.remove_item(&url).ok();
        }
    }

    async fn warm(slide: &RiseReleaseSlide) {
        let source = trimming_whitespaces_and_newlines(&slide.media_url).to_owned();
        if source.is_empty() {
            return;
        }
        let Some(url) = Url::from_string(&source) else {
            return;
        };

        match slide.media_kind.as_str() {
            MEDIA_KIND_VIDEO => Self::warm_video(url, &source).await,
            MEDIA_KIND_NONE => {}
            _ => Self::warm_image(url, &source).await,
        }
    }

    async fn warm_image(url: Url, source: &str) {
        let user_id = SaiAccountCacheScope::active_user_id();
        if CleverImageDiskCache::contains(source, user_id.as_deref(), None) {
            return;
        }
        let Some(data) = Self::download(url).await else {
            return;
        };
        CleverImageDiskCache::store_data(&data, source, user_id.as_deref(), None);
    }

    async fn warm_video(url: Url, source: &str) {
        let Some(destination) = Self::video_file_url(source) else {
            return;
        };
        if FileManager.file_exists(&destination) {
            return;
        }

        let Some(data) = Self::download(url).await else {
            return;
        };
        if data.len() > MAXIMUM_VIDEO_BYTES {
            return;
        }
        let Some(directory) = destination.parent().map(Path::to_path_buf) else {
            return;
        };
        FileManager.create_directory(&directory, true).ok();
        Self::write_atomically(&destination, &data).ok();
        Self::exclude_from_backup(&directory);
    }

    /// download with the status and emptiness checks both destinations share
    async fn download(url: Url) -> Option<Vec<u8>> {
        let mut request = URLRequest::new(url);
        request.timeout_interval = SESSION_RESOURCE_TIMEOUT;
        let (data, response) = URLSession::shared().data_for(request).await.ok()?;
        if response.status_code != HTTP_STATUS_OK || data.is_empty() {
            return None;
        }
        Some(data)
    }

    fn video_file_url(source: &str) -> Option<PathBuf> {
        let caches = Self::caches_directory()?;
        let name = CleverImageDiskCache::digest_hex(&CleverImageDiskCache::normalized_cache_key(source));
        if name.is_empty() {
            return None;
        }
        Some(
            caches
                .join(VIDEO_DIRECTORY_NAME)
                .join(format!("{name}.{VIDEO_FILE_EXTENSION}")),
        )
    }

    fn caches_directory() -> Option<PathBuf> {
        AppPaths::resolve(AppEnvironment::compiled().canon())
            .ok()
            .map(|paths| paths.cache_dir().to_path_buf())
    }

    fn write_atomically(destination: &Path, data: &[u8]) -> std::io::Result<()> {
        let directory = destination.parent().unwrap_or(destination);
        let staging = directory.join(format!(
            "{INCOMING_PREFIX}{}",
            Uuid::new_v4().uuid_string().to_lowercase()
        ));
        let written = std::fs::write(&staging, data).and_then(|()| std::fs::rename(&staging, destination));
        if written.is_err() {
            FileManager.remove_item(&staging).ok();
        }
        written
    }

    fn exclude_from_backup(directory: &Path) {
        let values = URLResourceValues {
            is_excluded_from_backup: Some(true),
        };
        set_resource_values(directory, &values).ok();
    }

    async fn with_deadline(
        deadline: Duration,
        work: impl Future<Output = ()> + Send + 'static,
    ) {
        let task = RiseAccountRuntime::current().spawn(work);
        tokio::select! {
            _ = task => {}
            () = tokio::time::sleep(deadline) => {}
        }
    }
}
