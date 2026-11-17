use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;

use futures::channel::oneshot;
use futures::future::{BoxFuture, Either, select};
use gpui::AssetSource;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{FilterType, flip_horizontal_in_place, overlay};
use image::{Rgb, RgbImage, Rgba, RgbaImage};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::video_export::{
    AudioLayer, AudioPlacement, ColorAdjustments, ExportJob, ExportOutcome, ExportPreset, FrameGeometry, FrameSize,
    Overlay, PROGRESS_POLL_INTERVAL, ProgressThrottle, RenderPlan, TimeSpan,
    VIDEO_NOTE_CANVAS_SIDE, VideoExportError, VideoProbe, VideoSegment, poster as video_poster,
    probe, segment_transform, start_export,
};
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use crate::core::assets::RiseAssets;
use crate::core::lib::sai::sai_outgoing_media_hooks::{OutgoingMediaIsCancelled, OutgoingMediaProgress};

/// one captured video note segment
/// a camera switch and a pause sit between segments
/// so mirroring and audio offset are stored per segment
#[derive(Clone, Debug, PartialEq)]
pub struct VideoNoteCaptureSegment {
    pub url: PathBuf,
    pub is_front_camera: bool,
    pub start_offset: f64,
}

/// inputs for composing a video note, no composition done yet
/// travels to the send queue, composition takes seconds and must not hold the finger or the screen
#[derive(Clone, Debug, PartialEq)]
pub struct VideoNoteCaptureRecipe {
    pub segments: Vec<VideoNoteCaptureSegment>,
    pub audio_url: Option<PathBuf>,
}

impl VideoNoteCaptureRecipe {
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VideoNoteCompositionExportError {
    #[error("Video track missing")]
    MissingVideoTrack,
    #[error("Failed to compose video")]
    CompositionFailed,
    #[error("Video export failed")]
    ExportFailed,
}

pub enum VideoNoteComposition {}

struct VideoNoteExportCancelGuard {
    ticket: Option<rise_media::video_export::ExportTicket>,
}

impl VideoNoteExportCancelGuard {
    fn disarm(&mut self) {
        self.ticket = None;
    }
}

impl Drop for VideoNoteExportCancelGuard {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            ticket.cancel();
        }
    }
}

struct VideoNoteSegmentData {
    source: PathBuf,
    duration_seconds: f64,
    probe: VideoProbe,
    is_front_camera: bool,
    start_offset: f64,
}

impl VideoNoteComposition {
    pub const CANVAS_SIZE: FrameSize =
        FrameSize::new(VIDEO_NOTE_CANVAS_SIDE as f64, VIDEO_NOTE_CANVAS_SIDE as f64);
    pub const CIRCLE_DIAMETER: f64 = VIDEO_NOTE_CANVAS_SIDE as f64;
    pub const CANVAS_BACKGROUND: [f64; 3] = [0.07, 0.07, 0.10];

    pub const TEMPORARY_FILE_PREFIX: &'static str = "ro-video-note-";

    const FRAME_RATE: u32 = 30;
    const POSTER_TIME_SECONDS: f64 = 0.05;
    const POSTER_MAXIMUM_SIDE: u32 = 512;
    const POSTER_JPEG_QUALITY: u8 = 60;
    const LOGO_ASSET: &'static str = "images/riseonly-logo.png";
    const LOGO_SIZE: u32 = 64;

    pub fn temporary_url(role: &str, path_extension: &str) -> PathBuf {
        let directory = temporary_directory();
        let _ = std::fs::create_dir_all(&directory);
        directory.join(format!(
            "{}{role}-{}.{path_extension}",
            Self::TEMPORARY_FILE_PREFIX,
            Uuid::new_v4().uuid_string()
        ))
    }

    /// composes segments on the square canvas with the continuous audio, circle cut out and the logo
    /// falls back to no audio and then to no overlay
    pub async fn export(
        recipe: &VideoNoteCaptureRecipe,
        on_progress: Option<OutgoingMediaProgress>,
        is_cancelled: Option<OutgoingMediaIsCancelled>,
    ) -> anyhow::Result<PathBuf> {
        if recipe.is_empty() {
            return Err(VideoNoteCompositionExportError::MissingVideoTrack.into());
        }
        Self::check_cancellation(&is_cancelled).await?;

        let output_url = Self::temporary_url("branded", "mp4");
        let _ = FileManager.remove_item(&output_url);

        let segments = recipe.segments.clone();
        let audio_url = recipe.audio_url.clone();
        let loaded = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let mut loaded = Vec::new();
                let mut failures = Vec::new();
                for (index, segment) in segments.into_iter().enumerate() {
                    match probe(&segment.url) {
                        Ok(probe) if probe.has_video => loaded.push(VideoNoteSegmentData {
                            source: segment.url,
                            duration_seconds: probe.duration_seconds,
                            probe,
                            is_front_camera: segment.is_front_camera,
                            start_offset: segment.start_offset,
                        }),
                        Ok(_) => failures.push(format!("pass1[{index}]: no video track, skipping")),
                        Err(error) => failures.push(format!("pass1[{index}]: load failed: {error}")),
                    }
                }
                let audio_duration = audio_url
                    .as_deref()
                    .and_then(|url| probe(url).ok())
                    .filter(|probe| probe.has_audio)
                    .map_or(0.0, |probe| probe.duration_seconds);
                (loaded, failures, audio_duration, Self::make_overlay_layers())
            })
            .await;
        let Ok((segment_data_list, failures, audio_duration, overlay_layer)) = loaded else {
            return Err(CancellationError.into());
        };
        if segment_data_list.is_empty() {
            return Err(anyhow::Error::new(VideoNoteCompositionExportError::MissingVideoTrack)
                .context(failures.join("; ")));
        }
        Self::check_cancellation(&is_cancelled).await?;

        let timeline: Vec<(f64, f64)> = segment_data_list
            .iter()
            .map(|data| (data.start_offset, data.duration_seconds))
            .collect();
        let starts = Self::gapless_starts(&timeline);
        let video_end = starts
            .last()
            .zip(segment_data_list.last())
            .map_or(0.0, |(start, data)| start + data.duration_seconds.max(0.0));

        let render_size = (VIDEO_NOTE_CANVAS_SIDE, VIDEO_NOTE_CANVAS_SIDE);
        let video_segments: Vec<VideoSegment> = segment_data_list
            .iter()
            .zip(&starts)
            .map(|(data, start)| VideoSegment {
                source: data.source.clone(),
                span: None,
                at_seconds: Some(*start),
                transform: segment_transform(
                    &data.probe,
                    FrameGeometry { mirrored: data.is_front_camera, ..FrameGeometry::default() },
                    render_size,
                ),
                volume: None,
            })
            .collect();

        let audio_layers: Vec<AudioLayer> = match recipe.audio_url.as_ref() {
            Some(source) if audio_duration > 0.0 && video_end > 0.0 => {
                Self::audio_pieces(&timeline, &starts, audio_duration)
                    .into_iter()
                    .map(|placement| AudioLayer { source: source.clone(), placement: Some(placement), volume: 1.0 })
                    .collect()
            }
            _ => Vec::new(),
        };
        if audio_layers.len() > 1 {
            tracing::debug!(target: "riseonly", pieces = audio_layers.len(), "X1939 video note sound cut per segment");
        }

        let job = |audio: Vec<AudioLayer>, overlay: Option<Overlay>| ExportJob {
            segments: video_segments.clone(),
            audio,
            render: Some(RenderPlan {
                size: render_size,
                frame_rate: Self::FRAME_RATE,
                adjustments: ColorAdjustments::IDENTITY,
                overlay,
                loop_layers: Vec::new(),
                badge: None,
            }),
            preset: ExportPreset::MediumQuality,
            target: output_url.clone(),
            file_length_limit: None,
            cap_frame_rate: false,
        };

        let attempts = [
            ("branded+audio", job(audio_layers, overlay_layer.clone())),
            ("branded-no-audio", job(Vec::new(), overlay_layer)),
            ("plain-no-overlay", job(Vec::new(), None)),
        ];
        let mut failures = failures;
        for (index, (label, job)) in attempts.into_iter().enumerate() {
            if index > 0 {
                Self::check_cancellation(&is_cancelled).await?;
            }
            match Self::run_export(job, &output_url, &on_progress, &is_cancelled).await {
                Ok(()) => return Ok(output_url),
                Err(reason) => failures.push(format!("export[{label}]: {reason}")),
            }
        }
        Err(anyhow::Error::new(VideoNoteCompositionExportError::ExportFailed).context(failures.join("; ")))
    }

    fn gapless_starts(timeline: &[(f64, f64)]) -> Vec<f64> {
        let mut cursor = 0.0_f64;
        timeline
            .iter()
            .map(|(_, duration)| {
                let start = cursor;
                cursor += duration.max(0.0);
                start
            })
            .collect()
    }

    fn audio_pieces(timeline: &[(f64, f64)], starts: &[f64], audio_duration: f64) -> Vec<AudioPlacement> {
        let mut used_until = 0.0_f64;
        let mut pieces = Vec::with_capacity(timeline.len());
        for ((offset, duration), start) in timeline.iter().zip(starts) {
            let from = offset.max(0.0).max(used_until);
            let until = (from + duration.max(0.0)).min(audio_duration);
            if until - from <= 0.0 {
                continue;
            }
            used_until = until;
            pieces.push(AudioPlacement {
                span: TimeSpan { start_seconds: from, duration_seconds: until - from },
                at_seconds: *start,
            });
        }
        pieces
    }

    /// frame for the optimistic bubble, taken from the raw segment and mirrored like the composed note
    /// else a selfie preview flips when the composed file replaces it
    pub async fn poster(recipe: &VideoNoteCaptureRecipe) -> Option<PathBuf> {
        let segment = recipe.segments.first()?.clone();
        RiseAccountRuntime::current()
            .spawn_blocking(move || Self::write_poster(&segment))
            .await
            .ok()
            .flatten()
    }

    /// raw segments and audio live only until composition
    /// the poster stays and fills the bubble until the server returns its own
    pub fn discard_sources(recipe: &VideoNoteCaptureRecipe) {
        for segment in &recipe.segments {
            let _ = FileManager.remove_item(&segment.url);
        }
        if let Some(audio_url) = &recipe.audio_url {
            let _ = FileManager.remove_item(audio_url);
        }
    }

    fn write_poster(segment: &VideoNoteCaptureSegment) -> Option<PathBuf> {
        let frame = video_poster(&segment.url, Self::POSTER_TIME_SECONDS, Self::POSTER_MAXIMUM_SIDE).ok()?;
        let rgb: Vec<u8> = frame
            .bgra
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
            .collect();
        let mut poster = RgbImage::from_raw(frame.width, frame.height, rgb)?;
        if segment.is_front_camera {
            flip_horizontal_in_place(&mut poster);
        }

        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, Self::POSTER_JPEG_QUALITY)
            .encode_image(&poster)
            .ok()?;
        let url = Self::temporary_url("poster", "jpg");
        match std::fs::write(&url, &data) {
            Ok(()) => Some(url),
            Err(_) => {
                let _ = FileManager.remove_item(&url);
                None
            }
        }
    }

    fn make_overlay_layers() -> Option<Overlay> {
        let side = VIDEO_NOTE_CANVAS_SIDE;
        let [red, green, blue] = Self::CANVAS_BACKGROUND.map(|component| (component * 255.0).round() as u8);
        let center_x = Self::CANVAS_SIZE.width / 2.0;
        let center_y = Self::CANVAS_SIZE.height / 2.0;
        let radius = Self::CIRCLE_DIAMETER / 2.0;

        let mut canvas = RgbaImage::from_fn(side, side, |x, y| {
            let dx = f64::from(x) + 0.5 - center_x;
            let dy = f64::from(y) + 0.5 - center_y;
            let coverage = ((dx * dx + dy * dy).sqrt() - radius + 0.5).clamp(0.0, 1.0);
            Rgba([red, green, blue, (coverage * 255.0).round() as u8])
        });

        let logo_size = f64::from(Self::LOGO_SIZE);
        let logo_radius = radius * 1.14375;
        let logo_center_x = center_x - logo_radius * 0.7071 - 6.0;
        let logo_center_y = center_y + logo_radius * 0.7071 + 8.0;
        if let Some(logo) = Self::logo_image() {
            let logo = logo.resize_to_fill(Self::LOGO_SIZE, Self::LOGO_SIZE, FilterType::Lanczos3).to_rgba8();
            overlay(
                &mut canvas,
                &logo,
                (logo_center_x - logo_size / 2.0).round() as i64,
                (logo_center_y - logo_size / 2.0).round() as i64,
            );
        }

        Some(Overlay { width: side, height: side, rgba: Arc::from(canvas.into_raw()) })
    }

    fn logo_image() -> Option<image::DynamicImage> {
        let bytes = RiseAssets::discover().load(Self::LOGO_ASSET).ok()??;
        image::load_from_memory(&bytes).ok()
    }

    async fn run_export(
        job: ExportJob,
        output_url: &Path,
        on_progress: &Option<OutgoingMediaProgress>,
        is_cancelled: &Option<OutgoingMediaIsCancelled>,
    ) -> Result<(), String> {
        let _ = FileManager.remove_item(output_url);
        let report = |progress: f64| {
            if let Some(on_progress) = on_progress {
                on_progress(progress);
            }
        };

        let (done, mut outcome_receiver) = oneshot::channel::<ExportOutcome>();
        let runtime = RiseAccountRuntime::current();
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
            .map_err(|_| "cancelled".to_owned())?;
        let ticket = started.map_err(|error: VideoExportError| format!("could not start: {error}"))?;
        let mut guard = VideoNoteExportCancelGuard { ticket: Some(ticket.clone()) };

        let mut throttle = ProgressThrottle::default();
        let mut monitoring = true;
        let outcome = loop {
            if monitoring {
                if Self::cancelled(is_cancelled).await {
                    ticket.cancel();
                    monitoring = false;
                } else if let Some(progress) = throttle.next(ticket.progress()) {
                    report(f64::from(progress));
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

        match outcome {
            Some(ExportOutcome::Completed { .. }) => {
                report(1.0);
                Ok(())
            }
            Some(ExportOutcome::Cancelled) => Err("cancelled".to_owned()),
            Some(ExportOutcome::Failed(error)) => Err(error.to_string()),
            None => Err("the encoder ended without an outcome".to_owned()),
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
}
