use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::{Either, select};
use gpui::{AsyncApp, RenderImage, Size, WeakEntity, size};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{Delay, DynamicImage, Frame, ImageEncoder, RgbaImage};
use parking_lot::Mutex;
use rise_core::foundation::foundation_character::scalar_is_control_character;
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_media::video_export::{
    AudioLayer, ExportJob, ExportOutcome as VideoExportOutcome, ExportPreset, ExportTicket, POSTER_MAX_SIDE, TimeSpan,
    VideoExportError, VideoProbe, VideoSegment, clamp_trim, music_placement, poster as video_poster,
    probe as video_probe, start_export,
};
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;
use rise_platform::photo_library::{
    self, ExportOutcome, ImageContentMode, ImageDeliveryMode, ImageRequestId, LibraryAsset, LibraryAssetKind,
    LibraryAssetResource, LibraryAssetResourceType, PhotoLibraryUnavailable,
};
use rise_platform::uniform_type::UTType;
use smallvec::SmallVec;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder, CleverImageBitmapEncoder};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerEditedImage, MediaPickerEditedVideo, MediaPickerItem, MediaPickerItemKind, MediaPickerTrackSelection,
};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_state::{EditorImageState, EditorVideoState};
use crate::modules::media_picker::editor::editor_state_codable::EditorStateCodec;
use crate::modules::media_picker::editor::editor_video_composition_builder::EditorVideoCompositionBuilder;
use crate::modules::media_picker::export::export_progress_view_state::ExportProgressViewState;
use crate::modules::media_picker::library::media_asset::{MediaAsset, MediaAssetMediaType};
use crate::modules::media_picker::library::photos_image_request::PhotosImageRequest;
use crate::modules::media_picker::selection::media_selection_state::{MediaSelectionEdit, MediaSelectionEditPayload};
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::shared::utils::music::music_track_waveform::MusicTrackWaveform;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::share_extension::shared::rise_outgoing_media::RiseOutgoingImageEncoder;
use rise_canon::RiseLimits;

trait MediaExporterCancellablePhotosRequest: Send + Sync {
    fn cancel(&self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaExporterPhotoLoadError {
    Unavailable,
}

impl fmt::Display for MediaExporterPhotoLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str(&tr("media_picker_photo_load_failed")),
        }
    }
}

impl std::error::Error for MediaExporterPhotoLoadError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MediaExporterFailure {
    code: i64,
    description: Option<String>,
}

impl MediaExporterFailure {
    fn code(code: i64) -> Self {
        Self { code, description: None }
    }
}

impl fmt::Display for MediaExporterFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.description {
            Some(description) => formatter.write_str(description),
            None => write!(formatter, "The operation couldn\u{2019}t be completed. (MediaExporter error {}.)", self.code),
        }
    }
}

impl std::error::Error for MediaExporterFailure {}

/// dropping the awaiting future cancels the request, after a finished request the cancel does nothing
struct MediaExporterCancelOnDrop(Arc<dyn MediaExporterCancellablePhotosRequest>);

impl Drop for MediaExporterCancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[derive(Default)]
struct MediaExporterImageRequestState {
    continuation: Option<oneshot::Sender<anyhow::Result<CleverImage>>>,
    request_id: Option<ImageRequestId>,
    is_finished: bool,
}

#[derive(Default)]
struct MediaExporterImageRequest {
    state: Mutex<MediaExporterImageRequestState>,
}

impl MediaExporterImageRequest {
    async fn load(self: &Arc<Self>, asset_id: &str, target_size: Size<f32>) -> anyhow::Result<CleverImage> {
        let (continuation, receiver) = oneshot::channel();
        let should_start = {
            let mut state = self.state.lock();
            if state.is_finished {
                false
            } else {
                state.continuation = Some(continuation);
                true
            }
        };
        if !should_start {
            return Err(CancellationError.into());
        }
        let _cancel_on_drop = MediaExporterCancelOnDrop(self.clone());

        let weak_self = Arc::downgrade(self);
        let request_id = PhotosImageRequest::request_image(
            asset_id,
            target_size,
            ImageContentMode::AspectFit,
            ImageDeliveryMode::HighQuality,
            true,
            move |delivery| {
                if delivery.is_degraded {
                    return;
                }
                let Some(this) = weak_self.upgrade() else { return };
                if delivery.is_cancelled {
                    this.finish(Err(CancellationError.into()));
                } else if let Some(image) = delivery.image {
                    this.finish(Ok(image));
                } else {
                    this.finish(Err(MediaExporterPhotoLoadError::Unavailable.into()));
                }
            },
        );
        if let Some(request_id) = request_id {
            let should_cancel_request = {
                let mut state = self.state.lock();
                if state.is_finished {
                    true
                } else {
                    state.request_id = Some(request_id);
                    false
                }
            };
            if should_cancel_request {
                photo_library::cancel_image(request_id);
            }
        }
        receiver.await.unwrap_or_else(|_| Err(CancellationError.into()))
    }

    fn finish(&self, result: anyhow::Result<CleverImage>) {
        let continuation = {
            let mut state = self.state.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            state.request_id = None;
            state.continuation.take()
        };
        if let Some(continuation) = continuation {
            let _ = continuation.send(result);
        }
    }
}

impl MediaExporterCancellablePhotosRequest for MediaExporterImageRequest {
    fn cancel(&self) {
        let (continuation, request_id) = {
            let mut state = self.state.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            (state.continuation.take(), state.request_id.take())
        };
        if let Some(request_id) = request_id {
            photo_library::cancel_image(request_id);
        }
        if let Some(continuation) = continuation {
            let _ = continuation.send(Err(CancellationError.into()));
        }
    }
}

#[derive(Default)]
struct MediaExporterPhotoResourceRequestState {
    continuation: Option<oneshot::Sender<anyhow::Result<Vec<u8>>>>,
    request_id: Option<ImageRequestId>,
    is_finished: bool,
}

#[derive(Default)]
struct MediaExporterPhotoResourceRequest {
    state: Mutex<MediaExporterPhotoResourceRequestState>,
}

impl MediaExporterPhotoResourceRequest {
    /// collects chunks and refuses past maximum_bytes, which comes back as unavailable
    async fn load(
        self: &Arc<Self>,
        asset_id: &str,
        resource: &LibraryAssetResource,
        maximum_bytes: usize,
    ) -> anyhow::Result<Vec<u8>> {
        let (continuation, receiver) = oneshot::channel();
        let should_start = {
            let mut state = self.state.lock();
            if state.is_finished {
                false
            } else {
                state.continuation = Some(continuation);
                true
            }
        };
        if !should_start {
            return Err(CancellationError.into());
        }
        let _cancel_on_drop = MediaExporterCancelOnDrop(self.clone());

        let weak_self = Arc::downgrade(self);
        let started = photo_library::request_resource_data(asset_id, resource, maximum_bytes, move |result| {
            let Some(this) = weak_self.upgrade() else { return };
            this.complete(result);
        });
        match started {
            Ok(request_id) => {
                let should_cancel_request = {
                    let mut state = self.state.lock();
                    if state.is_finished {
                        true
                    } else {
                        state.request_id = Some(request_id);
                        false
                    }
                };
                if should_cancel_request {
                    photo_library::cancel_resource_data(request_id);
                }
            }
            Err(error) => self.complete(Err(error)),
        }
        receiver.await.unwrap_or_else(|_| Err(CancellationError.into()))
    }

    fn complete(&self, result: Result<Vec<u8>, PhotoLibraryUnavailable>) {
        let result = match result {
            Ok(data) => Ok(data),
            Err(PhotoLibraryUnavailable::Cancelled) => Err(CancellationError.into()),
            Err(PhotoLibraryUnavailable::SystemRefused(_)) => Err(MediaExporterPhotoLoadError::Unavailable.into()),
            Err(error) => Err(error.into()),
        };
        self.finish(result);
    }

    fn finish(&self, result: anyhow::Result<Vec<u8>>) {
        let continuation = {
            let mut state = self.state.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            state.request_id = None;
            state.continuation.take()
        };
        if let Some(continuation) = continuation {
            let _ = continuation.send(result);
        }
    }
}

impl MediaExporterCancellablePhotosRequest for MediaExporterPhotoResourceRequest {
    fn cancel(&self) {
        let (continuation, request_id) = {
            let mut state = self.state.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            (state.continuation.take(), state.request_id.take())
        };
        if let Some(request_id) = request_id {
            photo_library::cancel_resource_data(request_id);
        }
        if let Some(continuation) = continuation {
            let _ = continuation.send(Err(CancellationError.into()));
        }
    }
}

#[derive(Default)]
struct MediaExporterAVAssetRequestState {
    continuation: Option<oneshot::Sender<anyhow::Result<PathBuf>>>,
    request_id: Option<ImageRequestId>,
    is_finished: bool,
}

#[derive(Default)]
struct MediaExporterAVAssetRequest {
    state: Mutex<MediaExporterAVAssetRequestState>,
}

impl MediaExporterAVAssetRequest {
    async fn load(self: &Arc<Self>, asset_id: &str) -> anyhow::Result<PathBuf> {
        let (continuation, receiver) = oneshot::channel();
        let should_start = {
            let mut state = self.state.lock();
            if state.is_finished {
                false
            } else {
                state.continuation = Some(continuation);
                true
            }
        };
        if !should_start {
            return Err(CancellationError.into());
        }
        let _cancel_on_drop = MediaExporterCancelOnDrop(self.clone());

        let weak_self = Arc::downgrade(self);
        let request_id = PhotosImageRequest::request_av_asset(asset_id, &temporary_directory(), move |outcome| {
            let Some(this) = weak_self.upgrade() else { return };
            match outcome {
                ExportOutcome::Exported(path) => this.finish(Ok(path)),
                ExportOutcome::Cancelled => this.finish(Err(CancellationError.into())),
                ExportOutcome::Failed(error) => this.finish(Err(error.into())),
            }
        });
        if let Some(request_id) = request_id {
            let should_cancel_request = {
                let mut state = self.state.lock();
                if state.is_finished {
                    true
                } else {
                    state.request_id = Some(request_id);
                    false
                }
            };
            if should_cancel_request {
                photo_library::cancel_export(request_id);
            }
        }
        receiver.await.unwrap_or_else(|_| Err(CancellationError.into()))
    }

    fn finish(&self, result: anyhow::Result<PathBuf>) {
        let continuation = {
            let mut state = self.state.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            state.request_id = None;
            state.continuation.take()
        };
        if let Some(continuation) = continuation {
            let _ = continuation.send(result);
        }
    }
}

impl MediaExporterCancellablePhotosRequest for MediaExporterAVAssetRequest {
    fn cancel(&self) {
        let (continuation, request_id) = {
            let mut state = self.state.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            (state.continuation.take(), state.request_id.take())
        };
        if let Some(request_id) = request_id {
            photo_library::cancel_export(request_id);
        }
        if let Some(continuation) = continuation {
            let _ = continuation.send(Err(CancellationError.into()));
        }
    }
}

/// the running export session, compared by identity
#[derive(Clone)]
struct MediaExporterExportSessionBox {
    session: ExportTicket,
    id: Uuid,
}

#[derive(Default)]
struct MediaExporterState {
    cancelled: bool,
    active_exporter: Option<MediaExporterExportSessionBox>,
    active_photos_requests: HashMap<Uuid, Arc<dyn MediaExporterCancellablePhotosRequest>>,
}

/// trimmed video with its own sound and the music piece
/// the job pieces the seam composes
/// and the state moved to the window 0 to duration
struct MediaExporterMixedMusic {
    span: TimeSpan,
    video_volume: Option<f32>,
    music: AudioLayer,
    state: EditorVideoState,
}

/// shared exporter, clones see one state
#[derive(Clone, Default)]
pub struct MediaExporter {
    state: Arc<Mutex<MediaExporterState>>,
}

impl MediaExporter {
    pub(crate) const INTERMEDIATE_JPEG_QUALITY: f64 = 0.95;

    const PHOTO_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

    const PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(100);

    pub fn new() -> Self {
        Self::default()
    }

    fn maximum_photo_resource_bytes() -> usize {
        usize::try_from(RiseLimits::value("legacy_http_image")).unwrap_or(0)
    }

    pub fn cancel(&self) {
        let (exporter, requests) = {
            let mut state = self.state.lock();
            state.cancelled = true;
            let requests: Vec<_> = state.active_photos_requests.drain().map(|(_, request)| request).collect();
            (state.active_exporter.clone(), requests)
        };
        if let Some(exporter) = exporter {
            exporter.session.cancel();
        }
        for request in requests {
            request.cancel();
        }
    }

    fn install_progress(&self, progress_state: &WeakEntity<ExportProgressViewState>, total: i64, cx: &mut AsyncApp) {
        let weak_state: Weak<Mutex<MediaExporterState>> = Arc::downgrade(&self.state);
        let _ = progress_state.update(cx, |progress, cx| {
            progress.reset(total, cx);
            progress.on_cancel = Some(Rc::new(move |_| {
                if let Some(state) = weak_state.upgrade() {
                    Self { state }.cancel();
                }
            }));
        });
    }

    fn update_progress(
        progress_state: &WeakEntity<ExportProgressViewState>,
        cx: &mut AsyncApp,
        update: impl FnOnce(&mut ExportProgressViewState),
    ) {
        let _ = progress_state.update(cx, |progress, cx| {
            update(progress);
            cx.notify();
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn export(
        &self,
        assets: Vec<LibraryAsset>,
        edits: HashMap<String, MediaSelectionEdit>,
        as_file: bool,
        defer_video_processing: bool,
        keeps_animated_images: bool,
        attached_track: Option<MediaPickerTrackSelection>,
        progress_state: WeakEntity<ExportProgressViewState>,
        cx: &mut AsyncApp,
    ) -> Vec<MediaPickerItem> {
        self.state.lock().cancelled = false;
        let total = i64::try_from(assets.len()).unwrap_or(i64::MAX);
        self.install_progress(&progress_state, total, cx);

        let mut items: Vec<MediaPickerItem> = Vec::new();
        let mut failed = false;
        for (index, asset) in assets.iter().enumerate() {
            if self.is_cancelled() {
                break;
            }
            let position = i64::try_from(index + 1).unwrap_or(i64::MAX);
            Self::update_progress(&progress_state, cx, |progress| {
                progress.current_label = tr_args(
                    "media_picker_exporting_item",
                    &[FormatArg::Int(position), FormatArg::Int(total)],
                );
            });
            let edit = edits.get(&asset.id);
            let exported: anyhow::Result<MediaPickerItem> = if as_file {
                self.export_as_file(asset).await
            } else if asset.kind == LibraryAssetKind::Video {
                self.export_video(asset, edit, defer_video_processing, attached_track.as_ref(), &progress_state, cx)
                    .await
            } else {
                match self.export_animated_image_if_needed(asset, edit, keeps_animated_images).await {
                    Ok(Some(item)) => Ok(item),
                    Ok(None) => self.export_image(asset, edit, defer_video_processing).await,
                    Err(error) => Err(error),
                }
            };
            match exported {
                Ok(item) => items.push(item),
                Err(error) if error.is::<CancellationError>() => {
                    self.state.lock().cancelled = true;
                    break;
                }
                Err(error) => {
                    failed = true;
                    let description = error.to_string();
                    Self::update_progress(&progress_state, cx, |progress| {
                        progress.failure_description = Some(description);
                    });
                    tracing::debug!(target: "riseonly", asset = %asset.id, %error, "MediaExporter failed asset");
                    break;
                }
            }
            Self::update_progress(&progress_state, cx, |progress| {
                progress.processed_items = position;
                progress.fraction_completed = position as f64 / total.max(1) as f64;
            });
        }
        if failed || self.is_cancelled() {
            Self::remove_temporary_files(&items);
            return Vec::new();
        }
        items
    }

    async fn export_animated_image_if_needed(
        &self,
        asset: &LibraryAsset,
        edit: Option<&MediaSelectionEdit>,
        keeps_animated_images: bool,
    ) -> anyhow::Result<Option<MediaPickerItem>> {
        if !keeps_animated_images {
            return Ok(None);
        }
        self.export_animated_image(asset, edit).await
    }

    fn remove_temporary_files(items: &[MediaPickerItem]) {
        let mut urls: Vec<Url> = Vec::new();
        let mut insert = |url: &Url| {
            if !urls.contains(url) {
                urls.push(url.clone());
            }
        };
        for item in items {
            insert(&item.url);
            if let Some(image_payload) = &item.image_payload {
                if let Some(original) = &image_payload.original_image_url {
                    insert(original);
                }
                if let Some(preview) = &image_payload.optimistic_preview_url {
                    insert(preview);
                }
            }
            if let Some(video_payload) = &item.video_payload {
                if let Some(poster) = &video_payload.poster_url {
                    insert(poster);
                }
                if let Some(preview) = &video_payload.optimistic_poster_url {
                    insert(preview);
                }
            }
        }
        let temporary_root = Url::from_file_path(&temporary_directory())
            .map(|url| url.standardized().path() + "/")
            .unwrap_or_default();
        for url in urls {
            let candidate = url.standardized();
            if !candidate.is_file_url() || temporary_root.is_empty() || !candidate.path().starts_with(&temporary_root) {
                continue;
            }
            let _ = FileManager.remove_item(&Self::file_path(&candidate));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn export_external(
        &self,
        source_url: PathBuf,
        source_image: Option<CleverImage>,
        media_asset: MediaAsset,
        edit: MediaSelectionEdit,
        attached_track: Option<MediaPickerTrackSelection>,
        progress_state: WeakEntity<ExportProgressViewState>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<MediaPickerItem> {
        let metadata = std::fs::symlink_metadata(&source_url).ok();
        let is_regular_file = metadata.as_ref().is_some_and(|metadata| metadata.file_type().is_file());
        let is_symbolic_link = metadata.as_ref().is_some_and(|metadata| metadata.file_type().is_symlink());
        if !source_url.is_absolute()
            || !FileManager.file_exists(&source_url)
            || !is_regular_file
            || is_symbolic_link
            || self.is_cancelled()
        {
            return Err(CancellationError.into());
        }

        self.install_progress(&progress_state, 1, cx);

        let item = match media_asset.media_type {
            MediaAssetMediaType::Image => {
                let (Some(source_image), MediaSelectionEditPayload::Image(state)) = (source_image, &edit.payload) else {
                    return Err(MediaExporterFailure::code(20).into());
                };
                let state = state.clone();
                let path_extension = if state.has_baked_edits() { "png" } else { "jpg" };
                let output_url =
                    temporary_directory().join(format!("ro-share-img-{}.{path_extension}", Uuid::new_v4().uuid_string()));
                let (data, pixel_size) =
                    Self::render_and_write(source_image, state.clone(), output_url.clone(), 21, 22).await?;
                if self.is_cancelled() {
                    let _ = FileManager.remove_item(&output_url);
                    return Err(CancellationError.into());
                }
                let mut payload = MediaPickerEditedImage::new(data, state.was_edited, pixel_size, state.caption.clone());
                payload.editor_state_data = if state.was_edited || state.music_plate.is_some() {
                    EditorStateCodec::encode_image(&state)
                } else {
                    None
                };
                payload.original_image_url = None;
                payload.person_tags = state.person_tags.clone().unwrap_or_default();
                payload.metadata_source_url = Self::file_url(&source_url).ok();
                MediaPickerItem::new(
                    Uuid::new_v4().uuid_string(),
                    MediaPickerItemKind::Image,
                    Self::file_url(&output_url)?,
                    Self::last_path_component(&output_url),
                    None,
                    Some(payload),
                    None,
                    None,
                    false,
                )
            }
            MediaAssetMediaType::Video => {
                let MediaSelectionEditPayload::Video(state) = &edit.payload else {
                    return Err(MediaExporterFailure::code(23).into());
                };
                self.make_video_item(
                    source_url,
                    state.clone(),
                    None,
                    true,
                    attached_track.as_ref(),
                    &progress_state,
                    cx,
                )
                .await?
            }
        };

        Self::update_progress(&progress_state, cx, |progress| {
            progress.processed_items = 1;
            progress.fraction_completed = 1.0;
        });
        Ok(item)
    }

    async fn render_and_write(
        source_image: CleverImage,
        state: EditorImageState,
        output_url: PathBuf,
        png_failure_code: i64,
        jpeg_failure_code: i64,
    ) -> anyhow::Result<(Vec<u8>, Size<f32>)> {
        let render = RiseAccountRuntime::current().spawn_blocking(move || {
            let written = (|| -> anyhow::Result<(Vec<u8>, Size<f32>)> {
                let rendered = EditorImageBake::render_edited_image(&source_image, &state);
                let data = if state.has_baked_edits() {
                    Self::png_data(&rendered).ok_or_else(|| MediaExporterFailure::code(png_failure_code))?
                } else {
                    Self::white_backed_jpeg_data(&rendered, Self::INTERMEDIATE_JPEG_QUALITY)
                        .ok_or_else(|| MediaExporterFailure::code(jpeg_failure_code))?
                };
                Self::write_atomically(&output_url, &data)?;
                Ok((data, Self::pixel_size(&rendered)))
            })();
            if written.is_err() {
                let _ = FileManager.remove_item(&output_url);
            }
            written
        });
        render.await.unwrap_or_else(|_| Err(CancellationError.into()))
    }

    async fn export_image(
        &self,
        asset: &LibraryAsset,
        edit: Option<&MediaSelectionEdit>,
        defer_processing: bool,
    ) -> anyhow::Result<MediaPickerItem> {
        let pixel_size = size(asset.pixel_width as f32, asset.pixel_height as f32);
        let state = match edit.map(|edit| &edit.payload) {
            Some(MediaSelectionEditPayload::Image(state)) => state.clone(),
            _ => EditorImageState::empty(Some(asset.id.clone()), pixel_size),
        };
        if defer_processing {
            let preview_url = self.optimistic_edited_preview(asset, Some(&state)).await;
            if self.is_cancelled() {
                if let Some(preview_url) = &preview_url {
                    let _ = FileManager.remove_item(preview_url);
                }
                return Err(CancellationError.into());
            }
            if state.has_baked_edits() && preview_url.is_none() {
                return Err(MediaExporterPhotoLoadError::Unavailable.into());
            }
            let preview_url = preview_url.as_deref().map(Self::file_url).transpose()?;
            let mut item = MediaPickerItem::deferred_photo_library_image(&asset.id, pixel_size, state, preview_url)
                .ok_or(MediaExporterPhotoLoadError::Unavailable)?;
            if let Some(payload) = item.image_payload.as_mut() {
                payload.metadata_source_url = Self::picked_metadata_source_url(&asset.id);
            }
            return Ok(item);
        }

        let base_image = self.load_full_image(asset).await?;
        let Some(base_image) = base_image else {
            return Err(MediaExporterFailure { code: 1, description: Some("image not available".to_owned()) }.into());
        };
        let path_extension = if state.has_baked_edits() { "png" } else { "jpg" };
        let output_url = temporary_directory().join(format!("ro-img-{}.{path_extension}", Uuid::new_v4().uuid_string()));
        let render_state = state.clone();
        let render_output = output_url.clone();
        let render = RiseAccountRuntime::current().spawn_blocking(move || {
            let mut original_image_url: Option<PathBuf> = None;
            let written = (|| -> anyhow::Result<(Vec<u8>, Size<f32>, Option<Vec<u8>>, Option<PathBuf>)> {
                let rendered = EditorImageBake::render_edited_image(&base_image, &render_state);
                let data = if render_state.has_baked_edits() {
                    Self::png_data(&rendered).ok_or_else(|| MediaExporterFailure::code(2))?
                } else {
                    Self::white_backed_jpeg_data(&rendered, Self::INTERMEDIATE_JPEG_QUALITY)
                        .ok_or_else(|| MediaExporterFailure::code(3))?
                };
                Self::write_atomically(&render_output, &data)?;
                if render_state.was_edited {
                    original_image_url = Self::write_reedit_original(&base_image).ok();
                }
                let editor_state_data = if render_state.was_edited || render_state.music_plate.is_some() {
                    EditorStateCodec::encode_image(&render_state)
                } else {
                    None
                };
                Ok((data, Self::pixel_size(&rendered), editor_state_data, original_image_url.clone()))
            })();
            if written.is_err() {
                let _ = FileManager.remove_item(&render_output);
                if let Some(original_image_url) = &original_image_url {
                    let _ = FileManager.remove_item(original_image_url);
                }
            }
            written
        });
        let (data, rendered_size, editor_state_data, original_image_url) =
            render.await.unwrap_or_else(|_| Err(CancellationError.into()))?;
        if self.is_cancelled() {
            let _ = FileManager.remove_item(&output_url);
            if let Some(original_image_url) = &original_image_url {
                let _ = FileManager.remove_item(original_image_url);
            }
            return Err(CancellationError.into());
        }

        let mut payload = MediaPickerEditedImage::new(data, state.was_edited, rendered_size, state.caption.clone());
        payload.editor_state_data = editor_state_data;
        payload.original_image_url = original_image_url.as_deref().map(Self::file_url).transpose()?;
        payload.person_tags = state.person_tags.clone().unwrap_or_default();
        payload.metadata_source_url = Self::picked_metadata_source_url(&asset.id);
        Ok(MediaPickerItem::new(
            Uuid::new_v4().uuid_string(),
            MediaPickerItemKind::Image,
            Self::file_url(&output_url)?,
            Self::last_path_component(&output_url),
            Some(asset.id.clone()),
            Some(payload),
            None,
            None,
            false,
        ))
    }

    async fn export_animated_image(
        &self,
        asset: &LibraryAsset,
        edit: Option<&MediaSelectionEdit>,
    ) -> anyhow::Result<Option<MediaPickerItem>> {
        let animated_asset_id = asset.id.clone();
        let is_animated = RiseAccountRuntime::current()
            .spawn_blocking(move || photo_library::is_animated_image(&animated_asset_id))
            .await;
        match is_animated {
            Ok(true) => {}
            Ok(false) => return Ok(None),
            Err(_) => return Err(CancellationError.into()),
        }
        let mut caption = String::new();
        if let Some(MediaSelectionEditPayload::Image(state)) = edit.map(|edit| &edit.payload) {
            if state.has_baked_edits() {
                return Ok(None);
            }
            caption = state.caption.clone();
        }
        let resources_asset_id = asset.id.clone();
        let Ok(resources) = RiseAccountRuntime::current()
            .spawn_blocking(move || photo_library::asset_resources(&resources_asset_id))
            .await
        else {
            return Err(CancellationError.into());
        };
        if resources.iter().any(|resource| resource.resource_type == LibraryAssetResourceType::FullSizePhoto) {
            return Ok(None);
        }
        let Some(original) = resources.iter().find(|resource| {
            resource.resource_type == LibraryAssetResourceType::Photo
                && UTType::from_identifier(&resource.uniform_type_identifier)
                    .is_some_and(|uniform_type| uniform_type.conforms_to(&UTType::gif()))
        }) else {
            return Ok(None);
        };

        let request = Arc::new(MediaExporterPhotoResourceRequest::default());
        let Some(token) = self.track(request.clone()) else {
            return Err(CancellationError.into());
        };
        let loaded = self
            .with_photo_request_timeout(request.load(&asset.id, original, Self::maximum_photo_resource_bytes()))
            .await;
        self.untrack(token);
        let data = match loaded {
            Ok(data) => data,
            Err(_) => {
                if self.is_cancelled() {
                    return Err(CancellationError.into());
                }
                return Ok(None);
            }
        };

        let url = temporary_directory().join(format!("ro-gif-{}.gif", Uuid::new_v4().uuid_string()));
        let write_url = url.clone();
        let written = RiseAccountRuntime::current()
            .spawn_blocking(move || Self::write_atomically(&write_url, &data).map(|()| data))
            .await;
        let data = match written {
            Ok(Ok(data)) => data,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => return Err(CancellationError.into()),
        };
        if self.is_cancelled() {
            let _ = FileManager.remove_item(&url);
            return Err(CancellationError.into());
        }
        let pixel_size = size(asset.pixel_width as f32, asset.pixel_height as f32);
        Ok(Some(MediaPickerItem::new(
            Uuid::new_v4().uuid_string(),
            MediaPickerItemKind::Image,
            Self::file_url(&url)?,
            Self::last_path_component(&url),
            Some(asset.id.clone()),
            Some(MediaPickerEditedImage::new(data, false, pixel_size, caption)),
            None,
            None,
            false,
        )))
    }

    async fn load_full_image(&self, asset: &LibraryAsset) -> anyhow::Result<Option<CleverImage>> {
        let max_side = MediaCompressionConfiguration::source_pixel_max_side();
        let pw = f64::from(asset.pixel_width.max(1));
        let ph = f64::from(asset.pixel_height.max(1));
        let longest = pw.max(ph);
        let scale = if longest > max_side { max_side / longest } else { 1.0 };
        let target = size((pw * scale) as f32, (ph * scale) as f32);

        let resources_asset_id = asset.id.clone();
        let Ok(resources) = RiseAccountRuntime::current()
            .spawn_blocking(move || photo_library::asset_resources(&resources_asset_id))
            .await
        else {
            return Err(CancellationError.into());
        };
        let edited_resource = resources
            .into_iter()
            .find(|resource| resource.resource_type == LibraryAssetResourceType::FullSizePhoto);
        if let Some(edited_resource) = edited_resource {
            let resource_request = Arc::new(MediaExporterPhotoResourceRequest::default());
            let image_request = Arc::new(MediaExporterImageRequest::default());
            let Some(resource_token) = self.track(resource_request.clone()) else {
                return Err(CancellationError.into());
            };
            let Some(image_token) = self.track(image_request.clone()) else {
                resource_request.cancel();
                self.untrack(resource_token);
                return Err(CancellationError.into());
            };

            let asset_id = asset.id.clone();
            let maximum_bytes = Self::maximum_photo_resource_bytes();
            let race = async {
                let decoded = async {
                    let data = resource_request.load(&asset_id, &edited_resource, maximum_bytes).await.ok()?;
                    let maximum_pixel_dimension = max_side as i64;
                    RiseAccountRuntime::current()
                        .spawn_blocking(move || CleverImageBitmapDecoder::decode(&data, Some(maximum_pixel_dimension)))
                        .await
                        .ok()
                        .flatten()
                };
                let requested = async { image_request.load(&asset_id, target).await.ok() };
                let image = match select(pin!(decoded), pin!(requested)).await {
                    Either::Left((Some(image), _)) | Either::Right((Some(image), _)) => Some(image),
                    Either::Left((None, requested)) => requested.await,
                    Either::Right((None, decoded)) => decoded.await,
                };
                image.ok_or_else(|| anyhow::Error::from(MediaExporterPhotoLoadError::Unavailable))
            };
            let loaded = self.with_photo_request_timeout(race).await;
            resource_request.cancel();
            image_request.cancel();
            self.untrack(resource_token);
            self.untrack(image_token);
            return loaded.map(Some);
        }

        let request = Arc::new(MediaExporterImageRequest::default());
        let Some(token) = self.track(request.clone()) else {
            return Err(CancellationError.into());
        };
        let loaded = self.with_photo_request_timeout(request.load(&asset.id, target)).await;
        self.untrack(token);
        loaded.map(Some)
    }

    /// baked frame for the optimistic row
    /// the real clip renders later so without this the author sees the raw source
    /// display only, never the server poster
    pub fn write_optimistic_media_preview(source: &CleverImage, spatial_edit: &EditorImageState) -> Option<PathBuf> {
        let baked = EditorImageBake::render_edited_image(source, spatial_edit);
        let data = Self::jpeg_data(&baked, 0.8).filter(|data| !data.is_empty())?;
        let url = temporary_directory().join(format!("ro-chat-img-poster-{}.jpg", Uuid::new_v4().uuid_string()));
        Self::write_atomically(&url, &data).ok()?;
        Some(url)
    }

    /// frame from the library, not the video decoder
    /// deferred export should return instantly
    /// loading the whole asset (eg an icloud clip) would defeat that
    async fn optimistic_edited_preview(
        &self,
        asset: &LibraryAsset,
        state: Option<&EditorImageState>,
    ) -> Option<PathBuf> {
        let spatial_edit = state.filter(|state| state.has_baked_edits())?.clone();
        let longest = f64::from(asset.pixel_width.max(asset.pixel_height).max(1));
        let scale = if longest > 1_280.0 { 1_280.0 / longest } else { 1.0 };
        let target = size(
            (f64::from(asset.pixel_width.max(1)) * scale) as f32,
            (f64::from(asset.pixel_height.max(1)) * scale) as f32,
        );
        let request = Arc::new(MediaExporterImageRequest::default());
        let token = self.track(request.clone())?;
        let loaded = self.with_photo_request_timeout(request.load(&asset.id, target)).await;
        self.untrack(token);
        let source = loaded.ok()?;
        if self.is_cancelled() {
            return None;
        }
        let preview_url = RiseAccountRuntime::current()
            .spawn_blocking(move || Self::write_optimistic_media_preview(&source, &spatial_edit))
            .await
            .ok()
            .flatten();
        if self.is_cancelled() {
            if let Some(preview_url) = &preview_url {
                let _ = FileManager.remove_item(preview_url);
            }
            return None;
        }
        preview_url
    }

    pub(crate) fn write_reedit_original(image: &CleverImage) -> anyhow::Result<PathBuf> {
        let max_side: f64 = 4096.0;
        let rgba = CleverImageBitmapEncoder::rgba(image).ok_or_else(|| MediaExporterFailure::code(4))?;
        let long_side = f64::from(rgba.width().max(rgba.height()));
        let source = if long_side > max_side && long_side > 1.0 {
            let scale = max_side / long_side;
            let width = ((f64::from(rgba.width()) * scale) as u32).max(1);
            let height = ((f64::from(rgba.height()) * scale) as u32).max(1);
            DynamicImage::ImageRgba8(rgba).resize_exact(width, height, FilterType::CatmullRom)
        } else {
            DynamicImage::ImageRgba8(rgba)
        };
        let data = Self::encode_jpeg(&source, 0.95).ok_or_else(|| MediaExporterFailure::code(4))?;
        let url = temporary_directory().join(format!("ro-orig-{}.jpg", Uuid::new_v4().uuid_string()));
        Self::write_atomically(&url, &data)?;
        Ok(url)
    }

    async fn probe(source: &Path) -> anyhow::Result<VideoProbe> {
        let source = source.to_path_buf();
        let probed = RiseAccountRuntime::current().spawn_blocking(move || video_probe(&source)).await;
        match probed {
            Ok(probe) => Ok(probe?),
            Err(_) => Err(CancellationError.into()),
        }
    }

    async fn video_poster_and_size(
        source: &Path,
        at_seconds: f64,
        spatial_edit: Option<EditorImageState>,
    ) -> (Option<PathBuf>, Size<f32>) {
        let source = source.to_path_buf();
        RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let Ok(frame) = video_poster(&source, at_seconds.max(0.0), POSTER_MAX_SIDE) else {
                    return (None, Size::default());
                };
                let Some(image) = Self::render_image_from_bgra(frame.width, frame.height, frame.bgra) else {
                    return (None, Size::default());
                };
                let poster = match &spatial_edit {
                    Some(spatial_edit) => EditorImageBake::render_edited_image(&image, spatial_edit),
                    None => image,
                };
                let poster_size = Self::pixel_size(&poster);
                let Some(data) = Self::jpeg_data(&poster, 0.7) else {
                    return (None, poster_size);
                };
                let url = temporary_directory().join(format!("ro-poster-{}.jpg", Uuid::new_v4().uuid_string()));
                let _ = Self::write_atomically(&url, &data);
                (Some(url), poster_size)
            })
            .await
            .unwrap_or((None, Size::default()))
    }

    async fn export_video(
        &self,
        asset: &LibraryAsset,
        edit: Option<&MediaSelectionEdit>,
        defer_processing: bool,
        attached_track: Option<&MediaPickerTrackSelection>,
        progress_state: &WeakEntity<ExportProgressViewState>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<MediaPickerItem> {
        let edited_state = match edit.map(|edit| &edit.payload) {
            Some(MediaSelectionEditPayload::Video(state)) => Some(state.clone()),
            _ => None,
        };

        if defer_processing {
            let state = edited_state
                .unwrap_or_else(|| EditorVideoState::empty(Some(asset.id.clone()), asset.duration_secs.max(0.0)));
            let optimistic_poster_url = self.optimistic_edited_preview(asset, state.spatial_edit.as_ref()).await;
            let optimistic_poster_url = optimistic_poster_url.as_deref().map(Self::file_url).transpose()?;
            return MediaPickerItem::deferred_photo_library_video(
                &asset.id,
                size(asset.pixel_width as f32, asset.pixel_height as f32),
                state,
                optimistic_poster_url,
            )
            .ok_or_else(|| MediaExporterPhotoLoadError::Unavailable.into());
        }

        let source = self.load_av_asset(asset).await?;
        let state = match edited_state {
            Some(edited_state) => edited_state,
            None => {
                let duration = Self::probe(&source).await.map(|probe| probe.duration_seconds).unwrap_or(0.0);
                EditorVideoState::empty(Some(asset.id.clone()), duration)
            }
        };
        self.make_video_item(source, state, Some(asset.id.clone()), false, attached_track, progress_state, cx)
            .await
    }

    async fn mixed_music_asset(
        source: &VideoProbe,
        state: &EditorVideoState,
        music: Option<&MediaPickerTrackSelection>,
        cx: &mut AsyncApp,
    ) -> Option<MediaExporterMixedMusic> {
        let music = music.filter(|music| music.end_ms - music.start_ms > 0)?;
        let audio_url = Self::resolved_music_file_url(&music.track, cx).await?;
        if !source.has_video {
            return None;
        }
        let trim = clamp_trim(source.duration_seconds, state.start_seconds, state.end_seconds)?;

        let video_volume = source.has_audio.then(|| state.effective_video_volume().clamp(0.0, 1.0) as f32);

        let music_asset = Self::probe(&audio_url).await.ok()?;
        if !music_asset.has_audio {
            return None;
        }
        let placement = music_placement(
            trim.span,
            state.music_offset_seconds,
            music.start_ms,
            music.end_ms,
            music_asset.duration_seconds,
        )?;

        let mut mixed_state = state.clone();
        mixed_state.start_seconds = 0.0;
        mixed_state.end_seconds = trim.span.duration_seconds;
        mixed_state.duration = mixed_state.end_seconds;
        Some(MediaExporterMixedMusic {
            span: trim.span,
            video_volume,
            music: AudioLayer {
                source: audio_url,
                placement: Some(placement),
                volume: state.effective_music_volume().clamp(0.0, 1.0) as f32,
            },
            state: mixed_state,
        })
    }

    async fn resolved_music_file_url(track: &RiseStoredMusicTrack, cx: &mut AsyncApp) -> Option<PathBuf> {
        if let Some(local) = MusicTrackWaveform::local_file_url(track) {
            return Some(Self::file_path(&local));
        }
        let id = track.id.as_deref().filter(|id| !id.is_empty())?;
        let normalized = RemoteMediaURL::optional_normalized(track.audio_url.as_deref())?;
        let remote = Url::from_string(&normalized)?;
        let token = Some(cx.update(|cx| AuthServicesStore::shared(cx).read(cx).tokens_and_other_data().access_token.clone()));
        let materialized = MusicPlayerState::materialize_audio_for_playback(
            remote,
            MusicPlaybackDiskCache::ambient_key(id),
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
            token,
            cx,
        )
        .await?;
        Some(Self::file_path(&materialized))
    }

    #[allow(clippy::too_many_arguments)]
    async fn make_video_item(
        &self,
        source: PathBuf,
        state: EditorVideoState,
        asset_identifier: Option<String>,
        requires_mp4: bool,
        attached_track: Option<&MediaPickerTrackSelection>,
        progress_state: &WeakEntity<ExportProgressViewState>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<MediaPickerItem> {
        let av_asset = Self::probe(&source).await?;
        let mut state = state;
        let mixed = Self::mixed_music_asset(&av_asset, &state, attached_track, cx).await;
        let mut mixed_audio: Option<(TimeSpan, Option<f32>, AudioLayer)> = None;
        if let Some(mixed) = mixed {
            state = mixed.state;
            mixed_audio = Some((mixed.span, mixed.video_volume, mixed.music));
        }

        let video_composition = EditorVideoCompositionBuilder::make(&av_asset, state.spatial_edit.as_ref());
        let path_extension = source.extension().and_then(|extension| extension.to_str()).unwrap_or_default().to_owned();
        if !state.has_baked_edits()
            && video_composition.is_none()
            && mixed_audio.is_none()
            && (!requires_mp4 || path_extension.to_lowercase() == "mp4")
        {
            let dest = if requires_mp4 {
                source.clone()
            } else {
                let path_extension = if path_extension.is_empty() { "mov".to_owned() } else { path_extension.clone() };
                let dest = temporary_directory().join(format!("ro-vid-{}.{path_extension}", Uuid::new_v4().uuid_string()));
                let copy_source = source.clone();
                let copy_dest = dest.clone();
                let copied = RiseAccountRuntime::current()
                    .spawn_blocking(move || {
                        let _ = FileManager.remove_item(&copy_dest);
                        let copied = FileManager.copy_item(&copy_source, &copy_dest);
                        if copied.is_err() {
                            let _ = FileManager.remove_item(&copy_dest);
                        }
                        copied
                    })
                    .await;
                match copied {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => return Err(error.into()),
                    Err(_) => return Err(CancellationError.into()),
                }
                dest
            };
            if self.is_cancelled() {
                if !requires_mp4 {
                    let _ = FileManager.remove_item(&dest);
                }
                return Err(CancellationError.into());
            }
            let (poster_url, poster_size) = Self::video_poster_and_size(&source, 0.0, None).await;
            let dur = av_asset.duration_seconds;
            let dur = if dur.is_finite() { dur } else { 0.0 };
            let dest_url = Self::file_url(&dest)?;
            let mut payload = MediaPickerEditedVideo::new(dest_url.clone(), 0.0, dur, dur, false, state.caption.clone());
            payload.pixel_size = poster_size;
            payload.poster_url = poster_url.as_deref().map(Self::file_url).transpose()?;
            payload.spatial_edit = state.spatial_edit.clone();
            return Ok(MediaPickerItem::new(
                Uuid::new_v4().uuid_string(),
                MediaPickerItemKind::Video,
                dest_url,
                Self::last_path_component(&dest),
                asset_identifier,
                None,
                Some(payload),
                None,
                false,
            ));
        }

        let output_url = temporary_directory().join(format!("ro-vid-{}.mp4", Uuid::new_v4().uuid_string()));
        let _ = FileManager.remove_item(&output_url);

        let total_seconds = match &mixed_audio {
            Some((span, _, _)) => span.duration_seconds,
            None => av_asset.duration_seconds,
        };
        if !total_seconds.is_finite() || total_seconds <= 0.0 {
            return Err(MediaExporterFailure::code(25).into());
        }
        let Some(trim) = clamp_trim(total_seconds, state.start_seconds, state.end_seconds) else {
            return Err(MediaExporterFailure::code(26).into());
        };
        let start_sec = trim.span.start_seconds;
        let end_sec = trim.span.end_seconds();
        let source_span = match &mixed_audio {
            Some((span, _, _)) => TimeSpan {
                start_seconds: span.start_seconds + start_sec,
                duration_seconds: trim.span.duration_seconds,
            },
            None => trim.span,
        };

        let (poster_url, poster_size) =
            Self::video_poster_and_size(&source, source_span.start_seconds, state.spatial_edit.clone()).await;

        let effective_video_volume = state.effective_video_volume();
        let (volume, audio) = match mixed_audio.clone() {
            Some((_, video_volume, music)) => (video_volume, vec![music]),
            None => {
                let volume = if effective_video_volume < 0.999 { effective_video_volume.clamp(0.0, 1.0) as f32 } else { 1.0 };
                (av_asset.has_audio.then_some(volume), Vec::new())
            }
        };
        let (render, transform) = match video_composition {
            Some(video_composition) => (Some(video_composition.render), video_composition.transform),
            None => (None, av_asset.upright()),
        };
        let job = ExportJob {
            segments: vec![VideoSegment {
                source: source.clone(),
                span: Some(source_span),
                at_seconds: Some(0.0),
                transform,
                volume,
            }],
            audio,
            render,
            preset: ExportPreset::HighestQuality,
            target: output_url.clone(),
            file_length_limit: None,
            cap_frame_rate: false,
        };
        if let Err(error) = self.run_exporter(job, progress_state, cx).await {
            let _ = FileManager.remove_item(&output_url);
            if let Some(poster_url) = &poster_url {
                let _ = FileManager.remove_item(poster_url);
            }
            return Err(error);
        }
        let output = Self::file_url(&output_url)?;
        let mut payload =
            MediaPickerEditedVideo::new(output.clone(), start_sec, end_sec, end_sec - start_sec, state.is_muted, state.caption.clone());
        payload.pixel_size = poster_size;
        payload.poster_url = poster_url.as_deref().map(Self::file_url).transpose()?;
        payload.spatial_edit = state.spatial_edit.clone();
        payload.audio_pre_baked = mixed_audio.is_some();
        Ok(MediaPickerItem::new(
            Uuid::new_v4().uuid_string(),
            MediaPickerItemKind::Video,
            output,
            Self::last_path_component(&output_url),
            asset_identifier,
            None,
            Some(payload),
            None,
            false,
        ))
    }

    fn export_start_failure(error: VideoExportError) -> anyhow::Error {
        match error {
            VideoExportError::PresetUnavailable(_) => MediaExporterFailure::code(10).into(),
            VideoExportError::Mp4Unsupported => MediaExporterFailure { code: 24, description: Some(tr("error")) }.into(),
            error => error.into(),
        }
    }

    async fn run_exporter(
        &self,
        job: ExportJob,
        progress_state: &WeakEntity<ExportProgressViewState>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        let (done, mut outcome_receiver) = oneshot::channel::<VideoExportOutcome>();
        let started = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                start_export(
                    job,
                    Box::new(move |outcome| {
                        let _ = done.send(outcome);
                    }),
                )
            })
            .await
            .map_err(|_| anyhow::Error::from(CancellationError))?;
        let session = started.map_err(Self::export_start_failure)?;
        let exporter_box = MediaExporterExportSessionBox { session: session.clone(), id: Uuid::new_v4() };
        self.state.lock().active_exporter = Some(exporter_box.clone());
        let _clear_active_exporter = MediaExporterActiveExporterGuard { state: self.state.clone(), id: exporter_box.id };

        let outcome = loop {
            let is_cancelled = progress_state.read_with(cx, |progress, _| progress.is_cancelled).unwrap_or(false);
            if is_cancelled {
                session.cancel();
                break (&mut outcome_receiver).await.ok();
            }
            let fraction = f64::from(session.progress());
            Self::update_progress(progress_state, cx, |progress| progress.fraction_completed = fraction);
            let timer = pin!(cx.background_executor().timer(Self::PROGRESS_POLL_INTERVAL));
            match select(&mut outcome_receiver, timer).await {
                Either::Left((outcome, _)) => break outcome.ok(),
                Either::Right(_) => {}
            }
        };
        match outcome {
            Some(VideoExportOutcome::Completed { .. }) => Ok(()),
            Some(VideoExportOutcome::Cancelled) => Err(CancellationError.into()),
            Some(VideoExportOutcome::Failed(error)) => Err(error.into()),
            None => Err(MediaExporterFailure::code(11).into()),
        }
    }

    async fn load_av_asset(&self, asset: &LibraryAsset) -> anyhow::Result<PathBuf> {
        let request = Arc::new(MediaExporterAVAssetRequest::default());
        let Some(token) = self.track(request.clone()) else {
            return Err(CancellationError.into());
        };
        let loaded = self.with_photo_request_timeout(request.load(&asset.id)).await;
        self.untrack(token);
        loaded
    }

    async fn export_as_file(&self, asset: &LibraryAsset) -> anyhow::Result<MediaPickerItem> {
        if asset.kind == LibraryAssetKind::Video {
            let source = self.load_av_asset(asset).await?;
            let path_extension = source.extension().and_then(|extension| extension.to_str()).unwrap_or_default().to_owned();
            let last_path_component = Self::last_path_component(&source);
            let dest = temporary_directory()
                .join(format!("ro-file-{}-{last_path_component}", Uuid::new_v4().uuid_string()));
            let copy_dest = dest.clone();
            let copied = RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    let _ = FileManager.remove_item(&copy_dest);
                    FileManager.copy_item(&source, &copy_dest)
                })
                .await;
            match copied {
                Ok(Ok(())) => {}
                Ok(Err(error)) => return Err(error.into()),
                Err(_) => return Err(CancellationError.into()),
            }
            let display_asset_id = asset.id.clone();
            let display_fallback = last_path_component.clone();
            let display_name = RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    Self::document_display_name(&display_asset_id, &path_extension, &display_fallback)
                })
                .await
                .unwrap_or(last_path_component);
            return Ok(MediaPickerItem::new(
                Uuid::new_v4().uuid_string(),
                MediaPickerItemKind::File,
                Self::file_url(&dest)?,
                display_name,
                Some(asset.id.clone()),
                None,
                None,
                None,
                true,
            ));
        }

        let (continuation, receiver) = oneshot::channel();
        let started = photo_library::request_image_data(&asset.id, move |result| {
            let _ = continuation.send(result);
        });
        if started.is_err() {
            return Err(MediaExporterFailure::code(13).into());
        }
        let Ok(Ok((data, uti))) = receiver.await else {
            return Err(MediaExporterFailure::code(13).into());
        };
        let path_extension = Self::extension_from_uti(uti.as_deref());
        let url = temporary_directory().join(format!("ro-file-{}.{path_extension}", Uuid::new_v4().uuid_string()));
        let write_url = url.clone();
        let display_asset_id = asset.id.clone();
        let display_fallback = Self::last_path_component(&url);
        let written = RiseAccountRuntime::current()
            .spawn_blocking(move || -> std::io::Result<String> {
                Self::write_atomically(&write_url, &data)?;
                Ok(Self::document_display_name(&display_asset_id, path_extension, &display_fallback))
            })
            .await;
        let display_name = match written {
            Ok(Ok(display_name)) => display_name,
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => return Err(CancellationError.into()),
        };
        Ok(MediaPickerItem::new(
            Uuid::new_v4().uuid_string(),
            MediaPickerItemKind::File,
            Self::file_url(&url)?,
            display_name,
            Some(asset.id.clone()),
            None,
            None,
            None,
            true,
        ))
    }

    /// name the attachment travels under and shows in the bubble
    /// disk file stays unique so same named sends dont overwrite each other
    /// shown name keeps the library name but with the extension that really shipped
    fn document_display_name(asset_id: &str, file_extension: &str, fallback: &str) -> String {
        let resources = photo_library::asset_resources(asset_id);
        let preferred = resources
            .iter()
            .find(|resource| {
                matches!(resource.resource_type, LibraryAssetResourceType::Photo | LibraryAssetResourceType::Video)
            })
            .or_else(|| resources.first());
        let sanitized: String = preferred
            .map(|resource| {
                resource
                    .original_filename
                    .replace('/', "_")
                    .replace('\\', "_")
                    .chars()
                    .filter(|character| !scalar_is_control_character(*character))
                    .collect()
            })
            .unwrap_or_default();
        let original = trimming_whitespaces_and_newlines(&sanitized);
        if original.is_empty() {
            return fallback.to_owned();
        }

        let normalized_extension = trimming_whitespaces_and_newlines(file_extension);
        if normalized_extension.is_empty() {
            return original.to_owned();
        }
        let original_path = Path::new(original);
        let original_extension = original_path.extension().and_then(|extension| extension.to_str()).unwrap_or_default();
        if compare_case_insensitive(original_extension, normalized_extension) == Ordering::Equal {
            return original.to_owned();
        }
        let stem = original_path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
        format!("{}.{normalized_extension}", if stem.is_empty() { original } else { stem })
    }

    fn extension_from_uti(uti: Option<&str>) -> &'static str {
        let Some(uti) = uti else { return "bin" };
        let u = uti.to_lowercase();
        if u.contains("png") {
            "png"
        } else if u.contains("gif") {
            "gif"
        } else if u.contains("heic") {
            "heic"
        } else if u.contains("jpeg") || u.contains("jpg") {
            "jpg"
        } else if u.contains("webp") {
            "webp"
        } else if u.contains("raw") {
            "dng"
        } else {
            "jpg"
        }
    }

    fn is_cancelled(&self) -> bool {
        self.state.lock().cancelled
    }

    fn track(&self, request: Arc<dyn MediaExporterCancellablePhotosRequest>) -> Option<MediaExporterTrackToken> {
        let token = Uuid::new_v4();
        let accepted = {
            let mut state = self.state.lock();
            if state.cancelled {
                false
            } else {
                state.active_photos_requests.insert(token, request.clone());
                true
            }
        };
        if !accepted {
            request.cancel();
            return None;
        }
        Some(MediaExporterTrackToken { state: Arc::downgrade(&self.state), token })
    }

    fn untrack(&self, token: MediaExporterTrackToken) {
        drop(token);
    }

    async fn with_photo_request_timeout<Value>(
        &self,
        operation: impl Future<Output = anyhow::Result<Value>>,
    ) -> anyhow::Result<Value> {
        let timeout = RiseAccountRuntime::current().spawn(async move { tokio::time::sleep(Self::PHOTO_REQUEST_TIMEOUT).await });
        match select(pin!(operation), timeout).await {
            Either::Left((value, _)) => value,
            Either::Right(_) => Err(MediaExporterPhotoLoadError::Unavailable.into()),
        }
    }

    /// file url of a file this exporter wrote
    /// rendered copy has no exif, a picked file is its own asset id
    /// so map photos read place and date from it
    fn picked_metadata_source_url(asset_id: &str) -> Option<Url> {
        if !photo_library::picked_files_library::is_picked_asset(asset_id) {
            return None;
        }
        Url::from_file_path(Path::new(asset_id))
    }

    fn file_url(path: &Path) -> anyhow::Result<Url> {
        Url::from_file_path(path).ok_or_else(|| MediaExporterFailure::code(12).into())
    }

    fn file_path(url: &Url) -> PathBuf {
        url.to_file_path().unwrap_or_else(|| {
            let component = url.path();
            PathBuf::from(component)
        })
    }

    fn last_path_component(path: &Path) -> String {
        path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn write_atomically(destination: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut staging = destination.as_os_str().to_owned();
        staging.push(".tmp");
        let staging = PathBuf::from(staging);
        if let Err(error) = std::fs::write(&staging, data).and_then(|()| std::fs::rename(&staging, destination)) {
            let _ = std::fs::remove_file(&staging);
            return Err(error);
        }
        Ok(())
    }

    /// size in pixels, gpui images are pixels at scale 1
    fn pixel_size(image: &RenderImage) -> Size<f32> {
        let pixels = image.size(0);
        size(pixels.width.0 as f32, pixels.height.0 as f32)
    }

    fn render_image_from_bgra(width: u32, height: u32, bgra: Vec<u8>) -> Option<CleverImage> {
        let bitmap = RgbaImage::from_raw(width, height, bgra)?;
        let frame = Frame::from_parts(bitmap, 0, 0, Delay::from_numer_denom_ms(0, 1));
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([frame]);
        Some(Arc::new(RenderImage::new(frames)))
    }

    fn png_data(image: &RenderImage) -> Option<Vec<u8>> {
        let rgba = CleverImageBitmapEncoder::rgba(image)?;
        let mut data = Vec::new();
        PngEncoder::new(&mut data)
            .write_image(rgba.as_raw(), rgba.width(), rgba.height(), image::ExtendedColorType::Rgba8)
            .ok()?;
        Some(data)
    }

    fn jpeg_data(image: &RenderImage, compression_quality: f64) -> Option<Vec<u8>> {
        let rgba = CleverImageBitmapEncoder::rgba(image)?;
        Self::encode_jpeg(&DynamicImage::ImageRgba8(rgba), compression_quality)
    }

    fn white_backed_jpeg_data(image: &RenderImage, compression_quality: f64) -> Option<Vec<u8>> {
        let rgba = CleverImageBitmapEncoder::rgba(image)?;
        RiseOutgoingImageEncoder::white_backed_jpeg_data(&DynamicImage::ImageRgba8(rgba), compression_quality)
    }

    fn encode_jpeg(image: &DynamicImage, compression_quality: f64) -> Option<Vec<u8>> {
        let quality = (compression_quality.clamp(0.0, 1.0) * 100.0).round().max(1.0) as u8;
        let rgb = image.to_rgb8();
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, quality).encode_image(&rgb).ok()?;
        Some(data)
    }
}

struct MediaExporterActiveExporterGuard {
    state: Arc<Mutex<MediaExporterState>>,
    id: Uuid,
}

impl Drop for MediaExporterActiveExporterGuard {
    fn drop(&mut self) {
        let mut state = self.state.lock();
        if state.active_exporter.as_ref().is_some_and(|active| active.id == self.id) {
            state.active_exporter = None;
        }
    }
}

/// token that track hands out, dropping it untracks the request
struct MediaExporterTrackToken {
    state: Weak<Mutex<MediaExporterState>>,
    token: Uuid,
}

impl Drop for MediaExporterTrackToken {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            state.lock().active_photos_requests.remove(&self.token);
        }
    }
}
