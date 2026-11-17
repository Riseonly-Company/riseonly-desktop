use std::fmt;
use std::fmt::Write as _;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;

use futures::channel::oneshot;
use futures::future::{Either, select};
use gpui::{AssetSource as _, AsyncApp, FontWeight, px};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage};
use rise_core::foundation::foundation_data::base64_encoded_string;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_media::video_export::{
    BadgeStretch, ColorAdjustments, ExportJob, ExportOutcome, ExportPreset, ExportTicket, MovingBadge, Overlay,
    PROGRESS_POLL_INTERVAL, ProgressThrottle, RenderPlan, VideoSegment, probe, render_frame_rate, start_export,
};
use rise_platform::file_manager::FileManager;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT};

use crate::core::assets::RiseAssets;
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::post::components::download::post_watermark_schedule::{
    PostWatermarkSchedule, PostWatermarkScheduleCorner,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// what the watermark says about the post
/// photos and videos carry the same Riseonly mark with the authors tag
/// coauthors add +N
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostWatermarkLabel {
    pub tag_line: String,
}

impl PostWatermarkLabel {
    pub fn new(post: &GetPostFeedResponse) -> Self {
        let extra = if post.coauthors_count > 0 { format!(" +{}", post.coauthors_count) } else { String::new() };
        let tag = trimming_whitespaces_and_newlines(&post.author.tag);
        let tag_line = if tag.is_empty() {
            extra.trim_start().to_owned()
        } else {
            format!("@{tag}{extra}")
        };
        Self { tag_line }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostWatermarkRendererRenderError {
    DecodeFailed,
    EncodeFailed,
    NoVideoTrack,
    ExportFailed,
}

impl fmt::Display for PostWatermarkRendererRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::DecodeFailed => "decodeFailed",
            Self::EncodeFailed => "encodeFailed",
            Self::NoVideoTrack => "noVideoTrack",
            Self::ExportFailed => "exportFailed",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for PostWatermarkRendererRenderError {}

/// sizes from the short side of the frame so the mark reads the same on any resolution
#[derive(Clone, Copy, Debug)]
struct PostWatermarkRendererMetrics {
    icon: f64,
    line1: f64,
    line2: f64,
    margin: f64,
    blur: f64,
    glow: f64,
}

impl PostWatermarkRendererMetrics {
    fn new(short: f64) -> Self {
        Self {
            icon: (short * 0.043).max(22.0),
            line1: (short * 0.0265).max(11.0),
            line2: (short * 0.022).max(9.5),
            margin: short * 0.031,
            blur: (short * 0.0018).max(1.5),
            glow: (short * 0.0065).max(4.0),
        }
    }
}

pub enum PostWatermarkRenderer {}

impl PostWatermarkRenderer {
    pub const VERSION: i64 = 9;

    const SHADOW_ALPHA: f64 = 0.55;
    const GLOW_ALPHA: f64 = 0.38;
    const JPEG_QUALITY: u8 = 92;
    const TITLE: &'static str = "Riseonly";
    const LOGO_ASSET: &'static str = "images/riseonly-logo.png";

    pub fn render_photo(
        source: &Path,
        label: &PostWatermarkLabel,
        output: &Path,
    ) -> Result<(), PostWatermarkRendererRenderError> {
        use PostWatermarkRendererRenderError as RenderError;
        let reader = ImageReader::open(source)
            .and_then(ImageReader::with_guessed_format)
            .map_err(|_| RenderError::DecodeFailed)?;
        let mut decoder = reader.into_decoder().map_err(|_| RenderError::DecodeFailed)?;
        let orientation = decoder.orientation().map_err(|_| RenderError::DecodeFailed)?;
        let mut image = DynamicImage::from_decoder(decoder).map_err(|_| RenderError::DecodeFailed)?;
        image.apply_orientation(orientation);
        let mut canvas = image.to_rgba8();
        let (width, height) = (f64::from(canvas.width()), f64::from(canvas.height()));
        let mark = PostWatermarkRendererMarkBlock::new(label, width.min(height)).ok_or(RenderError::EncodeFailed)?;
        let pad = mark.metrics.glow * 2.0;
        let badge = mark.bitmap(pad).ok_or(RenderError::EncodeFailed)?;
        let margin = mark.metrics.margin;
        imageops::overlay(
            &mut canvas,
            &badge,
            (margin - pad).round() as i64,
            (height - margin - mark.height - pad).round() as i64,
        );
        let opaque = DynamicImage::ImageRgba8(canvas).to_rgb8();
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut Cursor::new(&mut data), Self::JPEG_QUALITY)
            .encode_image(&opaque)
            .map_err(|_| RenderError::EncodeFailed)?;
        if data.is_empty() {
            return Err(RenderError::EncodeFailed);
        }
        std::fs::write(output, &data).map_err(|_| RenderError::EncodeFailed)
    }

    pub async fn render_video(
        source: PathBuf,
        label: PostWatermarkLabel,
        output: PathBuf,
        on_progress: &dyn Fn(f64),
        cx: &AsyncApp,
    ) -> Result<(), PostWatermarkRendererRenderError> {
        use PostWatermarkRendererRenderError as RenderError;
        let probe_source = source.clone();
        let probed = cx.background_executor().spawn(async move { probe(&probe_source) }).await;
        let probed = probed.map_err(|_| RenderError::NoVideoTrack)?;
        if !probed.has_video {
            return Err(RenderError::NoVideoTrack);
        }
        let display = probed.display_size();
        if !display.is_finite_positive() {
            return Err(RenderError::NoVideoTrack);
        }
        let even = |side: f64| ((side.round() as u32) / 2 * 2).max(2);
        let canvas = (even(display.width), even(display.height));
        let short = f64::from(canvas.0.min(canvas.1));
        let (badge, inset) = Self::video_badge(&label, short).ok_or(RenderError::EncodeFailed)?;
        let seconds = probed.duration_seconds.max(0.1);
        let track_start = probed.video_start_seconds;
        let track_seconds = probed.video_duration_seconds;
        let stretches = Self::badge_stretches(
            (f64::from(canvas.0), f64::from(canvas.1)),
            (f64::from(badge.width), f64::from(badge.height)),
            inset,
            short,
            if track_start.is_finite() { track_start } else { 0.0 },
            if track_seconds.is_finite() && track_seconds > 0.0 { track_seconds } else { seconds },
            seconds,
        );
        let job = ExportJob {
            segments: vec![VideoSegment {
                source: source.clone(),
                span: None,
                at_seconds: Some(0.0),
                transform: probed.upright(),
                volume: probed.has_audio.then_some(1.0),
            }],
            audio: Vec::new(),
            render: Some(RenderPlan {
                size: canvas,
                frame_rate: render_frame_rate(probed.nominal_frame_rate),
                adjustments: ColorAdjustments::IDENTITY,
                overlay: None,
                loop_layers: Vec::new(),
                badge: Some(MovingBadge { image: badge, stretches }),
            }),
            preset: ExportPreset::HighestQuality,
            target: output.clone(),
            file_length_limit: None,
            cap_frame_rate: false,
        };
        let _ = FileManager.remove_item(&output);
        let outcome = Self::run_export(job, on_progress, cx).await;
        match outcome {
            Some(ExportOutcome::Completed { .. }) => {
                on_progress(1.0);
                Ok(())
            }
            Some(ExportOutcome::Cancelled) | Some(ExportOutcome::Failed(_)) | None => {
                let _ = FileManager.remove_item(&output);
                Err(RenderError::ExportFailed)
            }
        }
    }

    async fn run_export(job: ExportJob, on_progress: &dyn Fn(f64), cx: &AsyncApp) -> Option<ExportOutcome> {
        let (done, mut outcome_receiver) = oneshot::channel::<ExportOutcome>();
        let started = cx
            .background_executor()
            .spawn(async move {
                start_export(
                    job,
                    Box::new(move |outcome| {
                        let _ = done.send(outcome);
                    }),
                )
            })
            .await;
        let ticket = started.ok()?;
        let mut guard = PostWatermarkExportGuard { ticket: Some(ticket.clone()) };
        let mut throttle = ProgressThrottle::default();
        let outcome = loop {
            if let Some(progress) = throttle.next(ticket.progress()) {
                on_progress(f64::from(progress));
            }
            let timer = pin!(cx.background_executor().timer(PROGRESS_POLL_INTERVAL));
            match select(&mut outcome_receiver, timer).await {
                Either::Left((outcome, _)) => break outcome.ok(),
                Either::Right(_) => {}
            }
        };
        guard.ticket = None;
        outcome
    }

    fn badge_stretches(
        canvas: (f64, f64),
        badge: (f64, f64),
        inset: f64,
        short: f64,
        start: f64,
        seconds: f64,
        asset_seconds: f64,
    ) -> Vec<BadgeStretch> {
        let margin = (PostWatermarkRendererMetrics::new(short).margin - inset).max(0.0);
        let left = margin;
        let right = canvas.0 - margin - badge.0;
        let top = margin;
        let bottom = canvas.1 - margin - badge.1;
        let schedule = PostWatermarkSchedule::new(seconds);
        (0..schedule.count)
            .map(|index| {
                let (x, y) = match schedule.corner(index) {
                    PostWatermarkScheduleCorner::TopLeft => (left, top),
                    PostWatermarkScheduleCorner::BottomRight => (right, bottom),
                    PostWatermarkScheduleCorner::TopRight => (right, top),
                    PostWatermarkScheduleCorner::BottomLeft => (left, bottom),
                };
                let local = index as f64 * schedule.span;
                let from = if index == 0 { 0.0_f64.min(start) } else { start + local };
                let local_end = if index == schedule.count - 1 { schedule.seconds } else { local + schedule.span };
                let to = if index == schedule.count - 1 {
                    (start + local_end).max(asset_seconds)
                } else {
                    start + local_end
                };
                BadgeStretch {
                    start_seconds: from,
                    end_seconds: to,
                    x: x.round(),
                    y: y.round(),
                    fade_in_seconds: if index > 0 { schedule.edge() } else { 0.0 },
                    fade_out_seconds: if index < schedule.count - 1 { schedule.edge() } else { 0.0 },
                }
            })
            .collect()
    }

    fn video_badge(label: &PostWatermarkLabel, short: f64) -> Option<(Overlay, f64)> {
        let mark = PostWatermarkRendererMarkBlock::new(label, short)?;
        let pad = mark.metrics.glow * 2.0;
        let bitmap = mark.bitmap(pad)?;
        let overlay = Overlay { width: bitmap.width(), height: bitmap.height(), rgba: Arc::from(bitmap.into_raw()) };
        Some((overlay, pad))
    }

    fn logo_png() -> Option<Vec<u8>> {
        let bytes = RiseAssets::discover().load(Self::LOGO_ASSET).ok()??;
        Some(bytes.into_owned())
    }
}

struct PostWatermarkExportGuard {
    ticket: Option<ExportTicket>,
}

impl Drop for PostWatermarkExportGuard {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            ticket.cancel();
        }
    }
}

/// riseonly logo with Riseonly over the authors tag, laid out once per frame size
/// photos and videos carry this same block
struct PostWatermarkRendererMarkBlock {
    metrics: PostWatermarkRendererMetrics,
    width: f64,
    height: f64,
    tag_line: String,
    title_height: f64,
    title_ascent: f64,
    tag_ascent: f64,
    text_height: f64,
    font_family: String,
}

impl PostWatermarkRendererMarkBlock {
    fn new(label: &PostWatermarkLabel, short: f64) -> Option<Self> {
        let metrics = PostWatermarkRendererMetrics::new(short);
        let text_system = EditorImageBake::text_system()?;
        let title_font = Self::font(FontWeight::BOLD);
        let tag_font = Self::font(FontWeight::SEMIBOLD);
        let measure = |font: &gpui::Font, size: f64, text: &str| -> (f64, f64, f64) {
            let font_id = text_system.resolve_font(font);
            let font_px = px(size as f32);
            let width = text
                .chars()
                .map(|ch| f64::from(f32::from(text_system.layout_width(font_id, font_px, ch))))
                .sum::<f64>();
            let ascent = f64::from(f32::from(text_system.ascent(font_id, font_px)));
            (width, size * f64::from(NATURAL_LINE_HEIGHT), ascent)
        };
        let (title_width, title_height, title_ascent) = measure(&title_font, metrics.line1, PostWatermarkRenderer::TITLE);
        let (tag_width, tag_height, tag_ascent) = if label.tag_line.is_empty() {
            (0.0, 0.0, 0.0)
        } else {
            measure(&tag_font, metrics.line2, &label.tag_line)
        };
        let text_height = title_height + tag_height;
        Some(Self {
            metrics,
            width: metrics.icon * 1.28 + title_width.max(tag_width),
            height: metrics.icon.max(text_height),
            tag_line: label.tag_line.clone(),
            title_height,
            title_ascent,
            tag_ascent,
            text_height,
            font_family: title_font.family.to_string(),
        })
    }

    fn font(weight: FontWeight) -> gpui::Font {
        let spec = Font::system_weight(0.0, weight);
        gpui::Font { family: spec.family(), features: spec.features(), fallbacks: None, weight, style: spec.style() }
    }

    /// block with pad of shadow room on every side, straight rgba
    /// logo and text are one group with a tight dark edge and the group casts the wide soft
    /// shadow so white reads on a light frame
    fn bitmap(&self, pad: f64) -> Option<RgbaImage> {
        let width = (self.width + pad * 2.0).ceil().max(1.0);
        let height = (self.height + pad * 2.0).ceil().max(1.0);
        let metrics = self.metrics;
        let icon_x = pad;
        let icon_y = pad + (self.height - metrics.icon) / 2.0;
        let text_x = icon_x + metrics.icon + metrics.icon * 0.28;
        let text_y = pad + (self.height - self.text_height) / 2.0;
        let family = Self::escaped(&self.font_family);
        let mut body = String::new();
        if let Some(logo) = PostWatermarkRenderer::logo_png() {
            let _ = write!(
                body,
                r#"<image x="{icon_x}" y="{icon_y}" width="{icon}" height="{icon}" preserveAspectRatio="xMidYMid meet" xlink:href="data:image/png;base64,{}"/>"#,
                base64_encoded_string(&logo),
                icon = metrics.icon,
            );
        }
        let _ = write!(
            body,
            r##"<text x="{text_x}" y="{}" font-family="{family}" font-weight="{}" font-size="{}" fill="#FFFFFF">{}</text>"##,
            text_y + self.title_ascent,
            FontWeight::BOLD.0,
            metrics.line1,
            PostWatermarkRenderer::TITLE,
        );
        if !self.tag_line.is_empty() {
            let _ = write!(
                body,
                r##"<text x="{text_x}" y="{}" xml:space="preserve" font-family="{family}" font-weight="{}" font-size="{}" fill="#FFFFFF">{}</text>"##,
                text_y + self.title_height + self.tag_ascent,
                FontWeight::SEMIBOLD.0,
                metrics.line2,
                Self::escaped(&self.tag_line),
            );
        }
        let document = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><defs><filter id="glow" filterUnits="userSpaceOnUse" x="0" y="0" width="{width}" height="{height}"><feDropShadow dx="0" dy="0" stdDeviation="{}" flood-color="#000000" flood-opacity="{}"/></filter><filter id="edge" filterUnits="userSpaceOnUse" x="0" y="0" width="{width}" height="{height}"><feDropShadow dx="0" dy="0" stdDeviation="{}" flood-color="#000000" flood-opacity="{}"/></filter></defs><g filter="url(#glow)"><g filter="url(#edge)">{body}</g></g></svg>"##,
            metrics.glow / 2.0,
            PostWatermarkRenderer::GLOW_ALPHA,
            metrics.blur / 2.0,
            PostWatermarkRenderer::SHADOW_ALPHA,
        );
        let bitmap = EditorImageBake::rasterize_svg_rgba(&document)?;
        let (target_width, target_height) = (width as u32, height as u32);
        if bitmap.width() == target_width && bitmap.height() == target_height {
            Some(bitmap)
        } else {
            Some(imageops::resize(&bitmap, target_width, target_height, FilterType::Triangle))
        }
    }

    fn escaped(text: &str) -> String {
        let mut escaped = String::with_capacity(text.len());
        for character in text.chars() {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                _ => escaped.push(character),
            }
        }
        escaped
    }
}
