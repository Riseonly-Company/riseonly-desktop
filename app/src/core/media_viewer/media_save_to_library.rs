use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use futures::channel::oneshot;
use gpui::{App, AsyncApp};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, ImageReader};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::core::engine_bridge::network_transport_config::NetworkClientSlot;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode, URLRequest};
use rise_i18n::tr;
use rise_platform::download_mark::{self, DownloadOrigin};
use rise_platform::file_manager::FileManager;
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::paths::temporary_directory;
use rise_platform::photo_library::{self, LibraryResourceKind, LibrarySaveFile, LibrarySaveOutcome, LibrarySaveRequest};
use rise_platform::uniform_type::UTType;

use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions, RiseMediaRedirectSessionDelegate,
};
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapEncoder};
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;

static EPHEMERAL_DOWNLOAD_SESSION: NetworkClientSlot = NetworkClientSlot::without_redirects();

const DEFAULT_VIDEO_EXTENSION: &str = "mp4";
const SAVED_IMAGE_EXTENSION: &str = "jpg";
const SAVED_IMAGE_JPEG_QUALITY: u8 = 92;
const NO_STORE: &str = "no-store";
const NO_CACHE: &str = "no-cache";
const FILE_TYPE_BOX: &[u8] = b"ftyp";
const QUICK_TIME_BRAND: &[u8] = b"qt  ";
const HEIC_BRANDS: [&[u8]; 4] = [b"heic", b"heix", b"heim", b"heis"];
const HEIF_BRANDS: [&[u8]; 2] = [b"mif1", b"msf1"];
const SNIFF_HEADER_LENGTH: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaSaveToLibraryResult {
    Saved,
    Failed,
    Denied,
}

#[derive(Clone, Debug)]
struct MediaSaveToLibraryLibraryFile {
    url: PathBuf,
    is_video: bool,
    content_type: Option<String>,
}

pub enum MediaSaveToLibrary {}

impl MediaSaveToLibrary {
    /// shows the save result toast, the viewer passes Bottom
    /// its top bar steps aside for a top toast and the back button would sit under it
    pub fn announce(result: MediaSaveToLibraryResult, position: NotifierPosition, offset: f32, cx: &mut App) {
        let (title, r#type, icon) = match result {
            MediaSaveToLibraryResult::Saved => (tr("save_to_gallery_done"), NotifierType::Success, "checkmark.circle.fill"),
            MediaSaveToLibraryResult::Denied => (
                tr("save_gallery_denied"),
                NotifierType::Warning,
                "exclamationmark.triangle.fill",
            ),
            MediaSaveToLibraryResult::Failed => (tr("save_failed"), NotifierType::Error, "exclamationmark.triangle.fill"),
        };
        Notifier::send(
            title,
            NotifierSendOptions {
                r#type,
                position,
                offset,
                icon: NotifierIcon::System(icon.to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub async fn save(image: CleverImage, cx: &AsyncApp) -> MediaSaveToLibraryResult {
        if !Self::ensure_add_authorization().await {
            let _ = notification(NotificationKind::Error);
            return MediaSaveToLibraryResult::Denied;
        }
        let staged = cx
            .background_executor()
            .spawn(async move { Self::stage_image(&image) })
            .await;
        let Ok(staged) = staged else {
            let _ = notification(NotificationKind::Error);
            return MediaSaveToLibraryResult::Failed;
        };
        let saved = Self::perform(LibrarySaveRequest::Image(staged.clone())).await;
        Self::remove_temporary(staged, cx);
        if saved {
            let _ = notification(NotificationKind::Success);
            MediaSaveToLibraryResult::Saved
        } else {
            let _ = notification(NotificationKind::Error);
            MediaSaveToLibraryResult::Failed
        }
    }

    pub async fn save_video(
        remote_url: &str,
        persist_to_disk: bool,
        cache_key: Option<String>,
        cx: &AsyncApp,
    ) -> MediaSaveToLibraryResult {
        if !Self::ensure_add_authorization().await {
            let _ = notification(NotificationKind::Error);
            return MediaSaveToLibraryResult::Denied;
        }
        let Some(source) = RemoteMediaURL::optional_normalized(Some(remote_url))
            .and_then(|normalized| Url::from_string(&normalized))
        else {
            let _ = notification(NotificationKind::Error);
            return MediaSaveToLibraryResult::Failed;
        };
        let Ok((materialized, is_temporary)) =
            Self::materialize_video(&source, persist_to_disk, cache_key, cx).await
        else {
            let _ = notification(NotificationKind::Error);
            return MediaSaveToLibraryResult::Failed;
        };
        let lease = RemoteMediaDiskCache::lease(Some(materialized.as_path()));
        let saved = Self::perform(LibrarySaveRequest::Video(materialized.clone())).await;
        drop(lease);
        if is_temporary {
            Self::remove_temporary(materialized, cx);
        }
        if saved {
            let _ = notification(NotificationKind::Success);
            MediaSaveToLibraryResult::Saved
        } else {
            let _ = notification(NotificationKind::Error);
            MediaSaveToLibraryResult::Failed
        }
    }

    pub async fn export_image(image: CleverImage, destination: PathBuf, cx: &AsyncApp) -> MediaSaveToLibraryResult {
        let exported = cx
            .background_executor()
            .spawn(async move {
                let staged = Self::stage_image(&image)?;
                let placed = Self::place_export(&staged, &destination);
                FileManager.remove_item(&staged).ok();
                placed
            })
            .await;
        Self::finish_export(exported)
    }

    pub async fn export_video(
        remote_url: &str,
        persist_to_disk: bool,
        cache_key: Option<String>,
        destination: PathBuf,
        cx: &AsyncApp,
    ) -> MediaSaveToLibraryResult {
        let Some(source) = RemoteMediaURL::optional_normalized(Some(remote_url))
            .and_then(|normalized| Url::from_string(&normalized))
        else {
            return Self::finish_export(Err(anyhow::anyhow!("the video has no address")));
        };
        let (materialized, is_temporary) = match Self::materialize_video(&source, persist_to_disk, cache_key, cx).await {
            Ok(materialized) => materialized,
            Err(error) => return Self::finish_export(Err(error)),
        };
        let lease = RemoteMediaDiskCache::lease(Some(materialized.as_path()));
        let copy_source = materialized.clone();
        let exported = cx
            .background_executor()
            .spawn(async move { Self::place_export(&copy_source, &destination) })
            .await;
        drop(lease);
        if is_temporary {
            Self::remove_temporary(materialized, cx);
        }
        Self::finish_export(exported)
    }

    pub fn export_name(remote_url: &str, is_video: bool) -> Option<String> {
        let url = RemoteMediaURL::optional_normalized(Some(remote_url)).and_then(|normalized| Url::from_string(&normalized))?;
        let component = url.last_path_component();
        let stem = Path::new(&component).file_stem()?.to_str()?.to_owned();
        if stem.is_empty() {
            return None;
        }
        Some(format!("{stem}.{}", Self::export_extension(remote_url, is_video)))
    }

    pub fn export_extension(remote_url: &str, is_video: bool) -> String {
        if !is_video {
            return SAVED_IMAGE_EXTENSION.to_owned();
        }
        let extension = RemoteMediaURL::optional_normalized(Some(remote_url))
            .and_then(|normalized| Url::from_string(&normalized))
            .map(|url| url.path_extension())
            .unwrap_or_default();
        if extension.is_empty() {
            DEFAULT_VIDEO_EXTENSION.to_owned()
        } else {
            extension
        }
    }

    pub fn announce_export(result: MediaSaveToLibraryResult, is_video: bool, offset: f32, cx: &mut App) {
        let (title, r#type, icon) = match result {
            MediaSaveToLibraryResult::Saved => (
                if is_video { tr("common_done") } else { tr("media_viewer_saved") },
                NotifierType::Success,
                "checkmark.circle.fill",
            ),
            MediaSaveToLibraryResult::Failed | MediaSaveToLibraryResult::Denied => {
                (tr("save_failed"), NotifierType::Error, "exclamationmark.triangle.fill")
            }
        };
        Notifier::send(
            title,
            NotifierSendOptions {
                r#type,
                position: NotifierPosition::Bottom,
                offset,
                icon: NotifierIcon::System(icon.to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    /// blocking file io, runs off the main thread
    fn place_export(source: &Path, destination: &Path) -> anyhow::Result<()> {
        if FileManager.file_exists(destination) {
            FileManager.remove_item(destination)?;
        }
        FileManager.copy_item(source, destination)?;
        if let Err(error) = download_mark::mark_downloaded(destination, &DownloadOrigin::default()) {
            tracing::warn!(target: "riseonly", %error, "exported media stays without the download mark");
        }
        Ok(())
    }

    fn finish_export(exported: anyhow::Result<()>) -> MediaSaveToLibraryResult {
        match exported {
            Ok(()) => {
                let _ = notification(NotificationKind::Success);
                MediaSaveToLibraryResult::Saved
            }
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "viewer save to a file failed");
                let _ = notification(NotificationKind::Error);
                MediaSaveToLibraryResult::Failed
            }
        }
    }

    /// writes materialized files into the library as original bytes
    /// so an uncompressed photo sent as a document isnt re-encoded
    pub async fn save_files(files: Vec<(PathBuf, bool)>, cx: &AsyncApp) -> MediaSaveToLibraryResult {
        if files.is_empty() {
            return MediaSaveToLibraryResult::Failed;
        }
        if !Self::ensure_add_authorization().await {
            let _ = notification(NotificationKind::Error);
            return MediaSaveToLibraryResult::Denied;
        }
        let resolved: Vec<MediaSaveToLibraryLibraryFile> = cx
            .background_executor()
            .spawn(async move {
                files
                    .into_iter()
                    .map(|(url, is_video)| {
                        let content_type = Self::sniffed_content_type(&url, is_video);
                        MediaSaveToLibraryLibraryFile {
                            url: Self::named_for_photos(&url, is_video, content_type.as_deref()),
                            is_video,
                            content_type,
                        }
                    })
                    .collect()
            })
            .await;
        if Self::perform_originals(&resolved).await {
            let _ = notification(NotificationKind::Success);
            return MediaSaveToLibraryResult::Saved;
        }
        let mut all_saved = true;
        for file in &resolved {
            if Self::perform_originals(std::slice::from_ref(file)).await {
                continue;
            }
            if Self::perform_reencoded(file, cx).await {
                continue;
            }
            all_saved = false;
        }
        let _ = notification(if all_saved { NotificationKind::Success } else { NotificationKind::Error });
        if all_saved {
            MediaSaveToLibraryResult::Saved
        } else {
            MediaSaveToLibraryResult::Failed
        }
    }

    fn photo_content_types() -> [String; 6] {
        [
            UTType::jpeg().identifier(),
            UTType::png().identifier(),
            UTType::heic().identifier(),
            UTType::heif().identifier(),
            UTType::tiff().identifier(),
            UTType::gif().identifier(),
        ]
    }

    async fn perform_originals(files: &[MediaSaveToLibraryLibraryFile]) -> bool {
        let accepted: Vec<&MediaSaveToLibraryLibraryFile> = files
            .iter()
            .filter(|file| file.is_video || file.content_type.is_some())
            .collect();
        if accepted.len() != files.len() {
            return false;
        }
        let request = LibrarySaveRequest::Originals(
            accepted
                .into_iter()
                .map(|file| LibrarySaveFile {
                    path: file.url.clone(),
                    kind: if file.is_video {
                        LibraryResourceKind::Video
                    } else {
                        LibraryResourceKind::Photo
                    },
                    content_type: file.content_type.clone(),
                })
                .collect(),
        );
        Self::perform(request).await
    }

    async fn perform_reencoded(file: &MediaSaveToLibraryLibraryFile, cx: &AsyncApp) -> bool {
        if file.is_video {
            return Self::perform(LibrarySaveRequest::Video(file.url.clone())).await;
        }
        let path = file.url.clone();
        let decodable = cx
            .background_executor()
            .spawn(async move {
                ImageReader::open(&path)
                    .ok()
                    .and_then(|reader| reader.with_guessed_format().ok())
                    .is_some_and(|reader| reader.decode().is_ok())
            })
            .await;
        if !decodable {
            return false;
        }
        Self::perform(LibrarySaveRequest::Image(file.url.clone())).await
    }

    fn named_for_photos(url: &Path, is_video: bool, content_type: Option<&str>) -> PathBuf {
        let family = if is_video { UTType::movie() } else { UTType::image() };
        let path_extension = url.extension().and_then(|extension| extension.to_str()).unwrap_or_default();
        if UTType::from_filename_extension(path_extension).is_some_and(|current| current.conforms_to(&family)) {
            return url.to_path_buf();
        }
        let Some(file_extension) = content_type
            .and_then(UTType::from_identifier)
            .and_then(|r#type| r#type.preferred_filename_extension())
            .or_else(|| {
                if is_video {
                    UTType::mpeg4_movie().preferred_filename_extension()
                } else {
                    None
                }
            })
        else {
            return url.to_path_buf();
        };
        let renamed = url.with_extension(file_extension);
        if FileManager.file_exists(&renamed) {
            return url.to_path_buf();
        }
        if std::fs::hard_link(url, &renamed).is_ok() || FileManager.copy_item(url, &renamed).is_ok() {
            return renamed;
        }
        url.to_path_buf()
    }

    fn sniffed_content_type(url: &Path, is_video: bool) -> Option<String> {
        let mut header = [0u8; SNIFF_HEADER_LENGTH];
        let mut handle = File::open(url).ok()?;
        handle.read_exact(&mut header).ok()?;
        let has_file_type_box = &header[4..8] == FILE_TYPE_BOX;
        let brand = &header[8..12];
        if is_video {
            if !has_file_type_box {
                return None;
            }
            return Some(if brand == QUICK_TIME_BRAND {
                UTType::quick_time_movie().identifier()
            } else {
                UTType::mpeg4_movie().identifier()
            });
        }
        let r#type = if has_file_type_box {
            if HEIC_BRANDS.contains(&brand) {
                UTType::heic().identifier()
            } else if HEIF_BRANDS.contains(&brand) {
                UTType::heif().identifier()
            } else {
                return None;
            }
        } else {
            match image::guess_format(&header).ok()? {
                ImageFormat::Jpeg => UTType::jpeg().identifier(),
                ImageFormat::Png => UTType::png().identifier(),
                ImageFormat::Gif => UTType::gif().identifier(),
                ImageFormat::Tiff => UTType::tiff().identifier(),
                _ => return None,
            }
        };
        Self::photo_content_types().contains(&r#type).then_some(r#type)
    }

    async fn materialize_video(
        url: &Url,
        persist_to_disk: bool,
        cache_key: Option<String>,
        cx: &AsyncApp,
    ) -> anyhow::Result<(PathBuf, bool)> {
        if persist_to_disk && !url.is_file_url() {
            let options = RemoteMediaDiskCacheMaterializeOptions {
                cache_key,
                suggested_filename: Some(url.last_path_component()),
                ..RemoteMediaDiskCacheMaterializeOptions::default()
            };
            if let Some(cached) = RemoteMediaDiskCache::shared().ensure_cached(url, options, cx).await {
                return Ok((cached, false));
            }
        }
        let path_extension = url.path_extension();
        let ext = if path_extension.is_empty() {
            DEFAULT_VIDEO_EXTENSION.to_owned()
        } else {
            path_extension
        };
        let directory = temporary_directory();
        let dest = directory.join(Uuid::new_v4().uuid_string()).with_extension(ext);
        let file_url = if url.is_file_url() {
            Some(
                url.to_file_path()
                    .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?,
            )
        } else {
            None
        };
        let copy_dest = dest.clone();
        cx.background_executor()
            .spawn(async move {
                std::fs::create_dir_all(&directory)?;
                FileManager.remove_item(&copy_dest).ok();
                if let Some(source) = file_url {
                    FileManager.copy_item(&source, &copy_dest)?;
                }
                anyhow::Ok(())
            })
            .await?;
        if url.is_file_url() {
            return Ok((dest, true));
        }
        let mut request = URLRequest::new(url.clone());
        request.set_value(NO_STORE, reqwest::header::CACHE_CONTROL.as_str());
        request.set_value(NO_CACHE, reqwest::header::PRAGMA.as_str());
        if let Some(bearer) = RemoteMediaDiskCache::bearer_token(url, None, cx).await {
            request.set_value(&format!("Bearer {bearer}"), reqwest::header::AUTHORIZATION.as_str());
        }
        let download_dest = dest.clone();
        let downloaded = RiseAccountRuntime::current()
            .spawn(async move { Self::download(request, &download_dest).await })
            .await
            .context("the download task ended before it answered")
            .and_then(|result| result);
        if let Err(error) = downloaded {
            Self::remove_temporary(dest, cx);
            return Err(error);
        }
        Ok((dest, true))
    }

    async fn download(request: URLRequest, dest: &Path) -> anyhow::Result<()> {
        let session = EPHEMERAL_DOWNLOAD_SESSION
            .current()
            .ok_or(URLError::new(URLErrorCode::SecureConnectionFailed))?;
        let (mut response, _) = RiseMediaRedirectSessionDelegate::send(&session, &request, None, |_, _| {}).await?;
        let mut file = BufWriter::new(File::create(dest)?);
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk)?;
        }
        file.flush()?;
        Ok(())
    }

    fn stage_image(image: &CleverImage) -> anyhow::Result<PathBuf> {
        let rgba = CleverImageBitmapEncoder::rgba(image).context("the bitmap has no readable frame")?;
        let rgb = DynamicImage::ImageRgba8(rgba).into_rgb8();
        let directory = temporary_directory();
        std::fs::create_dir_all(&directory)?;
        let dest = directory
            .join(Uuid::new_v4().uuid_string())
            .with_extension(SAVED_IMAGE_EXTENSION);
        let mut writer = BufWriter::new(File::create(&dest)?);
        let encoded = JpegEncoder::new_with_quality(&mut writer, SAVED_IMAGE_JPEG_QUALITY)
            .encode_image(&rgb)
            .map_err(anyhow::Error::from);
        let written = encoded.and_then(|()| writer.flush().map_err(anyhow::Error::from));
        drop(writer);
        if let Err(error) = written {
            FileManager.remove_item(&dest).ok();
            return Err(error);
        }
        Ok(dest)
    }

    fn remove_temporary(path: PathBuf, cx: &AsyncApp) {
        cx.background_executor()
            .spawn(async move {
                FileManager.remove_item(&path).ok();
            })
            .detach();
    }

    /// one change block of the library, true only when it reports the write done
    async fn perform(request: LibrarySaveRequest) -> bool {
        let (sender, receiver) = oneshot::channel();
        photo_library::save_to_library(request, move |outcome| {
            let _ = sender.send(outcome);
        });
        matches!(receiver.await, Ok(LibrarySaveOutcome::Saved))
    }

    async fn ensure_add_authorization() -> bool {
        let (sender, receiver) = oneshot::channel();
        let requested = photo_library::request_add_access(move |level| {
            let _ = sender.send(level);
        });
        if requested.is_err() {
            return false;
        }
        receiver.await.is_ok_and(|level| level.can_browse())
    }
}
