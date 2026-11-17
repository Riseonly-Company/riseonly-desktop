use std::borrow::Cow;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;

use futures::channel::oneshot;
use futures::future::{BoxFuture, Either, select};
use gpui::BackgroundExecutor;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage};
use rise_limits::{OUTGOING_MEDIA_PROFILES, OUTGOING_MEDIA_PROFILES_PREMIUM, OutgoingMediaProfile};
use rise_engine::CancellationError;
use rise_media::video_export::{
    ExportJob, ExportOutcome, ExportPreset, ExportTicket, PROGRESS_POLL_INTERVAL, ProgressThrottle,
    VideoExportError, VideoProbe, probe, start_export,
};
use rise_platform::file_manager::FileManager;

/// outgoing media profile by canon name, a missing one is a canon defect so it crashes on purpose
pub fn outgoing_media_profile(name: &str, premium: bool) -> &'static OutgoingMediaProfile {
    match rise_limits::outgoing_media_profile_for(name, premium) {
        Some(profile) => profile,
        None => panic!("Canon has no outgoing media profile named {name}"),
    }
}

/// largest side across both tiers, loading more from the library is wasted work
pub const fn outgoing_media_largest_image_side() -> u64 {
    let mut largest = OUTGOING_MEDIA_PROFILES[0].image_max_side;
    let mut index = 0;
    while index < OUTGOING_MEDIA_PROFILES.len() {
        if OUTGOING_MEDIA_PROFILES[index].image_max_side > largest {
            largest = OUTGOING_MEDIA_PROFILES[index].image_max_side;
        }
        if OUTGOING_MEDIA_PROFILES_PREMIUM[index].image_max_side > largest {
            largest = OUTGOING_MEDIA_PROFILES_PREMIUM[index].image_max_side;
        }
        index += 1;
    }
    largest
}

pub trait RiseOutgoingMediaProfileExt {
    fn image_quality_start(&self) -> f64;
    fn image_quality_floor(&self) -> f64;
    fn image_quality_step(&self) -> f64;

    fn video_export_preset(&self) -> ExportPreset;

    fn video_needs_transcode(&self, source_bytes: i64, duration_ms: i64) -> bool;

    fn video_file_length_ceiling(&self, source_bytes: i64, duration_ms: i64) -> i64;
}

fn canon_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

impl RiseOutgoingMediaProfileExt for OutgoingMediaProfile {
    fn image_quality_start(&self) -> f64 {
        self.image_quality_start_percent as f64 / 100.0
    }

    fn image_quality_floor(&self) -> f64 {
        self.image_quality_floor_percent as f64 / 100.0
    }

    fn image_quality_step(&self) -> f64 {
        self.image_quality_step_percent as f64 / 100.0
    }

    fn video_export_preset(&self) -> ExportPreset {
        ExportPreset::for_short_side(self.video_max_side)
    }

    fn video_needs_transcode(&self, source_bytes: i64, duration_ms: i64) -> bool {
        if source_bytes <= 0 || duration_ms <= 0 {
            return false;
        }
        let Some(source_bits_per_second) = source_bytes.checked_mul(8000) else {
            return true;
        };
        source_bits_per_second / duration_ms > canon_i64(self.video_floor_kbps).saturating_mul(1000)
    }

    fn video_file_length_ceiling(&self, source_bytes: i64, duration_ms: i64) -> i64 {
        let budget = canon_i64(self.video_target_kbps).saturating_mul(duration_ms) / 8;
        let cut = source_bytes.saturating_mul(canon_i64(self.video_max_source_fraction_percent)) / 100;
        let floor = canon_i64(self.video_floor_kbps).saturating_mul(duration_ms) / 8;
        floor.max(budget.min(cut))
    }
}

/// outgoing jpeg codec, source is decoded once and every pass scales from it
/// exif is not copied so the geotag never reaches the recipient
pub struct RiseOutgoingImageEncoder;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseOutgoingImageEncoderOutput {
    pub data: Vec<u8>,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseOutgoingImageEncoderFailure {
    #[error("unreadable")]
    Unreadable,
    #[error("encodingFailed")]
    EncodingFailed,
    #[error("byteLimitNotMet")]
    ByteLimitNotMet,
}

impl RiseOutgoingImageEncoder {
    const MAXIMUM_RESIZE_PASSES: u32 = 16;

    /// opens an image file, none when its not a readable image
    pub fn source_with_url(file_url: &Path) -> Option<DynamicImage> {
        let reader = ImageReader::open(file_url).ok()?.with_guessed_format().ok()?;
        Self::decode_oriented(reader)
    }

    pub fn source_with_data(data: &[u8]) -> Option<DynamicImage> {
        let reader = ImageReader::new(Cursor::new(data)).with_guessed_format().ok()?;
        Self::decode_oriented(reader)
    }

    fn decode_oriented<R: std::io::BufRead + std::io::Seek>(
        reader: ImageReader<R>,
    ) -> Option<DynamicImage> {
        let mut decoder = reader.into_decoder().ok()?;
        let orientation = decoder.orientation().ok();
        let mut image = DynamicImage::from_decoder(decoder).ok()?;
        if image.width() == 0 || image.height() == 0 {
            return None;
        }
        if let Some(orientation) = orientation {
            image.apply_orientation(orientation);
        }
        Some(image)
    }

    pub fn encode_jpeg<E: From<RiseOutgoingImageEncoderFailure>>(
        file_url: &Path,
        profile: &OutgoingMediaProfile,
        is_cancelled: &dyn Fn() -> Result<(), E>,
    ) -> Result<RiseOutgoingImageEncoderOutput, E> {
        let Some(source) = Self::source_with_url(file_url) else {
            return Err(RiseOutgoingImageEncoderFailure::Unreadable.into());
        };
        Self::encode_jpeg_source(source, profile, is_cancelled)
    }

    pub fn encode_jpeg_source<E: From<RiseOutgoingImageEncoderFailure>>(
        source: DynamicImage,
        profile: &OutgoingMediaProfile,
        is_cancelled: &dyn Fn() -> Result<(), E>,
    ) -> Result<RiseOutgoingImageEncoderOutput, E> {
        if source.width() == 0 || source.height() == 0 {
            return Err(RiseOutgoingImageEncoderFailure::Unreadable.into());
        }
        is_cancelled()?;

        let source = Self::flattened(source);
        let max_bytes = usize::try_from(profile.image_max_bytes).unwrap_or(usize::MAX);
        let mut side = profile.image_max_side;
        let mut image = Self::decode(&source, side);
        let mut quality = profile.image_quality_start();
        let mut data = Self::encode(&image, quality)?;
        while data.len() > max_bytes && quality > profile.image_quality_floor() {
            is_cancelled()?;
            quality = profile
                .image_quality_floor()
                .max(quality - profile.image_quality_step());
            data = Self::encode(&image, quality)?;
        }

        let mut passes = 0;
        while data.len() > max_bytes && passes < Self::MAXIMUM_RESIZE_PASSES {
            is_cancelled()?;
            let current_side = u64::from(image.width().max(image.height()));
            let Some(next) = profile.next_image_side(current_side, data.len() as u64) else {
                break;
            };
            side = next;
            image = Self::decode(&source, side);
            data = Self::encode(&image, profile.image_quality_floor())?;
            passes += 1;
        }
        if data.len() > max_bytes {
            return Err(RiseOutgoingImageEncoderFailure::ByteLimitNotMet.into());
        }
        Ok(RiseOutgoingImageEncoderOutput {
            data,
            pixel_width: image.width(),
            pixel_height: image.height(),
        })
    }

    /// fits the long side into max_pixel_size and never upscales
    fn decode(source: &DynamicImage, max_pixel_size: u64) -> Cow<'_, DynamicImage> {
        let long_side = u64::from(source.width().max(source.height()));
        if long_side <= max_pixel_size {
            return Cow::Borrowed(source);
        }
        let bound = u32::try_from(max_pixel_size.max(1)).unwrap_or(u32::MAX);
        Cow::Owned(source.resize(bound, bound, FilterType::CatmullRom))
    }

    /// jpeg of an in memory image on a white backing so transparent areas dont turn black
    pub fn white_backed_jpeg_data(image: &DynamicImage, compression_quality: f64) -> Option<Vec<u8>> {
        if image.width() == 0 || image.height() == 0 {
            return None;
        }
        let flat = Self::flattened(image.clone());
        Self::encode(&flat, compression_quality).ok()
    }

    /// jpeg has no alpha so the image is flattened onto white, result is 8 bit gray or rgb
    fn flattened(image: DynamicImage) -> DynamicImage {
        if matches!(image, DynamicImage::ImageLuma8(_) | DynamicImage::ImageRgb8(_)) {
            return image;
        }
        let color = image.color();
        if !color.has_alpha() {
            return if color.has_color() {
                DynamicImage::ImageRgb8(image.to_rgb8())
            } else {
                DynamicImage::ImageLuma8(image.to_luma8())
            };
        }
        let rgba = image.to_rgba8();
        let mut flat = RgbImage::new(rgba.width(), rgba.height());
        for (target, source) in flat.pixels_mut().zip(rgba.pixels()) {
            let alpha = u32::from(source.0[3]);
            for (out, channel) in target.0.iter_mut().zip(source.0.iter()) {
                let value = u32::from(*channel) * alpha + 255 * (255 - alpha);
                *out = ((value + 127) / 255) as u8;
            }
        }
        DynamicImage::ImageRgb8(flat)
    }

    fn encode(image: &DynamicImage, quality: f64) -> Result<Vec<u8>, RiseOutgoingImageEncoderFailure> {
        let percent = (quality * 100.0).round().clamp(1.0, 100.0) as u8;
        let mut data = Vec::new();
        let result = {
            let mut encoder = JpegEncoder::new_with_quality(&mut data, percent);
            match image {
                DynamicImage::ImageLuma8(buffer) => encoder.encode_image(buffer),
                DynamicImage::ImageRgb8(buffer) => encoder.encode_image(buffer),
                other => encoder.encode_image(&other.to_rgb8()),
            }
        };
        if result.is_err() || data.is_empty() {
            return Err(RiseOutgoingImageEncoderFailure::EncodingFailed);
        }
        Ok(data)
    }
}

/// plain clip transcode, preset from the profile plus a weight ceiling
/// the share flow sends the clip as is and uses this directly
pub struct RiseOutgoingVideoTranscoder;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseOutgoingVideoTranscoderOutput {
    pub url: PathBuf,
    pub byte_count: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseOutgoingVideoTranscoderFailure {
    #[error("exportFailed")]
    ExportFailed,
}

/// progress callback with a fraction from 0 to 1
pub type RiseOutgoingVideoTranscoderProgress = Arc<dyn Fn(f64) + Send + Sync>;

pub type RiseOutgoingVideoTranscoderIsCancelled = Arc<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync>;

struct RiseOutgoingVideoExportCancelGuard {
    ticket: Option<ExportTicket>,
}

impl RiseOutgoingVideoExportCancelGuard {
    fn disarm(&mut self) {
        self.ticket = None;
    }
}

impl Drop for RiseOutgoingVideoExportCancelGuard {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            ticket.cancel();
        }
    }
}

impl RiseOutgoingVideoTranscoder {
    pub async fn transcode_if_needed(
        executor: BackgroundExecutor,
        source_url: PathBuf,
        profile: OutgoingMediaProfile,
        output_url: PathBuf,
        on_progress: Option<RiseOutgoingVideoTranscoderProgress>,
        is_cancelled: Option<RiseOutgoingVideoTranscoderIsCancelled>,
    ) -> anyhow::Result<Option<RiseOutgoingVideoTranscoderOutput>> {
        let report = |progress: f64| {
            if let Some(on_progress) = &on_progress {
                on_progress(progress);
            }
        };
        let cancelled = || -> BoxFuture<'static, bool> {
            match &is_cancelled {
                Some(is_cancelled) => is_cancelled(),
                None => Box::pin(async { false }),
            }
        };

        let probed_url = source_url.clone();
        let source_probe: VideoProbe = executor.spawn(async move { probe(&probed_url) }).await?;
        let duration_ms = source_probe.duration_ms();
        let source_bytes = Self::file_byte_count(&source_url);
        if !profile.video_needs_transcode(source_bytes, duration_ms) {
            report(1.0);
            return Ok(None);
        }
        let _ = FileManager.remove_item(&output_url);
        let ceiling = profile.video_file_length_ceiling(source_bytes, duration_ms);
        let file_length_limit = u64::try_from(ceiling).ok().filter(|limit| *limit > 0);
        let job = ExportJob::transcode(
            &source_url,
            &source_probe,
            profile.video_export_preset(),
            &output_url,
            None,
            file_length_limit,
        );

        let (done, mut outcome_receiver) = oneshot::channel::<ExportOutcome>();
        let started = executor
            .spawn(async move {
                start_export(
                    job,
                    Box::new(move |outcome| {
                        let _ = done.send(outcome);
                    }),
                )
            })
            .await;
        let ticket = match started {
            Ok(ticket) => ticket,
            Err(VideoExportError::PresetUnavailable(_) | VideoExportError::Mp4Unsupported) => {
                report(1.0);
                return Ok(None);
            }
            Err(error) => {
                let _ = FileManager.remove_item(&output_url);
                return Err(error.into());
            }
        };
        let mut guard = RiseOutgoingVideoExportCancelGuard { ticket: Some(ticket.clone()) };

        let mut throttle = ProgressThrottle::default();
        let mut monitoring = true;
        let outcome = loop {
            if monitoring {
                if cancelled().await {
                    ticket.cancel();
                    monitoring = false;
                } else if let Some(progress) = throttle.next(ticket.progress()) {
                    report(f64::from(progress));
                }
            }
            if !monitoring {
                break (&mut outcome_receiver).await.ok();
            }
            let timer = pin!(executor.timer(PROGRESS_POLL_INTERVAL));
            match select(&mut outcome_receiver, timer).await {
                Either::Left((outcome, _)) => break outcome.ok(),
                Either::Right(((), _)) => {}
            }
        };
        guard.disarm();

        if cancelled().await {
            let _ = FileManager.remove_item(&output_url);
            return Err(CancellationError.into());
        }
        match outcome {
            Some(ExportOutcome::Completed { .. }) => {}
            Some(ExportOutcome::Cancelled) => {
                let _ = FileManager.remove_item(&output_url);
                return Err(CancellationError.into());
            }
            Some(ExportOutcome::Failed(error)) => {
                let _ = FileManager.remove_item(&output_url);
                return Err(error.into());
            }
            None => {
                let _ = FileManager.remove_item(&output_url);
                return Err(RiseOutgoingVideoTranscoderFailure::ExportFailed.into());
            }
        }
        let size = Self::file_byte_count(&output_url);
        if size <= 0 || (source_bytes != 0 && size >= source_bytes) {
            let _ = FileManager.remove_item(&output_url);
            report(1.0);
            return Ok(None);
        }
        report(1.0);
        Ok(Some(RiseOutgoingVideoTranscoderOutput { url: output_url, byte_count: size }))
    }

    /// byte count of a file, missing or unreadable counts as zero
    pub fn file_byte_count(at: &Path) -> i64 {
        FileManager
            .attributes_of_item(at)
            .map(|attributes| attributes.size)
            .unwrap_or(0)
    }
}
