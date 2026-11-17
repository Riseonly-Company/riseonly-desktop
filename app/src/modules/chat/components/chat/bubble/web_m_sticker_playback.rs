use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use gpui::{
    div, App, AppContext, AsyncApp, Context, IntoElement, ObjectFit, ParentElement, Render, RenderImage, Styled, Task,
    WeakEntity, Window,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::texture::native_surface::{
    prepare_host_bgra_frame, present_host_frame, HostBgraFrame, PreparedHostFrame,
};
use rise_media::video::av_player::AVLayerVideoGravity;
use rise_media::video::decoder::{open_software, DecoderRequest, HostI420Planes, VideoCodec, VideoDecoder};
use rise_ui::swiftui::alpha_shadow::{alpha_shadow_mask, AlphaShadowModifiers};

pub struct WebMStickerSupport;

impl WebMStickerSupport {
    pub fn is_web_m(source: &str) -> bool {
        let path = Url::from_string(source).map_or_else(|| source.to_string(), |url| url.path());
        Url::from_string_encoding_invalid_characters(&path, true)
            .is_some_and(|url| url.path_extension().to_lowercase() == "webm")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WebMStickerTrackCodec {
    Vp8,
    Vp9,
}

#[derive(Clone, Debug)]
pub struct WebMStickerTrackFrame {
    pub time_ms: i64,
    pub is_key: bool,
    pub data: Arc<[u8]>,
    pub alpha: Option<Arc<[u8]>>,
}

#[derive(Clone, Debug)]
pub struct WebMStickerTrack {
    pub codec: WebMStickerTrackCodec,
    pub width: usize,
    pub height: usize,
    pub frames: Arc<[WebMStickerTrackFrame]>,
    pub duration_ms: i64,
}

/// minimal ebml walk, masters are entered in place
/// so unknown size segment and cluster elements work without knowing where they end
pub struct WebMStickerDemuxer;

const WEB_M_STICKER_DEMUXER_MASTERS: [u32; 9] =
    [0x18538067, 0x1F43B675, 0x1654AE6B, 0xAE, 0xE0, 0x1549A966, 0xA0, 0x75A1, 0xA6];

struct WebMStickerRawFrame {
    track: u64,
    time: i64,
    key: bool,
    data: Arc<[u8]>,
    alpha: Option<Arc<[u8]>>,
}

struct WebMStickerBlock {
    track: u64,
    time: i64,
    flags: u8,
    data: Arc<[u8]>,
}

struct WebMStickerGroup {
    track: u64,
    time: i64,
    data: Arc<[u8]>,
}

#[derive(Default)]
struct WebMStickerParseState {
    video_track: Option<u64>,
    current_track_number: Option<u64>,
    current_track_type: Option<u64>,
    current_codec: Option<WebMStickerTrackCodec>,
    codec: Option<WebMStickerTrackCodec>,
    width: usize,
    height: usize,
    pending_width: usize,
    pending_height: usize,
    raw_frames: Vec<WebMStickerRawFrame>,
    group: Option<WebMStickerGroup>,
    group_alpha: Option<Arc<[u8]>>,
    group_add_id: u64,
    group_has_reference: bool,
}

impl WebMStickerParseState {
    fn close_track_entry(&mut self) {
        if self.current_track_type == Some(1) && self.video_track.is_none() {
            if let (Some(number), Some(entry_codec)) = (self.current_track_number, self.current_codec) {
                self.video_track = Some(number);
                self.codec = Some(entry_codec);
                self.width = self.pending_width;
                self.height = self.pending_height;
            }
        }
        self.current_track_number = None;
        self.current_track_type = None;
        self.current_codec = None;
    }

    fn flush_group(&mut self) {
        if let Some(group) = self.group.take() {
            self.raw_frames.push(WebMStickerRawFrame {
                track: group.track,
                time: group.time,
                key: !self.group_has_reference,
                data: group.data,
                alpha: self.group_alpha.take(),
            });
        }
        self.group_alpha = None;
        self.group_add_id = 1;
        self.group_has_reference = false;
    }
}

impl WebMStickerDemuxer {
    pub fn parse(data: &[u8]) -> Option<WebMStickerTrack> {
        let mut reader = WebMStickerDemuxerReader::new(data);
        let mut timecode_scale: i64 = 1_000_000;
        let mut cluster_time: i64 = 0;
        let mut state = WebMStickerParseState { group_add_id: 1, ..WebMStickerParseState::default() };

        while let Some((id, size)) = reader.next_element() {
            if WEB_M_STICKER_DEMUXER_MASTERS.contains(&id) {
                if id == 0xAE {
                    state.close_track_entry();
                }
                if id == 0xA0 || id == 0x1F43B675 {
                    state.flush_group();
                }
                continue;
            }
            let Some(size) = size else { break };
            let Some(payload) = reader.take(usize::try_from(size).unwrap_or(usize::MAX)) else { break };
            match id {
                0x2AD7B1 => timecode_scale = clamp_to_i64(Self::uint(payload)),
                0xE7 => cluster_time = clamp_to_i64(Self::uint(payload)),
                0xD7 => state.current_track_number = Some(Self::uint(payload)),
                0x83 => state.current_track_type = Some(Self::uint(payload)),
                0x86 => {
                    let name = String::from_utf8_lossy(payload);
                    state.current_codec = match name.as_ref() {
                        "V_VP9" => Some(WebMStickerTrackCodec::Vp9),
                        "V_VP8" => Some(WebMStickerTrackCodec::Vp8),
                        _ => None,
                    };
                }
                0xB0 => state.pending_width = clamp_to_usize(Self::uint(payload)),
                0xBA => state.pending_height = clamp_to_usize(Self::uint(payload)),
                0xA3 => {
                    state.flush_group();
                    if let Some(block) = Self::block(payload, cluster_time) {
                        state.raw_frames.push(WebMStickerRawFrame {
                            track: block.track,
                            time: block.time,
                            key: block.flags & 0x80 != 0,
                            data: block.data,
                            alpha: None,
                        });
                    }
                }
                0xA1 => {
                    if let Some(block) = Self::block(payload, cluster_time) {
                        state.group = Some(WebMStickerGroup { track: block.track, time: block.time, data: block.data });
                    }
                }
                0xFB => state.group_has_reference = true,
                0xEE => state.group_add_id = Self::uint(payload),
                0xA5 => {
                    if state.group_add_id == 1 {
                        state.group_alpha = Some(Arc::from(payload));
                    }
                }
                _ => {}
            }
        }
        state.flush_group();
        state.close_track_entry();

        let video_track = state.video_track?;
        let codec = state.codec.filter(|codec| *codec == WebMStickerTrackCodec::Vp9)?;
        if state.width == 0 || state.height == 0 {
            return None;
        }
        let scale_ms = timecode_scale as f64 / 1_000_000.0;
        let mut frames: Vec<WebMStickerTrackFrame> = Vec::with_capacity(state.raw_frames.len());
        for raw in state.raw_frames.into_iter().filter(|raw| raw.track == video_track) {
            let time_ms = exact_i64((raw.time as f64 * scale_ms).round())?;
            frames.push(WebMStickerTrackFrame { time_ms, is_key: raw.key, data: raw.data, alpha: raw.alpha });
        }
        frames.sort_by_key(|frame| frame.time_ms);
        let first = frames.first()?.time_ms;
        let last = frames.last()?.time_ms;
        let step = if frames.len() > 1 {
            (last.wrapping_sub(first) / i64::try_from(frames.len() - 1).unwrap_or(i64::MAX)).max(1)
        } else {
            33
        };
        Some(WebMStickerTrack {
            codec,
            width: state.width,
            height: state.height,
            frames: Arc::from(frames),
            duration_ms: last.wrapping_sub(first).wrapping_add(step),
        })
    }

    fn block(payload: &[u8], cluster_time: i64) -> Option<WebMStickerBlock> {
        let mut reader = WebMStickerDemuxerReader::new(payload);
        let (track, _) = reader.vint(false)?;
        let header = reader.take(3)?;
        let relative = i16::from_be_bytes([header[0], header[1]]);
        let flags = header[2];
        if flags & 0x06 != 0 {
            return None;
        }
        let data = Arc::from(reader.rest());
        Some(WebMStickerBlock { track, time: cluster_time.wrapping_add(i64::from(relative)), flags, data })
    }

    fn uint(payload: &[u8]) -> u64 {
        payload.iter().take(8).fold(0u64, |value, byte| (value << 8) | u64::from(*byte))
    }
}

fn clamp_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn clamp_to_usize(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn exact_i64(value: f64) -> Option<i64> {
    if !value.is_finite() || value < -9_223_372_036_854_775_808.0 || value >= 9_223_372_036_854_775_808.0 {
        return None;
    }
    Some(value as i64)
}

struct WebMStickerDemuxerReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> WebMStickerDemuxerReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn vint(&mut self, keep_marker: bool) -> Option<(u64, bool)> {
        let first = *self.bytes.get(self.offset)?;
        if first == 0 {
            return None;
        }
        let length = first.leading_zeros() as usize + 1;
        if self.offset + length > self.bytes.len() {
            return None;
        }
        let mask = 0xFFu8.checked_shr(length as u32).unwrap_or(0);
        let mut value = u64::from(if keep_marker { first } else { first & mask });
        let mut all_ones = (first & mask) == mask;
        for index in 1..length {
            let byte = self.bytes[self.offset + index];
            value = (value << 8) | u64::from(byte);
            if byte != 0xFF {
                all_ones = false;
            }
        }
        self.offset += length;
        Some((value, all_ones))
    }

    fn next_element(&mut self) -> Option<(u32, Option<u64>)> {
        let (id, _) = self.vint(true)?;
        let (size, unknown) = self.vint(false)?;
        Some((id as u32, if unknown { None } else { Some(size) }))
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(count)?;
        let slice = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(slice)
    }

    fn rest(&mut self) -> &'a [u8] {
        let slice = self.bytes.get(self.offset..).unwrap_or_default();
        self.offset = self.bytes.len();
        slice
    }
}

/// plane views of one decoded frame, valid until the next decode on the same decoder
struct WebMStickerI420<'a> {
    width: usize,
    height: usize,
    chroma_width: usize,
    chroma_height: usize,
    data_y: &'a [u8],
    data_u: &'a [u8],
    data_v: &'a [u8],
    stride_y: usize,
    stride_u: usize,
    stride_v: usize,
}

impl<'a> WebMStickerI420<'a> {
    fn from_planes(planes: HostI420Planes<'a>) -> Self {
        let width = planes.width as usize;
        let height = planes.height as usize;
        Self {
            width,
            height,
            chroma_width: width.div_ceil(2),
            chroma_height: height.div_ceil(2),
            data_y: planes.y,
            data_u: planes.u,
            data_v: planes.v,
            stride_y: planes.stride_y as usize,
            stride_u: planes.stride_u as usize,
            stride_v: planes.stride_v as usize,
        }
    }

    fn has_planes(&self) -> bool {
        fits(self.data_y, self.stride_y, self.width, self.height)
            && fits(self.data_u, self.stride_u, self.chroma_width, self.chroma_height)
            && fits(self.data_v, self.stride_v, self.chroma_width, self.chroma_height)
    }
}

fn fits(plane: &[u8], stride: usize, width: usize, height: usize) -> bool {
    height == 0 || (stride >= width && stride.checked_mul(height - 1).and_then(|rows| rows.checked_add(width)).is_some_and(|len| len <= plane.len()))
}

/// decodes one stream (colour or alpha) synchronously on the playback queue
struct WebMStickerStreamDecoder {
    decoder: Box<dyn VideoDecoder>,
}

impl WebMStickerStreamDecoder {
    fn new(codec: WebMStickerTrackCodec, width: usize, height: usize) -> Option<Self> {
        if codec != WebMStickerTrackCodec::Vp9 {
            return None;
        }
        let request = DecoderRequest {
            codec: VideoCodec::Vp9,
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
            bit_depth: 8,
        };
        open_software(&request).ok().map(|decoder| Self { decoder })
    }

    fn decode(&mut self, data: &[u8], timestamp_us: i64) -> Option<WebMStickerI420<'_>> {
        self.decoder.send_packet(data, timestamp_us).ok()?;
        self.decoder.receive_frame().ok()??;
        let image = WebMStickerI420::from_planes(self.decoder.host_planes()?);
        image.has_planes().then_some(image)
    }
}

pub struct WebMStickerFrameDecoderFrame {
    pub time_ms: i64,
    pub image: HostBgraFrame,
}

/// decodes a track in one pass for the media editor bake, off main thread
/// every frame is decoded since inter frames need predecessors, only every stride-th is kept
pub enum WebMStickerFrameDecoder {}

impl WebMStickerFrameDecoder {
    pub fn decode(track: &WebMStickerTrack, max_frames: usize) -> Vec<WebMStickerFrameDecoderFrame> {
        if max_frames == 0 || track.frames.is_empty() {
            return Vec::new();
        }
        let Some(mut color) = WebMStickerStreamDecoder::new(track.codec, track.width, track.height) else {
            return Vec::new();
        };
        let mut alpha_decoder = WebMStickerStreamDecoder::new(track.codec, track.width, track.height);
        let stride = track.frames.len().div_ceil(max_frames).max(1);
        let start_ms = track.frames[0].time_ms;
        let mut needs_key_frame = true;
        let mut result: Vec<WebMStickerFrameDecoderFrame> = Vec::new();
        for (index, frame) in track.frames.iter().enumerate() {
            if frame.is_key {
                needs_key_frame = false;
            } else if needs_key_frame {
                continue;
            }
            let timestamp_us = frame.time_ms.saturating_mul(1000);
            let Some(decoded) = color.decode(&frame.data, timestamp_us) else {
                continue;
            };
            let alpha = frame
                .alpha
                .as_ref()
                .and_then(|data| alpha_decoder.as_mut().and_then(|decoder| decoder.decode(data, timestamp_us)));
            if index % stride != 0 || result.len() >= max_frames {
                continue;
            }
            let Some(image) = WebMStickerPlayback::image(&decoded, alpha.as_ref(), 0, index as u64) else {
                continue;
            };
            result.push(WebMStickerFrameDecoderFrame {
                time_ms: frame.time_ms.wrapping_sub(start_ms),
                image,
            });
            if max_frames == 1 {
                break;
            }
        }
        result
    }
}

/// webm file the system video stack doesnt open
/// the media editor takes length and strip from the sticker decoder instead
pub enum WebMStickerFile {}

impl WebMStickerFile {
    pub fn is_web_m_path(path: &std::path::Path) -> bool {
        path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("webm"))
    }

    /// blocking, reads and parses the whole file
    pub fn track(path: &std::path::Path) -> Option<WebMStickerTrack> {
        let data = std::fs::read(path).ok()?;
        WebMStickerDemuxer::parse(&data)
    }

    /// container duration, or the span of its frames when the header has none
    pub fn duration_seconds(track: &WebMStickerTrack) -> f64 {
        let first = track.frames.first().map_or(0, |frame| frame.time_ms);
        let span = track.frames.last().map_or(0, |frame| frame.time_ms.saturating_sub(first));
        track.duration_ms.max(span).max(0) as f64 / 1000.0
    }

    /// up to count pictures spread over the track, none with a side past max_side
    /// as packed bgra rows. blocking since every frame is decoded
    pub fn strip(track: &WebMStickerTrack, count: usize, max_side: u32) -> Vec<(u32, u32, Vec<u8>)> {
        WebMStickerFrameDecoder::decode(track, count)
            .iter()
            .filter_map(|frame| Self::scaled(&frame.image, max_side))
            .collect()
    }

    fn scaled(frame: &HostBgraFrame, max_side: u32) -> Option<(u32, u32, Vec<u8>)> {
        if !frame.rows_fit() {
            return None;
        }
        let (width, height) = (frame.width, frame.height);
        let row = usize::try_from(width).ok()?.checked_mul(4)?;
        let stride = usize::try_from(frame.stride).ok()?;
        let mut packed = Vec::with_capacity(row.checked_mul(usize::try_from(height).ok()?)?);
        for line in 0..usize::try_from(height).ok()? {
            packed.extend_from_slice(frame.pixels.get(line * stride..line * stride + row)?);
        }
        let longest = width.max(height);
        if max_side == 0 || longest <= max_side {
            return Some((width, height, packed));
        }
        let picture = image::RgbaImage::from_raw(width, height, packed)?;
        let scale = f64::from(max_side) / f64::from(longest);
        let scaled_width = ((f64::from(width) * scale).round() as u32).max(1);
        let scaled_height = ((f64::from(height) * scale).round() as u32).max(1);
        let resized = image::imageops::resize(&picture, scaled_width, scaled_height, image::imageops::FilterType::Triangle);
        Some((scaled_width, scaled_height, resized.into_raw()))
    }
}

/// parsed tracks by source so a seen sticker starts without touching disk
pub struct WebMStickerTrackStore;

static WEB_M_STICKER_TRACK_CACHE: LazyLock<NSCache<String, WebMStickerTrack>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_total_cost_limit(32 * 1024 * 1024);
    cache
});

impl WebMStickerTrackStore {
    /// file read and parsing stay off the main thread
    pub async fn track(file: PathBuf, key: String) -> Option<WebMStickerTrack> {
        if let Some(track) = WEB_M_STICKER_TRACK_CACHE.object(&key) {
            return Some(track);
        }
        RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let data = std::fs::read(&file).ok()?;
                let track = WebMStickerDemuxer::parse(&data)?;
                WEB_M_STICKER_TRACK_CACHE.set_object_cost(track.clone(), key, data.len());
                Some(track)
            })
            .await
            .ok()
            .flatten()
    }
}

/// one decoded picture ready for the layer
/// the frame, its aspect and the alpha for its shadow
type WebMStickerRenderedFrame = (PreparedHostFrame, Option<f32>, Option<Arc<RenderImage>>);

/// layer a playback draws into, holds the last published frame and shows it in its bounds
/// under the host gravity (aspect fit by default)
#[derive(Default)]
pub struct WebMStickerLayer {
    contents: Option<PreparedHostFrame>,
    contents_aspect: Option<f32>,
    contents_shadow_mask: Option<Arc<RenderImage>>,
    contents_gravity: AVLayerVideoGravity,
}

impl WebMStickerLayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// follows the host video gravity, aspect fill or aspect fit
    pub fn set_contents_gravity(&mut self, contents_gravity: AVLayerVideoGravity, cx: &mut Context<Self>) {
        if self.contents_gravity == contents_gravity {
            return;
        }
        self.contents_gravity = contents_gravity;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.contents_aspect = None;
        self.contents_shadow_mask = None;
        if self.contents.take().is_some() {
            cx.notify();
        }
    }

}

impl Render for WebMStickerLayer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let aspect = self.contents.as_ref().and(self.contents_aspect);
        let frame = self.contents.as_ref().and_then(|contents| present_host_frame(contents).ok());
        let picture = div().flex_none().children(frame);
        let (picture, shadow_fit) = match (self.contents_gravity, aspect) {
            (AVLayerVideoGravity::ResizeAspectFill, Some(aspect)) => {
                (picture.aspect_ratio(aspect).min_w_full().min_h_full(), ObjectFit::Fill)
            }
            (AVLayerVideoGravity::ResizeAspect, _) => (picture.size_full(), ObjectFit::Contain),
            (AVLayerVideoGravity::Resize, _) | (_, None) => (picture.size_full(), ObjectFit::Fill),
        };
        let picture = match self.contents_shadow_mask.clone().filter(|_| self.contents.is_some()) {
            Some(mask) => picture.alpha_shadow_source(mask, shadow_fit).into_any_element(),
            None => picture.into_any_element(),
        };
        div().size_full().flex().items_center().justify_center().overflow_hidden().child(picture)
    }
}

#[derive(Default)]
struct WebMStickerDecoders {
    color_decoder: Option<WebMStickerStreamDecoder>,
    alpha_decoder: Option<WebMStickerStreamDecoder>,
}

fn lock_decoders(decoders: &Mutex<WebMStickerDecoders>) -> MutexGuard<'_, WebMStickerDecoders> {
    decoders.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct WebMStickerMainState {
    generation: Cell<u64>,
    has_published: Cell<bool>,
    on_first_frame: RefCell<Option<Rc<dyn Fn(&mut App)>>>,
}

impl WebMStickerMainState {
    /// bumped on every play, pause and stop
    /// queued steps of an older value drop out
    fn bump_generation(&self) -> u64 {
        let next = self.generation.get().wrapping_add(1);
        self.generation.set(next);
        next
    }
}

struct WebMStickerCursor {
    next_index: usize,
    needs_key_frame: bool,
    failed_frames_in_row: usize,
    loop_start: Duration,
    last_frame_due_at: Duration,
    published_in_pass: usize,
    failed_in_pass: usize,
}

impl WebMStickerCursor {
    const MINIMUM_FRAME_INTERVAL: Duration = Duration::from_micros(1_000_000 / 60);
}

static WEB_M_STICKER_STREAM_IDS: AtomicU64 = AtomicU64::new(1);

/// plays a parsed track into a layer, decoding runs in background and main thread only swaps contents
/// loops forever until paused or stopped
pub struct WebMStickerPlayback {
    track: WebMStickerTrack,
    layer: WeakEntity<WebMStickerLayer>,
    decoders: Arc<Mutex<WebMStickerDecoders>>,
    main: Rc<WebMStickerMainState>,
    stream_id: u64,
    frame_generations: Arc<AtomicU64>,
    is_playing: bool,
    queue: Option<Task<()>>,
}

impl WebMStickerPlayback {
    pub fn new(track: WebMStickerTrack, layer: WeakEntity<WebMStickerLayer>) -> Self {
        Self {
            track,
            layer,
            decoders: Arc::new(Mutex::new(WebMStickerDecoders::default())),
            main: Rc::new(WebMStickerMainState::default()),
            stream_id: WEB_M_STICKER_STREAM_IDS.fetch_add(1, Ordering::Relaxed),
            frame_generations: Arc::new(AtomicU64::new(0)),
            is_playing: false,
            queue: None,
        }
    }

    /// main thread, once
    /// first frame is on screen so a poster beneath can go
    pub fn on_first_frame(&self) -> Option<Rc<dyn Fn(&mut App)>> {
        self.main.on_first_frame.borrow().clone()
    }

    pub fn set_on_first_frame(&self, on_first_frame: Option<Rc<dyn Fn(&mut App)>>) {
        *self.main.on_first_frame.borrow_mut() = on_first_frame;
    }

    pub fn play(&mut self, cx: &mut App) {
        if self.is_playing {
            return;
        }
        self.is_playing = true;
        let generation = self.main.bump_generation();
        tracing::warn!(target: "riseonly::X2337", stream = self.stream_id, frames = self.track.frames.len(), "webm sticker: play");
        if self.track.frames.is_empty() {
            return;
        }
        let track = self.track.clone();
        let layer = self.layer.clone();
        let decoders = self.decoders.clone();
        let main = self.main.clone();
        let stream_id = self.stream_id;
        let frame_generations = self.frame_generations.clone();
        self.queue = Some(cx.spawn(async move |cx| {
            let cursor = WebMStickerCursor {
                next_index: 0,
                needs_key_frame: true,
                failed_frames_in_row: 0,
                loop_start: RiseClock::monotonic(),
                last_frame_due_at: Duration::ZERO,
                published_in_pass: 0,
                failed_in_pass: 0,
            };
            Self::run(track, layer, decoders, main, stream_id, frame_generations, generation, cursor, cx).await;
        }));
    }

    pub fn pause(&mut self) {
        if !self.is_playing {
            return;
        }
        self.is_playing = false;
        self.main.bump_generation();
        self.queue = None;
        tracing::warn!(target: "riseonly::X2337", stream = self.stream_id, "webm sticker: pause");
    }

    pub fn stop(&mut self, cx: &mut App) {
        self.is_playing = false;
        self.main.bump_generation();
        self.queue = None;
        let decoders = self.decoders.clone();
        cx.background_spawn(async move {
            let mut decoders = lock_decoders(&decoders);
            decoders.color_decoder = None;
            decoders.alpha_decoder = None;
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    async fn run(
        track: WebMStickerTrack,
        layer: WeakEntity<WebMStickerLayer>,
        decoders: Arc<Mutex<WebMStickerDecoders>>,
        main: Rc<WebMStickerMainState>,
        stream_id: u64,
        frame_generations: Arc<AtomicU64>,
        generation: u64,
        mut cursor: WebMStickerCursor,
        cx: &mut AsyncApp,
    ) {
        let first_time_ms = track.frames[0].time_ms;
        loop {
            if generation != main.generation.get() {
                return;
            }
            if cursor.next_index >= track.frames.len() {
                tracing::warn!(
                    target: "riseonly::X2337",
                    stream = stream_id,
                    published = cursor.published_in_pass,
                    failed = cursor.failed_in_pass,
                    "webm sticker: pass done"
                );
                cursor.published_in_pass = 0;
                cursor.failed_in_pass = 0;
                cursor.next_index = 0;
                cursor.needs_key_frame = true;
                cursor.loop_start += Duration::from_millis(u64::try_from(track.duration_ms).unwrap_or(0));
            }
            let frame = track.frames[cursor.next_index].clone();
            cursor.next_index += 1;

            let job_track = track.clone();
            let job_decoders = decoders.clone();
            let job_frame = frame.clone();
            let job_frame_generations = frame_generations.clone();
            let (returned, image) = cx
                .background_executor()
                .spawn(async move {
                    let mut cursor = cursor;
                    let image = Self::render(&job_track, &job_frame, &job_decoders, &mut cursor, stream_id, &job_frame_generations);
                    (cursor, image)
                })
                .await;
            cursor = returned;

            match image {
                Some(image) => {
                    cursor.failed_frames_in_row = 0;
                    let offset = u64::try_from(frame.time_ms.wrapping_sub(first_time_ms)).unwrap_or(0);
                    let due = cursor.loop_start + Duration::from_millis(offset);
                    let now = RiseClock::monotonic();
                    let delay = due
                        .max(cursor.last_frame_due_at + WebMStickerCursor::MINIMUM_FRAME_INTERVAL)
                        .saturating_sub(now);
                    cursor.last_frame_due_at = now + delay;
                    if !delay.is_zero() {
                        cx.background_executor().timer(delay).await;
                    }
                    if generation != main.generation.get() {
                        return;
                    }
                    if !Self::publish(&layer, &main, image, cx) {
                        tracing::warn!(target: "riseonly::X2337", stream = stream_id, "webm sticker: publish refused, stays on the poster");
                        return;
                    }
                    cursor.published_in_pass += 1;
                    if track.frames.len() <= 1 {
                        return;
                    }
                }
                None => {
                    cursor.failed_frames_in_row += 1;
                    cursor.failed_in_pass += 1;
                    if cursor.failed_frames_in_row >= track.frames.len() {
                        tracing::warn!(target: "riseonly::X2337", stream = stream_id, "webm sticker: no frame decoded in a pass, stays on the poster");
                        let decoders = decoders.clone();
                        cx.background_executor()
                            .spawn(async move {
                                let mut decoders = lock_decoders(&decoders);
                                decoders.color_decoder = None;
                                decoders.alpha_decoder = None;
                            })
                            .await;
                        return;
                    }
                }
            }
        }
    }

    fn publish(
        layer: &WeakEntity<WebMStickerLayer>,
        main: &WebMStickerMainState,
        image: WebMStickerRenderedFrame,
        cx: &mut AsyncApp,
    ) -> bool {
        let (image, aspect, shadow_mask) = image;
        if !main.has_published.get() && present_host_frame(&image).is_err() {
            return false;
        }
        let committed = layer.update(cx, |layer, cx| {
            layer.contents = Some(image);
            layer.contents_aspect = aspect;
            layer.contents_shadow_mask = shadow_mask;
            cx.notify();
        });
        if committed.is_err() {
            return false;
        }
        if !main.has_published.get() {
            main.has_published.set(true);
            let on_first_frame = main.on_first_frame.borrow().clone();
            if let Some(on_first_frame) = on_first_frame {
                cx.update(|cx| on_first_frame(cx));
            }
        }
        true
    }

    fn render(
        track: &WebMStickerTrack,
        frame: &WebMStickerTrackFrame,
        decoders: &Mutex<WebMStickerDecoders>,
        cursor: &mut WebMStickerCursor,
        stream_id: u64,
        frame_generations: &AtomicU64,
    ) -> Option<WebMStickerRenderedFrame> {
        if frame.is_key {
            cursor.needs_key_frame = false;
        } else if cursor.needs_key_frame {
            return None;
        }
        let timestamp_us = frame.time_ms.saturating_mul(1000);
        let mut decoders = lock_decoders(decoders);
        let WebMStickerDecoders { color_decoder, alpha_decoder } = &mut *decoders;
        if color_decoder.is_none() {
            *color_decoder = WebMStickerStreamDecoder::new(track.codec, track.width, track.height);
        }
        let color = color_decoder.as_mut()?.decode(&frame.data, timestamp_us)?;
        let mut alpha = None;
        if let Some(alpha_data) = &frame.alpha {
            if alpha_decoder.is_none() {
                *alpha_decoder = WebMStickerStreamDecoder::new(track.codec, track.width, track.height);
            }
            alpha = alpha_decoder.as_mut().and_then(|decoder| decoder.decode(alpha_data, timestamp_us));
        }
        let frame_generation = frame_generations.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let image = Self::image(&color, alpha.as_ref(), stream_id, frame_generation)?;
        let aspect = (image.height > 0).then(|| image.width as f32 / image.height as f32);
        let shadow_mask = alpha_shadow_mask(
            &image.pixels,
            usize::try_from(image.width).ok()?,
            usize::try_from(image.height).ok()?,
            usize::try_from(image.stride).ok()?,
        );
        prepare_host_bgra_frame(image).ok().map(|prepared| (prepared, aspect, shadow_mask))
    }

    /// i420 to bgra with the bt.601 video range matrix
    /// then alpha plane written over the alpha byte and colour premultiplied
    fn image(
        color: &WebMStickerI420<'_>,
        alpha: Option<&WebMStickerI420<'_>>,
        stream_id: u64,
        generation: u64,
    ) -> Option<HostBgraFrame> {
        let width = color.width;
        let height = color.height;
        if width == 0 || height == 0 {
            return None;
        }
        let row_bytes = width.checked_mul(4)?;
        let mut pixels = vec![0u8; row_bytes.checked_mul(height)?];
        let alpha = alpha.filter(|alpha| alpha.width == width && alpha.height == height);

        for row in 0..height {
            let y_row = &color.data_y[row * color.stride_y..];
            let u_row = &color.data_u[(row / 2) * color.stride_u..];
            let v_row = &color.data_v[(row / 2) * color.stride_v..];
            let a_row = alpha.map(|alpha| &alpha.data_y[row * alpha.stride_y..]);
            let out = &mut pixels[row * row_bytes..(row + 1) * row_bytes];
            for column in 0..width {
                let c = i32::from(y_row[column]) - 16;
                let d = i32::from(u_row[column / 2]) - 128;
                let e = i32::from(v_row[column / 2]) - 128;
                let r = clamp_channel((298 * c + 409 * e + 128) >> 8);
                let g = clamp_channel((298 * c - 100 * d - 208 * e + 128) >> 8);
                let b = clamp_channel((298 * c + 516 * d + 128) >> 8);
                let pixel = &mut out[column * 4..column * 4 + 4];
                match a_row {
                    Some(a_row) => {
                        let a = a_row[column];
                        pixel[0] = premultiply(b, a);
                        pixel[1] = premultiply(g, a);
                        pixel[2] = premultiply(r, a);
                        pixel[3] = a;
                    }
                    None => {
                        pixel[0] = b;
                        pixel[1] = g;
                        pixel[2] = r;
                        pixel[3] = 255;
                    }
                }
            }
        }

        Some(HostBgraFrame {
            stream_id,
            generation,
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
            stride: u32::try_from(row_bytes).ok()?,
            pixels: Arc::from(pixels),
        })
    }
}

fn clamp_channel(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn premultiply(channel: u8, alpha: u8) -> u8 {
    ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8
}
