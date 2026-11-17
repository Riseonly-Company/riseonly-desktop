use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use gpui::AsyncApp;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use crate::core::media_viewer::media_viewer_image_cache::MediaViewerBitmapLoader;
use crate::core::media_viewer::media_viewer_types::{MediaViewerGalleryItem, MediaViewerGalleryItemKind};
use crate::core::ui::clever_image_disk_cache::CleverImageBitmapEncoder;
use crate::core::widgets::sheets::share_sheet::ShareSheetPhotoDraft;

/// stages the shown photo as a local file for sharing
/// a copy not the url, album objects are hard deleted when the owner removes it
/// bytes usually in memory already, just decode and write
pub enum MediaViewerShareDraft {}

impl MediaViewerShareDraft {
    const MAXIMUM_PIXEL_SIDE: f32 = 1920.0;
    const JPEG_QUALITY: f32 = 0.86;
    const MIME_TYPE: &'static str = "image/jpeg";
    const FILE_PREFIX: &'static str = "rise-share-photo-";
    const FILE_EXTENSION: &'static str = "jpg";

    pub async fn make(from_item: &MediaViewerGalleryItem, cx: &AsyncApp) -> Option<ShareSheetPhotoDraft> {
        if from_item.kind == MediaViewerGalleryItemKind::Video {
            return None;
        }
        let image = MediaViewerBitmapLoader::load_into_memory(
            &from_item.display_source(),
            true,
            from_item.persist_media_to_disk,
            from_item.image_cache_policy,
            cx,
        )
        .await?;
        let rgba = CleverImageBitmapEncoder::rgba(&image)?;

        let staged = RiseAccountRuntime::current()
            .spawn_blocking(move || Self::stage(rgba))
            .await
            .ok()
            .flatten()?;

        let (path, width, height) = staged;
        let file_url = Url::from_file_path(&path)?;
        Some(ShareSheetPhotoDraft {
            filename: file_url.last_path_component(),
            file_url,
            mime_type: Self::MIME_TYPE.to_owned(),
            width,
            height,
        })
    }

    fn stage(rgba: RgbaImage) -> Option<(PathBuf, i64, i64)> {
        let resized = Self::downscaled(rgba);
        let width = i64::from(resized.width());
        let height = i64::from(resized.height());
        let rgb = DynamicImage::ImageRgba8(resized).into_rgb8();
        let directory = temporary_directory();
        std::fs::create_dir_all(&directory).ok()?;
        let path = directory
            .join(format!("{}{}", Self::FILE_PREFIX, Uuid::new_v4().uuid_string()))
            .with_extension(Self::FILE_EXTENSION);
        let file = File::create(&path).ok()?;
        let mut writer = BufWriter::new(file);
        let quality = (Self::JPEG_QUALITY * 100.0).round().clamp(1.0, 100.0) as u8;
        let written = JpegEncoder::new_with_quality(&mut writer, quality)
            .encode_image(&rgb)
            .is_ok()
            && writer.flush().is_ok();
        drop(writer);
        if !written {
            FileManager.remove_item(&path).ok();
            return None;
        }
        Some((path, width, height))
    }

    fn downscaled(image: RgbaImage) -> RgbaImage {
        let pixel_width = image.width() as f32;
        let pixel_height = image.height() as f32;
        let longest = pixel_width.max(pixel_height);
        if longest <= Self::MAXIMUM_PIXEL_SIDE || longest <= 0.0 {
            return image;
        }
        let ratio = Self::MAXIMUM_PIXEL_SIDE / longest;
        let target_width = (pixel_width * ratio).round().max(1.0) as u32;
        let target_height = (pixel_height * ratio).round().max(1.0) as u32;
        imageops::resize(&image, target_width, target_height, FilterType::Triangle)
    }
}
