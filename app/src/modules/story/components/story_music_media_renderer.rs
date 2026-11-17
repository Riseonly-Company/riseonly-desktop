use std::fmt;
use std::path::{Path, PathBuf};

use futures::channel::oneshot;
use gpui::{AsyncApp, Size, size};
use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::video_export::{
    AudioLayer, AudioPlacement, ExportJob, ExportOutcome, ExportPreset, ExportTicket, PosterFrame, StillVideoJob,
    TimeSpan, VideoExportError, VideoSegment, probe as video_probe, start_export, start_still_video,
};
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerEditedVideo, MediaPickerItem, MediaPickerItemKind,
};
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StoryMusicMediaRenderError {
    MissingAudio,
    MissingImage,
    MissingVideo,
    InvalidSegment,
    WriterUnavailable,
    ExportUnavailable,
    ExportFailed,
}

impl StoryMusicMediaRenderError {
    /// case index used as the error code of a plain error enum
    fn code(self) -> i64 {
        match self {
            Self::MissingAudio => 0,
            Self::MissingImage => 1,
            Self::MissingVideo => 2,
            Self::InvalidSegment => 3,
            Self::WriterUnavailable => 4,
            Self::ExportUnavailable => 5,
            Self::ExportFailed => 6,
        }
    }
}

impl fmt::Display for StoryMusicMediaRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "The operation couldn\u{2019}t be completed. (Riseonly.StoryMusicMediaRenderError error {}.)",
            self.code()
        )
    }
}

impl std::error::Error for StoryMusicMediaRenderError {}

/// bakes the chosen piece of a track into story media, video gets the track as its only sound
/// photo becomes a still video as long as the piece
pub enum StoryMusicMediaRenderer {}

impl StoryMusicMediaRenderer {
    const IMAGE_STORY_DURATION: f64 = 15.0;
    const MAXIMUM_VIDEO_SIDE: f64 = 1_080.0;
    const STATIC_VIDEO_AVERAGE_BIT_RATE: u32 = 5_000_000;
    const TIMELINE_EPSILON_SECONDS: f64 = 0.000_001;

    pub async fn render(
        item: MediaPickerItem,
        track: &RiseStoredMusicTrack,
        start_ms: i64,
        end_ms: i64,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<MediaPickerItem> {
        if end_ms <= start_ms {
            return Err(StoryMusicMediaRenderError::InvalidSegment.into());
        }
        let Some(audio_source) = RemoteMediaURL::optional_normalized(track.audio_url.as_deref()) else {
            return Err(StoryMusicMediaRenderError::InvalidSegment.into());
        };
        let Some(remote_audio_url) = Url::from_string(&audio_source) else {
            return Err(StoryMusicMediaRenderError::InvalidSegment.into());
        };

        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let authorization_token =
            cx.update(|cx| AuthServicesStore::shared(cx).read(cx).tokens_and_other_data().access_token.clone());
        let cache_key = track
            .id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .and_then(nil_if_empty)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("story-music-url:{audio_source}"));
        let Some(local_audio_url) = MusicPlayerState::materialize_audio_for_playback(
            remote_audio_url,
            cache_key,
            account_user_id.clone(),
            account_epoch.clone(),
            Some(authorization_token),
            cx,
        )
        .await
        else {
            return Err(StoryMusicMediaRenderError::MissingAudio.into());
        };
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return Err(CancellationError.into());
        }

        let request = StoryMusicRenderRequest {
            item,
            local_audio_url: Self::file_path(&local_audio_url),
            start_ms,
            end_ms,
        };
        Self::render_prepared_media(request).await
    }

    pub async fn render_published_audio(
        item: MediaPickerItem,
        published_audio_url: &Path,
    ) -> anyhow::Result<MediaPickerItem> {
        let probed_url = published_audio_url.to_path_buf();
        let probed = RiseAccountRuntime::current()
            .spawn_blocking(move || video_probe(&probed_url))
            .await
            .map_err(|_| anyhow::Error::from(CancellationError))?;
        let duration = probed?.duration_seconds;
        if !duration.is_finite() || duration <= 0.0 {
            return Err(StoryMusicMediaRenderError::MissingAudio.into());
        }
        let request = StoryMusicRenderRequest {
            item,
            local_audio_url: published_audio_url.to_path_buf(),
            start_ms: 0,
            end_ms: (duration * 1_000.0).round() as i64,
        };
        Self::render_prepared_media(request).await
    }

    async fn render_prepared_media(request: StoryMusicRenderRequest) -> anyhow::Result<MediaPickerItem> {
        let StoryMusicRenderRequest {
            item,
            local_audio_url,
            start_ms,
            end_ms,
        } = request;
        let source_video_url: PathBuf;
        let generated_video: Option<StoryMusicTemporaryFile>;
        let poster_url: Option<Url>;
        let pixel_size: Size<f32>;

        match item.kind {
            MediaPickerItemKind::Video => {
                let video_url = item.video_payload.as_ref().map(|payload| &payload.url).unwrap_or(&item.url);
                source_video_url = Self::file_path(video_url);
                generated_video = None;
                poster_url = item.video_payload.as_ref().and_then(|payload| payload.poster_url.clone());
                pixel_size = item.video_payload.as_ref().map(|payload| payload.pixel_size).unwrap_or_default();
            }
            MediaPickerItemKind::Image => {
                let image_data = item.image_payload.as_ref().map(|payload| payload.data.clone());
                let image_path = Self::file_path(&item.url);
                let image = RiseAccountRuntime::current()
                    .spawn_blocking(move || Self::load_image(image_data, &image_path))
                    .await
                    .map_err(|_| anyhow::Error::from(CancellationError))?;
                let Some(image) = image else {
                    return Err(StoryMusicMediaRenderError::MissingImage.into());
                };
                let image_size = size(f64::from(image.width()), f64::from(image.height()));
                let segment_duration = (end_ms - start_ms) as f64 / 1_000.0;
                let duration = Self::IMAGE_STORY_DURATION.min(segment_duration.max(0.1));
                let output_url = Self::temporary_url("ro-story-image", "mp4");
                let generated = StoryMusicTemporaryFile::new(output_url.clone());
                Self::write_static_video(image, duration, &output_url).await?;
                source_video_url = output_url;
                generated_video = Some(generated);
                poster_url = Some(item.url.clone());
                let target = Self::target_video_size(image_size);
                pixel_size = size(target.width as f32, target.height as f32);
            }
            MediaPickerItemKind::File => {
                return Err(StoryMusicMediaRenderError::MissingVideo.into());
            }
        }

        let _generated_video = generated_video;

        let output_url = Self::temporary_url("ro-story-music", "mp4");
        let output = StoryMusicTemporaryFile::new(output_url.clone());
        let duration = Self::replace_audio(&source_video_url, &local_audio_url, start_ms, end_ms, &output_url).await?;
        let Some(output_file_url) = Url::from_file_path(&output_url) else {
            return Err(StoryMusicMediaRenderError::ExportFailed.into());
        };
        let caption = item
            .image_payload
            .as_ref()
            .map(|payload| payload.caption.clone())
            .or_else(|| item.video_payload.as_ref().map(|payload| payload.caption.clone()))
            .unwrap_or_default();
        let mut payload =
            MediaPickerEditedVideo::new(output_file_url.clone(), 0.0, duration, duration, false, caption);
        payload.pixel_size = pixel_size;
        payload.poster_url = poster_url;
        payload.spatial_edit = None;
        let suggested_file_name = output_file_url.last_path_component();
        output.keep();
        Ok(MediaPickerItem::new(
            format!("story-music:{}", Uuid::new_v4().uuid_string().to_lowercase()),
            MediaPickerItemKind::Video,
            output_file_url,
            suggested_file_name,
            None,
            None,
            Some(payload),
            None,
            false,
        ))
    }

    async fn replace_audio(
        video_url: &Path,
        audio_url: &Path,
        start_ms: i64,
        end_ms: i64,
        output_url: &Path,
    ) -> anyhow::Result<f64> {
        let probed_video_url = video_url.to_path_buf();
        let probed_audio_url = audio_url.to_path_buf();
        let (video_asset, audio_asset) = RiseAccountRuntime::current()
            .spawn_blocking(move || (video_probe(&probed_video_url), video_probe(&probed_audio_url)))
            .await
            .map_err(|_| anyhow::Error::from(CancellationError))?;
        let video_asset = video_asset?;
        let audio_asset = audio_asset?;
        if !video_asset.has_video || !audio_asset.has_audio {
            return Err(StoryMusicMediaRenderError::MissingVideo.into());
        }

        let video_duration = video_asset.duration_seconds;
        let audio_duration = audio_asset.duration_seconds;
        let start = start_ms as f64 / 1_000.0;
        let requested_audio_duration = (end_ms - start_ms) as f64 / 1_000.0;
        let available_audio_duration = audio_duration - start;
        let segment_duration = requested_audio_duration.min(available_audio_duration);
        if !video_duration.is_finite()
            || video_duration <= 0.0
            || !segment_duration.is_finite()
            || segment_duration <= 0.0
        {
            return Err(StoryMusicMediaRenderError::InvalidSegment.into());
        }

        let mut audio = Vec::new();
        let mut insertion_time = 0.0;
        while video_duration - insertion_time > Self::TIMELINE_EPSILON_SECONDS {
            let remaining_video_duration = video_duration - insertion_time;
            let insertion_duration = segment_duration.min(remaining_video_duration);
            audio.push(AudioLayer {
                source: audio_url.to_path_buf(),
                placement: Some(AudioPlacement {
                    span: TimeSpan {
                        start_seconds: start,
                        duration_seconds: insertion_duration,
                    },
                    at_seconds: insertion_time,
                }),
                volume: 1.0,
            });
            insertion_time += insertion_duration;
        }

        let job = ExportJob {
            segments: vec![VideoSegment {
                source: video_url.to_path_buf(),
                span: Some(TimeSpan {
                    start_seconds: 0.0,
                    duration_seconds: video_duration,
                }),
                at_seconds: Some(0.0),
                transform: video_asset.upright(),
                volume: None,
            }],
            audio,
            render: None,
            preset: ExportPreset::HighestQuality,
            target: output_url.to_path_buf(),
            file_length_limit: None,
            cap_frame_rate: false,
        };
        let _ = FileManager.remove_item(output_url);
        let (done, outcome) = oneshot::channel::<ExportOutcome>();
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
        let exporter = started.map_err(Self::export_start_failure)?;
        let mut cancellable_exporter = StoryMusicExportSession::new(exporter);
        let outcome = outcome.await.ok();
        cancellable_exporter.finish();
        match outcome {
            Some(ExportOutcome::Completed { .. }) => Ok(video_duration),
            Some(ExportOutcome::Cancelled) => Err(CancellationError.into()),
            Some(ExportOutcome::Failed(error)) => Err(error.into()),
            None => Err(StoryMusicMediaRenderError::ExportFailed.into()),
        }
    }

    fn export_start_failure(error: VideoExportError) -> anyhow::Error {
        match error {
            VideoExportError::PresetUnavailable(_) | VideoExportError::Mp4Unsupported => {
                StoryMusicMediaRenderError::ExportUnavailable.into()
            }
            error => error.into(),
        }
    }

    async fn write_static_video(image: RgbaImage, duration: f64, output_url: &Path) -> anyhow::Result<()> {
        let target = output_url.to_path_buf();
        let pixel_buffer = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let size = Self::target_video_size(size(f64::from(image.width()), f64::from(image.height())));
                Self::make_pixel_buffer(&image, size)
            })
            .await
            .map_err(|_| anyhow::Error::from(CancellationError))?;
        let Some(pixel_buffer) = pixel_buffer else {
            return Err(StoryMusicMediaRenderError::WriterUnavailable.into());
        };
        let _ = FileManager.remove_item(output_url);
        let job = StillVideoJob {
            frame: pixel_buffer,
            duration_seconds: duration,
            average_bit_rate: Self::STATIC_VIDEO_AVERAGE_BIT_RATE,
            target,
        };
        let (done, outcome) = oneshot::channel::<ExportOutcome>();
        let started = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                start_still_video(
                    job,
                    Box::new(move |outcome| {
                        let _ = done.send(outcome);
                    }),
                )
            })
            .await
            .map_err(|_| anyhow::Error::from(CancellationError))?;
        let writer = started.map_err(|error| match error {
            VideoExportError::EncoderUnavailable(_) | VideoExportError::InvalidJob(_) => {
                anyhow::Error::from(StoryMusicMediaRenderError::WriterUnavailable)
            }
            error => anyhow::Error::from(error),
        })?;
        let mut cancellation = StoryMusicExportSession::new(writer);
        let outcome = outcome.await.ok();
        cancellation.finish();
        match outcome {
            Some(ExportOutcome::Completed { .. }) => Ok(()),
            Some(ExportOutcome::Cancelled) => Err(CancellationError.into()),
            Some(ExportOutcome::Failed(error)) => Err(error.into()),
            None => Err(StoryMusicMediaRenderError::WriterUnavailable.into()),
        }
    }

    fn load_image(data: Option<Vec<u8>>, path: &Path) -> Option<RgbaImage> {
        if let Some(data) = data.filter(|data| !data.is_empty())
            && let Ok(image) = image::load_from_memory(&data)
        {
            return Some(image.to_rgba8());
        }
        image::open(path).ok().map(|image| image.to_rgba8())
    }

    fn target_video_size(source: Size<f64>) -> Size<f64> {
        let width = source.width.max(2.0);
        let height = source.height.max(2.0);
        let scale = 1.0_f64.min(Self::MAXIMUM_VIDEO_SIDE / width.max(height));
        size(
            ((width * scale / 2.0).floor() * 2.0).max(2.0),
            ((height * scale / 2.0).floor() * 2.0).max(2.0),
        )
    }

    fn make_pixel_buffer(image: &RgbaImage, size: Size<f64>) -> Option<PosterFrame> {
        let width = size.width as u32;
        let height = size.height as u32;
        if width == 0 || height == 0 || image.width() == 0 || image.height() == 0 {
            return None;
        }
        let mut context = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));
        let source_width = f64::from(image.width());
        let source_height = f64::from(image.height());
        let scale = (size.width / source_width).max(size.height / source_height);
        let draw_width = source_width * scale;
        let draw_height = source_height * scale;
        let draw_x = (size.width - draw_width) / 2.0;
        let draw_y = (size.height - draw_height) / 2.0;
        let drawn = imageops::resize(
            image,
            draw_width.round().max(1.0) as u32,
            draw_height.round().max(1.0) as u32,
            FilterType::Triangle,
        );
        imageops::overlay(&mut context, &drawn, draw_x.round() as i64, draw_y.round() as i64);
        let mut bgra = context.into_raw();
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        Some(PosterFrame { width, height, bgra })
    }

    fn temporary_url(prefix: &str, extension: &str) -> PathBuf {
        temporary_directory().join(format!("{prefix}-{}.{extension}", Uuid::new_v4().uuid_string().to_lowercase()))
    }

    fn file_path(url: &Url) -> PathBuf {
        url.to_file_path().unwrap_or_default()
    }
}

struct StoryMusicRenderRequest {
    item: MediaPickerItem,
    local_audio_url: PathBuf,
    start_ms: i64,
    end_ms: i64,
}

/// dropping the awaiting future cancels the export, a finished one is left alone
struct StoryMusicExportSession {
    exporter: ExportTicket,
    is_finished: bool,
}

impl StoryMusicExportSession {
    fn new(exporter: ExportTicket) -> Self {
        Self {
            exporter,
            is_finished: false,
        }
    }

    fn finish(&mut self) {
        self.is_finished = true;
    }

    fn cancel(&self) {
        self.exporter.cancel();
    }
}

impl Drop for StoryMusicExportSession {
    fn drop(&mut self) {
        if !self.is_finished {
            self.cancel();
        }
    }
}

/// temp render removed when it goes out of scope unless kept as the result
struct StoryMusicTemporaryFile {
    path: PathBuf,
    is_kept: bool,
}

impl StoryMusicTemporaryFile {
    fn new(path: PathBuf) -> Self {
        Self { path, is_kept: false }
    }

    fn keep(mut self) {
        self.is_kept = true;
    }
}

impl Drop for StoryMusicTemporaryFile {
    fn drop(&mut self) {
        if !self.is_kept {
            let _ = FileManager.remove_item(&self.path);
        }
    }
}

fn nil_if_empty(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}
