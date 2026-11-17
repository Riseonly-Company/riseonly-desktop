use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rise_platform::{HostOs, PlatformSupport};
use thiserror::Error;

pub const MINIMUM_CLIP_SECONDS: f64 = 0.1;
pub const UNTOUCHED_TRIM_SECONDS: f64 = 0.05;
pub const MINIMUM_MUSIC_SECONDS: f64 = 0.05;
pub const MAXIMUM_SOURCE_FRAME_RATE: f32 = 60.0;
pub const CAPPED_FRAME_RATE: u32 = 30;
pub const DEFAULT_FRAME_RATE: u32 = 30;
pub const FALLBACK_RENDER_SIZE: FrameSize = FrameSize { width: 1280.0, height: 720.0 };
pub const ROTATION_EPSILON_DEGREES: f64 = 0.0001;
pub const CROP_EPSILON: f64 = 0.000_000_1;
pub const PROGRESS_CEILING: f32 = 0.99;
pub const PROGRESS_STEP: f32 = 0.005;
pub const PROGRESS_POLL_INTERVAL: Duration = Duration::from_millis(80);
pub const VIDEO_NOTE_CANVAS_SIDE: u32 = 640;
pub const POSTER_MAX_SIDE: u32 = 720;

/// encoder preset by the export session names, the ffmpeg arms map each to their own scale and quality
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ExportPreset {
    Size640x480,
    Size1280x720,
    Size1920x1080,
    Size3840x2160,
    MediumQuality,
    HighestQuality,
}

impl ExportPreset {
    /// preset for the video short side of a canon profile
    pub const fn for_short_side(video_max_side: u64) -> Self {
        match video_max_side {
            0..=480 => Self::Size640x480,
            481..=720 => Self::Size1280x720,
            721..=1080 => Self::Size1920x1080,
            _ => Self::Size3840x2160,
        }
    }

    /// largest frame the preset writes, long side by short side and none keeps the source size
    pub const fn bounds(self) -> Option<(u32, u32)> {
        match self {
            Self::Size640x480 => Some((640, 480)),
            Self::Size1280x720 => Some((1280, 720)),
            Self::Size1920x1080 => Some((1920, 1080)),
            Self::Size3840x2160 => Some((3840, 2160)),
            Self::MediumQuality | Self::HighestQuality => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct FrameSize {
    pub width: f64,
    pub height: f64,
}

impl FrameSize {
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    pub fn is_finite_positive(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width > 0.0 && self.height > 0.0
    }
}

/// affine transform x' = a x + c y + tx and y' = b x + d y + ty
/// in the top left y down frame of layer instructions
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Affine {
    pub const IDENTITY: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    pub const fn translation(tx: f64, ty: f64) -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx, ty }
    }

    pub const fn scale(sx: f64, sy: f64) -> Self {
        Self { a: sx, b: 0.0, c: 0.0, d: sy, tx: 0.0, ty: 0.0 }
    }

    /// turn by radians, positive is clockwise on screen because y points down
    pub fn rotation(radians: f64) -> Self {
        let (sin, cos) = radians.sin_cos();
        Self { a: cos, b: sin, c: -sin, d: cos, tx: 0.0, ty: 0.0 }
    }

    /// self first and then next
    pub fn then(self, next: Self) -> Self {
        Self {
            a: self.a * next.a + self.b * next.c,
            b: self.a * next.b + self.b * next.d,
            c: self.c * next.a + self.d * next.c,
            d: self.c * next.b + self.d * next.d,
            tx: self.tx * next.a + self.ty * next.c + next.tx,
            ty: self.tx * next.b + self.ty * next.d + next.ty,
        }
    }

    /// the transform that undoes self, none when it collapses the plane
    pub fn inverted(self) -> Option<Self> {
        let determinant = self.a * self.d - self.b * self.c;
        if !determinant.is_finite() || determinant.abs() < f64::EPSILON {
            return None;
        }
        let (a, b, c, d) = (self.d / determinant, -self.b / determinant, -self.c / determinant, self.a / determinant);
        Some(Self { a, b, c, d, tx: -(self.tx * a + self.ty * c), ty: -(self.tx * b + self.ty * d) })
    }

    pub fn apply(self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }

    /// bounding box of the rect from 0 0 to size after the transform
    /// gives min x and min y and the size
    pub fn bounds_of(self, size: FrameSize) -> (f64, f64, FrameSize) {
        let corners = [
            self.apply(0.0, 0.0),
            self.apply(size.width, 0.0),
            self.apply(0.0, size.height),
            self.apply(size.width, size.height),
        ];
        let min_x = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let max_x = corners.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        let min_y = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let max_y = corners.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        (min_x, min_y, FrameSize::new(max_x - min_x, max_y - min_y))
    }
}

/// what a file holds, read before a job is built
/// duration and natural size and transform and frame rate
#[derive(Clone, PartialEq, Debug)]
pub struct VideoProbe {
    pub duration_seconds: f64,
    pub video_start_seconds: f64,
    pub video_duration_seconds: f64,
    pub natural_size: FrameSize,
    pub preferred_transform: Affine,
    pub nominal_frame_rate: f32,
    pub has_video: bool,
    pub has_audio: bool,
    pub byte_count: u64,
}

impl VideoProbe {
    pub fn duration_ms(&self) -> i64 {
        if self.duration_seconds.is_finite() && self.duration_seconds > 0.0 {
            (self.duration_seconds * 1000.0) as i64
        } else {
            0
        }
    }

    /// frame as displayed, natural size through the preferred transform or the stored size
    /// or 1280x720 if neither is usable
    pub fn display_size(&self) -> FrameSize {
        let (_, _, turned) = self.preferred_transform.bounds_of(self.natural_size);
        if turned.is_finite_positive() {
            turned
        } else if self.natural_size.is_finite_positive() {
            self.natural_size
        } else {
            FALLBACK_RENDER_SIZE
        }
    }

    /// preferred transform moved so the displayed frame starts at the origin
    pub fn upright(&self) -> Affine {
        let (min_x, min_y, _) = self.preferred_transform.bounds_of(self.natural_size);
        if min_x.is_finite() && min_y.is_finite() {
            self.preferred_transform.then(Affine::translation(-min_x, -min_y))
        } else {
            self.preferred_transform
        }
    }

    /// true when presets need a capped frame rate for this source, 240 fps slow motion fails otherwise
    pub fn needs_frame_rate_cap(&self) -> bool {
        self.nominal_frame_rate.is_finite() && self.nominal_frame_rate > MAXIMUM_SOURCE_FRAME_RATE
    }
}

/// crop in 0..1 of the displayed frame, origin top left
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RelativeRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl RelativeRect {
    pub fn is_effectively_full(self) -> bool {
        self.x <= CROP_EPSILON
            && self.y <= CROP_EPSILON
            && self.width >= 1.0 - CROP_EPSILON
            && self.height >= 1.0 - CROP_EPSILON
    }

    /// standardized and cut to the unit square, none when nothing is left
    pub fn clamped(self) -> Option<Self> {
        let (x0, x1) = if self.width < 0.0 { (self.x + self.width, self.x) } else { (self.x, self.x + self.width) };
        let (y0, y1) = if self.height < 0.0 { (self.y + self.height, self.y) } else { (self.y, self.y + self.height) };
        let (x0, x1, y0, y1) = (x0.max(0.0), x1.min(1.0), y0.max(0.0), y1.min(1.0));
        let all_finite = [x0, x1, y0, y1].iter().all(|v| v.is_finite());
        (all_finite && x1 > x0 && y1 > y0).then_some(Self { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })
    }
}

/// spatial edit of a video from the editor, without colour adjustments (those go in the render plan)
/// and without strokes and texts (those come baked into the overlay)
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct FrameGeometry {
    pub rotation_steps: i32,
    pub mirrored: bool,
    pub free_rotation_degrees: f64,
    pub crop: Option<RelativeRect>,
}

impl FrameGeometry {
    fn quarter_turns(self) -> i32 {
        self.rotation_steps.rem_euclid(4)
    }

    fn has_free_rotation(self) -> bool {
        self.free_rotation_degrees.abs() > ROTATION_EPSILON_DEGREES
    }

    fn effective_crop(self) -> Option<RelativeRect> {
        self.crop.filter(|crop| !crop.is_effectively_full()).and_then(RelativeRect::clamped)
    }

    /// true when theres no turn or mirror or free rotation or crop
    pub fn is_identity(self) -> bool {
        self.quarter_turns() == 0 && !self.mirrored && !self.has_free_rotation() && self.effective_crop().is_none()
    }
}

pub const NEUTRAL_KELVIN: f64 = 6500.0;
pub const KELVIN_PER_TEMPERATURE: f64 = 2200.0;
pub const VIGNETTE_RADIUS: f64 = 1.5;

/// colour adjustments from the editor, sliders in -1..1 and enhance and vignette in 0..1
/// every source frame goes through them before the geometry, the methods give the filter parameters
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ColorAdjustments {
    pub enhance: f64,
    pub brightness: f64,
    pub contrast: f64,
    pub saturation: f64,
    pub temperature: f64,
    pub highlights: f64,
    pub shadows: f64,
    pub vignette: f64,
}

/// color controls, brightness is added and contrast and saturation are factors around 1
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColorControls {
    pub brightness: f64,
    pub contrast: f64,
    pub saturation: f64,
}

/// highlight amount around 1 and shadow amount around 0
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HighlightShadow {
    pub highlight_amount: f64,
    pub shadow_amount: f64,
}

impl ColorAdjustments {
    pub const IDENTITY: Self = Self {
        enhance: 0.0,
        brightness: 0.0,
        contrast: 0.0,
        saturation: 0.0,
        temperature: 0.0,
        highlights: 0.0,
        shadows: 0.0,
        vignette: 0.0,
    };

    fn values(self) -> [f64; 8] {
        [
            self.enhance,
            self.brightness,
            self.contrast,
            self.saturation,
            self.temperature,
            self.highlights,
            self.shadows,
            self.vignette,
        ]
    }

    /// true when every slider is at zero, frames pass untouched
    pub fn is_identity(self) -> bool {
        self.values().iter().all(|value| *value == 0.0)
    }

    pub fn is_finite(self) -> bool {
        self.values().iter().all(|value| value.is_finite())
    }

    /// how far the auto enhanced frame is dissolved in, 0..1 and none without enhance
    pub fn enhance_fraction(self) -> Option<f64> {
        (self.enhance > 0.0).then(|| self.enhance.clamp(0.0, 1.0))
    }

    pub fn color_controls(self) -> Option<ColorControls> {
        (self.brightness != 0.0 || self.contrast != 0.0 || self.saturation != 0.0).then(|| ColorControls {
            brightness: self.brightness * 0.5,
            contrast: 1.0 + self.contrast * 0.6,
            saturation: 1.0 + self.saturation * 0.7,
        })
    }

    /// the white the neutral kelvin is moved to, a positive slider lowers it (warmer)
    pub fn target_neutral_kelvin(self) -> Option<f64> {
        (self.temperature != 0.0).then(|| NEUTRAL_KELVIN - self.temperature * KELVIN_PER_TEMPERATURE)
    }

    pub fn highlight_shadow(self) -> Option<HighlightShadow> {
        (self.highlights != 0.0 || self.shadows != 0.0).then(|| HighlightShadow {
            highlight_amount: 1.0 + self.highlights * 0.6,
            shadow_amount: self.shadows * 0.6,
        })
    }

    /// vignette input intensity, none without a vignette
    pub fn vignette_intensity(self) -> Option<f64> {
        (self.vignette > 0.0).then(|| self.vignette * 1.5)
    }
}

fn even_dimension(value: f64) -> u32 {
    if !value.is_finite() || value < 2.0 {
        return 2;
    }
    let integer = value.floor().min(f64::from(u32::MAX)) as u32;
    (integer - integer % 2).max(2)
}

/// frame the editor export renders at, displayed size turned and rotated and cropped
/// then scaled down to maximum_side with both sides even
pub fn render_size(probe: &VideoProbe, geometry: FrameGeometry, maximum_side: f64) -> (u32, u32) {
    let mut size = probe.display_size();
    if geometry.quarter_turns() % 2 != 0 {
        size = FrameSize::new(size.height, size.width);
    }
    if geometry.has_free_rotation() {
        let radians = geometry.free_rotation_degrees.to_radians();
        let (cosine, sine) = (radians.cos().abs(), radians.sin().abs());
        size = FrameSize::new(
            size.width * cosine + size.height * sine,
            size.width * sine + size.height * cosine,
        );
    }
    if let Some(crop) = geometry.effective_crop() {
        size = FrameSize::new(size.width * crop.width, size.height * crop.height);
    }
    let long_side = size.width.max(size.height);
    if long_side.is_finite() && maximum_side > 0.0 && long_side > maximum_side {
        let scale = maximum_side / long_side;
        size = FrameSize::new(size.width * scale, size.height * scale);
    }
    (even_dimension(size.width), even_dimension(size.height))
}

/// layer transform that draws a stored frame into the render with the edit applied
/// upright, quarter turns and mirror and free rotation and crop and then scaled to fill and centred
pub fn segment_transform(probe: &VideoProbe, geometry: FrameGeometry, render: (u32, u32)) -> Affine {
    let mut transform = probe.upright();
    let mut size = probe.display_size();
    for _ in 0..geometry.quarter_turns() {
        transform = transform.then(Affine { a: 0.0, b: 1.0, c: -1.0, d: 0.0, tx: size.height, ty: 0.0 });
        size = FrameSize::new(size.height, size.width);
    }
    if geometry.mirrored {
        transform = transform.then(Affine { a: -1.0, b: 0.0, c: 0.0, d: 1.0, tx: size.width, ty: 0.0 });
    }
    if geometry.has_free_rotation() {
        let (cx, cy) = (size.width / 2.0, size.height / 2.0);
        let turn = Affine::translation(-cx, -cy)
            .then(Affine::rotation(geometry.free_rotation_degrees.to_radians()))
            .then(Affine::translation(cx, cy));
        let (min_x, min_y, turned) = turn.bounds_of(size);
        transform = transform.then(turn).then(Affine::translation(-min_x, -min_y));
        size = turned;
    }
    if let Some(crop) = geometry.effective_crop() {
        transform = transform.then(Affine::translation(-crop.x * size.width, -crop.y * size.height));
        size = FrameSize::new(crop.width * size.width, crop.height * size.height);
    }
    let (render_width, render_height) = (f64::from(render.0), f64::from(render.1));
    if !size.is_finite_positive() {
        return transform;
    }
    let scale = (render_width / size.width).max(render_height / size.height);
    if !scale.is_finite() || scale <= 0.0 {
        return transform;
    }
    transform
        .then(Affine::scale(scale, scale))
        .then(Affine::translation(
            (render_width - size.width * scale) / 2.0,
            (render_height - size.height * scale) / 2.0,
        ))
}

/// frames per second a composition renders at, rounded source rate
/// 30 when it has none and capped to 30 above 60
pub fn render_frame_rate(nominal: f32) -> u32 {
    if !nominal.is_finite() || nominal <= 0.0 {
        return DEFAULT_FRAME_RATE;
    }
    if nominal > MAXIMUM_SOURCE_FRAME_RATE {
        return CAPPED_FRAME_RATE;
    }
    (nominal.round() as u32).max(1)
}

/// span of a source timeline in seconds
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TimeSpan {
    pub start_seconds: f64,
    pub duration_seconds: f64,
}

impl TimeSpan {
    pub fn end_seconds(self) -> f64 {
        self.start_seconds + self.duration_seconds
    }

    fn is_valid(self) -> bool {
        self.start_seconds.is_finite()
            && self.duration_seconds.is_finite()
            && self.start_seconds >= 0.0
            && self.duration_seconds > 0.0
    }
}

/// a trim clamped into the clip
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Trim {
    pub span: TimeSpan,
    pub cuts: bool,
}

/// clamps a start and end into a clip of duration_seconds, keeping at least the minimum clip length
/// none for a clip with no usable duration
pub fn clamp_trim(duration_seconds: f64, start_seconds: f64, end_seconds: f64) -> Option<Trim> {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return None;
    }
    let minimum = MINIMUM_CLIP_SECONDS.min(duration_seconds);
    let start = start_seconds.min(duration_seconds - minimum).max(0.0);
    let end = (start + minimum).max(end_seconds.min(duration_seconds)).min(duration_seconds);
    if !(start.is_finite() && end.is_finite()) || end <= start {
        return None;
    }
    Some(Trim {
        span: TimeSpan { start_seconds: start, duration_seconds: end - start },
        cuts: start > UNTOUCHED_TRIM_SECONDS || end < duration_seconds - UNTOUCHED_TRIM_SECONDS,
    })
}

/// where a music piece lands in the trimmed window, the chosen segment shifted by its offset
/// in source time and cut to the window, none when less than the minimum would play
pub fn music_placement(
    trim: TimeSpan,
    music_offset_seconds: f64,
    segment_start_ms: i64,
    segment_end_ms: i64,
    music_duration_seconds: f64,
) -> Option<AudioPlacement> {
    if segment_end_ms <= segment_start_ms || !music_duration_seconds.is_finite() {
        return None;
    }
    let segment_start = segment_start_ms.max(0) as f64 / 1000.0;
    let segment_length = (segment_end_ms - segment_start_ms).max(0) as f64 / 1000.0;
    let available = music_duration_seconds - segment_start;
    let offset_in_window = music_offset_seconds - trim.start_seconds;
    let lead = offset_in_window.max(0.0);
    let skipped = (-offset_in_window).max(0.0);
    let room = trim.duration_seconds - lead;
    let usable = (segment_length.min(available).min(trim.duration_seconds) - skipped).min(room);
    if !usable.is_finite() || usable <= MINIMUM_MUSIC_SECONDS || room <= MINIMUM_MUSIC_SECONDS {
        return None;
    }
    Some(AudioPlacement {
        span: TimeSpan { start_seconds: segment_start + skipped, duration_seconds: usable },
        at_seconds: lead,
    })
}

/// a piece of a source placed on the output timeline
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AudioPlacement {
    pub span: TimeSpan,
    pub at_seconds: f64,
}

/// where video note segments start on the shared timeline
/// first at zero, each next at its recorded offset but never before the previous one ends
pub fn segment_starts(segments: &[(f64, f64)]) -> Vec<f64> {
    let mut starts = Vec::with_capacity(segments.len());
    let mut cursor = 0.0_f64;
    for (index, (offset, duration)) in segments.iter().enumerate() {
        let start = if index == 0 { 0.0 } else { offset.max(0.0).max(cursor) };
        starts.push(start);
        cursor = start + duration.max(0.0);
    }
    starts
}

/// full frame picture composited over every rendered frame, strokes and texts or the note ring and logo
/// straight alpha rgba, rows packed and top row first
#[derive(Clone, PartialEq, Debug)]
pub struct Overlay {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

/// how frames are rendered when the job changes the picture
/// without one source frames go through the preset as they are
#[derive(Clone, PartialEq, Debug)]
pub struct RenderPlan {
    pub size: (u32, u32),
    pub frame_rate: u32,
    pub adjustments: ColorAdjustments,
    pub overlay: Option<Overlay>,
    pub loop_layers: Vec<LoopLayer>,
    pub badge: Option<MovingBadge>,
}

impl RenderPlan {
    /// true when something in the plan changes per frame or per pixel
    /// so every frame goes through a per frame compositor, not a still layer over the source
    pub fn draws_every_frame(&self) -> bool {
        !self.adjustments.is_identity() || !self.loop_layers.is_empty() || self.badge.is_some()
    }
}

/// one picture of a looping layer, straight alpha rgba packed rows top first
/// plus the placement from its pixels into the render
#[derive(Clone, PartialEq, Debug)]
pub struct LoopFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub placement: Affine,
}

/// looping sticker, frames shown one after another for frame_duration seconds each from the
/// start, round and round
#[derive(Clone, PartialEq, Debug)]
pub struct LoopLayer {
    pub frames: Vec<LoopFrame>,
    pub frame_duration: f64,
}

impl LoopLayer {
    /// frame shown at an output time, time wraps over the loop and a negative one counts from the end
    /// a time that isnt finite shows the first
    pub fn frame_index_at(&self, at_seconds: f64) -> Option<usize> {
        loop_frame_index(self.frames.len(), self.frame_duration, at_seconds)
    }

    pub fn frame_at(&self, at_seconds: f64) -> Option<&LoopFrame> {
        self.frame_index_at(at_seconds).and_then(|index| self.frames.get(index))
    }
}

/// same as the loop layer one for a branch that keeps only the count and timing
pub fn loop_frame_index(count: usize, frame_duration: f64, at_seconds: f64) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let loop_seconds = frame_duration * count as f64;
    if count == 1 || !at_seconds.is_finite() || !loop_seconds.is_finite() || loop_seconds <= 0.0 {
        return Some(0);
    }
    let local = at_seconds.rem_euclid(loop_seconds);
    let index = (local / frame_duration).floor();
    Some(if index.is_finite() && index > 0.0 { (index as usize).min(count - 1) } else { 0 })
}

/// watermark bitmap and the stretches it stands in, one corner each
#[derive(Clone, PartialEq, Debug)]
pub struct MovingBadge {
    pub image: Overlay,
    pub stretches: Vec<BadgeStretch>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BadgeStretch {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub x: f64,
    pub y: f64,
    pub fade_in_seconds: f64,
    pub fade_out_seconds: f64,
}

/// where the badge is drawn at a moment and how opaque
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BadgeSpot {
    pub x: f64,
    pub y: f64,
    pub opacity: f64,
}

impl MovingBadge {
    /// badge at an output time, the stretch holding the time
    /// opacity rises linearly over the fade in and falls over the fade out
    /// none outside every stretch or fully faded
    pub fn spot_at(&self, at_seconds: f64) -> Option<BadgeSpot> {
        if !at_seconds.is_finite() {
            return None;
        }
        let stretch = self
            .stretches
            .iter()
            .find(|stretch| stretch.start_seconds <= at_seconds && at_seconds < stretch.end_seconds)?;
        let mut opacity = 1.0_f64;
        if stretch.fade_in_seconds > 0.0 {
            opacity = opacity.min((at_seconds - stretch.start_seconds) / stretch.fade_in_seconds);
        }
        if stretch.fade_out_seconds > 0.0 {
            opacity = opacity.min((stretch.end_seconds - at_seconds) / stretch.fade_out_seconds);
        }
        let opacity = opacity.clamp(0.0, 1.0);
        (opacity > 0.0).then_some(BadgeSpot { x: stretch.x, y: stretch.y, opacity })
    }

    fn is_valid(&self) -> bool {
        let image_ok = self.image.width > 0
            && self.image.height > 0
            && self.image.rgba.len() as u64 == u64::from(self.image.width) * u64::from(self.image.height) * 4;
        let stretches_ok = self.stretches.iter().all(|stretch| {
            [stretch.start_seconds, stretch.end_seconds, stretch.x, stretch.y, stretch.fade_in_seconds, stretch.fade_out_seconds]
                .iter()
                .all(|value| value.is_finite())
                && stretch.end_seconds > stretch.start_seconds
                && stretch.fade_in_seconds >= 0.0
                && stretch.fade_out_seconds >= 0.0
        });
        image_ok && stretches_ok
    }
}

fn loop_layer_is_valid(layer: &LoopLayer) -> bool {
    !layer.frames.is_empty()
        && layer.frame_duration.is_finite()
        && layer.frame_duration > 0.0
        && layer.frames.iter().all(|frame| {
            let placement = frame.placement;
            frame.width > 0
                && frame.height > 0
                && frame.rgba.len() as u64 == u64::from(frame.width) * u64::from(frame.height) * 4
                && [placement.a, placement.b, placement.c, placement.d, placement.tx, placement.ty]
                    .iter()
                    .all(|value| value.is_finite())
        })
}

/// colour tags a per frame render writes so source colours dont shift
/// none means the source has no such tag
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ColorTags {
    pub primaries: &'static str,
    pub transfer: &'static str,
    pub matrix: &'static str,
}

pub const TAG_ITU_R_709_2: &str = "ITU_R_709_2";
pub const TAG_ITU_R_601_4: &str = "ITU_R_601_4";
pub const TAG_SMPTE_C: &str = "SMPTE_C";
pub const TAG_EBU_3213: &str = "EBU_3213";
const STANDARD_DEFINITION_SIDE: u32 = 576;

impl ColorTags {
    pub fn of_source(primaries: Option<&str>, transfer: Option<&str>, matrix: Option<&str>, render: (u32, u32)) -> Self {
        let bt709 = Self { primaries: TAG_ITU_R_709_2, transfer: TAG_ITU_R_709_2, matrix: TAG_ITU_R_709_2 };
        if transfer.is_some_and(|transfer| transfer != TAG_ITU_R_709_2) {
            return bt709;
        }
        let matrix = match matrix {
            Some(TAG_ITU_R_709_2) => TAG_ITU_R_709_2,
            Some(_) => TAG_ITU_R_601_4,
            None if render.0.min(render.1) <= STANDARD_DEFINITION_SIDE => TAG_ITU_R_601_4,
            None => TAG_ITU_R_709_2,
        };
        let primaries = match primaries.unwrap_or(TAG_ITU_R_709_2) {
            TAG_ITU_R_709_2 if matrix == TAG_ITU_R_601_4 => TAG_SMPTE_C,
            TAG_SMPTE_C => TAG_SMPTE_C,
            TAG_EBU_3213 => TAG_EBU_3213,
            _ => TAG_ITU_R_709_2,
        };
        Self { primaries, transfer: TAG_ITU_R_709_2, matrix }
    }
}

/// one video source on the output timeline
#[derive(Clone, PartialEq, Debug)]
pub struct VideoSegment {
    pub source: PathBuf,
    pub span: Option<TimeSpan>,
    pub at_seconds: Option<f64>,
    pub transform: Affine,
    pub volume: Option<f32>,
}

/// a sound file mixed in, music or the continuous microphone take of a video note
#[derive(Clone, PartialEq, Debug)]
pub struct AudioLayer {
    pub source: PathBuf,
    pub placement: Option<AudioPlacement>,
    pub volume: f32,
}

/// one export, sources on a timeline and an optional render and the preset and the output
#[derive(Clone, PartialEq, Debug)]
pub struct ExportJob {
    pub segments: Vec<VideoSegment>,
    pub audio: Vec<AudioLayer>,
    pub render: Option<RenderPlan>,
    pub preset: ExportPreset,
    pub target: PathBuf,
    pub file_length_limit: Option<u64>,
    pub cap_frame_rate: bool,
}

impl ExportJob {
    /// plain transcode of one clip
    /// a preset and an optional trim and volume and the size ceiling
    pub fn transcode(
        source: &Path,
        probe: &VideoProbe,
        preset: ExportPreset,
        target: &Path,
        trim: Option<TimeSpan>,
        file_length_limit: Option<u64>,
    ) -> Self {
        Self {
            segments: vec![VideoSegment {
                source: source.to_path_buf(),
                span: trim,
                at_seconds: Some(0.0),
                transform: probe.upright(),
                volume: probe.has_audio.then_some(1.0),
            }],
            audio: Vec::new(),
            render: None,
            preset,
            target: target.to_path_buf(),
            file_length_limit,
            cap_frame_rate: probe.needs_frame_rate_cap(),
        }
    }

    /// true when the single source asset can be exported directly
    /// one segment from the timeline start and no other sound
    pub fn is_single_source(&self) -> bool {
        self.segments.len() == 1
            && self.audio.is_empty()
            && self.segments[0].at_seconds.is_none_or(|at| at == 0.0)
    }

    pub fn validate(&self) -> Result<(), VideoExportError> {
        if self.segments.is_empty() {
            return Err(VideoExportError::InvalidJob("a job needs at least one video segment"));
        }
        let volume_ok = |volume: f32| volume.is_finite() && (0.0..=1.0).contains(&volume);
        for segment in &self.segments {
            if segment.span.is_some_and(|span| !span.is_valid()) {
                return Err(VideoExportError::InvalidJob("a segment span is empty or not finite"));
            }
            if segment.at_seconds.is_some_and(|at| !at.is_finite() || at < 0.0) {
                return Err(VideoExportError::InvalidJob("a segment starts before the timeline"));
            }
            if segment.volume.is_some_and(|volume| !volume_ok(volume)) {
                return Err(VideoExportError::InvalidJob("a volume is outside 0..1"));
            }
        }
        for layer in &self.audio {
            if !volume_ok(layer.volume) {
                return Err(VideoExportError::InvalidJob("a volume is outside 0..1"));
            }
            if layer.placement.is_some_and(|p| !p.span.is_valid() || !p.at_seconds.is_finite() || p.at_seconds < 0.0) {
                return Err(VideoExportError::InvalidJob("an audio placement is empty or not finite"));
            }
        }
        if let Some(render) = &self.render {
            if render.size.0 < 2 || render.size.1 < 2 || render.size.0 % 2 != 0 || render.size.1 % 2 != 0 {
                return Err(VideoExportError::InvalidJob("the render size must be even and at least 2"));
            }
            if render.frame_rate == 0 {
                return Err(VideoExportError::InvalidJob("the render frame rate is zero"));
            }
            if !render.adjustments.is_finite() {
                return Err(VideoExportError::InvalidJob("a colour adjustment is not finite"));
            }
            if let Some(overlay) = &render.overlay {
                let expected = u64::from(overlay.width) * u64::from(overlay.height) * 4;
                if (overlay.width, overlay.height) != render.size || overlay.rgba.len() as u64 != expected {
                    return Err(VideoExportError::InvalidJob("the overlay is not one RGBA frame of the render size"));
                }
            }
            if !render.loop_layers.iter().all(loop_layer_is_valid) {
                return Err(VideoExportError::InvalidJob("a loop layer is empty, untimed or not RGBA frames"));
            }
            if render.badge.as_ref().is_some_and(|badge| !badge.is_valid()) {
                return Err(VideoExportError::InvalidJob("the badge is not RGBA or a stretch is empty or not finite"));
            }
        }
        if self.file_length_limit == Some(0) {
            return Err(VideoExportError::InvalidJob("a zero byte ceiling writes nothing"));
        }
        Ok(())
    }
}

/// how an export ended
#[derive(Clone, PartialEq, Debug)]
pub enum ExportOutcome {
    Completed { path: PathBuf, byte_count: u64 },
    Cancelled,
    Failed(VideoExportError),
}

pub type ExportSink = Box<dyn FnOnce(ExportOutcome) + Send>;

/// the running export as the os layer exposes it
pub trait ExportControl: Send + Sync {
    /// progress of the encoder, 0..1
    fn progress(&self) -> f32;
    /// asks the encoder to stop, the sink then gets cancelled
    fn cancel(&self);
}

/// a started export, poll its progress or cancel it
/// the outcome arrives once through the sink
#[derive(Clone)]
pub struct ExportTicket {
    control: Arc<dyn ExportControl>,
}

impl ExportTicket {
    pub fn new(control: Arc<dyn ExportControl>) -> Self {
        Self { control }
    }

    /// progress clamped to the ceiling, only the outcome says done
    pub fn progress(&self) -> f32 {
        let raw = self.control.progress();
        if raw.is_finite() { raw.clamp(0.0, PROGRESS_CEILING) } else { 0.0 }
    }

    pub fn cancel(&self) {
        self.control.cancel();
    }
}

/// reports progress only after it moved by the step
#[derive(Clone, Copy, Debug)]
pub struct ProgressThrottle {
    last: f32,
}

impl Default for ProgressThrottle {
    fn default() -> Self {
        Self { last: -1.0 }
    }
}

impl ProgressThrottle {
    pub fn next(&mut self, progress: f32) -> Option<f32> {
        if progress - self.last >= PROGRESS_STEP {
            self.last = progress;
            Some(progress)
        } else {
            None
        }
    }
}

/// one frame of a video, bgra rows packed and top first and opaque
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PosterFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub const STILL_VIDEO_TIMESCALE: i32 = 600;
pub const STILL_VIDEO_TAIL_SECONDS: f64 = 1.0 / 30.0;

/// one picture held as an h264 high mpeg4 for duration_seconds, used under a story music
/// frame at zero and again at duration - 1/30, session ends at the duration
#[derive(Clone, PartialEq, Debug)]
pub struct StillVideoJob {
    pub frame: PosterFrame,
    pub duration_seconds: f64,
    pub average_bit_rate: u32,
    pub target: PathBuf,
}

/// presentation times of a still video in timescale ticks
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StillVideoTicks {
    pub repeat: Option<i64>,
    pub end: i64,
}

fn still_ticks(seconds: f64) -> i64 {
    (seconds * f64::from(STILL_VIDEO_TIMESCALE)).round() as i64
}

impl StillVideoJob {
    pub fn validate(&self) -> Result<(), VideoExportError> {
        let PosterFrame { width, height, bgra } = &self.frame;
        if *width < 2 || *height < 2 || width % 2 != 0 || height % 2 != 0 {
            return Err(VideoExportError::InvalidJob("a still video needs even sides of at least 2"));
        }
        let expected = u64::from(*width) * u64::from(*height) * 4;
        if bgra.len() as u64 != expected {
            return Err(VideoExportError::InvalidJob("the still is not one BGRA frame of its size"));
        }
        if !self.duration_seconds.is_finite() || self.duration_seconds <= 0.0 || still_ticks(self.duration_seconds) < 1 {
            return Err(VideoExportError::InvalidJob("a still video needs a finite duration above zero"));
        }
        if self.average_bit_rate == 0 {
            return Err(VideoExportError::InvalidJob("a still video needs a bit rate"));
        }
        Ok(())
    }

    /// the repeat and the end of a validated job
    pub fn ticks(&self) -> StillVideoTicks {
        let end = still_ticks(self.duration_seconds);
        let repeat = still_ticks((self.duration_seconds - STILL_VIDEO_TAIL_SECONDS).max(0.0));
        StillVideoTicks { repeat: (repeat > 0 && repeat < end).then_some(repeat), end }
    }
}

/// frame by frame h264 mpeg4, one high track at the asked average rate and not real time
/// frame index shows at index / frames_per_second
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FrameWriterSpec {
    pub target: PathBuf,
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub average_bit_rate: u32,
}

impl FrameWriterSpec {
    pub fn validate(&self) -> Result<(), VideoExportError> {
        if self.width < 2 || self.height < 2 || self.width % 2 != 0 || self.height % 2 != 0 {
            return Err(VideoExportError::InvalidJob("a frame video needs even sides of at least 2"));
        }
        if self.frames_per_second == 0 || i32::try_from(self.frames_per_second).is_err() {
            return Err(VideoExportError::InvalidJob("a frame video needs a frame rate"));
        }
        if self.average_bit_rate == 0 {
            return Err(VideoExportError::InvalidJob("a frame video needs a bit rate"));
        }
        Ok(())
    }

    /// bytes of one packed bgra frame of the spec size
    pub fn frame_bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

/// straight alpha rgba rows as the premultiplied bgra a 32bgra buffer holds
/// the encoder drops alpha so a translucent pixel lands as if over black
pub fn premultiplied_bgra(rgba: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        let scale = |value: u8| ((u16::from(value) * alpha + 127) / 255) as u8;
        out.extend_from_slice(&[scale(pixel[2]), scale(pixel[1]), scale(pixel[0]), pixel[3]]);
    }
}

/// presentation times only grow, first frame at zero or later and every next one after the last
pub fn frame_index_follows(last: Option<i64>, index: i64) -> bool {
    index >= 0 && last.is_none_or(|last| index > last)
}

/// os writer behind the frame video writer
/// dropped before finish it cancels the writing and leaves the target to the caller
pub trait FrameSink: Send {
    fn append_bgra(&mut self, bgra: &[u8], index: i64) -> Result<(), VideoExportError>;

    /// ends the session at duration_seconds and waits until the file is written
    fn finish(self: Box<Self>, duration_seconds: f64) -> Result<(), VideoExportError>;
}

/// writes rgba frames as an h264 mpeg4
/// every call blocks (encoder back pressure, final write) so run it off the main thread
/// dropped without finish the writing is cancelled
pub struct FrameVideoWriter {
    spec: FrameWriterSpec,
    sink: Box<dyn FrameSink>,
    last_index: Option<i64>,
    bgra: Vec<u8>,
}

impl FrameVideoWriter {
    pub fn create(
        path: &Path,
        width: u32,
        height: u32,
        frames_per_second: u32,
        average_bit_rate: u32,
    ) -> Result<Self, VideoExportError> {
        let spec = FrameWriterSpec { target: path.to_path_buf(), width, height, frames_per_second, average_bit_rate };
        spec.validate()?;
        let sink = super::os::NativeVideoExport.frame_writer(&spec)?;
        Ok(Self { bgra: Vec::with_capacity(spec.frame_bytes()), spec, sink, last_index: None })
    }

    /// encodes rgba as frame index, shown at index / frames_per_second and blocks while the encoder is busy
    pub fn append(&mut self, rgba: &image::RgbaImage, index: i64) -> Result<(), VideoExportError> {
        if rgba.width() != self.spec.width || rgba.height() != self.spec.height {
            return Err(VideoExportError::InvalidJob("the frame is not the size the writer was opened with"));
        }
        if !frame_index_follows(self.last_index, index) {
            return Err(VideoExportError::InvalidJob("a frame index must grow from zero"));
        }
        premultiplied_bgra(rgba.as_raw(), &mut self.bgra);
        self.sink.append_bgra(&self.bgra, index)?;
        self.last_index = Some(index);
        Ok(())
    }

    /// ends the video at duration_seconds and waits for the file
    pub fn finish(self, duration_seconds: f64) -> Result<(), VideoExportError> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err(VideoExportError::InvalidJob("a frame video needs a finite duration above zero"));
        }
        if self.last_index.is_none() {
            return Err(VideoExportError::InvalidJob("a frame video needs at least one frame"));
        }
        self.sink.finish(duration_seconds)
    }
}

#[derive(Clone, PartialEq, Debug, Error)]
pub enum VideoExportError {
    #[error("video export is not written for {0:?} yet")]
    BranchNotWritten(HostOs),
    #[error("no video encoder: {0}")]
    EncoderUnavailable(String),
    #[error("the export job is rejected: {0}")]
    InvalidJob(&'static str),
    #[error("{0} cannot be read: {1}")]
    SourceUnreadable(PathBuf, String),
    #[error("{0} has no video track")]
    NoVideoTrack(PathBuf),
    #[error("the {0:?} preset does not apply to this source")]
    PresetUnavailable(ExportPreset),
    #[error("this source cannot be written as MPEG-4")]
    Mp4Unsupported,
    #[error("the composition could not be built: {0}")]
    CompositionFailed(String),
    #[error("the output file failed: {0}")]
    TargetFailed(String),
    #[error("the encoder failed: {0}")]
    ExportFailed(String),
    #[error("no frame at that time: {0}")]
    PosterFailed(String),
    #[error("the remote video cannot be read: {0}")]
    RemoteUnreadable(String),
}

pub trait VideoExportSource {
    fn support(&self) -> Result<(), VideoExportError> {
        Err(VideoExportError::BranchNotWritten(HostOs::current()))
    }

    fn probe(&self, _source: &Path) -> Result<VideoProbe, VideoExportError> {
        Err(VideoExportError::BranchNotWritten(HostOs::current()))
    }

    /// displayed frame at at_seconds turned upright, no side longer than max_side
    fn poster(&self, _source: &Path, _at_seconds: f64, _max_side: u32) -> Result<PosterFrame, VideoExportError> {
        Err(VideoExportError::BranchNotWritten(HostOs::current()))
    }

    fn filmstrip_frame(&self, source: &Path, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
        self.poster(source, at_seconds, max_side)
    }

    fn remote_poster(&self, _url: &str, _at_seconds: f64, _max_side: u32) -> Result<PosterFrame, VideoExportError> {
        Err(VideoExportError::BranchNotWritten(HostOs::current()))
    }

    fn start_export(&self, _job: ExportJob, _done: ExportSink) -> Result<ExportTicket, VideoExportError> {
        Err(VideoExportError::BranchNotWritten(HostOs::current()))
    }

    fn start_still_video(&self, _job: StillVideoJob, _done: ExportSink) -> Result<ExportTicket, VideoExportError> {
        Err(VideoExportError::BranchNotWritten(HostOs::current()))
    }

    fn frame_writer(&self, _spec: &FrameWriterSpec) -> Result<Box<dyn FrameSink>, VideoExportError> {
        Err(VideoExportError::BranchNotWritten(HostOs::current()))
    }
}

pub fn export_support() -> PlatformSupport {
    match super::os::NativeVideoExport.support() {
        Ok(()) => PlatformSupport::Performed,
        Err(_) => PlatformSupport::Unsupported,
    }
}

pub fn export_unavailable_reason() -> Option<VideoExportError> {
    super::os::NativeVideoExport.support().err()
}

pub fn probe(source: &Path) -> Result<VideoProbe, VideoExportError> {
    super::os::NativeVideoExport.probe(source)
}

/// poster shown in the bubble, upright and at most the poster max side unless asked otherwise
pub fn poster(source: &Path, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
    if !at_seconds.is_finite() || at_seconds < 0.0 || max_side == 0 {
        return Err(VideoExportError::InvalidJob("a poster needs a time at or after zero and a side"));
    }
    super::os::NativeVideoExport.poster(source, at_seconds, max_side)
}

pub fn filmstrip_frame(source: &Path, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
    if !at_seconds.is_finite() || at_seconds < 0.0 || max_side == 0 {
        return Err(VideoExportError::InvalidJob("a poster needs a time at or after zero and a side"));
    }
    super::os::NativeVideoExport.filmstrip_frame(source, at_seconds, max_side)
}

pub fn remote_poster(url: &str, at_seconds: f64, max_side: u32) -> Result<PosterFrame, VideoExportError> {
    if !at_seconds.is_finite() || at_seconds < 0.0 || max_side == 0 {
        return Err(VideoExportError::InvalidJob("a poster needs a time at or after zero and a side"));
    }
    let scheme = url.split_once("://").map(|(scheme, _)| scheme.to_ascii_lowercase());
    if !matches!(scheme.as_deref(), Some("http" | "https")) {
        return Err(VideoExportError::InvalidJob("a remote poster needs an http or https URL"));
    }
    super::os::NativeVideoExport.remote_poster(url, at_seconds, max_side)
}

pub fn start_export(job: ExportJob, done: ExportSink) -> Result<ExportTicket, VideoExportError> {
    job.validate()?;
    super::os::NativeVideoExport.start_export(job, done)
}

pub fn start_still_video(job: StillVideoJob, done: ExportSink) -> Result<ExportTicket, VideoExportError> {
    job.validate()?;
    super::os::NativeVideoExport.start_still_video(job, done)
}
