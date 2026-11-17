use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write as _;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

use gpui::{App, Bounds, Point, Rgba, Size, point, size};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};
use parking_lot::Mutex;
use rise_canon::{RiseLimits, RiseStoryCanvasPhotoShape};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::AppPaths;
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileManager};
use rise_platform::paths::temporary_directory;
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, CleverImageBitmapEncoder,
};
use crate::modules::media_picker::editor::editor_entity_snapping::EditorEntitySideHandle;

/// photo story as a 9:16 canvas, two color background and photos as movable items
/// center is 0..1 of the canvas
/// optional on EditorImageState so old drafts stay flat
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorStoryCanvas {
    pub background_top: String,
    pub background_bottom: String,
    pub photos: Vec<EditorCanvasPhotoEntity>,
}

impl EditorStoryCanvas {
    pub fn is_initial_layout(&self) -> bool {
        let [photo] = self.photos.as_slice() else {
            return false;
        };
        photo.center == point(0.5, 0.5)
            && photo.scale == 1.0
            && photo.rotation == 0.0
            && photo.shape_kind() == RiseStoryCanvasPhotoShape::Square
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorCanvasPhotoEntity {
    pub id: Uuid,
    pub photo_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    pub pixel_width: i64,
    pub pixel_height: i64,
    pub center: Point<f32>,
    pub scale: f32,
    pub rotation: f32,
    pub shape: String,
}

impl EditorCanvasPhotoEntity {
    pub fn shape_kind(&self) -> RiseStoryCanvasPhotoShape {
        RiseStoryCanvasPhotoShape::from_wire(&self.shape).unwrap_or(RiseStoryCanvasPhotoShape::Square)
    }

    pub fn next_shape(&self) -> RiseStoryCanvasPhotoShape {
        let all = RiseStoryCanvasPhotoShape::ALL;
        let current = self.shape_kind();
        let index = all.iter().position(|shape| *shape == current).unwrap_or(0);
        all[(index + 1) % all.len()]
    }
}

pub enum StoryCanvasGeometry {}

impl StoryCanvasGeometry {
    pub const RENDER_SIZE: Size<f32> = Size { width: 1080.0, height: 1920.0 };
    pub const ASPECT: f32 = Self::RENDER_SIZE.width / Self::RENDER_SIZE.height;
    pub const PHOTO_SCALE_RANGE: RangeInclusive<f32> = 0.1..=8.0;
    pub const ENTITY_SCALE_RANGE: RangeInclusive<f32> = EditorEntitySideHandle::SCALE_RANGE;
    pub const ADDED_PHOTO_SCALE: f32 = 0.6;
    pub const ROUNDED_CORNER_FRACTION: f32 = 0.12;

    pub fn photos_max() -> i64 {
        RiseLimits::service_value("story_canvas_photos_max")
    }

    pub fn edit_window_ms() -> i64 {
        RiseLimits::service_value("story_edit_window_ms")
    }

    pub fn image_story_duration() -> f64 {
        RiseLimits::value("story_image_duration") as f64 / 1_000.0
    }

    pub fn photo_box(pixel_width: i64, pixel_height: i64, canvas: Size<f32>) -> Size<f32> {
        let width = pixel_width.max(1) as f32;
        let height = pixel_height.max(1) as f32;
        if !(canvas.width > 0.0 && canvas.height > 0.0) {
            return Size::default();
        }
        size(canvas.width, canvas.width * height / width)
    }

    /// box the photo is drawn in after its shape, a circle keeps the centered square
    pub fn shaped_box(photo: &EditorCanvasPhotoEntity, canvas: Size<f32>) -> Size<f32> {
        let box_size = Self::photo_box(photo.pixel_width, photo.pixel_height, canvas);
        if photo.shape_kind() != RiseStoryCanvasPhotoShape::Circle {
            return box_size;
        }
        let side = box_size.width.min(box_size.height);
        size(side, side)
    }

    pub fn corner_radius(photo: &EditorCanvasPhotoEntity, box_size: Size<f32>) -> f32 {
        let short = box_size.width.min(box_size.height);
        match photo.shape_kind() {
            RiseStoryCanvasPhotoShape::Square => 0.0,
            RiseStoryCanvasPhotoShape::Rounded => short * Self::ROUNDED_CORNER_FRACTION,
            RiseStoryCanvasPhotoShape::Circle => short / 2.0,
        }
    }

    /// rect a photo of image_size fills inside rect, aspect fill and centered
    pub fn fill_rect(image_size: Size<f32>, rect: Bounds<f32>) -> Bounds<f32> {
        if !(image_size.width > 0.0 && image_size.height > 0.0) {
            return rect;
        }
        let fill = (rect.size.width / image_size.width).max(rect.size.height / image_size.height);
        let filled = size(image_size.width * fill, image_size.height * fill);
        let mid_x = rect.origin.x + rect.size.width / 2.0;
        let mid_y = rect.origin.y + rect.size.height / 2.0;
        Bounds::new(point(mid_x - filled.width / 2.0, mid_y - filled.height / 2.0), filled)
    }
}

/// default background is the mean color of the top and bottom strip of the first photo
/// photo shrunk to 16 x 16, two top rows and two bottom rows averaged
/// hex goes into the document so a re-render never drifts
pub enum StoryCanvasBackground {}

impl StoryCanvasBackground {
    pub const FALLBACK_HEX: &'static str = "#1C1C1E";

    pub fn colors(image: &CleverImage) -> (String, String) {
        let side: u32 = 16;
        let Some(source) = CleverImageBitmapEncoder::rgba(image) else {
            return (Self::FALLBACK_HEX.to_owned(), Self::FALLBACK_HEX.to_owned());
        };
        if source.width() == 0 || source.height() == 0 {
            return (Self::FALLBACK_HEX.to_owned(), Self::FALLBACK_HEX.to_owned());
        }
        let small = imageops::resize(&source, side, side, FilterType::Triangle);
        let pixel = |row: u32, column: u32| -> [u32; 3] {
            let [r, g, b, a] = small.get_pixel(column, row).0;
            let alpha = u32::from(a);
            [u32::from(r) * alpha / 255, u32::from(g) * alpha / 255, u32::from(b) * alpha / 255]
        };
        let mean = |rows: [u32; 2]| -> String {
            let (mut r, mut g, mut b, mut n) = (0_u32, 0_u32, 0_u32, 0_u32);
            for row in rows {
                for column in 0..side {
                    let [pr, pg, pb] = pixel(row, column);
                    r += pr;
                    g += pg;
                    b += pb;
                    n += 1;
                }
            }
            if n == 0 {
                return Self::FALLBACK_HEX.to_owned();
            }
            format!("#{:02X}{:02X}{:02X}", r / n, g / n, b / n)
        };
        (mean([0, 1]), mean([side - 2, side - 1]))
    }

    pub fn color(hex: &str) -> Rgba {
        let clean = hex.trim_matches(|character| character == '#' || character == ' ');
        let value = (clean.chars().count() == 6).then(|| u32::from_str_radix(clean, 16).ok()).flatten();
        let Some(value) = value else {
            return Rgba { r: 0.11, g: 0.11, b: 0.12, a: 1.0 };
        };
        Rgba {
            r: ((value >> 16) & 0xFF) as f32 / 255.0,
            g: ((value >> 8) & 0xFF) as f32 / 255.0,
            b: (value & 0xFF) as f32 / 255.0,
            a: 1.0,
        }
    }

    /// vertical gradient from top to bottom over this size, extended past both ends
    pub fn draw(top: &str, bottom: &str, size: Size<f32>, ctx: &mut RgbaImage) {
        let top = Self::color(top);
        let bottom = Self::color(bottom);
        let height = size.height.max(1.0);
        let to_byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        for (y, row) in ctx.rows_mut().enumerate() {
            let t = ((y as f32 + 0.5) / height).clamp(0.0, 1.0);
            let color = [
                to_byte(top.r + (bottom.r - top.r) * t),
                to_byte(top.g + (bottom.g - top.g) * t),
                to_byte(top.b + (bottom.b - top.b) * t),
                255,
            ];
            for pixel in row {
                pixel.0 = color;
            }
        }
    }

    /// editor stage under the photos, a small 9:16 gradient scaled up by the stage
    pub fn stage_image(top: &str, bottom: &str) -> CleverImage {
        let stage = size(270.0_f32, 480.0);
        let mut bitmap = RgbaImage::new(stage.width as u32, stage.height as u32);
        Self::draw(top, bottom, stage, &mut bitmap);
        CleverImageBitmapDecoder::render_image(bitmap)
    }
}

type StoryCanvasPhotoPreviewWaiter = Box<dyn FnOnce(Option<CleverImage>, &mut App)>;

thread_local! {
    static PREVIEW_WAITERS: RefCell<HashMap<String, Vec<StoryCanvasPhotoPreviewWaiter>>> =
        RefCell::new(HashMap::new());
}

#[derive(Default)]
struct StoryCanvasPhotoStoreState {
    previews: HashMap<String, CleverImage>,
    order: Vec<String>,
    pending: HashMap<String, CleverImage>,
    did_purge: bool,
}

/// downscaled originals of the canvas photos, one jpeg per photo in Caches
/// editor shows a preview from memory, bake and publish read the file
/// files outlive the editor a bit then purge_stale_once drops them
pub struct StoryCanvasPhotoStore {
    lock: Mutex<StoryCanvasPhotoStoreState>,
    pub directory: PathBuf,
}

static SHARED: LazyLock<StoryCanvasPhotoStore> = LazyLock::new(StoryCanvasPhotoStore::new);

impl StoryCanvasPhotoStore {
    pub const MAX_PIXEL_SIDE: f32 = 2048.0;
    const PREVIEW_PIXEL_SIDE: f32 = 1280.0;
    const PREVIEW_ENTRIES: usize = 12;
    const STALE_AGE: f64 = 2.0 * 24.0 * 3600.0;
    const JPEG_QUALITY: u8 = 92;

    pub fn shared() -> &'static Self {
        &SHARED
    }

    fn new() -> Self {
        let caches = AppPaths::resolve(AppEnvironment::current().canon())
            .ok()
            .map(|paths| paths.cache_dir().to_path_buf())
            .unwrap_or_else(temporary_directory);
        let directory = caches.join("ro-story-canvas");
        let _ = FileManager.create_directory(&directory, true);
        Self { lock: Mutex::new(StoryCanvasPhotoStoreState::default()), directory }
    }

    pub fn file_url(&self, key: &str) -> PathBuf {
        self.directory.join(key)
    }

    fn new_key() -> String {
        format!("p-{}.jpg", Uuid::new_v4().uuid_string().to_lowercase())
    }

    fn fitted_pixel_size(pixels: Size<f32>) -> Size<f32> {
        let longest = pixels.width.max(pixels.height);
        if !(longest > Self::MAX_PIXEL_SIDE && longest > 0.0) {
            return pixels;
        }
        let factor = Self::MAX_PIXEL_SIDE / longest;
        size((pixels.width * factor).floor(), (pixels.height * factor).floor())
    }

    fn pixel_size(image: &CleverImage) -> Size<f32> {
        let pixels = image.size(0);
        size(pixels.width.0 as f32, pixels.height.0 as f32)
    }

    /// photo the editor already decoded goes under a new key at once
    /// its jpeg is written off the main thread
    pub fn adopt(&self, image: &CleverImage) -> (String, Size<f32>) {
        let key = Self::new_key();
        let fitted = Self::fitted_pixel_size(Self::pixel_size(image));
        self.lock.lock().pending.insert(key.clone(), image.clone());
        self.remember(&key, image.clone());
        let write_key = key.clone();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let _ = Self::shared().write_pending_if_needed(&write_key);
        }));
        (key, fitted)
    }

    pub fn import_file(&self, url: &Path) -> Option<(String, Size<f32>)> {
        let data = std::fs::read(url).ok()?;
        self.import_source(&data)
    }

    pub fn import_data(&self, data: &[u8]) -> Option<(String, Size<f32>)> {
        self.import_source(data)
    }

    fn import_source(&self, data: &[u8]) -> Option<(String, Size<f32>)> {
        let image = CleverImageBitmapDecoder::decode(data, Some(Self::MAX_PIXEL_SIDE as i64))?;
        let jpeg = CleverImageBitmapEncoder::jpeg_data(&image, Self::JPEG_QUALITY)?;
        let key = Self::new_key();
        write_atomically(&self.file_url(&key), &jpeg).ok()?;
        self.remember(&key, Self::preview_sized(&image));
        Some((key, Self::pixel_size(&image)))
    }

    /// writes an adopted photo now if the background write hasnt yet, blocking
    pub fn write_pending_if_needed(&self, key: &str) -> Option<PathBuf> {
        let url = self.file_url(key);
        let image = self.lock.lock().pending.get(key).cloned();
        let Some(image) = image else {
            return FileManager.file_exists(&url).then_some(url);
        };
        let fitted = Self::fitted_pixel_size(Self::pixel_size(&image));
        let mut bitmap = CleverImageBitmapEncoder::rgba(&image)?;
        let (width, height) = (fitted.width.max(1.0) as u32, fitted.height.max(1.0) as u32);
        if bitmap.width() != width || bitmap.height() != height {
            bitmap = imageops::resize(&bitmap, width, height, FilterType::Triangle);
        }
        let data = jpeg_data(bitmap)?;
        write_atomically(&url, &data).ok()?;
        self.lock.lock().pending.remove(key);
        Some(url)
    }

    pub fn upload_file(&self, key: &str) -> Option<PathBuf> {
        self.write_pending_if_needed(key)
    }

    /// full size photo for the bake, blocking
    pub fn image(&self, key: &str) -> Option<CleverImage> {
        let adopted = self.lock.lock().pending.get(key).cloned();
        if adopted.is_some() {
            return adopted;
        }
        let data = std::fs::read(self.file_url(key)).ok()?;
        CleverImageBitmapDecoder::decode(&data, None)
    }

    pub fn preview(&self, key: &str) -> Option<CleverImage> {
        self.lock.lock().previews.get(key).cloned()
    }

    pub fn load_preview(
        &self,
        key: &str,
        cx: &mut App,
        completion: impl FnOnce(Option<CleverImage>, &mut App) + 'static,
    ) {
        if let Some(hit) = self.preview(key) {
            cx.defer(move |cx| completion(Some(hit), cx));
            return;
        }
        let first = PREVIEW_WAITERS.with(|waiters| {
            let mut waiters = waiters.borrow_mut();
            let first = !waiters.contains_key(key);
            waiters.entry(key.to_owned()).or_default().push(Box::new(completion));
            first
        });
        if !first {
            return;
        }
        let url = self.file_url(key);
        let key = key.to_owned();
        let decoded_key = key.clone();
        let decode = cx.background_executor().spawn(async move {
            let data = std::fs::read(&url).ok()?;
            let image = CleverImageBitmapDecoder::decode(&data, Some(Self::PREVIEW_PIXEL_SIDE as i64))?;
            Self::shared().remember(&decoded_key, image.clone());
            Some(image)
        });
        cx.spawn(async move |cx| {
            let image = decode.await;
            let callbacks =
                PREVIEW_WAITERS.with(|waiters| waiters.borrow_mut().remove(&key).unwrap_or_default());
            cx.update(|cx| {
                for callback in callbacks {
                    callback(image.clone(), cx);
                }
            });
        })
        .detach();
    }

    fn remember(&self, key: &str, image: CleverImage) {
        let mut state = self.lock.lock();
        if !state.previews.contains_key(key) {
            state.order.push(key.to_owned());
        }
        state.previews.insert(key.to_owned(), image);
        while state.order.len() > Self::PREVIEW_ENTRIES {
            let dropped = state.order.remove(0);
            state.previews.remove(&dropped);
        }
    }

    fn preview_sized(image: &CleverImage) -> CleverImage {
        let pixels = Self::pixel_size(image);
        let longest = pixels.width.max(pixels.height);
        if longest <= Self::PREVIEW_PIXEL_SIDE {
            return image.clone();
        }
        let factor = Self::PREVIEW_PIXEL_SIDE / longest;
        let target = size((pixels.width * factor).floor().max(1.0), (pixels.height * factor).floor().max(1.0));
        let Some(bitmap) = CleverImageBitmapEncoder::rgba(image) else {
            return image.clone();
        };
        let resized = imageops::resize(&bitmap, target.width as u32, target.height as u32, FilterType::Triangle);
        CleverImageBitmapDecoder::render_image(resized)
    }

    pub fn purge_stale_once(&self) {
        {
            let mut state = self.lock.lock();
            if state.did_purge {
                return;
            }
            state.did_purge = true;
        }
        let directory = self.directory.clone();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let manager = FileManager;
            let Ok(files) = manager.contents_of_directory_at(&directory, DirectoryEnumerationOptions::default())
            else {
                return;
            };
            let cutoff = RiseClock::now().time_interval_since_1970() - Self::STALE_AGE;
            for file in files {
                let modified = manager
                    .attributes_of_item(&file)
                    .ok()
                    .and_then(|attributes| attributes.modification_date)
                    .and_then(|date| date.duration_since(UNIX_EPOCH).ok())
                    .map(|elapsed| elapsed.as_secs_f64());
                if modified.is_some_and(|modified| modified < cutoff) {
                    let _ = manager.remove_item(&file);
                }
            }
        }));
    }
}

impl EditorStoryCanvas {
    /// new canvas around the photo the editor opened
    /// background from its colors, photo fitted in the middle and square
    pub fn make(first_photo: &CleverImage) -> EditorStoryCanvas {
        let store = StoryCanvasPhotoStore::shared();
        store.purge_stale_once();
        let (top, bottom) = StoryCanvasBackground::colors(first_photo);
        let (key, pixel_size) = store.adopt(first_photo);
        let photo = EditorCanvasPhotoEntity {
            id: Uuid::new_v4(),
            photo_key: key,
            file_id: None,
            pixel_width: pixel_size.width as i64,
            pixel_height: pixel_size.height as i64,
            center: point(0.5, 0.5),
            scale: 1.0,
            rotation: 0.0,
            shape: RiseStoryCanvasPhotoShape::Square.wire().to_owned(),
        };
        EditorStoryCanvas { background_top: top, background_bottom: bottom, photos: vec![photo] }
    }
}

fn jpeg_data(bitmap: RgbaImage) -> Option<Vec<u8>> {
    let rgb = DynamicImage::ImageRgba8(bitmap).into_rgb8();
    let mut data = Vec::new();
    JpegEncoder::new_with_quality(&mut data, StoryCanvasPhotoStore::JPEG_QUALITY).encode_image(&rgb).ok()?;
    Some(data)
}

fn write_atomically(destination: &Path, data: &[u8]) -> std::io::Result<()> {
    let name = destination.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = destination.with_file_name(format!(".{name}.{}.tmp", Uuid::new_v4().uuid_string()));
    let result = std::fs::File::create(&temporary).and_then(|mut file| {
        file.write_all(data)?;
        file.sync_all()
    });
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    std::fs::rename(&temporary, destination).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}
