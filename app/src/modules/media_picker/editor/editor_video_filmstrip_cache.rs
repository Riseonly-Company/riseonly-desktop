use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::{App, AppContext, Global, RenderImage};
use image::{Frame, RgbaImage};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use rise_media::video::av_asset::AVURLAsset;
use smallvec::SmallVec;

use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::chat::components::chat::bubble::web_m_sticker_playback::WebMStickerFile;

type EditorVideoFilmstripCompletion = Box<dyn FnOnce(Vec<CleverImage>, &mut App)>;

static IMAGE_CACHE: LazyLock<NSCache<String, Arc<Vec<CleverImage>>>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_total_cost_limit(64 * 1024 * 1024);
    cache
});

/// inflight completions per asset
/// only main thread touches it so a global replaces the lock
#[derive(Default)]
struct EditorVideoFilmstripInflight(HashMap<String, Vec<EditorVideoFilmstripCompletion>>);

impl Global for EditorVideoFilmstripInflight {}

pub enum EditorVideoFilmstripCache {}

impl EditorVideoFilmstripCache {
    pub const FRAME_COUNT: usize = 10;

    const FRAME_WIDTH: f32 = 48.0;
    const FRAME_HEIGHT: f32 = 36.0;
    const GENERATION_TIMEOUT: Duration = Duration::from_secs(12);

    pub fn cache_key(asset: &AVURLAsset, count: usize) -> String {
        format!("{}|{}", asset.url().absolute_string(), count)
    }

    pub fn cached_frames(asset: &AVURLAsset, count: usize) -> Option<Vec<CleverImage>> {
        IMAGE_CACHE.object(&Self::cache_key(asset, count)).map(|frames| (*frames).clone())
    }

    /// scale is the pixel scale of the window showing the strip
    pub fn load(
        asset: &AVURLAsset,
        count: usize,
        scale: f32,
        completion: impl FnOnce(Vec<CleverImage>, &mut App) + 'static,
        cx: &mut App,
    ) {
        let key = Self::cache_key(asset, count);
        if let Some(cached) = Self::cached_frames(asset, count) {
            cx.defer(move |cx| completion(cached, cx));
            return;
        }

        let inflight = cx.default_global::<EditorVideoFilmstripInflight>();
        if let Some(waiting) = inflight.0.get_mut(&key) {
            waiting.push(Box::new(completion));
            return;
        }
        inflight.0.insert(key.clone(), vec![Box::new(completion)]);

        let url = asset.url().clone();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            let generated_key = key.clone();
            let frames = cx
                .background_spawn(async move {
                    let frames = Self::generate(&url, count, scale, &executor);
                    if !frames.is_empty() {
                        let cost: usize = frames
                            .iter()
                            .filter_map(|frame| frame.as_bytes(0).map(<[u8]>::len))
                            .sum();
                        IMAGE_CACHE.set_object_cost(Arc::new(frames.clone()), generated_key, cost.max(1));
                    }
                    frames
                })
                .await;
            cx.update(|cx| Self::finish(&key, frames, cx));
        })
        .detach();
    }

    fn generate(
        url: &Url,
        count: usize,
        scale: f32,
        executor: &gpui::BackgroundExecutor,
    ) -> Vec<CleverImage> {
        let Some(source) = url.is_file_url().then(|| url.to_file_path()).flatten() else {
            return Vec::new();
        };
        if WebMStickerFile::is_web_m_path(&source) {
            return Self::generate_web_m(&source, count, scale);
        }
        let Ok(probe) = rise_media::video_export::probe(&source) else {
            return Vec::new();
        };
        let duration = probe.duration_ms() as f64 / 1000.0;
        if !duration.is_finite() || duration <= 0.0 || count == 0 {
            return Vec::new();
        }

        let max_side = (Self::FRAME_WIDTH.max(Self::FRAME_HEIGHT) * scale).ceil() as u32;
        let copy_frame = |at_seconds: f64| -> Option<CleverImage> {
            let frame = rise_media::video_export::filmstrip_frame(&source, at_seconds, max_side).ok()?;
            let buffer = RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
            let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
            Some(Arc::new(RenderImage::new(frames)))
        };

        let interval = duration / count.saturating_sub(1).max(1) as f64;
        let placeholder = copy_frame(0.0);
        let mut slots: Vec<Option<CleverImage>> = vec![placeholder.clone(); count];

        let started = executor.now();
        for (index, slot) in slots.iter_mut().enumerate() {
            if executor.now().saturating_duration_since(started) >= Self::GENERATION_TIMEOUT {
                break;
            }
            if let Some(image) = copy_frame(index as f64 * interval) {
                *slot = Some(image);
            }
        }

        let mut frames: Vec<CleverImage> = slots.into_iter().flatten().collect();
        if frames.len() < count {
            if let Some(placeholder) = placeholder {
                frames = vec![placeholder; count];
            }
        }
        frames
    }

    fn generate_web_m(source: &std::path::Path, count: usize, scale: f32) -> Vec<CleverImage> {
        let Some(track) = WebMStickerFile::track(source) else {
            return Vec::new();
        };
        if count == 0 || WebMStickerFile::duration_seconds(&track) <= 0.0 {
            return Vec::new();
        }
        let max_side = (Self::FRAME_WIDTH.max(Self::FRAME_HEIGHT) * scale).ceil() as u32;
        let decoded: Vec<CleverImage> = WebMStickerFile::strip(&track, count, max_side)
            .into_iter()
            .filter_map(|(width, height, bgra)| {
                let buffer = RgbaImage::from_raw(width, height, bgra)?;
                let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
                Some(Arc::new(RenderImage::new(frames)))
            })
            .collect();
        if decoded.is_empty() {
            return Vec::new();
        }
        (0..count).map(|slot| decoded[slot * decoded.len() / count].clone()).collect()
    }

    fn finish(key: &str, images: Vec<CleverImage>, cx: &mut App) {
        let completions = cx
            .default_global::<EditorVideoFilmstripInflight>()
            .0
            .remove(key)
            .unwrap_or_default();
        for completion in completions {
            completion(images.clone(), cx);
        }
    }
}
