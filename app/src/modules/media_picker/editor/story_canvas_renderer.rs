use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use futures::channel::oneshot;
use gpui::{AsyncApp, Bounds, RenderImage, Size, point};
use image::{Frame, RgbaImage};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::video_export::{FrameVideoWriter, VideoExportError};
use rise_platform::paths::temporary_directory;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::clever_image_disk_cache::CleverImageBitmapEncoder;
use crate::core::ui::lottie_sticker::lottie_sticker_loader::{LottieStickerLoader, LottieStickerPayload};
use crate::core::ui::lottie_sticker::rise_lottie_frame_cache::{RiseLottieFrameSequence, RiseLottieSequenceKey};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_state::EditorImageState;
use crate::modules::media_picker::editor::editor_sticker_entity::{
    EditorStickerArtStore, EditorStickerEntity, EditorStickerRenderer,
};
use crate::modules::media_picker::editor::story_canvas_model::{
    EditorCanvasPhotoEntity, EditorStoryCanvas, StoryCanvasBackground, StoryCanvasGeometry, StoryCanvasPhotoStore,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerAssetType;

/// pixels of a story canvas, background and photos then the drawing
/// blocking so call it off the main thread
/// why: bake context is a straight rgba bitmap with top left origin
pub enum StoryCanvasRenderer {}

impl StoryCanvasRenderer {
    pub fn render(
        state: &EditorImageState,
        canvas: &EditorStoryCanvas,
        size: Size<f32>,
        leaves_out_animated_stickers: bool,
    ) -> Arc<RenderImage> {
        let base = Self::render_base(canvas, size);
        let mut ctx = Self::bitmap(&base, size);
        EditorImageBake::draw_strokes(&state.strokes, size, &mut ctx, &base);
        let mut stickers = state.stickers.clone().unwrap_or_default();
        if leaves_out_animated_stickers {
            stickers.retain(|sticker| {
                EditorStickerArtStore::shared().art_for_bake(sticker).map(|art| art.is_animated()) != Some(true)
            });
        }
        let links = state.links.clone().unwrap_or_default();
        EditorImageBake::draw_stacked(&state.texts, &stickers, &links, state.entity_order.as_deref(), size, &mut ctx);
        Self::render_image(ctx)
    }

    pub fn render_default(state: &EditorImageState, canvas: &EditorStoryCanvas) -> Arc<RenderImage> {
        Self::render(state, canvas, StoryCanvasGeometry::RENDER_SIZE, false)
    }

    pub fn render_base(canvas: &EditorStoryCanvas, size: Size<f32>) -> Arc<RenderImage> {
        let mut ctx = Self::opaque_context(size);
        StoryCanvasBackground::draw(&canvas.background_top, &canvas.background_bottom, size, &mut ctx);
        for photo in &canvas.photos {
            let Some(image) = StoryCanvasPhotoStore::shared().image(&photo.photo_key) else {
                continue;
            };
            Self::draw_photo(photo, &image, size, &mut ctx);
        }
        Self::render_image(ctx)
    }

    pub fn draw_photo(photo: &EditorCanvasPhotoEntity, image: &RenderImage, size: Size<f32>, ctx: &mut RgbaImage) {
        let box_size = StoryCanvasGeometry::shaped_box(photo, size);
        if box_size.width <= 0.0 || box_size.height <= 0.0 {
            return;
        }
        let Some(source) = CleverImageBitmapEncoder::rgba(image) else {
            return;
        };
        let radius = StoryCanvasGeometry::corner_radius(photo, box_size);
        let rect = Bounds {
            origin: point(-box_size.width / 2.0, -box_size.height / 2.0),
            size: box_size,
        };
        let image_size = gpui::size(source.width() as f32, source.height() as f32);
        let fill = StoryCanvasGeometry::fill_rect(image_size, rect);
        let center = (
            f64::from(photo.center.x) * f64::from(size.width),
            f64::from(photo.center.y) * f64::from(size.height),
        );
        let rotation = f64::from(photo.rotation);
        let scale = f64::from(photo.scale);
        if scale <= 0.0 || fill.size.width <= 0.0 || fill.size.height <= 0.0 {
            return;
        }
        let (sin, cos) = rotation.sin_cos();
        let half_w = f64::from(box_size.width) / 2.0 * scale;
        let half_h = f64::from(box_size.height) / 2.0 * scale;
        let reach = (half_w * half_w + half_h * half_h).sqrt();
        let min_x = ((center.0 - reach).floor().max(0.0)) as u32;
        let min_y = ((center.1 - reach).floor().max(0.0)) as u32;
        let max_x = ((center.0 + reach).ceil().min(f64::from(ctx.width()))) as u32;
        let max_y = ((center.1 + reach).ceil().min(f64::from(ctx.height()))) as u32;
        let radius = f64::from(radius);
        let half_box = (f64::from(box_size.width) / 2.0, f64::from(box_size.height) / 2.0);
        for y in min_y..max_y {
            for x in min_x..max_x {
                let dx = f64::from(x) + 0.5 - center.0;
                let dy = f64::from(y) + 0.5 - center.1;
                let local_x = (dx * cos + dy * sin) / scale;
                let local_y = (-dx * sin + dy * cos) / scale;
                if !Self::inside_rounded_rect(local_x, local_y, half_box, radius) {
                    continue;
                }
                let u = (local_x - f64::from(fill.origin.x)) / f64::from(fill.size.width) * f64::from(source.width());
                let v = (local_y - f64::from(fill.origin.y)) / f64::from(fill.size.height) * f64::from(source.height());
                let Some(sample) = Self::sample(&source, u, v) else {
                    continue;
                };
                Self::blend(ctx, x, y, sample);
            }
        }
    }

    pub fn animated_stickers(state: &EditorImageState) -> Vec<EditorStickerEntity> {
        state
            .stickers
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|sticker| {
                EditorStickerArtStore::shared().art_for_bake(sticker).map(|art| art.is_animated()) == Some(true)
            })
            .collect()
    }

    /// zones of the links in fractions of the published frame
    /// url plus center x and y and size and rotation
    pub fn link_areas(state: &EditorImageState, size: Size<f32>) -> Vec<(String, f64, f64, f64, f64, f64)> {
        state
            .links
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|link| {
                let (center_x, center_y, width, height, rotation) = link.area(size);
                (link.url.clone(), center_x, center_y, width, height, rotation)
            })
            .collect()
    }

    fn opaque_context(size: Size<f32>) -> RgbaImage {
        let width = size.width.round().max(1.0) as u32;
        let height = size.height.round().max(1.0) as u32;
        RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]))
    }

    fn bitmap(image: &RenderImage, size: Size<f32>) -> RgbaImage {
        CleverImageBitmapEncoder::rgba(image).unwrap_or_else(|| Self::opaque_context(size))
    }

    /// straight rgba bake context back into the bgra frame gpui holds
    fn render_image(mut bitmap: RgbaImage) -> Arc<RenderImage> {
        for pixel in bitmap.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Arc::new(RenderImage::new([Frame::new(bitmap)]))
    }

    fn inside_rounded_rect(x: f64, y: f64, half: (f64, f64), radius: f64) -> bool {
        let ax = x.abs();
        let ay = y.abs();
        if ax > half.0 || ay > half.1 {
            return false;
        }
        let radius = radius.min(half.0).min(half.1).max(0.0);
        let corner_x = ax - (half.0 - radius);
        let corner_y = ay - (half.1 - radius);
        if corner_x <= 0.0 || corner_y <= 0.0 {
            return true;
        }
        corner_x * corner_x + corner_y * corner_y <= radius * radius
    }

    fn sample(source: &RgbaImage, u: f64, v: f64) -> Option<[u8; 4]> {
        let width = f64::from(source.width());
        let height = f64::from(source.height());
        if u < 0.0 || v < 0.0 || u >= width || v >= height {
            return None;
        }
        let x = (u - 0.5).max(0.0);
        let y = (v - 0.5).max(0.0);
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(source.width() - 1);
        let y1 = (y0 + 1).min(source.height() - 1);
        let fx = x - x.floor();
        let fy = y - y.floor();
        let mut out = [0u8; 4];
        for (channel, value) in out.iter_mut().enumerate() {
            let p00 = f64::from(source.get_pixel(x0, y0).0[channel]);
            let p10 = f64::from(source.get_pixel(x1, y0).0[channel]);
            let p01 = f64::from(source.get_pixel(x0, y1).0[channel]);
            let p11 = f64::from(source.get_pixel(x1, y1).0[channel]);
            let top = p00 + (p10 - p00) * fx;
            let bottom = p01 + (p11 - p01) * fx;
            *value = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8;
        }
        Some(out)
    }

    fn blend(ctx: &mut RgbaImage, x: u32, y: u32, source: [u8; 4]) {
        let alpha = f64::from(source[3]) / 255.0;
        let destination = ctx.get_pixel_mut(x, y);
        for channel in 0..3 {
            let over = f64::from(source[channel]) * alpha + f64::from(destination.0[channel]) * (1.0 - alpha);
            destination.0[channel] = over.round().clamp(0.0, 255.0) as u8;
        }
        destination.0[3] = 255;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoryCanvasVideoRenderError {
    #[error("story canvas video writer unavailable")]
    WriterUnavailable,
    #[error("story canvas still frame missing")]
    NoFrame,
    #[error(transparent)]
    Writer(#[from] VideoExportError),
}

/// canvas with animated stickers as an mp4 of the image story length
/// still frame once, sticker frames drawn over it by their own clock
/// 60 fps 1080 x 1920 because 30 fps played stickers at 16 fps
pub enum StoryCanvasVideoRenderer {}

impl StoryCanvasVideoRenderer {
    const FRAMES_PER_SECOND: u32 = 60;
    const AVERAGE_BIT_RATE: u32 = 6_000_000;

    fn lottie_pixel_side() -> i64 {
        RiseLimits::client_value("media_editor_sticker_max_pixel_side")
    }

    pub async fn render(
        state: EditorImageState,
        canvas: EditorStoryCanvas,
        cx: &AsyncApp,
    ) -> Result<PathBuf, StoryCanvasVideoRenderError> {
        let animated = Self::animated_on_background(&state, cx).await;
        let live = Self::live_lottie_clips(&animated, cx).await;
        cx.background_executor().spawn(async move { Self::write(state, canvas, animated, live).await }).await
    }

    async fn animated_on_background(state: &EditorImageState, cx: &AsyncApp) -> Vec<EditorStickerEntity> {
        let state = state.clone();
        cx.background_executor().spawn(async move { StoryCanvasRenderer::animated_stickers(&state) }).await
    }

    async fn write(
        state: EditorImageState,
        canvas: EditorStoryCanvas,
        animated: Vec<EditorStickerEntity>,
        mut live: HashMap<Uuid, LiveLottieClip>,
    ) -> Result<PathBuf, StoryCanvasVideoRenderError> {
        let _teardown = LiveLottieTeardown(live.values().map(|clip| Arc::clone(&clip.sequence)).collect());
        let size = StoryCanvasGeometry::RENDER_SIZE;
        let duration = StoryCanvasGeometry::image_story_duration().max(1.0);
        let arts: Vec<_> = animated
            .iter()
            .filter_map(|entity| EditorStickerArtStore::shared().art_for_bake(entity).map(|art| (entity.clone(), art)))
            .collect();
        let still = StoryCanvasRenderer::render(&state, &canvas, size, true);
        let still_image = CleverImageBitmapEncoder::rgba(&still).ok_or(StoryCanvasVideoRenderError::NoFrame)?;

        let output_url = temporary_directory()
            .join(format!("ro-story-canvas-{}.mp4", Uuid::new_v4().uuid_string().to_lowercase()));
        let width = size.width.round() as u32;
        let height = size.height.round() as u32;
        let mut writer =
            FrameVideoWriter::create(&output_url, width, height, Self::FRAMES_PER_SECOND, Self::AVERAGE_BIT_RATE)?;
        let mut cleanup = StoryCanvasOutputCleanup { path: Some(output_url.clone()) };

        let total_frames = (duration * f64::from(Self::FRAMES_PER_SECOND)) as i64;
        for index in 0..total_frames {
            let seconds = index as f64 / f64::from(Self::FRAMES_PER_SECOND);
            let mut frames: Vec<(EditorStickerEntity, Arc<RenderImage>)> = Vec::with_capacity(arts.len());
            for (entity, art) in &arts {
                let live_frame = match live.get_mut(&entity.id) {
                    Some(clip) => clip.frame(seconds).await,
                    None => None,
                };
                let frame = match live_frame {
                    Some(frame) => Some(frame),
                    None => {
                        let last = art.frames.len().saturating_sub(1);
                        art.frames.get(last.min(art.frame_index(seconds))).cloned()
                    }
                };
                if let Some(frame) = frame {
                    frames.push((entity.clone(), frame));
                }
            }
            let buffer = Self::draw(&still_image, &frames, size);
            writer.append(&buffer, index)?;
        }
        writer.finish(duration)?;
        cleanup.path = None;
        Ok(output_url)
    }

    /// the still frame with every sticker frame over it
    /// bake bitmap already has a top left origin so no flip needed
    fn draw(still: &RgbaImage, frames: &[(EditorStickerEntity, Arc<RenderImage>)], size: Size<f32>) -> RgbaImage {
        let mut context = still.clone();
        for (entity, frame) in frames {
            EditorStickerRenderer::draw(entity, frame, size, &mut context);
        }
        context
    }

    async fn live_lottie_clips(stickers: &[EditorStickerEntity], cx: &AsyncApp) -> HashMap<Uuid, LiveLottieClip> {
        let mut clips: HashMap<Uuid, LiveLottieClip> = HashMap::new();
        for entity in stickers {
            if entity.emoji.is_some() || StickerAssetType::from_wire(&entity.asset_type) != Some(StickerAssetType::Lottie) {
                continue;
            }
            let Ok(LottieStickerPayload::Animation(render)) =
                LottieStickerLoader::shared().payload(&entity.file_url).await
            else {
                continue;
            };
            let key = RiseLottieSequenceKey {
                cache_key: format!("story-export|{}", render.cache_key),
                dimension: Self::lottie_pixel_side(),
                mirrored: false,
            };
            let Some(sequence) = RiseLottieFrameSequence::make(key, Arc::clone(&render.json_data), cx).await else {
                continue;
            };
            let Some(first) = sequence.first_visible_frame_index().await else {
                sequence.teardown();
                continue;
            };
            clips.insert(entity.id, LiveLottieClip::new(sequence, first));
        }
        clips
    }
}

struct LiveLottieTeardown(Vec<Arc<RiseLottieFrameSequence>>);

impl Drop for LiveLottieTeardown {
    fn drop(&mut self) {
        for sequence in &self.0 {
            sequence.teardown();
        }
    }
}

struct StoryCanvasOutputCleanup {
    path: Option<PathBuf>,
}

impl Drop for StoryCanvasOutputCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// lottie sticker played from its file at its own frame rate, from the first painted frame
struct LiveLottieClip {
    sequence: Arc<RiseLottieFrameSequence>,
    first_visible: i64,
    last_index: i64,
    last_frame: Option<Arc<RenderImage>>,
}

impl LiveLottieClip {
    fn new(sequence: Arc<RiseLottieFrameSequence>, first_visible: i64) -> Self {
        Self { sequence, first_visible, last_index: -1, last_frame: None }
    }

    async fn frame(&mut self, seconds: f64) -> Option<Arc<RenderImage>> {
        let total = self.sequence.frame_count;
        if total <= 0 || self.sequence.frame_rate <= 0.0 {
            return None;
        }
        let index = (self.first_visible + (seconds * self.sequence.frame_rate).floor() as i64) % total;
        if index == self.last_index
            && let Some(last_frame) = self.last_frame.as_ref()
        {
            return Some(Arc::clone(last_frame));
        }
        let (sender, receiver) = oneshot::channel::<Option<Arc<RenderImage>>>();
        self.sequence.request_frame(
            index,
            Arc::new(|| true),
            Some(Box::new(move |image| {
                let _ = sender.send(image);
            })),
        );
        let image = receiver.await.ok().flatten();
        if let Some(image) = image.as_ref() {
            self.last_index = index;
            self.last_frame = Some(Arc::clone(image));
        }
        image.or_else(|| self.last_frame.clone())
    }
}
