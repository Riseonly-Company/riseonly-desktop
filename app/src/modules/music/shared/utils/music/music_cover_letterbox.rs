use std::sync::{Arc, LazyLock};

use gpui::RenderImage;
use image::imageops::{self, FilterType};
use image::{Frame, ImageBuffer, Rgba};
use rise_core::foundation::ns_cache::NSCache;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::CleverImage;

/// removes the black padding baked into a track cover
/// video thumbnails arrive letterboxed, eg 45 black rows top and bottom
/// those rows are bitmap so aspect fill cant remove them, cropped once per cover
pub enum MusicCoverLetterbox {}

/// insets of the padding in the bitmaps own pixels
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PaddingInsets {
    top: u32,
    left: u32,
    bottom: u32,
    right: u32,
}

/// the bgra frame gpui holds, read in place
type BitmapView<'a> = ImageBuffer<Rgba<u8>, &'a [u8]>;

static INSETS_CACHE: LazyLock<NSCache<String, PaddingInsets>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(512);
    cache
});

const SAMPLE_SIDE: u32 = 320;
const DARK_CHANNEL_CEILING: u8 = 20;
const MAXIMUM_PADDING_FRACTION: f64 = 0.55;
const MINIMUM_PADDING_FRACTION: f64 = 0.03;

impl MusicCoverLetterbox {
    pub fn trimmed_cache_key(image: &CleverImage, cache_key: &str) -> CleverImage {
        let Some(source) = Self::bitmap(image) else { return image.clone() };
        let key = format!(
            "{}|{}|{}x{}",
            SaiAccountCacheScope::active_user_id().unwrap_or_else(|| "anonymous".to_owned()),
            cache_key,
            source.width(),
            source.height()
        );
        let insets = match INSETS_CACHE.object(&key) {
            Some(cached) => cached,
            None => {
                let insets = Self::padding(&source);
                INSETS_CACHE.set_object(insets, key);
                insets
            }
        };
        Self::cropped(image, &source, insets)
    }

    pub fn trimmed(image: &CleverImage) -> CleverImage {
        let Some(source) = Self::bitmap(image) else { return image.clone() };
        Self::cropped(image, &source, Self::padding(&source))
    }

    fn bitmap(image: &RenderImage) -> Option<BitmapView<'_>> {
        if image.frame_count() != 1 {
            return None;
        }
        let size = image.size(0);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        ImageBuffer::from_raw(width, height, image.as_bytes(0)?)
    }

    fn cropped(image: &CleverImage, source: &BitmapView<'_>, padding: PaddingInsets) -> CleverImage {
        if padding == PaddingInsets::default() {
            return image.clone();
        }
        let width = source.width().checked_sub(padding.left).and_then(|rest| rest.checked_sub(padding.right));
        let height = source.height().checked_sub(padding.top).and_then(|rest| rest.checked_sub(padding.bottom));
        let (Some(width), Some(height)) = (width, height) else { return image.clone() };
        if width < 8 || height < 8 {
            return image.clone();
        }
        let cropped = ImageBuffer::from_fn(width, height, |x, y| *source.get_pixel(x + padding.left, y + padding.top));
        let frame = Frame::from_parts(cropped, 0, 0, image.delay(0));
        Arc::new(RenderImage::new(vec![frame]))
    }

    fn padding(source: &BitmapView<'_>) -> PaddingInsets {
        let width = source.width();
        let height = source.height();
        if width < 16 || height < 16 {
            return PaddingInsets::default();
        }

        let scale = (f64::from(SAMPLE_SIDE) / f64::from(width.max(height))).min(1.0);
        let sample_width = ((f64::from(width) * scale).round() as u32).max(1);
        let sample_height = ((f64::from(height) * scale).round() as u32).max(1);
        let sample = imageops::resize(source, sample_width, sample_height, FilterType::Triangle);

        let is_dark = |x: u32, y: u32| {
            let pixel = sample.get_pixel(x, y).0;
            let alpha = u32::from(pixel[3]);
            pixel[..3].iter().all(|&channel| {
                let over_white = (u32::from(channel) * alpha + 255 * (255 - alpha)) / 255;
                over_white <= u32::from(DARK_CHANNEL_CEILING)
            })
        };

        let mut top = 0;
        while top < sample_height && (0..sample_width).all(|x| is_dark(x, top)) {
            top += 1;
        }
        let mut bottom = 0;
        while bottom < sample_height - top && (0..sample_width).all(|x| is_dark(x, sample_height - 1 - bottom)) {
            bottom += 1;
        }
        let (top, bottom) = Self::accepted(top, bottom, sample_height);

        let band = top..(sample_height - bottom);
        let mut left = 0;
        while left < sample_width && band.clone().all(|y| is_dark(left, y)) {
            left += 1;
        }
        let mut right = 0;
        while right < sample_width - left && band.clone().all(|y| is_dark(sample_width - 1 - right, y)) {
            right += 1;
        }
        let (left, right) = Self::accepted(left, right, sample_width);

        PaddingInsets {
            top: Self::mapped(top, sample_height, height),
            left: Self::mapped(left, sample_width, width),
            bottom: Self::mapped(bottom, sample_height, height),
            right: Self::mapped(right, sample_width, width),
        }
    }

    /// padding is padding only when it frames the picture on both sides
    /// a dark edge on one side belongs to the artwork, a band wider than the picture isnt a band
    fn accepted(leading: u32, trailing: u32, extent: u32) -> (u32, u32) {
        if leading == 0 || trailing == 0 {
            return (0, 0);
        }
        let total = f64::from(leading + trailing);
        if total > f64::from(extent) * MAXIMUM_PADDING_FRACTION || total < f64::from(extent) * MINIMUM_PADDING_FRACTION {
            return (0, 0);
        }
        (leading, trailing)
    }

    /// the sampled copy blends a bands last row with the pictures first
    /// row so the mapped inset grows by one sampled row
    /// a lost pixel is invisible, a dark seam isnt
    fn mapped(sampled: u32, extent: u32, source_extent: u32) -> u32 {
        if sampled == 0 {
            return 0;
        }
        let ratio = f64::from(source_extent) / f64::from(extent);
        let grown = (f64::from(sampled) * ratio).floor() as u32 + ratio.ceil() as u32;
        source_extent.min(grown)
    }
}
