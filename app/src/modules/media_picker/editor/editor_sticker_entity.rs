use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, LazyLock};
use std::thread::ThreadId;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{
    App, AsyncApp, Bounds, BoxShadow, Context, DispatchPhase, ElementId, Entity, Hitbox,
    HitboxBehavior, Hsla, ImageSource, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ObjectFit, ParentElement, PathBuilder, Pixels, Point, Render, RenderImage,
    RenderOnce, Rgba, SharedString, Size, Styled, StyledImage, Task, Window, canvas, div, hsla, img,
    point, px, size,
};
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::{self, FilterType};
use image::{AnimationDecoder, DynamicImage, Frame, ImageFormat, ImageReader, RgbaImage};
use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_media::video_export::{self as video_export, Affine};
use rise_platform::emoji_raster;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::rotation_effect::RotationEffectModifiers;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use smallvec::SmallVec;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions,
};
use crate::core::media::rise_media_transfer_hub::RiseMediaCacheIdentity;
use crate::core::ui::clever_image_disk_cache::CleverImageBitmapDecoder;
use crate::core::ui::lottie_sticker::lottie_sticker_loader::{
    LottieStickerLoader, LottieStickerPayload,
};
use crate::core::ui::lottie_sticker::lottie_sticker_view::LottieStickerView;
use crate::core::ui::lottie_sticker::rise_lottie_frame_cache::{
    RiseLottieFrameSequence, RiseLottieSequenceKey,
};
use crate::modules::chat::components::chat::bubble::web_m_sticker_playback::{
    WebMStickerDemuxer, WebMStickerFrameDecoder, WebMStickerSupport,
};
use crate::modules::media_picker::editor::editor_entity_snapping::{
    EditorEntitySideHandle, EditorEntitySnap, EditorEntitySnapFeedback,
};
use crate::modules::media_picker::editor::editor_two_finger_transform::EditorTwoFingerTransformable;
use crate::modules::media_picker::editor::story_canvas_model::StoryCanvasGeometry;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    StickerAssetType, StickerItem,
};

/// sticker from a real pack, center is 0..1 of the canvas and box a fraction of short side
/// baked into export so all clients and moderation see the same
/// photo gets the first frame, video loops the whole clip
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorStickerEntity {
    pub id: Uuid,
    pub sticker_id: String,
    pub pack_id: String,
    pub file_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    pub asset_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    pub aspect_ratio: f32,
    pub center: Point<f32>,
    pub rotation: f32,
    pub scale: f32,
    pub loops: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

impl EditorStickerEntity {
    pub const BASE_SIDE_FRACTION: f32 = 0.36;

    pub fn from_sticker(sticker: &StickerItem, center: Point<f32>, loops: bool) -> Self {
        let ratio = if sticker.height > 0 { sticker.width as f32 / sticker.height as f32 } else { 1.0 };
        Self {
            id: Uuid::new_v4(),
            sticker_id: sticker.id.clone(),
            pack_id: sticker.pack_id.clone(),
            file_url: sticker.file_url.clone(),
            thumbnail_url: sticker.thumbnail_url.clone(),
            asset_type: sticker.asset_type.wire().to_owned(),
            mime_type: sticker.mime_type.clone(),
            codec: sticker.codec.clone(),
            aspect_ratio: if ratio.is_finite() && ratio > 0.0 { ratio } else { 1.0 },
            center,
            rotation: 0.0,
            scale: 1.0,
            loops,
            emoji: None,
        }
    }

    pub fn from_emoji(glyph: &str, center: Point<f32>) -> Self {
        Self {
            id: Uuid::new_v4(),
            sticker_id: format!("emoji|{glyph}"),
            pack_id: String::new(),
            file_url: String::new(),
            thumbnail_url: None,
            asset_type: StickerAssetType::StaticImage.wire().to_owned(),
            mime_type: None,
            codec: None,
            aspect_ratio: 1.0,
            center,
            rotation: 0.0,
            scale: 1.0,
            loops: false,
            emoji: Some(glyph.to_owned()),
        }
    }

    /// unscaled box on this canvas, the art fitted in a square of the base side
    pub fn box_size(aspect_ratio: f32, canvas: Size<f32>) -> Size<f32> {
        let side = Self::BASE_SIDE_FRACTION * canvas.width.min(canvas.height).max(1.0);
        let ratio = if aspect_ratio > 0.0 && aspect_ratio.is_finite() { aspect_ratio } else { 1.0 };
        if ratio >= 1.0 { size(side, side / ratio) } else { size(side * ratio, side) }
    }

    pub fn art_key(&self) -> String {
        Self::art_key_for(&self.sticker_id, self.loops)
    }

    fn art_key_for(sticker_id: &str, loops: bool) -> String {
        format!("{}|{}", sticker_id, if loops { "loop" } else { "still" })
    }

    fn is_video_art(&self) -> bool {
        self.emoji.is_none()
            && (self.kind() == StickerAssetType::Video
                || WebMStickerSupport::is_web_m(&self.file_url)
                || self.mime_type.as_deref().is_some_and(|mime| mime.to_lowercase().contains("webm")))
    }

    fn kind(&self) -> StickerAssetType {
        StickerAssetType::from_wire(&self.asset_type).unwrap_or(StickerAssetType::StaticImage)
    }
}

#[derive(Clone)]
pub struct EditorStickerArtStoreArt {
    pub frames: Arc<[Arc<RenderImage>]>,
    pub times: Arc<[f64]>,
    pub duration: f64,
}

impl EditorStickerArtStoreArt {
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1 && self.duration > 0.0
    }

    pub fn frame_index(&self, seconds: f64) -> usize {
        if !self.is_animated() {
            return 0;
        }
        let t = seconds % self.duration;
        let local = if t < 0.0 { t + self.duration } else { t };
        let mut index = 0;
        for (i, start) in self.times.iter().enumerate() {
            if *start <= local {
                index = i;
            }
        }
        index
    }

    fn single(frame: Arc<RenderImage>) -> Self {
        Self { frames: Arc::from(vec![frame]), times: Arc::from(vec![0.0]), duration: 0.0 }
    }
}

type EditorStickerArtWaiter = Rc<dyn Fn(Option<EditorStickerArtStoreArt>, &mut App)>;
type EditorStickerArtPreview<'a> = &'a dyn Fn(EditorStickerArtStoreArt, &AsyncApp);

thread_local! {
    static ART_WAITERS: RefCell<HashMap<String, Vec<EditorStickerArtWaiter>>> =
        RefCell::new(HashMap::new());
}

struct EditorStickerArtBakeRequest {
    entity: EditorStickerEntity,
    reply: std::sync::mpsc::SyncSender<Option<EditorStickerArtStoreArt>>,
}

#[derive(Default)]
struct EditorStickerArtStoreState {
    arts: HashMap<String, EditorStickerArtStoreArt>,
    order: Vec<String>,
}

/// decoded sticker frames for canvas and bake
/// loaded once when the sticker is placed
/// thread safe since bake and video compositor read it off main
pub struct EditorStickerArtStore {
    lock: Mutex<EditorStickerArtStoreState>,
    bake_lane: Mutex<Option<mpsc::UnboundedSender<EditorStickerArtBakeRequest>>>,
    main_thread: Mutex<Option<ThreadId>>,
}

static SHARED_ART_STORE: LazyLock<EditorStickerArtStore> = LazyLock::new(|| EditorStickerArtStore {
    lock: Mutex::new(EditorStickerArtStoreState::default()),
    bake_lane: Mutex::new(None),
    main_thread: Mutex::new(None),
});

impl EditorStickerArtStore {
    const BAKE_DECODE_TIMEOUT: Duration = Duration::from_secs(20);

    pub fn shared() -> &'static Self {
        &SHARED_ART_STORE
    }

    fn max_frames() -> usize {
        usize::try_from(RiseLimits::client_value("media_editor_sticker_max_frames")).unwrap_or(1).max(1)
    }

    fn max_pixel_side() -> f32 {
        RiseLimits::client_value("media_editor_sticker_max_pixel_side") as f32
    }

    fn max_arts() -> usize {
        usize::try_from(RiseLimits::client_value("media_editor_sticker_art_cache_entries")).unwrap_or(0)
    }

    pub fn install(cx: &mut App) {
        let store = Self::shared();
        let mut lane = store.bake_lane.lock();
        if lane.is_some() {
            return;
        }
        *store.main_thread.lock() = Some(std::thread::current().id());
        let (sender, mut inbox) = mpsc::unbounded::<EditorStickerArtBakeRequest>();
        *lane = Some(sender);
        drop(lane);
        cx.spawn(async move |cx| {
            while let Some(request) = inbox.next().await {
                cx.spawn(async move |cx| {
                    let art = Self::decode(&request.entity, None, cx).await;
                    let _ = request.reply.send(art);
                })
                .detach();
            }
        })
        .detach();
    }

    pub fn art(&self, entity: &EditorStickerEntity) -> Option<EditorStickerArtStoreArt> {
        self.lock.lock().arts.get(&entity.art_key()).cloned()
    }

    fn is_main_thread(&self) -> bool {
        self.main_thread.lock().is_some_and(|id| id == std::thread::current().id())
    }

    /// art for export and bake
    /// a sticker still downloading, evicted or from a draft is decoded here on the bake thread
    /// so its never dropped, on main only the cache is read
    pub fn art_for_bake(&self, entity: &EditorStickerEntity) -> Option<EditorStickerArtStoreArt> {
        if let Some(ready) = self.art(entity) {
            return Some(ready);
        }
        if self.is_main_thread() {
            return None;
        }
        let sender = self.bake_lane.lock().clone();
        let (reply, done) = sync_channel(1);
        let sent = sender.is_some_and(|sender| {
            sender.unbounded_send(EditorStickerArtBakeRequest { entity: entity.clone(), reply }).is_ok()
        });
        let art = if sent { done.recv_timeout(Self::BAKE_DECODE_TIMEOUT).ok().flatten() } else { None };
        let Some(art) = art else {
            tracing::error!(
                target: "StickerBake",
                "[StickerBake] art missing for {} sticker, bake skips it",
                entity.asset_type
            );
            return None;
        };
        self.remember(&entity.art_key(), art.clone());
        Some(art)
    }

    fn remember(&self, key: &str, art: EditorStickerArtStoreArt) {
        let mut state = self.lock.lock();
        state.arts.insert(key.to_owned(), art);
        state.order.retain(|existing| existing != key);
        state.order.push(key.to_owned());
        let limit = Self::max_arts();
        while state.order.len() > limit {
            let dropped = state.order.remove(0);
            state.arts.remove(&dropped);
        }
    }

    pub fn load(
        &self,
        entity: &EditorStickerEntity,
        completion: impl Fn(Option<EditorStickerArtStoreArt>, &mut App) + 'static,
        cx: &mut App,
    ) {
        Self::install(cx);
        let key = entity.art_key();
        let ready = self.lock.lock().arts.get(&key).cloned();
        if let Some(ready) = ready {
            completion(Some(ready), cx);
            return;
        }
        let already_loading = ART_WAITERS.with(|waiters| {
            let mut waiters = waiters.borrow_mut();
            let already_loading = waiters.contains_key(&key);
            waiters.entry(key.clone()).or_default().push(Rc::new(completion));
            already_loading
        });
        if already_loading {
            return;
        }
        let snapshot = entity.clone();
        cx.spawn(async move |cx| {
            let preview_key = key.clone();
            let preview = move |first: EditorStickerArtStoreArt, cx: &AsyncApp| {
                cx.update(|cx| Self::deliver_preview(&preview_key, first, cx));
            };
            let art = Self::decode(&snapshot, Some(&preview), cx).await;
            if let Some(art) = &art {
                Self::shared().remember(&key, art.clone());
            }
            let pending = ART_WAITERS.with(|waiters| waiters.borrow_mut().remove(&key).unwrap_or_default());
            cx.update(|cx| {
                for waiter in pending {
                    waiter(art.clone(), cx);
                }
            });
        })
        .detach();
    }

    fn deliver_preview(key: &str, art: EditorStickerArtStoreArt, cx: &mut App) {
        let pending = ART_WAITERS.with(|waiters| waiters.borrow().get(key).cloned().unwrap_or_default());
        for waiter in pending {
            waiter(Some(art.clone()), cx);
        }
    }

    async fn decode(
        entity: &EditorStickerEntity,
        preview: Option<EditorStickerArtPreview<'_>>,
        cx: &AsyncApp,
    ) -> Option<EditorStickerArtStoreArt> {
        let background = cx.background_executor().clone();
        if let Some(glyph) = entity.emoji.clone() {
            return background.spawn(async move { Self::render_emoji(&glyph) }).await;
        }
        let kind = entity.kind();
        if kind == StickerAssetType::Lottie {
            if let Some(art) = Self::decode_lottie(&entity.file_url, preview, cx).await {
                return Some(art);
            }
            let thumb = entity.thumbnail_url.as_deref()?;
            let thumb_file = Self::local_file(thumb, cx).await?;
            return background.spawn(async move { Self::decode_image(&thumb_file, 1) }).await;
        }
        let source = entity.file_url.clone();
        let Some(file) = Self::local_file(&source, cx).await else {
            let thumb = entity.thumbnail_url.as_deref()?;
            let thumb_file = Self::local_file(thumb, cx).await?;
            return background.spawn(async move { Self::decode_image(&thumb_file, 1) }).await;
        };
        let limit = if entity.loops { Self::max_frames() } else { 1 };
        let is_web_m = WebMStickerSupport::is_web_m(&source)
            || entity.mime_type.as_deref().is_some_and(|mime| mime.to_lowercase().contains("webm"));
        if !is_web_m && kind != StickerAssetType::Video {
            return background.spawn(async move { Self::decode_image(&file, limit) }).await;
        }
        let decoded = if is_web_m {
            background.spawn(async move { Self::decode_web_m(&file, limit) }).await
        } else {
            background.spawn(async move { Self::decode_video(&file, limit) }).await
        };
        if decoded.is_some() {
            return decoded;
        }
        let thumb = entity.thumbnail_url.as_deref()?;
        let thumb_file = Self::local_file(thumb, cx).await?;
        background.spawn(async move { Self::decode_image(&thumb_file, 1) }).await
    }

    fn decode_web_m(file: &Path, limit: usize) -> Option<EditorStickerArtStoreArt> {
        let data = std::fs::read(file).ok()?;
        let track = WebMStickerDemuxer::parse(&data)?;
        let decoded = WebMStickerFrameDecoder::decode(&track, limit);
        let last_time_ms = decoded.last().map_or(0, |frame| frame.time_ms);
        let mut frames = Vec::with_capacity(decoded.len());
        let mut times = Vec::with_capacity(decoded.len());
        for frame in &decoded {
            let image = &frame.image;
            if !image.rows_fit() {
                continue;
            }
            let row_bytes = image.width as usize * 4;
            let mut packed = Vec::with_capacity(row_bytes * image.height as usize);
            for row in 0..image.height as usize {
                let start = row * image.stride as usize;
                let Some(bytes) = image.pixels.get(start..start + row_bytes) else {
                    break;
                };
                packed.extend_from_slice(bytes);
            }
            let Some(bitmap) = RgbaImage::from_raw(image.width, image.height, packed) else {
                continue;
            };
            frames.push(bgra_render_image(Self::downscaled(bitmap)));
            times.push(frame.time_ms as f64 / 1000.0);
        }
        if frames.is_empty() {
            return None;
        }
        Some(EditorStickerArtStoreArt {
            frames: Arc::from(frames),
            times: Arc::from(times),
            duration: track.duration_ms.max(last_time_ms) as f64 / 1000.0,
        })
    }

    async fn decode_lottie(
        source: &str,
        preview: Option<EditorStickerArtPreview<'_>>,
        cx: &AsyncApp,
    ) -> Option<EditorStickerArtStoreArt> {
        let Ok(LottieStickerPayload::Animation(render)) = LottieStickerLoader::shared().payload(source).await
        else {
            return None;
        };
        let key = RiseLottieSequenceKey {
            cache_key: format!("media-editor|{}", render.cache_key),
            dimension: Self::max_pixel_side() as i64,
            mirrored: false,
        };
        let sequence = RiseLottieFrameSequence::make(key, render.json_data.clone(), cx).await?;
        let _teardown = EditorStickerLottieTeardown(Arc::clone(&sequence));
        let first_visible = sequence.first_visible_frame_index().await?;
        let total = sequence.frame_count;
        if total <= 0 {
            return None;
        }
        let count = total.min(i64::try_from(Self::max_frames()).unwrap_or(1).max(1));
        let stride = total as f64 / count as f64;
        let mut frames: Vec<Arc<RenderImage>> = Vec::new();
        let mut times: Vec<f64> = Vec::new();
        for step in 0..count {
            if step == 1
                && let (Some(preview), Some(first)) = (preview, frames.first())
            {
                preview(EditorStickerArtStoreArt::single(Arc::clone(first)), cx);
            }
            let index = (first_visible + (step as f64 * stride).floor() as i64) % total;
            let (sender, receiver) = oneshot::channel::<Option<Arc<RenderImage>>>();
            sequence.request_frame(
                index,
                Arc::new(|| true),
                Some(Box::new(move |image: Option<Arc<RenderImage>>| {
                    let _ = sender.send(image);
                })),
            );
            let Ok(Some(image)) = receiver.await else {
                continue;
            };
            frames.push(image);
            times.push(step as f64 * stride / sequence.frame_rate);
        }
        if frames.is_empty() {
            return None;
        }
        let duration = if frames.len() > 1 { total as f64 / sequence.frame_rate } else { 0.0 };
        Some(EditorStickerArtStoreArt { frames: Arc::from(frames), times: Arc::from(times), duration })
    }

    async fn local_file(source: &str, cx: &AsyncApp) -> Option<PathBuf> {
        let trimmed = trimming_whitespaces_and_newlines(source);
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.starts_with('/') {
            return Some(PathBuf::from(trimmed));
        }
        let url = Url::from_string(trimmed)?;
        if url.is_file_url() {
            return url.to_file_path();
        }
        let scheme = url.scheme()?.to_lowercase();
        if scheme != "https" && scheme != "http" {
            return None;
        }
        let options = RemoteMediaDiskCacheMaterializeOptions {
            cache_key: Some(RiseMediaCacheIdentity::key(&url)),
            suggested_filename: Some(url.last_path_component()),
            ..Default::default()
        };
        RemoteMediaDiskCache::shared().ensure_cached(&url, options, cx).await
    }

    fn decode_image(file: &Path, max_frames: usize) -> Option<EditorStickerArtStoreArt> {
        let data = std::fs::read(file).ok()?;
        let format = ImageReader::new(Cursor::new(&data)).with_guessed_format().ok()?.format();
        let animated = match format {
            Some(ImageFormat::Gif) => {
                Self::decode_animation(|| GifDecoder::new(Cursor::new(&data)).ok(), max_frames)
            }
            Some(ImageFormat::WebP) => {
                let has_animation =
                    WebPDecoder::new(Cursor::new(&data)).ok().is_some_and(|decoder| decoder.has_animation());
                has_animation
                    .then(|| Self::decode_animation(|| WebPDecoder::new(Cursor::new(&data)).ok(), max_frames))
                    .flatten()
            }
            Some(ImageFormat::Png) => {
                let is_apng = PngDecoder::new(Cursor::new(&data))
                    .ok()
                    .is_some_and(|decoder| decoder.is_apng().unwrap_or(false));
                is_apng
                    .then(|| {
                        Self::decode_animation(
                            || PngDecoder::new(Cursor::new(&data)).ok()?.apng().ok(),
                            max_frames,
                        )
                    })
                    .flatten()
            }
            _ => None,
        };
        if animated.is_some() {
            return animated;
        }
        let still = CleverImageBitmapDecoder::decode(&data, Some(Self::max_pixel_side() as i64))?;
        Some(EditorStickerArtStoreArt::single(still))
    }

    /// frames of an animated source, every stride-th one up to max_frames
    /// each starts where the delays of the frames before it end
    fn decode_animation<'a, D: AnimationDecoder<'a>>(
        make: impl Fn() -> Option<D>,
        max_frames: usize,
    ) -> Option<EditorStickerArtStoreArt> {
        let delays: Vec<f64> =
            make()?.into_frames().map_while(Result::ok).map(|frame| Self::frame_delay(&frame)).collect();
        let count = delays.len().max(1);
        let stride = count.div_ceil(max_frames.max(1)).max(1);
        let max_side = Self::max_pixel_side().max(1.0) as u32;
        let mut frames: Vec<Arc<RenderImage>> = Vec::new();
        let mut times: Vec<f64> = Vec::new();
        let mut clock = 0.0;
        for (index, frame) in make()?.into_frames().map_while(Result::ok).enumerate() {
            let delay = delays.get(index).copied().unwrap_or(0.1);
            if index % stride == 0 && frames.len() < max_frames {
                let mut image = DynamicImage::ImageRgba8(frame.into_buffer());
                if image.width().max(image.height()) > max_side {
                    image = image.thumbnail(max_side, max_side);
                }
                frames.push(CleverImageBitmapDecoder::render_image(image.into_rgba8()));
                times.push(clock);
            }
            clock += delay;
        }
        if frames.is_empty() {
            return None;
        }
        let duration = if frames.len() > 1 { clock } else { 0.0 };
        Some(EditorStickerArtStoreArt { frames: Arc::from(frames), times: Arc::from(times), duration })
    }

    fn frame_delay(frame: &Frame) -> f64 {
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        if denominator == 0 {
            return 0.1;
        }
        let value = f64::from(numerator) / f64::from(denominator) / 1000.0;
        if value > 0.011 { value } else { 0.1 }
    }

    fn decode_video(file: &Path, max_frames: usize) -> Option<EditorStickerArtStoreArt> {
        let length = video_export::probe(file).map_or(0.0, |probe| probe.duration_seconds);
        let length = if length.is_finite() { length.max(0.0) } else { 0.0 };
        let count = if length > 0.0 { max_frames.min(((length * 16.0) as usize).max(1)) } else { 1 };
        let step = if count > 1 { length / count as f64 } else { 0.0 };
        let max_side = Self::max_pixel_side().max(1.0) as u32;
        let mut frames: Vec<Arc<RenderImage>> = Vec::new();
        let mut times: Vec<f64> = Vec::new();
        for index in 0..count {
            let at = index as f64 * step;
            let Ok(poster) = video_export::poster(file, at, max_side) else {
                continue;
            };
            let Some(bitmap) = RgbaImage::from_raw(poster.width, poster.height, poster.bgra) else {
                continue;
            };
            frames.push(bgra_render_image(bitmap));
            times.push(at);
        }
        if frames.is_empty() {
            return None;
        }
        let duration = if frames.len() > 1 { length } else { 0.0 };
        Some(EditorStickerArtStoreArt { frames: Arc::from(frames), times: Arc::from(times), duration })
    }

    /// glyph centered in a transparent square of the sticker pixel side
    fn render_emoji(glyph: &str) -> Option<EditorStickerArtStoreArt> {
        let side = Self::max_pixel_side().max(1.0);
        let font_size = side * 0.8;
        let mut raster = emoji_raster::render_emoji(glyph, font_size).ok()?;
        let fit = 1.0_f32
            .min(side / (raster.width as f32).max(1.0))
            .min(side / (raster.height as f32).max(1.0));
        if fit < 1.0 {
            raster = emoji_raster::render_emoji(glyph, font_size * fit).ok()?;
        }
        let glyph_bitmap = RgbaImage::from_raw(raster.width, raster.height, raster.rgba)?;
        let pixel_side = side as u32;
        let mut square = RgbaImage::new(pixel_side, pixel_side);
        let x = (i64::from(pixel_side) - i64::from(glyph_bitmap.width())) / 2;
        let y = (i64::from(pixel_side) - i64::from(glyph_bitmap.height())) / 2;
        imageops::overlay(&mut square, &glyph_bitmap, x, y);
        Some(EditorStickerArtStoreArt::single(CleverImageBitmapDecoder::render_image(square)))
    }

    fn downscaled(image: RgbaImage) -> RgbaImage {
        let side = image.width().max(image.height()) as f32;
        let max_side = Self::max_pixel_side();
        if side <= max_side {
            return image;
        }
        let factor = max_side / side;
        let width = ((image.width() as f32 * factor).round() as u32).max(1);
        let height = ((image.height() as f32 * factor).round() as u32).max(1);
        imageops::resize(&image, width, height, FilterType::CatmullRom)
    }
}

struct EditorStickerLottieTeardown(Arc<RiseLottieFrameSequence>);

impl Drop for EditorStickerLottieTeardown {
    fn drop(&mut self) {
        self.0.teardown();
    }
}

/// wraps rows that are already bgra as the render image gpui draws
fn bgra_render_image(bitmap: RgbaImage) -> Arc<RenderImage> {
    let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(bitmap)]);
    Arc::new(RenderImage::new(frames))
}

pub enum EditorStickerRenderer {}

impl EditorStickerRenderer {
    /// draws the first frame of every loaded sticker into the straight rgba bitmap of this size
    pub fn draw_still(stickers: &[EditorStickerEntity], size: Size<f32>, ctx: &mut RgbaImage) {
        for entity in stickers {
            let Some(art) = EditorStickerArtStore::shared().art_for_bake(entity) else {
                continue;
            };
            let Some(frame) = art.frames.first() else {
                continue;
            };
            Self::draw(entity, frame, size, ctx);
        }
    }

    /// draws the frame into its box around the entity center
    /// turned and scaled, order is translate then rotate then scale
    pub fn draw(entity: &EditorStickerEntity, frame: &RenderImage, size: Size<f32>, ctx: &mut RgbaImage) {
        let box_size = EditorStickerEntity::box_size(entity.aspect_ratio, size);
        let frame_size = frame.size(0);
        let (frame_width, frame_height) = (frame_size.width.0.max(0) as u32, frame_size.height.0.max(0) as u32);
        let Some(bytes) = frame.as_bytes(0) else {
            return;
        };
        if frame_width == 0 || frame_height == 0 || bytes.len() < (frame_width * frame_height * 4) as usize {
            return;
        }
        let forward = Affine::scale(
            f64::from(box_size.width) / f64::from(frame_width),
            f64::from(box_size.height) / f64::from(frame_height),
        )
        .then(Affine::translation(-f64::from(box_size.width) / 2.0, -f64::from(box_size.height) / 2.0))
        .then(Affine::scale(f64::from(entity.scale), f64::from(entity.scale)))
        .then(Affine::rotation(f64::from(entity.rotation)))
        .then(Affine::translation(
            f64::from(entity.center.x * size.width),
            f64::from(entity.center.y * size.height),
        ));
        let Some(inverse) = forward.inverted() else {
            return;
        };
        let corners = [
            forward.apply(0.0, 0.0),
            forward.apply(f64::from(frame_width), 0.0),
            forward.apply(0.0, f64::from(frame_height)),
            forward.apply(f64::from(frame_width), f64::from(frame_height)),
        ];
        let min_x = corners.iter().map(|corner| corner.0).fold(f64::INFINITY, f64::min).floor().max(0.0);
        let min_y = corners.iter().map(|corner| corner.1).fold(f64::INFINITY, f64::min).floor().max(0.0);
        let max_x = corners.iter().map(|corner| corner.0).fold(f64::NEG_INFINITY, f64::max).ceil();
        let max_y = corners.iter().map(|corner| corner.1).fold(f64::NEG_INFINITY, f64::max).ceil();
        let max_x = max_x.min(f64::from(ctx.width()));
        let max_y = max_y.min(f64::from(ctx.height()));
        if !(min_x < max_x && min_y < max_y) {
            return;
        }
        for y in min_y as u32..max_y as u32 {
            for x in min_x as u32..max_x as u32 {
                let (u, v) = inverse.apply(f64::from(x) + 0.5, f64::from(y) + 0.5);
                let Some(source) = sample_bgra_premultiplied(bytes, frame_width, frame_height, u - 0.5, v - 0.5)
                else {
                    continue;
                };
                blend_premultiplied_over(ctx.get_pixel_mut(x, y), source);
            }
        }
    }
}

fn sample_bgra_premultiplied(bytes: &[u8], width: u32, height: u32, x: f64, y: f64) -> Option<[f64; 4]> {
    if x < -1.0 || y < -1.0 || x > f64::from(width) || y > f64::from(height) {
        return None;
    }
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let texel = |tx: f64, ty: f64| -> [f64; 4] {
        if tx < 0.0 || ty < 0.0 || tx >= f64::from(width) || ty >= f64::from(height) {
            return [0.0; 4];
        }
        let offset = ((ty as usize) * width as usize + tx as usize) * 4;
        let alpha = f64::from(bytes[offset + 3]) / 255.0;
        [
            f64::from(bytes[offset + 2]) / 255.0 * alpha,
            f64::from(bytes[offset + 1]) / 255.0 * alpha,
            f64::from(bytes[offset]) / 255.0 * alpha,
            alpha,
        ]
    };
    let top_left = texel(x0, y0);
    let top_right = texel(x0 + 1.0, y0);
    let bottom_left = texel(x0, y0 + 1.0);
    let bottom_right = texel(x0 + 1.0, y0 + 1.0);
    let mut out = [0.0; 4];
    for channel in 0..4 {
        let top = top_left[channel] + (top_right[channel] - top_left[channel]) * fx;
        let bottom = bottom_left[channel] + (bottom_right[channel] - bottom_left[channel]) * fx;
        out[channel] = top + (bottom - top) * fy;
    }
    (out[3] > 0.0).then_some(out)
}

fn blend_premultiplied_over(pixel: &mut image::Rgba<u8>, source: [f64; 4]) {
    let [r, g, b, a] = pixel.0;
    let destination_alpha = f64::from(a) / 255.0;
    let keep = 1.0 - source[3];
    let out_alpha = source[3] + destination_alpha * keep;
    if out_alpha <= 0.0 {
        return;
    }
    let channel = |source_channel: f64, destination: u8| -> u8 {
        let premultiplied = source_channel + f64::from(destination) / 255.0 * destination_alpha * keep;
        ((premultiplied / out_alpha).clamp(0.0, 1.0) * 255.0).round() as u8
    };
    pixel.0 = [
        channel(source[0], r),
        channel(source[1], g),
        channel(source[2], b),
        (out_alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    ];
}

/// one frame of a loop layer with its placement in the render rect
#[derive(Clone)]
pub struct EditorStickerRendererPlacedFrame {
    pub image: Arc<RenderImage>,
    pub placement: Affine,
}

#[derive(Clone)]
pub struct EditorStickerRendererLoopLayer {
    pub entity_id: Uuid,
    pub art: EditorStickerArtStoreArt,
    pub frames: Vec<EditorStickerRendererPlacedFrame>,
}

impl EditorStickerRendererLoopLayer {
    pub fn new(entity: &EditorStickerEntity, render_size: Size<f32>) -> Option<Self> {
        let art = EditorStickerArtStore::shared().art_for_bake(entity)?;
        if !art.is_animated() {
            return None;
        }
        let box_size = EditorStickerEntity::box_size(entity.aspect_ratio, render_size);
        let frames = art
            .frames
            .iter()
            .map(|frame| {
                let pixels = frame.size(0);
                let width = f64::from(pixels.width.0.max(1));
                let height = f64::from(pixels.height.0.max(1));
                let placement = Affine::scale(f64::from(box_size.width) / width, f64::from(box_size.height) / height)
                    .then(Affine::translation(-f64::from(box_size.width) / 2.0, -f64::from(box_size.height) / 2.0))
                    .then(Affine::rotation(f64::from(entity.rotation)))
                    .then(Affine::scale(f64::from(entity.scale), f64::from(entity.scale)))
                    .then(Affine::translation(
                        f64::from(entity.center.x * render_size.width),
                        f64::from(entity.center.y * render_size.height),
                    ));
                EditorStickerRendererPlacedFrame { image: Arc::clone(frame), placement }
            })
            .collect();
        Some(Self { entity_id: entity.id, art, frames })
    }

    pub fn image(&self, seconds: f64) -> Option<&EditorStickerRendererPlacedFrame> {
        if self.frames.is_empty() {
            return None;
        }
        self.frames.get(self.art.frame_index(seconds).min(self.frames.len() - 1))
    }
}

pub enum EditorStickerEntityViewMetrics {}

impl EditorStickerEntityViewMetrics {
    pub const SELECTION_PADDING: f32 = 8.0;
    pub const HANDLE_VISUAL_SIZE: f32 = 12.0;
    pub const HANDLE_HIT_OUTSET: f32 = 22.0;
}

const PAN_MINIMUM_DISTANCE: f32 = 4.0;

pub type EditorStickerEntityUpdate = Rc<dyn Fn(EditorStickerEntity, &mut App)>;
pub type EditorStickerEntityAction = Rc<dyn Fn(Uuid, &mut App)>;
pub type EditorStickerEntitySnapGuides = Rc<dyn Fn(bool, bool, &mut App)>;

/// one left button press from down to up
#[derive(Clone, Copy)]
enum EditorStickerEntityPress {
    Body { press: Point<Pixels>, began_at: Option<Point<Pixels>>, can_pan: bool },
    SideHandle { sign: f32, press: Point<Pixels>, began_at: Option<Point<Pixels>> },
}

pub struct EditorStickerEntityView {
    pub entity_id: Uuid,

    current: Option<EditorStickerEntity>,
    shown_art_key: Option<String>,
    shown_art: Option<EditorStickerArtStoreArt>,
    art_started_at: Duration,
    frame_ticker: Option<Task<()>>,
    live_lottie_painted: bool,
    canvas_size: Size<f32>,
    is_selected: bool,
    handles_visible: bool,
    allows_body_drag: bool,

    move_start_center: Point<f32>,
    side_start_scale: f32,
    side_start_rotation: f32,
    side_start_handle_offset: Point<f32>,
    side_start_center: Point<f32>,
    is_side_handle_dragging: bool,
    two_finger_start_scale: f32,
    two_finger_start_rotation: f32,

    pub on_update: Option<EditorStickerEntityUpdate>,
    pub on_select: Option<EditorStickerEntityAction>,
    pub on_request_delete: Option<EditorStickerEntityAction>,
    pub on_snap_guides_changed: Option<EditorStickerEntitySnapGuides>,

    snap_feedback: EditorEntitySnapFeedback,
    press: Option<EditorStickerEntityPress>,
    canvas_origin: Rc<Cell<Point<Pixels>>>,
}

impl EditorStickerEntityView {
    pub fn new(entity_id: Uuid, _cx: &mut Context<Self>) -> Self {
        Self {
            entity_id,
            current: None,
            shown_art_key: None,
            shown_art: None,
            art_started_at: Duration::ZERO,
            frame_ticker: None,
            live_lottie_painted: false,
            canvas_size: Size::default(),
            is_selected: false,
            handles_visible: false,
            allows_body_drag: true,
            move_start_center: Point::default(),
            side_start_scale: 1.0,
            side_start_rotation: 0.0,
            side_start_handle_offset: Point::default(),
            side_start_center: Point::default(),
            is_side_handle_dragging: false,
            two_finger_start_scale: 1.0,
            two_finger_start_rotation: 0.0,
            on_update: None,
            on_select: None,
            on_request_delete: None,
            on_snap_guides_changed: None,
            snap_feedback: EditorEntitySnapFeedback::new(),
            press: None,
            canvas_origin: Rc::new(Cell::new(Point::default())),
        }
    }

    pub fn set_interaction_state(&mut self, handles_visible: bool, allows_body_drag: bool, cx: &mut Context<Self>) {
        self.handles_visible = handles_visible;
        self.allows_body_drag = allows_body_drag;
        if !handles_visible {
            self.is_side_handle_dragging = false;
        }
        cx.notify();
    }

    pub fn apply(
        &mut self,
        entity: &EditorStickerEntity,
        canvas_size: Size<f32>,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) {
        self.canvas_size = canvas_size;
        self.current = Some(entity.clone());
        self.is_selected = is_selected;
        self.show_art_if_needed(entity, cx);
        cx.notify();
    }

    fn show_art_if_needed(&mut self, entity: &EditorStickerEntity, cx: &mut Context<Self>) {
        let shown_loops = entity.loops || entity.is_video_art();
        let key = EditorStickerEntity::art_key_for(&entity.sticker_id, shown_loops);
        if self.shown_art_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.shown_art_key = Some(key.clone());
        self.live_lottie_painted = false;
        let view = cx.weak_entity();
        let mut shown = entity.clone();
        shown.loops = shown_loops;
        EditorStickerArtStore::shared().load(&shown, move |art, cx| {
            let Some(art) = art else {
                return;
            };
            let view = view.clone();
            let key = key.clone();
            cx.defer(move |cx| {
                let _ = view.update(cx, |view, cx| {
                    if view.shown_art_key.as_deref() == Some(key.as_str()) {
                        view.show(art, cx);
                    }
                });
            });
        }, cx);
    }

    fn show(&mut self, art: EditorStickerArtStoreArt, cx: &mut Context<Self>) {
        self.frame_ticker = None;
        self.art_started_at = RiseClock::monotonic();
        let animated = art.is_animated();
        self.shown_art = Some(art.clone());
        cx.notify();
        if !animated {
            return;
        }
        let started_at = self.art_started_at;
        self.frame_ticker = Some(cx.spawn(async move |view, cx| {
            loop {
                let elapsed = RiseClock::monotonic().saturating_sub(started_at).as_secs_f64();
                let local = elapsed % art.duration;
                let next = art.times.iter().copied().find(|start| *start > local).unwrap_or(art.duration);
                let wait = Duration::from_secs_f64((next - local).max(0.001));
                cx.background_executor().timer(wait).await;
                if view.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        }));
    }

    fn shown_frame(&self) -> Option<Arc<RenderImage>> {
        let art = self.shown_art.as_ref()?;
        let elapsed = RiseClock::monotonic().saturating_sub(self.art_started_at).as_secs_f64();
        art.frames.get(art.frame_index(elapsed)).cloned()
    }

    fn plays_live_lottie(entity: &EditorStickerEntity) -> bool {
        entity.emoji.is_none() && entity.kind() == StickerAssetType::Lottie
    }

    /// entity center in canvas points
    fn center(&self) -> Point<f32> {
        self.current.as_ref().map_or(Point::default(), |entity| {
            point(entity.center.x * self.canvas_size.width, entity.center.y * self.canvas_size.height)
        })
    }

    /// unscaled box of the art
    fn bounds(&self) -> Size<f32> {
        self.current
            .as_ref()
            .map_or(Size::default(), |entity| EditorStickerEntity::box_size(entity.aspect_ratio, self.canvas_size))
    }

    /// selection rect in the views own points
    fn selection_rect(&self) -> Bounds<f32> {
        let pad = EditorStickerEntityViewMetrics::SELECTION_PADDING;
        let bounds = self.bounds();
        Bounds::new(point(-pad, -pad), size(bounds.width + pad * 2.0, bounds.height + pad * 2.0))
    }

    /// frame of the left or right handle
    fn handle_frame(&self, sign: f32) -> Bounds<f32> {
        let selection = self.selection_rect();
        let dot = EditorStickerEntityViewMetrics::HANDLE_VISUAL_SIZE;
        let x = if sign > 0.0 { selection.origin.x + selection.size.width } else { selection.origin.x };
        Bounds::new(
            point(x - dot / 2.0, selection.origin.y + selection.size.height / 2.0 - dot / 2.0),
            size(dot, dot),
        )
    }

    fn handle_hit_rect(&self, sign: f32) -> Bounds<f32> {
        let frame = self.handle_frame(sign);
        let outset = EditorStickerEntityViewMetrics::HANDLE_HIT_OUTSET;
        Bounds::new(
            point(frame.origin.x - outset, frame.origin.y - outset),
            size(frame.size.width + outset * 2.0, frame.size.height + outset * 2.0),
        )
    }

    /// hit test, point is in the views own points
    pub fn point_inside(&self, point: Point<f32>) -> bool {
        if self.handles_visible
            && (rect_contains(self.handle_hit_rect(-1.0), point) || rect_contains(self.handle_hit_rect(1.0), point))
        {
            return true;
        }
        rect_contains(self.selection_rect(), point)
    }

    pub fn contains_canvas_point(&self, canvas_point: Point<f32>) -> bool {
        self.point_inside(self.local_point(canvas_point))
    }

    /// undoes the view transform, scale and rotation around its center
    fn local_point(&self, canvas_point: Point<f32>) -> Point<f32> {
        let center = self.center();
        let bounds = self.bounds();
        let (scale, rotation) = self.current.as_ref().map_or((1.0, 0.0), |entity| (entity.scale, entity.rotation));
        let dx = canvas_point.x - center.x;
        let dy = canvas_point.y - center.y;
        let (sin, cos) = rotation.sin_cos();
        let scale = scale.max(f32::EPSILON);
        point((dx * cos + dy * sin) / scale + bounds.width / 2.0, (-dx * sin + dy * cos) / scale + bounds.height / 2.0)
    }

    /// converts a local point to canvas coordinates
    fn canvas_point(&self, local: Point<f32>) -> Point<f32> {
        let center = self.center();
        let bounds = self.bounds();
        let (scale, rotation) = self.current.as_ref().map_or((1.0, 0.0), |entity| (entity.scale, entity.rotation));
        let dx = (local.x - bounds.width / 2.0) * scale;
        let dy = (local.y - bounds.height / 2.0) * scale;
        let (sin, cos) = rotation.sin_cos();
        point(center.x + dx * cos - dy * sin, center.y + dx * sin + dy * cos)
    }

    fn window_to_canvas(&self, position: Point<Pixels>) -> Point<f32> {
        let origin = self.canvas_origin.get();
        point(f32::from(position.x - origin.x), f32::from(position.y - origin.y))
    }

    fn handle_tap(this: &Entity<Self>, cx: &mut App) {
        let (entity_id, on_select) = {
            let view = this.read(cx);
            (view.entity_id, view.on_select.clone())
        };
        if let Some(on_select) = on_select {
            on_select(entity_id, cx);
        }
    }

    fn handle_double_tap(this: &Entity<Self>, cx: &mut App) {
        Self::handle_tap(this, cx);
    }

    fn edit_menu_items(&self) -> Vec<ContextMenuItem> {
        let entity_id = self.entity_id;
        let on_request_delete = self.on_request_delete.clone();
        vec![
            ContextMenuItem::new(0, tr("editor_text_delete"), None, "delete", move |_, cx| {
                if let Some(on_request_delete) = &on_request_delete {
                    on_request_delete(entity_id, cx);
                }
            })
            .destructive(true),
        ]
    }

    fn handle_pan_began(this: &Entity<Self>, cx: &mut App) {
        let began = this.update(cx, |view, _| {
            if !view.allows_body_drag || view.is_side_handle_dragging || view.current.is_none() {
                return false;
            }
            view.move_start_center = view.center();
            view.snap_feedback.begin();
            true
        });
        if began {
            Self::handle_tap(this, cx);
        }
    }

    fn handle_pan_changed(this: &Entity<Self>, translation: Point<f32>, cx: &mut App) {
        let outcome = this.update(cx, |view, cx| {
            if !view.allows_body_drag || view.is_side_handle_dragging {
                return None;
            }
            let mut entity = view.current.clone()?;
            let proposed =
                point(view.move_start_center.x + translation.x, view.move_start_center.y + translation.y);
            let (snapped, snapped_x, snapped_y) = EditorEntitySnap::snap_center(proposed, view.canvas_size);
            view.snap_feedback.update_center(snapped_x, snapped_y);
            let updated = (view.canvas_size.width > 0.0 && view.canvas_size.height > 0.0).then(|| {
                entity.center = point(
                    (snapped.x / view.canvas_size.width).clamp(0.0, 1.0),
                    (snapped.y / view.canvas_size.height).clamp(0.0, 1.0),
                );
                view.current = Some(entity.clone());
                cx.notify();
                entity
            });
            Some((view.on_snap_guides_changed.clone(), snapped_x, snapped_y, view.on_update.clone(), updated))
        });
        let Some((on_snap_guides_changed, snapped_x, snapped_y, on_update, updated)) = outcome else {
            return;
        };
        if let Some(on_snap_guides_changed) = on_snap_guides_changed {
            on_snap_guides_changed(snapped_x, snapped_y, cx);
        }
        if let (Some(on_update), Some(updated)) = (on_update, updated) {
            on_update(updated, cx);
        }
    }

    fn handle_pan_ended(this: &Entity<Self>, cx: &mut App) {
        let view = this.read(cx);
        if !view.allows_body_drag || view.is_side_handle_dragging || view.current.is_none() {
            return;
        }
        if let Some(on_snap_guides_changed) = view.on_snap_guides_changed.clone() {
            on_snap_guides_changed(false, false, cx);
        }
    }

    fn apply_side_handle_began(this: &Entity<Self>, sign: f32, cx: &mut App) {
        let began = this.update(cx, |view, _| {
            let Some((scale, rotation)) = view.current.as_ref().map(|entity| (entity.scale, entity.rotation)) else {
                return false;
            };
            view.is_side_handle_dragging = true;
            view.side_start_scale = scale;
            view.side_start_rotation = rotation;
            let center = view.center();
            view.side_start_center = center;
            let handle = view.handle_frame(sign);
            let handle_center = view.canvas_point(point(
                handle.origin.x + handle.size.width / 2.0,
                handle.origin.y + handle.size.height / 2.0,
            ));
            view.side_start_handle_offset = point(handle_center.x - center.x, handle_center.y - center.y);
            view.snap_feedback.begin();
            true
        });
        if began {
            Self::handle_tap(this, cx);
        }
    }

    fn apply_side_handle_changed(this: &Entity<Self>, translation: Point<f32>, cx: &mut App) {
        let outcome = this.update(cx, |view, cx| {
            let mut entity = view.current.clone()?;
            let (scale, rotation) = EditorEntitySideHandle::scale_and_rotation(
                view.side_start_handle_offset,
                translation,
                view.side_start_scale,
                view.side_start_rotation,
            );
            let (angle, snapped) = EditorEntitySnap::snap_rotation(rotation);
            view.snap_feedback.update_rotation(snapped);
            entity.rotation = angle;
            entity.scale = scale;
            view.current = Some(entity.clone());
            cx.notify();
            Some((view.on_update.clone(), entity))
        });
        if let Some((Some(on_update), updated)) = outcome {
            on_update(updated, cx);
        }
    }

    fn handle_mouse_down(this: &Entity<Self>, event: &MouseDownEvent, cx: &mut App) {
        let view = this.read(cx);
        let local = view.local_point(view.window_to_canvas(event.position));
        if !view.point_inside(local) {
            return;
        }
        let handle_sign = view
            .handles_visible
            .then(|| [-1.0_f32, 1.0].into_iter().find(|sign| rect_contains(view.handle_frame(*sign), local)))
            .flatten();
        let can_pan = !view.is_side_handle_dragging
            && !(view.handles_visible
                && (rect_contains(view.handle_hit_rect(-1.0), local)
                    || rect_contains(view.handle_hit_rect(1.0), local)));
        cx.stop_propagation();

        if event.click_count >= 2 {
            Self::handle_double_tap(this, cx);
            return;
        }
        let press = match handle_sign {
            Some(sign) => EditorStickerEntityPress::SideHandle { sign, press: event.position, began_at: None },
            None => EditorStickerEntityPress::Body { press: event.position, began_at: None, can_pan },
        };
        this.update(cx, |view, _| view.press = Some(press));
    }

    fn handle_mouse_move(this: &Entity<Self>, event: &MouseMoveEvent, cx: &mut App) {
        let Some(press) = this.read(cx).press else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            Self::finish_press(this, press, cx);
            return;
        }
        match press {
            EditorStickerEntityPress::Body { press, began_at, can_pan } => {
                if !can_pan {
                    return;
                }
                let began_at = match began_at {
                    Some(began_at) => began_at,
                    None => {
                        if !past_pan_distance(press, event.position) {
                            return;
                        }
                        let began_at = event.position;
                        this.update(cx, |view, _| {
                            view.press =
                                Some(EditorStickerEntityPress::Body { press, began_at: Some(began_at), can_pan });
                        });
                        Self::handle_pan_began(this, cx);
                        began_at
                    }
                };
                Self::handle_pan_changed(this, translation_of(began_at, event.position), cx);
            }
            EditorStickerEntityPress::SideHandle { sign, press, began_at } => {
                let began_at = match began_at {
                    Some(began_at) => began_at,
                    None => {
                        if !past_pan_distance(press, event.position) {
                            return;
                        }
                        let began_at = event.position;
                        this.update(cx, |view, _| {
                            view.press =
                                Some(EditorStickerEntityPress::SideHandle { sign, press, began_at: Some(began_at) });
                        });
                        Self::apply_side_handle_began(this, sign, cx);
                        began_at
                    }
                };
                Self::apply_side_handle_changed(this, translation_of(began_at, event.position), cx);
            }
        }
    }

    fn finish_press(this: &Entity<Self>, press: EditorStickerEntityPress, cx: &mut App) {
        this.update(cx, |view, _| view.press = None);
        match press {
            EditorStickerEntityPress::Body { began_at: Some(_), .. } => {
                Self::handle_pan_ended(this, cx);
            }
            EditorStickerEntityPress::SideHandle { began_at: Some(_), .. } => {
                this.update(cx, |view, _| view.is_side_handle_dragging = false);
            }
            EditorStickerEntityPress::Body { began_at: None, .. }
            | EditorStickerEntityPress::SideHandle { began_at: None, .. } => {
                Self::handle_tap(this, cx);
            }
        }
    }

    /// takes the pointer on the turned shape of the view, not on the box gpui hit tests
    fn gesture_layer(
        this: Entity<Self>,
        hit_bounds: Bounds<Pixels>,
        canvas_origin: Rc<Cell<Point<Pixels>>>,
        canvas_origin_in_hit: Point<Pixels>,
    ) -> impl IntoElement {
        canvas(
            move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut App| {
                canvas_origin.set(bounds.origin - canvas_origin_in_hit);
                window.insert_hitbox(bounds, HitboxBehavior::Normal)
            },
            move |_: Bounds<Pixels>, hitbox: Hitbox, window: &mut Window, _: &mut App| {
                let down = this.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                        return;
                    }
                    match event.button {
                        MouseButton::Left => Self::handle_mouse_down(&down, event, cx),
                        MouseButton::Right => {
                            let view = down.read(cx);
                            let local = view.local_point(view.window_to_canvas(event.position));
                            if view.point_inside(local) {
                                Self::handle_tap(&down, cx);
                            } else {
                                cx.stop_propagation();
                            }
                        }
                        _ => {}
                    }
                });
                let moved = this.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        Self::handle_mouse_move(&moved, event, cx);
                    }
                });
                let up = this.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                        return;
                    }
                    if let Some(press) = up.read(cx).press {
                        Self::finish_press(&up, press, cx);
                    }
                });
            },
        )
        .absolute()
        .left_0()
        .top_0()
        .w(hit_bounds.size.width)
        .h(hit_bounds.size.height)
    }

    fn art_layer(&self, entity: &EditorStickerEntity, frame: Size<f32>, cx: &mut Context<Self>) -> gpui::Div {
        let mut layer = div().absolute().left_0().top_0().size_full();
        let live = Self::plays_live_lottie(entity);
        if !(live && self.live_lottie_painted)
            && let Some(image) = self.shown_frame()
        {
            layer = layer.child(img(ImageSource::Render(image)).size_full().object_fit(ObjectFit::Contain));
        }
        if live {
            let side = frame.width.min(frame.height);
            let view = cx.weak_entity();
            let player = LottieStickerView::new(
                ElementId::Name(SharedString::from(format!("editor-sticker-live-{:?}", self.entity_id))),
                entity.file_url.clone(),
            )
            .size(side)
            .r#loop(true)
            .on_first_frame(move |cx| {
                let view = view.clone();
                cx.defer(move |cx| {
                    let _ = view.update(cx, |view, cx| {
                        if !view.live_lottie_painted {
                            view.live_lottie_painted = true;
                            cx.notify();
                        }
                    });
                });
            });
            layer = layer.child(div().absolute().size_full().flex().items_center().justify_center().child(player));
        }
        layer
    }
}

impl EditorTwoFingerTransformable for EditorStickerEntityView {
    fn entity_id(&self) -> Uuid {
        self.entity_id
    }

    fn begin_two_finger_transform(&mut self, cx: &mut Context<Self>) {
        let Some((scale, rotation)) = self.current.as_ref().map(|entity| (entity.scale, entity.rotation)) else {
            return;
        };
        if self.is_side_handle_dragging {
            return;
        }
        self.two_finger_start_scale = scale;
        self.two_finger_start_rotation = rotation;
        self.snap_feedback.begin();
        if let Some(on_select) = self.on_select.clone() {
            on_select(self.entity_id, cx);
        }
    }

    fn apply_two_finger_transform(&mut self, scale: f32, rotation: f32, cx: &mut Context<Self>) {
        if self.is_side_handle_dragging {
            return;
        }
        let Some(mut entity) = self.current.clone() else {
            return;
        };
        let range = StoryCanvasGeometry::ENTITY_SCALE_RANGE;
        entity.scale = (self.two_finger_start_scale * scale).max(*range.start()).min(*range.end());
        let (angle, snapped) = EditorEntitySnap::snap_rotation(self.two_finger_start_rotation + rotation);
        self.snap_feedback.update_rotation(snapped);
        entity.rotation = angle;
        self.current = Some(entity.clone());
        cx.notify();
        if let Some(on_update) = self.on_update.clone() {
            on_update(entity, cx);
        }
    }

    fn end_two_finger_transform(&mut self, _cx: &mut Context<Self>) {}
}

impl Render for EditorStickerEntityView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(entity) = self.current.clone() else {
            return div().into_any_element();
        };
        let bounds = self.bounds();
        let scale = entity.scale;
        let center = self.center();
        let frame = size(bounds.width * scale, bounds.height * scale);
        let (sin, cos) = entity.rotation.sin_cos();
        let selection = self.selection_rect();
        let reach = {
            let handle_reach = (EditorStickerEntityViewMetrics::HANDLE_HIT_OUTSET
                + EditorStickerEntityViewMetrics::HANDLE_VISUAL_SIZE / 2.0)
                * scale;
            let half_w = selection.size.width * scale / 2.0 + handle_reach;
            let half_h = selection.size.height * scale / 2.0 + handle_reach;
            size(half_w * cos.abs() + half_h * sin.abs(), half_w * sin.abs() + half_h * cos.abs())
        };
        let hit_origin = point(center.x - reach.width, center.y - reach.height);
        let hit_size = size(reach.width * 2.0, reach.height * 2.0);
        let hit_bounds =
            Bounds::new(point(px(hit_origin.x), px(hit_origin.y)), size(px(hit_size.width), px(hit_size.height)));
        let canvas_origin_in_hit = hit_bounds.origin;

        let mut turned = div().relative().size_full().child(self.art_layer(&entity, frame, cx));
        if self.is_selected && self.handles_visible {
            turned = turned.child(selection_frame_layer(scale));
        }
        if self.handles_visible {
            turned = turned
                .child(EditorStickerHandleDotView { frame: self.handle_frame(-1.0), scale })
                .child(EditorStickerHandleDotView { frame: self.handle_frame(1.0), scale });
        }

        let positioned = div()
            .absolute()
            .left(px(center.x - frame.width / 2.0 - hit_origin.x))
            .top(px(center.y - frame.height / 2.0 - hit_origin.y))
            .w(px(frame.width))
            .h(px(frame.height))
            .child(turned.rotation_effect(entity.rotation.to_degrees()));

        let content = div().relative().w(px(hit_size.width)).h(px(hit_size.height)).child(positioned).child(
            Self::gesture_layer(cx.entity(), hit_bounds, self.canvas_origin.clone(), canvas_origin_in_hit),
        );
        let menu_id =
            ElementId::Name(SharedString::from(format!("editor-sticker-entity-menu-{:?}", self.entity_id)));
        div()
            .absolute()
            .left(px(hit_origin.x))
            .top(px(hit_origin.y))
            .child(content.context_menu(menu_id, self.edit_menu_items()))
            .into_any_element()
    }
}

fn rect_contains(rect: Bounds<f32>, point: Point<f32>) -> bool {
    point.x >= rect.origin.x
        && point.x <= rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y <= rect.origin.y + rect.size.height
}

fn past_pan_distance(press: Point<Pixels>, position: Point<Pixels>) -> bool {
    let delta = translation_of(press, position);
    delta.x.hypot(delta.y) >= PAN_MINIMUM_DISTANCE
}

fn translation_of(from: Point<Pixels>, to: Point<Pixels>) -> Point<f32> {
    point(f32::from(to.x - from.x), f32::from(to.y - from.y))
}

fn selection_frame_layer(scale: f32) -> impl IntoElement {
    let pad = EditorStickerEntityViewMetrics::SELECTION_PADDING * scale;
    canvas(
        |_: Bounds<Pixels>, _: &mut Window, _: &mut App| {},
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
            let corner = px(12.0 * scale);
            let radii = point(corner, corner);
            let (left, top, right, bottom) = (bounds.left(), bounds.top(), bounds.right(), bounds.bottom());
            let mut builder = PathBuilder::stroke(px(scale)).dash_array(&[px(5.0 * scale), px(4.0 * scale)]);
            builder.move_to(point(left + corner, top));
            builder.line_to(point(right - corner, top));
            builder.arc_to(radii, px(0.0), false, true, point(right, top + corner));
            builder.line_to(point(right, bottom - corner));
            builder.arc_to(radii, px(0.0), false, true, point(right - corner, bottom));
            builder.line_to(point(left + corner, bottom));
            builder.arc_to(radii, px(0.0), false, true, point(left, bottom - corner));
            builder.line_to(point(left, top + corner));
            builder.arc_to(radii, px(0.0), false, true, point(left + corner, top));
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, hsla(0.0, 0.0, 1.0, 0.95));
            }
        },
    )
    .absolute()
    .left(px(-pad))
    .top(px(-pad))
    .right(px(-pad))
    .bottom(px(-pad))
}

/// handle dot view, frame in the views own points
#[derive(IntoElement)]
struct EditorStickerHandleDotView {
    frame: Bounds<f32>,
    scale: f32,
}

impl RenderOnce for EditorStickerHandleDotView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let scale = self.scale;
        let fill: Hsla = Rgba { r: 0.31, g: 0.55, b: 1.0, a: 1.0 }.into();
        div()
            .absolute()
            .left(px(self.frame.origin.x * scale))
            .top(px(self.frame.origin.y * scale))
            .w(px(self.frame.size.width * scale))
            .h(px(self.frame.size.height * scale))
            .circle()
            .bg(fill)
            .stroke_border(hsla(0.0, 0.0, 1.0, 1.0), 0.5 * scale)
            .shadow(vec![BoxShadow {
                color: hsla(0.0, 0.0, 0.0, 0.55),
                offset: point(px(0.0), px(scale)),
                blur_radius: px(2.0 * scale),
                spread_radius: px(0.0),
                inset: false,
            }])
    }
}
