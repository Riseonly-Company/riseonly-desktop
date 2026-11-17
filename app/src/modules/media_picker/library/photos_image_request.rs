use std::path::Path;
use std::sync::Arc;

use gpui::{RenderImage, Size};
use image::{Delay, Frame, RgbaImage};
use parking_lot::Mutex;
use rise_platform::photo_library::{
    self, ExportOutcome, ImageContentMode, ImageDelivery, ImageDeliveryMode, ImageRequestId, ImageSpec, LibraryImage,
};
use smallvec::SmallVec;

use crate::core::ui::clever_image_disk_cache::CleverImage;

/// one answer of an image request, the picture or why there is none yet
#[derive(Clone)]
pub struct PhotoImageDelivery {
    pub image: Option<CleverImage>,
    pub is_cancelled: bool,
    pub is_degraded: bool,
}

impl PhotoImageDelivery {
    fn from_library(delivery: ImageDelivery) -> Self {
        Self {
            image: delivery.image.and_then(Self::render_image),
            is_cancelled: delivery.cancelled,
            is_degraded: delivery.degraded,
        }
    }

    fn render_image(image: LibraryImage) -> Option<CleverImage> {
        let bitmap = RgbaImage::from_raw(image.width, image.height, image.bgra)?;
        let frame = Frame::from_parts(bitmap, 0, 0, Delay::from_numer_denom_ms(0, 1));
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([frame]);
        Some(Arc::new(RenderImage::new(frames)))
    }
}

pub enum PhotosImageRequest {}

impl PhotosImageRequest {
    pub fn request_image(
        asset_id: &str,
        target_size: Size<f32>,
        content_mode: ImageContentMode,
        delivery_mode: ImageDeliveryMode,
        is_network_access_allowed: bool,
        result: impl FnMut(PhotoImageDelivery) + Send + 'static,
    ) -> Option<ImageRequestId> {
        let spec = Self::image_spec(target_size, content_mode, delivery_mode, is_network_access_allowed);
        Self::started_request(result, |sink| photo_library::request_image(asset_id, spec, sink))
    }

    pub fn request_image_with_progress(
        asset_id: &str,
        target_size: Size<f32>,
        content_mode: ImageContentMode,
        delivery_mode: ImageDeliveryMode,
        is_network_access_allowed: bool,
        on_progress: impl Fn(f64) + Send + Sync + 'static,
        result: impl FnMut(PhotoImageDelivery) + Send + 'static,
    ) -> Option<ImageRequestId> {
        let spec = Self::image_spec(target_size, content_mode, delivery_mode, is_network_access_allowed);
        Self::started_request(result, |sink| {
            photo_library::request_image_with_progress(asset_id, spec, on_progress, sink)
        })
    }

    fn image_spec(
        target_size: Size<f32>,
        content_mode: ImageContentMode,
        delivery_mode: ImageDeliveryMode,
        is_network_access_allowed: bool,
    ) -> ImageSpec {
        ImageSpec {
            width: Self::pixel_side(target_size.width),
            height: Self::pixel_side(target_size.height),
            mode: content_mode,
            delivery: delivery_mode,
            allow_network: is_network_access_allowed,
        }
    }

    fn started_request<Failure: std::fmt::Display>(
        result: impl FnMut(PhotoImageDelivery) + Send + 'static,
        start: impl FnOnce(Box<dyn FnMut(ImageDelivery) + Send>) -> Result<ImageRequestId, Failure>,
    ) -> Option<ImageRequestId> {
        let result: Arc<Mutex<Option<Box<dyn FnMut(PhotoImageDelivery) + Send>>>> =
            Arc::new(Mutex::new(Some(Box::new(result))));
        let sink_result = Arc::clone(&result);
        let started = start(Box::new(move |delivery| {
            if let Some(result) = sink_result.lock().as_mut() {
                result(PhotoImageDelivery::from_library(delivery));
            }
        }));
        match started {
            Ok(request_id) => Some(request_id),
            Err(error) => {
                tracing::debug!(target: "riseonly", %error, "photo library image request not started");
                if let Some(mut result) = result.lock().take() {
                    result(PhotoImageDelivery {
                        image: None,
                        is_cancelled: false,
                        is_degraded: false,
                    });
                }
                None
            }
        }
    }

    /// desktop has no AVAsset so the assets video arrives as a file in directory
    /// cancelled and error info keys map to the Cancelled and Failed outcomes
    pub fn request_av_asset(
        asset_id: &str,
        directory: &Path,
        result: impl FnOnce(ExportOutcome) + Send + 'static,
    ) -> Option<ImageRequestId> {
        let result = Arc::new(Mutex::new(Some(
            Box::new(result) as Box<dyn FnOnce(ExportOutcome) + Send>
        )));
        let export_result = Arc::clone(&result);
        let started = photo_library::export_video(asset_id, directory, move |outcome| {
            if let Some(result) = export_result.lock().take() {
                result(outcome);
            }
        });
        match started {
            Ok(request_id) => Some(request_id),
            Err(error) => {
                if let Some(result) = result.lock().take() {
                    result(ExportOutcome::Failed(error));
                }
                None
            }
        }
    }

    fn pixel_side(side: f32) -> u32 {
        if side.is_finite() && side >= 1.0 {
            side.round().min(u32::MAX as f32) as u32
        } else {
            1
        }
    }
}
