use std::sync::{Arc, LazyLock};

use gpui::RenderImage;
use image::{Frame, RgbaImage};
use rise_media::map_view::MapFrame;
use rise_platform::AppPaths;
use smallvec::SmallVec;

use crate::core::config::app_environment::AppEnvironment;

static APP_PATHS: LazyLock<Option<AppPaths>> =
    LazyLock::new(|| AppPaths::resolve(AppEnvironment::current().canon()).ok());

pub fn location_app_paths() -> Option<&'static AppPaths> {
    APP_PATHS.as_ref()
}

/// bgra picture of a map frame as one gpui image
pub fn location_map_image(frame: MapFrame) -> Option<Arc<RenderImage>> {
    let buffer = RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
    Some(Arc::new(RenderImage::new(frames)))
}
