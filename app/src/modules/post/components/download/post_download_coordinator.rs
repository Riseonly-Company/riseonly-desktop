use std::cell::Cell;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{Either, select};
use gpui::{App, AsyncApp};
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_platform::AppPaths;
use rise_platform::download_mark::{self, DownloadOrigin};
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileManager};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::rise_app_quit_drain::QuitDrain;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions, RemoteMediaDownloadProgress,
};
use crate::core::media::rise_media_transfer_hub::MediaViewerVideoCacheKey;
use crate::core::media_viewer::media_save_to_library::MediaSaveToLibraryResult;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::post::components::download::post_watermark_renderer::{
    PostWatermarkLabel, PostWatermarkRenderer, PostWatermarkRendererRenderError,
};
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

const LOG_NAME: &str = "PostDownload";

/// rendered watermarked files kept so a second download of the same media is instant
/// keyed by media url not slide index since an edit can replace the media in place
pub enum PostDownloadCache {}

impl PostDownloadCache {
    /// file dates compare against the wall clock read through RiseClock so tests can pin it
    fn wall_now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs_f64(RiseClock::now().time_interval_since_1970().max(0.0))
    }

    pub const BYTE_LIMIT: i64 = 300 * 1024 * 1024;
    pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
    const DIRECTORY_NAME: &'static str = "post-downloads";
    const PART_MARK: &'static str = ".part-";

    pub fn directory() -> Option<PathBuf> {
        let url = AppPaths::resolve(AppEnvironment::current().canon())
            .ok()
            .map(|paths| paths.cache_dir().join(Self::DIRECTORY_NAME))?;
        let _ = FileManager.create_directory(&url, true);
        Some(url)
    }

    pub fn file(post_id: &str, media_url: &str, label: &PostWatermarkLabel, is_video: bool) -> Option<PathBuf> {
        let raw = [post_id, media_url, "wm", &PostWatermarkRenderer::VERSION.to_string(), &label.tag_line].join("|");
        let digest = sha256_hex(raw.as_bytes());
        let extension = if is_video { "mp4" } else { "jpg" };
        Some(Self::directory()?.join(format!("{digest}.{extension}")))
    }

    /// a hit is touched so the size sweep drops the least recently used file first
    pub fn hit(url: &Path) -> bool {
        let size = FileManager.attributes_of_item(url).map(|attributes| attributes.size).unwrap_or(0);
        if size <= 0 {
            return false;
        }
        if let Ok(file) = File::options().write(true).open(url) {
            let _ = file.set_modified(Self::wall_now());
        }
        true
    }

    pub fn is_part(url: &Path) -> bool {
        url.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.contains(Self::PART_MARK))
    }

    /// age spares a render that may still be running, zero when none is
    pub fn sweep_parts(older_than: Duration) {
        let Some(directory) = Self::directory() else {
            return;
        };
        let Ok(files) = FileManager.contents_of_directory_at(&directory, DirectoryEnumerationOptions::default())
        else {
            return;
        };
        let now = Self::wall_now();
        for url in files.iter().filter(|url| Self::is_part(url)) {
            let date = FileManager
                .attributes_of_item(url)
                .ok()
                .and_then(|attributes| attributes.modification_date)
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if now.duration_since(date).unwrap_or_default() >= older_than {
                let _ = FileManager.remove_item(url);
            }
        }
    }

    pub fn remove_staging(staging: &Path) {
        let Some(prefix) = staging.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        let Some(parent) = staging.parent() else {
            return;
        };
        let siblings =
            FileManager.contents_of_directory_at(parent, DirectoryEnumerationOptions::default()).unwrap_or_default();
        for url in siblings {
            if url.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(prefix)) {
                let _ = FileManager.remove_item(&url);
            }
        }
    }

    pub fn prune() {
        Self::sweep_parts(Duration::ZERO);
        let Some(directory) = Self::directory() else {
            return;
        };
        let Ok(files) = FileManager.contents_of_directory_at(&directory, DirectoryEnumerationOptions::default())
        else {
            return;
        };
        let now = Self::wall_now();
        let mut alive: Vec<(PathBuf, SystemTime, i64)> = Vec::new();
        for url in files {
            let attributes = FileManager.attributes_of_item(&url).ok();
            let date = attributes
                .as_ref()
                .and_then(|attributes| attributes.modification_date)
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if now.duration_since(date).unwrap_or_default() > Self::MAX_AGE {
                let _ = FileManager.remove_item(&url);
                continue;
            }
            alive.push((url, date, attributes.map(|attributes| attributes.size).unwrap_or(0)));
        }
        let mut total: i64 = alive.iter().map(|entry| entry.2).sum();
        alive.sort_by(|a, b| a.1.cmp(&b.1));
        for (url, _, size) in alive {
            if total <= Self::BYTE_LIMIT {
                break;
            }
            let _ = FileManager.remove_item(&url);
            total -= size;
        }
    }
}

pub type PostDownloadCoordinatorProgress = Rc<dyn Fn(usize, f64, &mut App)>;

/// one download run, source from the shared media cache
/// watermark when asked, each file goes into the picked folder
/// a failed render never falls back to the bare file
pub enum PostDownloadCoordinator {}

impl PostDownloadCoordinator {
    const SOURCE_ATTEMPTS: usize = 3;
    const VIDEO_FETCH_SHARE: f64 = 0.5;
    const PHOTO_FETCH_SHARE: f64 = 0.8;

    pub async fn run(
        post: GetPostFeedResponse,
        items: Vec<PostMediaSlide>,
        watermark: bool,
        folder: PathBuf,
        on_progress: PostDownloadCoordinatorProgress,
        cx: &AsyncApp,
    ) -> MediaSaveToLibraryResult {
        if items.is_empty() {
            return MediaSaveToLibraryResult::Failed;
        }
        let label = PostWatermarkLabel::new(&post);
        let _background = cx.update(|cx| QuitDrain::lease("post-download", cx));
        cx.background_executor().spawn(async { PostDownloadCache::prune() }).await;

        let total = items.len() as f64;
        let mut saved = 0_usize;
        let mut failed = 0_usize;
        for (index, slide) in items.iter().enumerate() {
            cx.update(|cx| on_progress(index + 1, index as f64 / total, cx));
            let reporting = Rc::clone(&on_progress);
            let progress_cx = cx.clone();
            let on_fraction = move |fraction: f64| {
                progress_cx.update(|cx| reporting(index + 1, (index as f64 + fraction) / total, cx));
            };
            match Self::produce(slide, &post, &label, watermark, &on_fraction, cx).await {
                Ok((file, is_video)) => {
                    let lease = RemoteMediaDiskCache::lease(Some(file.as_path()));
                    let destination_folder = folder.clone();
                    let name = Self::file_name(&post.id, index, is_video);
                    let written = cx
                        .background_executor()
                        .spawn(async move { Self::write_into(&file, &destination_folder, &name) })
                        .await;
                    drop(lease);
                    if written {
                        saved += 1;
                    } else {
                        cx.update(|cx| {
                            Sai::log(LOG_NAME, || format!("item {index}: folder write failed"), SaiLogKind::Ui, cx)
                        });
                        failed += 1;
                    }
                }
                Err(error) => {
                    cx.update(|cx| Sai::log(LOG_NAME, || format!("item {index} failed: {error}"), SaiLogKind::Ui, cx));
                    failed += 1;
                }
            }
        }
        cx.update(|cx| on_progress(items.len(), 1.0, cx));
        if saved == 0 {
            return MediaSaveToLibraryResult::Failed;
        }
        if failed > 0 {
            return MediaSaveToLibraryResult::Failed;
        }
        MediaSaveToLibraryResult::Saved
    }

    async fn produce(
        slide: &PostMediaSlide,
        post: &GetPostFeedResponse,
        label: &PostWatermarkLabel,
        watermark: bool,
        on_fraction: &dyn Fn(f64),
        cx: &AsyncApp,
    ) -> Result<(PathBuf, bool), PostWatermarkRendererRenderError> {
        use PostWatermarkRendererRenderError as RenderError;
        let media_url = match slide {
            PostMediaSlide::Image { url, .. } | PostMediaSlide::Video { url, .. } => url.as_str(),
        };
        let normalized = RemoteMediaURL::optional_normalized(Some(media_url)).ok_or(RenderError::DecodeFailed)?;
        let remote = Url::from_string(&normalized).ok_or(RenderError::DecodeFailed)?;
        let is_video = slide.is_video();
        let target =
            PostDownloadCache::file(&post.id, &normalized, label, is_video).ok_or(RenderError::EncodeFailed)?;
        if watermark && PostDownloadCache::hit(&target) {
            return Ok((target, is_video));
        }

        let share = if is_video { Self::VIDEO_FETCH_SHARE } else { Self::PHOTO_FETCH_SHARE };
        let moved = Cell::new(false);
        let cache_key = if is_video { MediaViewerVideoCacheKey::resolve(&normalized) } else { None };
        let filename = remote.last_path_component();
        let source = Self::fetch_source(
            &remote,
            cache_key,
            &filename,
            &|fraction| {
                moved.set(true);
                on_fraction(share * fraction);
            },
            cx,
        )
        .await?;
        if !watermark {
            return Ok((source, is_video));
        }
        let base = if moved.get() { share } else { 0.0 };

        let lease = RemoteMediaDiskCache::lease(Some(source.as_path()));
        let extension = if is_video { "mp4" } else { "jpg" };
        let staging = target.with_extension(format!("part-{}.{extension}", Uuid::new_v4().uuid_string()));
        let _cleanup = PostDownloadStagingGuard { staging: staging.clone() };
        if is_video {
            PostWatermarkRenderer::render_video(
                source,
                label.clone(),
                staging.clone(),
                &|fraction| on_fraction(base + (1.0 - base) * fraction),
                cx,
            )
            .await?;
        } else {
            let photo_source = source.clone();
            let photo_label = label.clone();
            let photo_staging = staging.clone();
            cx.background_executor()
                .spawn(async move { PostWatermarkRenderer::render_photo(&photo_source, &photo_label, &photo_staging) })
                .await?;
            on_fraction(1.0);
        }
        drop(lease);
        let _ = FileManager.remove_item(&target);
        FileManager.move_item(&staging, &target).map_err(|_| RenderError::EncodeFailed)?;
        Ok((target, is_video))
    }

    /// downloads the source file through materialize so a failure has a cause to log
    /// a transfer shared with the player and cancelled under a live run is started again as its own
    pub async fn fetch_source(
        remote: &Url,
        cache_key: Option<String>,
        filename: &str,
        on_fraction: &dyn Fn(f64),
        cx: &AsyncApp,
    ) -> Result<PathBuf, PostWatermarkRendererRenderError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let (sender, mut receiver) = mpsc::unbounded::<f64>();
            let options = RemoteMediaDiskCacheMaterializeOptions {
                cache_key: cache_key.clone(),
                suggested_filename: Some(filename.to_owned()),
                on_progress: Some(Arc::new(move |progress: RemoteMediaDownloadProgress| {
                    if let Some(fraction) = progress.fraction_completed() {
                        let _ = sender.unbounded_send(fraction);
                    }
                })),
                ..RemoteMediaDiskCacheMaterializeOptions::default()
            };
            let cache = RemoteMediaDiskCache::shared();
            let mut work = pin!(cache.materialize(remote, options, cx));
            let result = loop {
                match select(work.as_mut(), receiver.next()).await {
                    Either::Left((result, _)) => break result,
                    Either::Right((Some(fraction), _)) => on_fraction(fraction),
                    Either::Right((None, _)) => break work.as_mut().await,
                }
            };
            match result {
                Ok(path) => return Ok(path),
                Err(error) if error.downcast_ref::<CancellationError>().is_some() => {
                    if attempt >= Self::SOURCE_ATTEMPTS {
                        cx.update(|cx| {
                            Sai::log(
                                LOG_NAME,
                                || format!("source {filename}: shared transfer cancelled {attempt} times"),
                                SaiLogKind::Ui,
                                cx,
                            )
                        });
                        return Err(PostWatermarkRendererRenderError::DecodeFailed);
                    }
                }
                Err(error) => {
                    cx.update(|cx| Sai::log(LOG_NAME, || format!("source {filename}: {error}"), SaiLogKind::Ui, cx));
                    return Err(PostWatermarkRendererRenderError::DecodeFailed);
                }
            }
        }
    }

    fn file_name(post_id: &str, index: usize, is_video: bool) -> String {
        let extension = if is_video { "mp4" } else { "jpg" };
        format!("riseonly-{post_id}-{}.{extension}", index + 1)
    }

    fn write_into(file: &Path, folder: &Path, name: &str) -> bool {
        let destination = Self::available_destination(folder, name);
        if FileManager.copy_item(file, &destination).is_err() {
            return false;
        }
        if let Err(error) = download_mark::mark_downloaded(&destination, &DownloadOrigin::default()) {
            tracing::warn!(target: "riseonly", %error, "post download stays without the download mark");
        }
        true
    }

    fn available_destination(folder: &Path, name: &str) -> PathBuf {
        let candidate = folder.join(name);
        if !FileManager.file_exists(&candidate) {
            return candidate;
        }
        let path = Path::new(name);
        let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
        let extension = path.extension().map(|extension| extension.to_string_lossy().into_owned());
        let mut index: i64 = 2;
        loop {
            let numbered = match &extension {
                Some(extension) => format!("{stem} {index}.{extension}"),
                None => format!("{stem} {index}"),
            };
            let candidate = folder.join(numbered);
            if !FileManager.file_exists(&candidate) {
                return candidate;
            }
            index += 1;
        }
    }
}

struct PostDownloadStagingGuard {
    staging: PathBuf,
}

impl Drop for PostDownloadStagingGuard {
    fn drop(&mut self) {
        PostDownloadCache::remove_staging(&self.staging);
    }
}
