use std::f64::consts::FRAC_PI_2;
use std::fmt::Write as _;

use super::video_export::*;

pub const VIDEO_SINK: &str = "out_v";
pub const AUDIO_SINK: &str = "out_a";
pub const MP4_MUXER: &str = "mp4";
pub const AAC_ENCODER: &str = "aac";
pub const H264_ENCODERS: [&str; 1] = ["h264_vaapi"];
pub const VAAPI_ENCODER: &str = "h264_vaapi";
pub const DEFAULT_VAAPI_DEVICE: &str = "/dev/dri/renderD128";
pub const OUTPUT_SAMPLE_RATE: u32 = 44_100;
pub const OUTPUT_CHANNEL_LAYOUT: &str = "stereo";
pub const TRAILER_RESERVE_BYTES: u64 = 64 * 1024;
pub const QUARTER_TURN_EPSILON: f64 = 0.000_001;

/// stream behind a buffer source of the graph
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputStream {
    SegmentVideo(usize),
    SegmentAudio(usize),
    AudioLayer(usize),
    Overlay,
}

/// buffer source to create under name and feed from stream
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GraphInput {
    pub name: String,
    pub stream: InputStream,
}

/// how a stored frame turns before scaling, clockwise on screen
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Turn {
    None,
    Clockwise,
    Half,
    CounterClockwise,
    Free(f64),
}

/// layer transform of a stored frame as filters
/// mirror then turn then scale to the transformed box
/// drawn at its top left on the render
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placement {
    pub mirrored: bool,
    pub turn: Turn,
    pub turned_size: (u32, u32),
    pub size: (u32, u32),
    pub x: i64,
    pub y: i64,
}

fn pixels(value: f64) -> u32 {
    if value.is_finite() && value >= 1.0 { value.round().min(f64::from(u32::MAX)) as u32 } else { 1 }
}

fn quarter(angle: f64) -> Turn {
    let steps = angle / FRAC_PI_2;
    let nearest = steps.round();
    if (steps - nearest).abs() >= QUARTER_TURN_EPSILON {
        return Turn::Free(angle);
    }
    match (nearest as i64).rem_euclid(4) {
        1 => Turn::Clockwise,
        2 => Turn::Half,
        3 => Turn::CounterClockwise,
        _ => Turn::None,
    }
}

impl Placement {
    /// splits a transform of a stored frame into mirror, turn and uniform scale
    /// none for a degenerate transform or frame, editor transforms are always these
    pub fn of(transform: Affine, stored: FrameSize) -> Option<Self> {
        if !stored.is_finite_positive() {
            return None;
        }
        let determinant = transform.a * transform.d - transform.b * transform.c;
        if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
            return None;
        }
        let mirrored = determinant < 0.0;
        let (a, b) = if mirrored { (-transform.a, -transform.b) } else { (transform.a, transform.b) };
        let turn = quarter(b.atan2(a));
        let turned = match turn {
            Turn::None | Turn::Half => stored,
            Turn::Clockwise | Turn::CounterClockwise => FrameSize::new(stored.height, stored.width),
            Turn::Free(angle) => {
                let (cosine, sine) = (angle.cos().abs(), angle.sin().abs());
                FrameSize::new(
                    stored.width * cosine + stored.height * sine,
                    stored.width * sine + stored.height * cosine,
                )
            }
        };
        let (min_x, min_y, bounds) = transform.bounds_of(stored);
        if !bounds.is_finite_positive() || !min_x.is_finite() || !min_y.is_finite() {
            return None;
        }
        Some(Self {
            mirrored,
            turn,
            turned_size: (pixels(turned.width), pixels(turned.height)),
            size: (pixels(bounds.width), pixels(bounds.height)),
            x: min_x.round() as i64,
            y: min_y.round() as i64,
        })
    }

    /// the filter chain, no labels
    pub fn filters(&self) -> String {
        let mut chain: Vec<String> = Vec::new();
        if self.mirrored {
            chain.push("hflip".to_owned());
        }
        match self.turn {
            Turn::None => {}
            Turn::Clockwise => chain.push("transpose=clock".to_owned()),
            Turn::Half => {
                chain.push("hflip".to_owned());
                chain.push("vflip".to_owned());
            }
            Turn::CounterClockwise => chain.push("transpose=cclock".to_owned()),
            Turn::Free(angle) => chain.push(format!(
                "rotate=a={}:ow={}:oh={}:c=black",
                number(angle),
                self.turned_size.0,
                self.turned_size.1
            )),
        }
        if self.size != self.turned_size || chain.is_empty() {
            chain.push(format!("scale=w={}:h={}:flags=bicubic", self.size.0, self.size.1));
        }
        chain.join(",")
    }
}

/// number as filter options take it, fixed point and no exponent
pub fn number(value: f64) -> String {
    let value = if value.is_finite() { value } else { 0.0 };
    let text = format!("{value:.6}");
    if text == "-0.000000" { "0.000000".to_owned() } else { text }
}

pub fn fill_transform(transform: Affine, stored: FrameSize, size: (u32, u32)) -> Affine {
    let (min_x, min_y, bounds) = transform.bounds_of(stored);
    if !bounds.is_finite_positive() || !min_x.is_finite() || !min_y.is_finite() {
        return transform;
    }
    let (width, height) = (f64::from(size.0), f64::from(size.1));
    let scale = (width / bounds.width).max(height / bounds.height);
    if !scale.is_finite() || scale <= 0.0 {
        return transform;
    }
    transform
        .then(Affine::translation(-min_x, -min_y))
        .then(Affine::scale(scale, scale))
        .then(Affine::translation((width - bounds.width * scale) / 2.0, (height - bounds.height * scale) / 2.0))
}

/// frame a preset writes for a source with no render
/// displayed frame scaled down (never up) into the preset long and short side, both even
pub fn preset_size(probe: &VideoProbe, preset: ExportPreset) -> (u32, u32) {
    let display = probe.display_size();
    let long_side = display.width.max(display.height);
    let short_side = display.width.min(display.height);
    let scale = match preset.bounds() {
        Some((long_bound, short_bound)) => {
            (f64::from(long_bound) / long_side).min(f64::from(short_bound) / short_side).min(1.0)
        }
        None => 1.0,
    };
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    (even_side(display.width * scale), even_side(display.height * scale))
}

fn even_side(value: f64) -> u32 {
    if !value.is_finite() || value < 2.0 {
        return 2;
    }
    let integer = (value + 0.000_001).floor().min(f64::from(u32::MAX)) as u32;
    (integer - integer % 2).max(2)
}

/// frame format step of the encoder at the end of the video chain
pub fn encoder_tail(encoder: &str) -> &'static str {
    if encoder == VAAPI_ENCODER { "format=nv12,hwupload" } else { "format=yuv420p" }
}

/// graph for one job, with the sources to feed and what comes out
#[derive(Clone, PartialEq, Debug)]
pub struct GraphPlan {
    pub description: String,
    pub inputs: Vec<GraphInput>,
    pub size: (u32, u32),
    pub frame_rate: Option<u32>,
    pub total_seconds: f64,
    pub has_audio: bool,
}

struct Placed {
    span: TimeSpan,
    at: f64,
}

/// where every segment lands, its span (whole file by default) and its start on the timeline
/// default is right after the previous one
fn timeline(job: &ExportJob, probes: &[VideoProbe]) -> Vec<Placed> {
    let mut cursor = 0.0_f64;
    job.segments
        .iter()
        .zip(probes)
        .map(|(segment, probe)| {
            let span = segment
                .span
                .unwrap_or(TimeSpan { start_seconds: 0.0, duration_seconds: probe.duration_seconds.max(0.0) });
            let at = segment.at_seconds.unwrap_or(cursor);
            cursor = at + span.duration_seconds;
            Placed { span, at }
        })
        .collect()
}

pub const CURVE_BEND: f64 = 0.25;
pub const VIGNETTE_ANGLE_PER_INTENSITY: f64 = std::f64::consts::PI / 5.0;

/// colour pass of the adjustments as filters, none when they change nothing
pub fn color_filters(adjustments: ColorAdjustments) -> Option<String> {
    if adjustments.is_identity() {
        return None;
    }
    let mut filters: Vec<String> = Vec::new();
    if let Some(fraction) = adjustments.enhance_fraction() {
        filters.push(format!("normalize=strength={}", number(fraction)));
    }
    if let Some(controls) = adjustments.color_controls() {
        if controls.saturation != 1.0 {
            filters.push(format!("hue=s={}", number(controls.saturation.clamp(0.0, 10.0))));
        }
        if controls.brightness != 0.0 || controls.contrast != 1.0 {
            filters.push(format!("curves=all='{}'", tone_line(controls.brightness, controls.contrast)));
        }
    }
    if let Some(kelvin) = adjustments.target_neutral_kelvin() {
        filters.push(format!("colortemperature=temperature={}", number(kelvin.clamp(1000.0, 40000.0))));
    }
    if let Some(bend) = adjustments.highlight_shadow() {
        let quarter = (0.25 + bend.shadow_amount * CURVE_BEND).clamp(0.0, 1.0);
        let three_quarters = (0.75 + (bend.highlight_amount - 1.0) * CURVE_BEND).clamp(0.0, 1.0);
        filters.push(format!("curves=all='0/0 0.25/{} 0.75/{} 1/1'", number(quarter), number(three_quarters)));
    }
    if let Some(intensity) = adjustments.vignette_intensity() {
        let angle = (intensity * VIGNETTE_ANGLE_PER_INTENSITY).clamp(0.0, std::f64::consts::FRAC_PI_2);
        filters.push(format!("vignette=angle={}", number(angle)));
    }
    (!filters.is_empty()).then(|| filters.join(","))
}

fn tone_line(brightness: f64, contrast: f64) -> String {
    let slope = contrast.max(0.01);
    let line = |x: f64| slope * (x - 0.5) + 0.5 + brightness;
    let at_black = 0.5 - (0.5 + brightness) / slope;
    let at_white = 0.5 + (0.5 - brightness) / slope;
    let start = match line(0.0) {
        y if y < 0.0 => (at_black, 0.0),
        y if y > 1.0 => (at_white, 1.0),
        y => (0.0, y),
    };
    let end = match line(1.0) {
        y if y > 1.0 => (at_white, 1.0),
        y if y < 0.0 => (at_black, 0.0),
        y => (1.0, y),
    };
    let clamp = |value: f64| value.clamp(0.0, 1.0);
    format!("{}/{} {}/{}", number(clamp(start.0)), number(clamp(start.1)), number(clamp(end.0)), number(clamp(end.1)))
}

/// builds the graph for a validated job
/// probes are the segment files in order, layer_seconds the audio layer durations
/// encoder the picked h264 one
pub fn plan_graph(
    job: &ExportJob,
    probes: &[VideoProbe],
    layer_seconds: &[f64],
    encoder: &str,
) -> Result<GraphPlan, VideoExportError> {
    job.validate()?;
    if probes.len() != job.segments.len() || layer_seconds.len() != job.audio.len() {
        return Err(VideoExportError::InvalidJob("a probe is missing for a source of the job"));
    }
    for (segment, probe) in job.segments.iter().zip(probes) {
        if !probe.has_video || !probe.natural_size.is_finite_positive() {
            return Err(VideoExportError::NoVideoTrack(segment.source.clone()));
        }
    }
    let placed = timeline(job, probes);
    let total = placed.iter().map(|p| p.at + p.span.duration_seconds).fold(0.0_f64, f64::max);
    if !total.is_finite() || total <= 0.0 {
        return Err(VideoExportError::InvalidJob("the timeline is empty"));
    }
    let first = &probes[0];
    let (size, frame_rate) = match &job.render {
        Some(render) => (render.size, Some(render.frame_rate)),
        None => {
            let lone = job.segments.len() == 1 && placed[0].at == 0.0;
            let rate = if job.cap_frame_rate || !lone {
                Some(render_frame_rate(first.nominal_frame_rate))
            } else {
                None
            };
            (preset_size(first, job.preset), rate)
        }
    };
    let mut inputs = Vec::new();
    let mut graph = String::new();
    let direct = job.render.is_none() && job.segments.len() == 1 && placed[0].at == 0.0;
    let colors = job.render.as_ref().and_then(|render| color_filters(render.adjustments));
    let mut base = "base0".to_owned();
    if !direct {
        let rate = frame_rate.unwrap_or(DEFAULT_FRAME_RATE);
        let _ = write!(graph, "color=c=black:s={}x{}:r={}:d={}[{}];", size.0, size.1, rate, number(total), base);
    }
    for (index, ((segment, probe), place)) in job.segments.iter().zip(probes).zip(&placed).enumerate() {
        let transform = match job.render {
            Some(_) => segment.transform,
            None => fill_transform(segment.transform, probe.natural_size, size),
        };
        let placement = Placement::of(transform, probe.natural_size)
            .ok_or_else(|| VideoExportError::CompositionFailed(format!("segment {index} has a degenerate transform")))?;
        let name = format!("in_v{index}");
        let _ = write!(
            graph,
            "[{name}]trim=start={}:end={},setpts=PTS-STARTPTS,",
            number(place.span.start_seconds),
            number(place.span.end_seconds()),
        );
        if let Some(color) = colors.as_deref() {
            let _ = write!(graph, "{color},");
        }
        let _ = write!(graph, "{}", placement.filters());
        if let Some(rate) = frame_rate {
            let _ = write!(graph, ",fps={rate}");
        }
        inputs.push(GraphInput { name, stream: InputStream::SegmentVideo(index) });
        if direct {
            let _ = write!(graph, "[v{index}];");
            base = format!("v{index}");
        } else {
            let _ = write!(graph, ",setpts=PTS+{}/TB[v{index}];", number(place.at));
            let next = format!("base{}", index + 1);
            let _ = write!(
                graph,
                "[{base}][v{index}]overlay=x={}:y={}:eof_action=pass[{next}];",
                placement.x, placement.y
            );
            base = next;
        }
    }
    if job.render.as_ref().is_some_and(|render| render.overlay.is_some()) {
        let name = "in_o".to_owned();
        let _ = write!(graph, "[{base}][{name}]overlay=x=0:y=0:format=auto[composed];");
        inputs.push(GraphInput { name, stream: InputStream::Overlay });
        base = "composed".to_owned();
    }
    let _ = write!(graph, "[{base}]{}[{VIDEO_SINK}]", encoder_tail(encoder));
    let mut sounds = Vec::new();
    let sound_format = format!("aresample={OUTPUT_SAMPLE_RATE},aformat=channel_layouts={OUTPUT_CHANNEL_LAYOUT}");
    for (index, ((segment, probe), place)) in job.segments.iter().zip(probes).zip(&placed).enumerate() {
        let Some(volume) = segment.volume.filter(|_| probe.has_audio) else {
            continue;
        };
        let name = format!("in_a{index}");
        let label = format!("a{index}");
        let _ = write!(
            graph,
            ";[{name}]atrim=start={}:end={},asetpts=PTS-STARTPTS,{sound_format},adelay=delays={}:all=1,volume={}[{label}]",
            number(place.span.start_seconds),
            number(place.span.end_seconds()),
            delay_ms(place.at),
            number(f64::from(volume))
        );
        inputs.push(GraphInput { name, stream: InputStream::SegmentAudio(index) });
        sounds.push(label);
    }
    for (index, (layer, seconds)) in job.audio.iter().zip(layer_seconds).enumerate() {
        let placement = layer.placement.unwrap_or(AudioPlacement {
            span: TimeSpan { start_seconds: 0.0, duration_seconds: seconds.min(total) },
            at_seconds: 0.0,
        });
        if placement.span.duration_seconds.is_nan() || placement.span.duration_seconds <= 0.0 {
            continue;
        }
        let name = format!("in_m{index}");
        let label = format!("m{index}");
        let _ = write!(
            graph,
            ";[{name}]atrim=start={}:end={},asetpts=PTS-STARTPTS,{sound_format},adelay=delays={}:all=1,volume={}[{label}]",
            number(placement.span.start_seconds),
            number(placement.span.end_seconds()),
            delay_ms(placement.at_seconds),
            number(f64::from(layer.volume))
        );
        inputs.push(GraphInput { name, stream: InputStream::AudioLayer(index) });
        sounds.push(label);
    }
    let has_audio = !sounds.is_empty();
    if has_audio {
        let labels: String = sounds.iter().map(|label| format!("[{label}]")).collect();
        let mix = if sounds.len() == 1 {
            "anull".to_owned()
        } else {
            format!("amix=inputs={}:duration=longest:normalize=0", sounds.len())
        };
        let _ = write!(
            graph,
            ";{labels}{mix},atrim=end={},aformat=sample_fmts=fltp:sample_rates={OUTPUT_SAMPLE_RATE}:channel_layouts={OUTPUT_CHANNEL_LAYOUT}[{AUDIO_SINK}]",
            number(total)
        );
    }
    Ok(GraphPlan { description: graph, inputs, size, frame_rate, total_seconds: total, has_audio })
}

fn delay_ms(seconds: f64) -> i64 {
    if seconds.is_finite() && seconds > 0.0 { (seconds * 1000.0).round() as i64 } else { 0 }
}

/// filters that turn a stored frame upright for a poster of at most max_side, ending in bgra
/// plus the poster size
pub fn poster_filters(probe: &VideoProbe, max_side: u32) -> Result<(String, (u32, u32)), VideoExportError> {
    let display = probe.display_size();
    let long_side = display.width.max(display.height);
    let scale = if max_side > 0 && long_side > f64::from(max_side) { f64::from(max_side) / long_side } else { 1.0 };
    let transform = probe.upright().then(Affine::scale(scale, scale));
    let placement = Placement::of(transform, probe.natural_size)
        .ok_or_else(|| VideoExportError::PosterFailed("the frame has no usable size".to_owned()))?;
    Ok((format!("{},format=bgra", placement.filters()), placement.size))
}

/// picks the encoder from what libavcodec has here and whether the vaapi device opened
/// the error says what is missing
pub fn pick_video_encoder(has_encoder: impl Fn(&str) -> bool, vaapi_device: bool) -> Result<&'static str, VideoExportError> {
    if !has_encoder(AAC_ENCODER) {
        return Err(VideoExportError::EncoderUnavailable(format!("libavcodec has no {AAC_ENCODER} encoder")));
    }
    for encoder in H264_ENCODERS {
        if !has_encoder(encoder) {
            continue;
        }
        if encoder == VAAPI_ENCODER && !vaapi_device {
            continue;
        }
        return Ok(encoder);
    }
    Err(VideoExportError::EncoderUnavailable(format!(
        "no H.264 encoder: {} needs a VA-API device with H.264 encoding ({DEFAULT_VAAPI_DEVICE})",
        H264_ENCODERS.join(", ")
    )))
}

/// video bit rate for a preset at the output size and rate
/// the system gives no numbers so bits per pixel are picked
/// to keep the preset order and land near native for camera footage
pub fn video_bit_rate(preset: ExportPreset, size: (u32, u32), frame_rate: u32) -> u64 {
    let bits_per_pixel = match preset {
        ExportPreset::MediumQuality => 0.07,
        ExportPreset::HighestQuality => 0.2,
        _ => 0.12,
    };
    let rate = if frame_rate == 0 { DEFAULT_FRAME_RATE } else { frame_rate };
    let bits = f64::from(size.0) * f64::from(size.1) * f64::from(rate) * bits_per_pixel;
    (bits.round() as u64).clamp(250_000, 50_000_000)
}

pub const fn audio_bit_rate(preset: ExportPreset) -> u32 {
    match preset {
        ExportPreset::MediumQuality => 64_000,
        _ => 128_000,
    }
}

/// true when the muxer must stop taking packets, the export ends early and keeps what was written
/// so the arm stops feeding, flushes and writes the trailer
pub fn byte_ceiling_reached(written: u64, limit: Option<u64>) -> bool {
    limit.is_some_and(|limit| written.saturating_add(TRAILER_RESERVE_BYTES.min(limit / 8)) >= limit)
}

/// encoder progress from the last output timestamp
pub fn progress(encoded_seconds: f64, total_seconds: f64) -> f32 {
    if !encoded_seconds.is_finite() || !total_seconds.is_finite() || total_seconds <= 0.0 {
        return 0.0;
    }
    (encoded_seconds / total_seconds).clamp(0.0, 1.0) as f32
}

/// display matrix of the stream as the preferred transform, identity without one
/// the 9 values are a b u c d v x y w, a to y in 16.16 fixed point
pub fn affine_from_display_matrix(matrix: Option<[i32; 9]>) -> Affine {
    let Some(matrix) = matrix else {
        return Affine::IDENTITY;
    };
    let fixed = |value: i32| f64::from(value) / 65_536.0;
    let affine = Affine {
        a: fixed(matrix[0]),
        b: fixed(matrix[1]),
        c: fixed(matrix[3]),
        d: fixed(matrix[4]),
        tx: fixed(matrix[6]),
        ty: fixed(matrix[7]),
    };
    if (affine.a * affine.d - affine.b * affine.c).abs() <= f64::EPSILON { Affine::IDENTITY } else { affine }
}
