use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use futures::channel::oneshot;
use futures::future::{BoxFuture, Either, select};
use tokio::task::AbortHandle;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::video_export::{
    CAPPED_FRAME_RATE, ExportJob, ExportOutcome, ExportTicket, MAXIMUM_SOURCE_FRAME_RATE,
    PROGRESS_POLL_INTERVAL, ProgressThrottle, VideoExportError, VideoProbe, clamp_trim, probe,
    start_export,
};
use rise_platform::file_manager::{
    CocoaError, CocoaErrorCode, DirectoryEnumerationOptions, FileAttributeType, FileManager,
};
use rise_platform::paths::temporary_directory;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
pub use crate::core::lib::sai::sai_outgoing_media_hooks::OutgoingMediaCompressorResult;
use crate::core::lib::sai::sai_outgoing_media_hooks::{OutgoingMediaIsCancelled, OutgoingMediaProgress};
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerEditedVideo;
use crate::modules::media_picker::editor::editor_video_composition_builder::EditorVideoCompositionBuilder;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::media_picker::export::video_note_composition::{VideoNoteCaptureRecipe, VideoNoteComposition};
use crate::modules::share_extension::shared::rise_outgoing_media::{
    RiseOutgoingImageEncoder, RiseOutgoingImageEncoderFailure, RiseOutgoingMediaProfileExt,
    RiseOutgoingVideoTranscoderFailure,
};

pub enum OutgoingMediaCompressor {}

struct OutgoingMediaCancelOnDrop {
    flag: Arc<AtomicBool>,
    operation: AbortHandle,
    armed: bool,
}

impl Drop for OutgoingMediaCancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.flag.store(true, Ordering::SeqCst);
            self.operation.abort();
        }
    }
}

struct OutgoingMediaExportCancelGuard {
    ticket: Option<ExportTicket>,
}

impl OutgoingMediaExportCancelGuard {
    fn disarm(&mut self) {
        self.ticket = None;
    }
}

impl Drop for OutgoingMediaExportCancelGuard {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            ticket.cancel();
        }
    }
}

static STALE_TEMPORARY_CLEANUP_STARTED: Once = Once::new();

impl OutgoingMediaCompressor {
    const STALE_TEMPORARY_FILE_AGE: Duration = Duration::from_secs(24 * 60 * 60);
    pub const MAXIMUM_OUTGOING_FRAME_RATE: f32 = MAXIMUM_SOURCE_FRAME_RATE;
    pub const CAPPED_OUTGOING_FRAME_RATE: u32 = CAPPED_FRAME_RATE;
    const PROBED_IMAGE_MIMES: [&'static str; 3] = ["image/jpeg", "image/png", "image/webp"];

    fn chat_image_maximum_dimension() -> i64 {
        RiseLimits::service_value("max_chat_image_dimension")
    }

    fn chat_image_maximum_pixels() -> i64 {
        RiseLimits::service_value("max_chat_image_pixels")
    }

    fn start_stale_temporary_cleanup() {
        STALE_TEMPORARY_CLEANUP_STARTED.call_once(|| {
            let _sweep = RiseAccountRuntime::current().spawn_blocking(|| {
                let manager = FileManager;
                let Ok(files) = manager.contents_of_directory_at(
                    &temporary_directory(),
                    DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES,
                ) else {
                    return;
                };
                let cutoff = RiseClock::now().time_interval_since_1970()
                    - Self::STALE_TEMPORARY_FILE_AGE.as_secs_f64();
                let mut prefixes = MediaCompressionConfiguration::temporary_file_prefixes();
                prefixes.push(VideoNoteComposition::TEMPORARY_FILE_PREFIX.to_owned());
                for file in files {
                    let Some(name) = file.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    if !prefixes.iter().any(|prefix| name.starts_with(prefix.as_str())) {
                        continue;
                    }
                    let Ok(values) = manager.attributes_of_item(&file) else {
                        continue;
                    };
                    let stale = values.r#type == FileAttributeType::TypeRegular
                        && values.modification_date.is_some_and(|modified_at| {
                            modified_at
                                .duration_since(UNIX_EPOCH)
                                .is_ok_and(|since_1970| since_1970.as_secs_f64() < cutoff)
                        });
                    if stale {
                        let _ = manager.remove_item(&file);
                    }
                }
            });
        });
    }

    /// compresses a file if needed, runs on a background task
    /// dropping the returned future cancels it
    pub async fn compress_if_needed(
        local_file: RiseOutgoingMediaFile,
        video_edit: Option<MediaPickerEditedVideo>,
        configuration: MediaCompressionConfiguration,
        on_progress: Option<OutgoingMediaProgress>,
        is_cancelled: Option<OutgoingMediaIsCancelled>,
    ) -> anyhow::Result<OutgoingMediaCompressorResult> {
        let task_cancelled = Arc::new(AtomicBool::new(false));
        let operation = RiseAccountRuntime::current().spawn(Self::compress_off_main(
            local_file,
            video_edit,
            configuration,
            on_progress,
            is_cancelled,
            task_cancelled.clone(),
        ));
        let mut cancel_on_drop = OutgoingMediaCancelOnDrop {
            flag: task_cancelled,
            operation: operation.abort_handle(),
            armed: true,
        };
        let result = operation.await;
        cancel_on_drop.armed = false;
        result.unwrap_or_else(|_| Err(CancellationError.into()))
    }

    /// legacy non-message upload path, message uploads convert at their boundary and never call it
    pub async fn compress_if_needed_optimistic(
        local_file: SaiWsOptimisticLocalFile,
        video_edit: Option<MediaPickerEditedVideo>,
        configuration: MediaCompressionConfiguration,
        on_progress: Option<OutgoingMediaProgress>,
        is_cancelled: Option<OutgoingMediaIsCancelled>,
    ) -> anyhow::Result<OutgoingMediaCompressorResult> {
        let source_path = Self::file_path(&local_file.url);
        let mut outgoing = RiseOutgoingMediaFile::new(
            source_path.clone(),
            local_file.filename.clone(),
            local_file.content_type.clone(),
        );
        outgoing.file_context = local_file.file_context_override.clone();
        outgoing.media_type = local_file.preferred_media_type.clone();
        outgoing.byte_count = local_file.byte_count;
        let mut result = Self::compress_if_needed(
            outgoing,
            video_edit,
            configuration,
            on_progress,
            is_cancelled,
        )
        .await?;
        if Self::file_path(&result.url) == source_path {
            result.url = local_file.url;
        }
        Ok(result)
    }

    async fn compress_off_main(
        local_file: RiseOutgoingMediaFile,
        video_edit: Option<MediaPickerEditedVideo>,
        configuration: MediaCompressionConfiguration,
        on_progress: Option<OutgoingMediaProgress>,
        is_cancelled: Option<OutgoingMediaIsCancelled>,
        task_cancelled: Arc<AtomicBool>,
    ) -> anyhow::Result<OutgoingMediaCompressorResult> {
        Self::start_stale_temporary_cleanup();
        Self::check_cancellation(&is_cancelled).await?;
        let mime = local_file.content_type.to_lowercase();
        if local_file.media_type.as_deref().map(str::to_lowercase).as_deref() == Some("document") {
            Self::report(&on_progress, 1.0);
            return Ok(Self::document_result(Self::plain_result(&local_file.url, &mime)?));
        }
        if mime.starts_with("image/") {
            return Self::compress_image(&local_file.url, &mime, &configuration, &on_progress, &is_cancelled, &task_cancelled)
                .await;
        }
        if let Some(recipe) = &local_file.video_note {
            return Self::compose_video_note(recipe, on_progress, is_cancelled).await;
        }
        if mime.starts_with("video/") {
            return Self::compress_video(
                &local_file.url,
                &mime,
                video_edit.as_ref(),
                &configuration,
                &on_progress,
                &is_cancelled,
            )
            .await;
        }
        Self::report(&on_progress, 1.0);
        Self::plain_result(&local_file.url, &mime)
    }

    async fn compress_image(
        file_url: &Path,
        mime: &str,
        configuration: &MediaCompressionConfiguration,
        on_progress: &Option<OutgoingMediaProgress>,
        is_cancelled: &Option<OutgoingMediaIsCancelled>,
        task_cancelled: &Arc<AtomicBool>,
    ) -> anyhow::Result<OutgoingMediaCompressorResult> {
        Self::report(on_progress, 0.05);
        Self::check_cancellation(is_cancelled).await?;
        if mime == "image/gif" {
            Self::report(on_progress, 1.0);
            return Self::plain_result(file_url, mime);
        }
        let runtime = RiseAccountRuntime::current();
        let source_url = file_url.to_path_buf();
        let Some(source) = runtime
            .spawn_blocking(move || RiseOutgoingImageEncoder::source_with_url(&source_url))
            .await
            .map_err(|_| CancellationError)?
        else {
            return Err(RiseOutgoingImageEncoderFailure::Unreadable.into());
        };
        Self::report(on_progress, 0.2);
        let profile = configuration.profile;
        let encoder_cancelled = task_cancelled.clone();
        let encoded = runtime
            .spawn_blocking(move || {
                RiseOutgoingImageEncoder::encode_jpeg_source::<anyhow::Error>(source, profile, &|| {
                    if encoder_cancelled.load(Ordering::SeqCst) {
                        Err(CancellationError.into())
                    } else {
                        Ok(())
                    }
                })
            })
            .await
            .map_err(|_| CancellationError)??;
        Self::check_cancellation(is_cancelled).await?;
        Self::report(on_progress, 0.9);
        let out_url = Self::temporary_file(&format!(
            "{}-{}.jpg",
            configuration.image_temp_filename_prefix,
            Uuid::new_v4().uuid_string()
        ));
        Self::write_atomically(&out_url, &encoded.data)?;
        if let Err(error) = Self::check_cancellation(is_cancelled).await {
            let _ = FileManager.remove_item(&out_url);
            return Err(error);
        }
        Self::report(on_progress, 1.0);
        Ok(OutgoingMediaCompressorResult {
            url: Self::file_url(&out_url)?,
            content_type: "image/jpeg".to_owned(),
            byte_count: i64::try_from(encoded.data.len()).unwrap_or(i64::MAX),
            pixel_width: Some(i64::from(encoded.pixel_width)),
            pixel_height: Some(i64::from(encoded.pixel_height)),
        })
    }

    async fn compose_video_note(
        recipe: &VideoNoteCaptureRecipe,
        on_progress: Option<OutgoingMediaProgress>,
        is_cancelled: Option<OutgoingMediaIsCancelled>,
    ) -> anyhow::Result<OutgoingMediaCompressorResult> {
        Self::check_cancellation(&is_cancelled).await?;
        Self::report(&on_progress, 0.0);
        let output_url = VideoNoteComposition::export(recipe, on_progress.clone(), is_cancelled).await?;
        VideoNoteComposition::discard_sources(recipe);
        let size = FileManager.attributes_of_item(&output_url)?.size;
        Self::report(&on_progress, 1.0);
        Ok(OutgoingMediaCompressorResult {
            url: Self::file_url(&output_url)?,
            content_type: "video/mp4".to_owned(),
            byte_count: size,
            pixel_width: Some(VideoNoteComposition::CANVAS_SIZE.width as i64),
            pixel_height: Some(VideoNoteComposition::CANVAS_SIZE.height as i64),
        })
    }

    async fn compress_video(
        file_url: &Path,
        mime: &str,
        video_edit: Option<&MediaPickerEditedVideo>,
        configuration: &MediaCompressionConfiguration,
        on_progress: &Option<OutgoingMediaProgress>,
        is_cancelled: &Option<OutgoingMediaIsCancelled>,
    ) -> anyhow::Result<OutgoingMediaCompressorResult> {
        Self::check_cancellation(is_cancelled).await?;
        Self::report(on_progress, 0.0);
        let runtime = RiseAccountRuntime::current();
        let probed_url = file_url.to_path_buf();
        let source_probe: Option<VideoProbe> = runtime
            .spawn_blocking(move || probe(&probed_url).ok())
            .await
            .map_err(|_| CancellationError)?;
        let out_url = Self::temporary_file(&format!(
            "{}-{}.mp4",
            configuration.video_temp_filename_prefix,
            Uuid::new_v4().uuid_string()
        ));

        let mut renders_edits = false;
        let mut exported_seconds = source_probe.as_ref().map_or(f64::NAN, |probe| probe.duration_seconds);
        let mut trim = None;
        let mut composition = None;
        if let Some(video_edit) = video_edit {
            if let Some(probe) = &source_probe
                && let Some(clamped) = clamp_trim(probe.duration_seconds, video_edit.start_seconds, video_edit.end_seconds)
            {
                trim = Some(clamped.span);
                exported_seconds = clamped.span.duration_seconds;
                renders_edits = clamped.cuts;
            }
            composition = source_probe
                .as_ref()
                .and_then(|probe| EditorVideoCompositionBuilder::make(probe, video_edit.spatial_edit.as_ref()));
            if composition.is_some() {
                renders_edits = true;
            }
            if video_edit.is_muted {
                renders_edits = true;
            }
        }

        let Some(source_probe) = source_probe else {
            tracing::warn!(target: "riseonly::X1487", "video compress: probe failed path={}", file_url.display());
            return Self::export_failed_fallback(file_url, mime, renders_edits, on_progress, None);
        };

        let source_bytes = Self::file_byte_count(file_url);
        let duration_ms = if exported_seconds.is_finite() && exported_seconds > 0.0 {
            (exported_seconds * 1000.0) as i64
        } else {
            0
        };
        let mut file_length_limit = None;
        if duration_ms > 0 && source_bytes > 0 {
            let profile = configuration.profile;
            if !renders_edits && !profile.video_needs_transcode(source_bytes, duration_ms) {
                let _ = FileManager.remove_item(&out_url);
                Self::report(on_progress, 1.0);
                return Self::plain_result(file_url, mime);
            }
            file_length_limit = u64::try_from(profile.video_file_length_ceiling(source_bytes, duration_ms))
                .ok()
                .filter(|limit| *limit > 0);
        }

        let mut job = ExportJob::transcode(
            file_url,
            &source_probe,
            configuration.profile.video_export_preset(),
            &out_url,
            trim,
            file_length_limit,
        );
        if video_edit.is_some_and(|edit| edit.is_muted)
            && let Some(segment) = job.segments.first_mut()
        {
            segment.volume = source_probe.has_audio.then_some(0.0);
        }
        if let Some(composition) = composition {
            job.render = Some(composition.render);
            if let Some(segment) = job.segments.first_mut() {
                segment.transform = composition.transform;
            }
            job.cap_frame_rate = false;
        }

        let (done, mut outcome_receiver) = oneshot::channel::<ExportOutcome>();
        let started = runtime
            .spawn_blocking(move || {
                start_export(
                    job,
                    Box::new(move |outcome| {
                        let _ = done.send(outcome);
                    }),
                )
            })
            .await
            .map_err(|_| CancellationError)?;
        let ticket = match started {
            Ok(ticket) => ticket,
            Err(error @ (VideoExportError::PresetUnavailable(_) | VideoExportError::Mp4Unsupported)) => {
                tracing::warn!(target: "riseonly::X1487", "video compress: no export session error={error:?}");
                let _ = FileManager.remove_item(&out_url);
                return Err(RiseOutgoingVideoTranscoderFailure::ExportFailed.into());
            }
            Err(error) => {
                tracing::warn!(target: "riseonly::X1487", "video compress: export start failed error={error:?} edits={renders_edits}");
                let _ = FileManager.remove_item(&out_url);
                return Self::export_failed_fallback(file_url, mime, renders_edits, on_progress, Some(error.into()));
            }
        };
        let mut guard = OutgoingMediaExportCancelGuard { ticket: Some(ticket.clone()) };

        let mut throttle = ProgressThrottle::default();
        let mut monitoring = true;
        let outcome = loop {
            if monitoring {
                if Self::cancelled(is_cancelled).await {
                    ticket.cancel();
                    monitoring = false;
                } else if let Some(progress) = throttle.next(ticket.progress()) {
                    Self::report(on_progress, f64::from(progress));
                }
            }
            if !monitoring {
                break (&mut outcome_receiver).await.ok();
            }
            let timer = pin!(runtime.spawn(async move { tokio::time::sleep(PROGRESS_POLL_INTERVAL).await }));
            match select(&mut outcome_receiver, timer).await {
                Either::Left((outcome, _)) => break outcome.ok(),
                Either::Right((_, _)) => {}
            }
        };
        guard.disarm();

        if let Err(error) = Self::check_cancellation(is_cancelled).await {
            let _ = FileManager.remove_item(&out_url);
            return Err(error);
        }
        match outcome {
            Some(ExportOutcome::Completed { .. }) => {
                let size = FileManager.attributes_of_item(&out_url)?.size;
                if !renders_edits && source_bytes > 0 && size >= source_bytes {
                    let _ = FileManager.remove_item(&out_url);
                    Self::report(on_progress, 1.0);
                    return Self::plain_result(file_url, mime);
                }
                Self::report(on_progress, 1.0);
                let positive = |side: f32| Some(side as i64).filter(|side| *side > 0);
                Ok(OutgoingMediaCompressorResult {
                    url: Self::file_url(&out_url)?,
                    content_type: "video/mp4".to_owned(),
                    byte_count: size,
                    pixel_width: video_edit.and_then(|edit| positive(edit.pixel_size.width)),
                    pixel_height: video_edit.and_then(|edit| positive(edit.pixel_size.height)),
                })
            }
            Some(ExportOutcome::Cancelled) => {
                let _ = FileManager.remove_item(&out_url);
                Err(CancellationError.into())
            }
            Some(ExportOutcome::Failed(error)) => {
                tracing::warn!(target: "riseonly::X1487", "video compress: export failed error={error:?} edits={renders_edits}");
                let _ = FileManager.remove_item(&out_url);
                Self::export_failed_fallback(file_url, mime, renders_edits, on_progress, Some(error.into()))
            }
            None => {
                let _ = FileManager.remove_item(&out_url);
                Self::export_failed_fallback(file_url, mime, renders_edits, on_progress, None)
            }
        }
    }

    /// an unedited clip the encoder refuses still goes out as is, like one already too light to transcode
    /// only rendered edits have no original to fall back to
    fn export_failed_fallback(
        file_url: &Path,
        mime: &str,
        renders_edits: bool,
        on_progress: &Option<OutgoingMediaProgress>,
        error: Option<anyhow::Error>,
    ) -> anyhow::Result<OutgoingMediaCompressorResult> {
        if !renders_edits {
            Self::report(on_progress, 1.0);
            return Self::plain_result(file_url, mime);
        }
        Err(error.unwrap_or_else(|| RiseOutgoingVideoTranscoderFailure::ExportFailed.into()))
    }

    fn file_byte_count(at: &Path) -> i64 {
        FileManager.attributes_of_item(at).map(|attributes| attributes.size).unwrap_or(0)
    }

    fn report(on_progress: &Option<OutgoingMediaProgress>, progress: f64) {
        if let Some(on_progress) = on_progress {
            on_progress(progress);
        }
    }

    fn cancelled(is_cancelled: &Option<OutgoingMediaIsCancelled>) -> BoxFuture<'static, bool> {
        match is_cancelled {
            Some(is_cancelled) => is_cancelled(),
            None => Box::pin(async { false }),
        }
    }

    async fn check_cancellation(is_cancelled: &Option<OutgoingMediaIsCancelled>) -> anyhow::Result<()> {
        if Self::cancelled(is_cancelled).await {
            return Err(CancellationError.into());
        }
        Ok(())
    }

    fn document_result(result: OutgoingMediaCompressorResult) -> OutgoingMediaCompressorResult {
        let (Some(width), Some(height)) = (result.pixel_width, result.pixel_height) else {
            return result;
        };
        let maximum_dimension = Self::chat_image_maximum_dimension();
        let oversized = width > maximum_dimension
            || height > maximum_dimension
            || width.saturating_mul(height) > Self::chat_image_maximum_pixels();
        if !Self::PROBED_IMAGE_MIMES.contains(&result.content_type.as_str()) || !oversized {
            return result;
        }
        OutgoingMediaCompressorResult {
            content_type: "application/octet-stream".to_owned(),
            ..result
        }
    }

    fn plain_result(file_url: &Path, mime: &str) -> anyhow::Result<OutgoingMediaCompressorResult> {
        let size = FileManager.attributes_of_item(file_url)?.size;
        let dimensions = if mime.starts_with("image/") { Self::pixel_dimensions(file_url) } else { None };
        Ok(OutgoingMediaCompressorResult {
            url: Self::file_url(file_url)?,
            content_type: mime.to_owned(),
            byte_count: size,
            pixel_width: dimensions.map(|(width, _)| width),
            pixel_height: dimensions.map(|(_, height)| height),
        })
    }

    fn pixel_dimensions(file_url: &Path) -> Option<(i64, i64)> {
        let (width, height) = image::image_dimensions(file_url).ok()?;
        (width > 0 && height > 0).then_some((i64::from(width), i64::from(height)))
    }

    /// path in the temp directory for this name
    fn temporary_file(name: &str) -> PathBuf {
        let directory = temporary_directory();
        let _ = std::fs::create_dir_all(&directory);
        directory.join(name)
    }

    /// file url for an absolute path in this hosts path flavor
    fn file_url(path: &Path) -> anyhow::Result<Url> {
        Url::from_file_path(path).ok_or_else(|| {
            let mut error = CocoaError::new(CocoaErrorCode::FileReadInvalidFileName);
            error.path = Some(path.to_path_buf());
            error.into()
        })
    }

    /// path of a local file url on this host, windows drive urls included
    fn file_path(url: &Url) -> PathBuf {
        url.to_file_path().unwrap_or_else(|| {
            let component = url.path();
            PathBuf::from(component)
        })
    }

    fn write_atomically(destination: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut staging = destination.as_os_str().to_owned();
        staging.push(".tmp");
        let staging = PathBuf::from(staging);
        std::fs::write(&staging, data)
            .and_then(|()| std::fs::rename(&staging, destination))
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&staging);
            })
    }
}
