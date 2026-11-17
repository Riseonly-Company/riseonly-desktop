use std::path::PathBuf;
use std::sync::LazyLock;

use gpui::RenderImage;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};
use parking_lot::Mutex;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_manager::FileManager;
use rise_platform::media_keys::Artwork;
use rise_platform::paths::temporary_directory;

use crate::core::ui::clever_image_disk_cache::CleverImage;

/// cover for the system now playing surfaces
/// they draw in a fixed square and aspect fit, so non square covers get black bars
pub enum MusicNowPlayingArtwork {}

static PUBLISHED_FILE: LazyLock<Mutex<Option<PathBuf>>> = LazyLock::new(|| Mutex::new(None));

const JPEG_QUALITY: u8 = 90;

impl MusicNowPlayingArtwork {
    /// makes the square cover file for now playing, encodes and writes so run it off the main thread
    pub fn make(image: &CleverImage) -> Option<Artwork> {
        let source = Self::rgba(image)?;
        let filled = Self::filling(&source, Self::square_bounds(&source));
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, JPEG_QUALITY)
            .encode_image(&DynamicImage::ImageRgba8(filled).into_rgb8())
            .ok()?;

        let path = temporary_directory().join(format!("now-playing-artwork-{}.jpg", Uuid::new_v4().uuid_string()));
        if !FileManager.create_file(&path, Some(&data)) {
            return None;
        }
        let previous = PUBLISHED_FILE.lock().replace(path.clone());
        if let Some(previous) = previous {
            let _ = FileManager.remove_item(&previous);
        }
        Some(Artwork::File(path))
    }

    fn square_bounds(image: &RgbaImage) -> (u32, u32) {
        let side = image.width().min(image.height());
        if side < 1 {
            return image.dimensions();
        }
        (side, side)
    }

    fn filling(image: &RgbaImage, size: (u32, u32)) -> RgbaImage {
        let target = (size.0.max(1), size.1.max(1));
        if image.width() < 1 || image.height() < 1 {
            return image.clone();
        }

        let scale = (f64::from(target.0) / f64::from(image.width())).max(f64::from(target.1) / f64::from(image.height()));
        let drawn = (
            ((f64::from(image.width()) * scale).round() as u32).max(target.0),
            ((f64::from(image.height()) * scale).round() as u32).max(target.1),
        );
        let resized = if drawn == image.dimensions() {
            image.clone()
        } else {
            imageops::resize(image, drawn.0, drawn.1, FilterType::Triangle)
        };
        imageops::crop_imm(&resized, (drawn.0 - target.0) / 2, (drawn.1 - target.1) / 2, target.0, target.1).to_image()
    }

    /// converts the BGRA frame gpui holds back to RGBA for the encoder
    fn rgba(image: &RenderImage) -> Option<RgbaImage> {
        let size = image.size(0);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        let mut bytes = image.as_bytes(0)?.to_vec();
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        RgbaImage::from_raw(width, height, bytes)
    }
}
