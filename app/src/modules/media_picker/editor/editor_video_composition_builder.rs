use std::sync::Arc;

use gpui::RenderImage;
use image::imageops::{FilterType, resize};
use image::{Frame, RgbaImage};
use rise_canon::RiseLimits;
use rise_media::video_export::{
    Affine, ColorAdjustments, FrameGeometry, Overlay, RelativeRect, RenderPlan, VideoProbe, render_frame_rate,
    render_size, segment_transform,
};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::media_picker::editor::editor_crop_auto_fit::EditorCropAutoFitPlanner;
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_state::{EditorImageAdjustments, EditorImageState};
use crate::modules::media_picker::editor::editor_sticker_entity::EditorStickerArtStore;

/// how export frames are rendered and the transform that places the clips stored frame into that render
#[derive(Clone, Debug, PartialEq)]
pub struct EditorVideoComposition {
    pub render: RenderPlan,
    pub transform: Affine,
}

pub enum EditorVideoCompositionBuilder {}

impl EditorVideoCompositionBuilder {
    const ROTATION_EPSILON_DEGREES: f64 = 0.0001;

    fn maximum_render_side() -> f64 {
        RiseLimits::client_value("editor_video_composition_builder_maximum_render_side") as f64
    }

    /// builds the composition, none when the edit changes no pixels or the clip has no video track
    /// then export keeps the plain preset path
    pub fn make(probe: &VideoProbe, spatial: Option<&EditorImageState>) -> Option<EditorVideoComposition> {
        let spatial = spatial?;
        if !Self::has_visual_edits(spatial) || !probe.has_video {
            return None;
        }
        let geometry = Self::geometry(spatial);
        let target_size = render_size(probe, geometry, Self::maximum_render_side());
        let overlay = Self::overlay_image(target_size, spatial);
        let stickers = spatial.stickers.as_deref().unwrap_or_default();
        if !stickers.is_empty() {
            let loops = stickers.iter().filter(|entity| entity.loops).count();
            let cached = stickers
                .iter()
                .filter(|entity| EditorStickerArtStore::shared().art_for_bake(entity).is_some())
                .count();
            tracing::info!(
                target: "StickerBake",
                "[StickerBake] video size={}x{} stickers={} loops={} cached={} overlay={}",
                target_size.0,
                target_size.1,
                stickers.len(),
                loops,
                cached,
                overlay.is_some()
            );
        }
        Some(EditorVideoComposition {
            render: RenderPlan {
                size: target_size,
                frame_rate: render_frame_rate(probe.nominal_frame_rate),
                adjustments: Self::adjustments(&spatial.adjustments),
                overlay,
                loop_layers: Vec::new(),
                badge: None,
            },
            transform: segment_transform(probe, geometry, target_size),
        })
    }

    fn has_visual_edits(spatial: &EditorImageState) -> bool {
        spatial.rotation_steps % 4 != 0
            || spatial.is_mirrored
            || spatial.free_rotation_degrees.abs() > Self::ROTATION_EPSILON_DEGREES
            || !EditorCropAutoFitPlanner::is_effectively_full(spatial.crop_rect_relative)
            || !spatial.adjustments.is_identity()
            || !spatial.strokes.is_empty()
            || !spatial.texts.is_empty()
            || spatial.stickers.as_ref().is_some_and(|stickers| !stickers.is_empty())
            || spatial.links.as_ref().is_some_and(|links| !links.is_empty())
    }

    /// geometry members of the edit, applied in order
    /// quarter turns then mirror then free rotation then crop
    fn geometry(spatial: &EditorImageState) -> FrameGeometry {
        FrameGeometry {
            rotation_steps: spatial.rotation_steps.rem_euclid(4) as i32,
            mirrored: spatial.is_mirrored,
            free_rotation_degrees: spatial.free_rotation_degrees,
            crop: spatial.crop_rect_relative.map(|crop| RelativeRect {
                x: f64::from(crop.origin.x),
                y: f64::from(crop.origin.y),
                width: f64::from(crop.size.width),
                height: f64::from(crop.size.height),
            }),
        }
    }

    /// color adjustments per frame, same filter chain runs on every source frame before the geometry
    fn adjustments(adjustments: &EditorImageAdjustments) -> ColorAdjustments {
        ColorAdjustments {
            enhance: adjustments.enhance,
            brightness: adjustments.brightness,
            contrast: adjustments.contrast,
            saturation: adjustments.saturation,
            temperature: adjustments.temperature,
            highlights: adjustments.highlights,
            shadows: adjustments.shadows,
            vignette: adjustments.vignette,
        }
    }

    /// strokes and texts baked on a clear canvas of the render size
    /// spatial members are reset so they land where the editor drew them
    fn overlay_image(size: (u32, u32), spatial: &EditorImageState) -> Option<Overlay> {
        if spatial.strokes.is_empty()
            && spatial.texts.is_empty()
            && spatial.stickers.as_ref().is_none_or(|stickers| stickers.is_empty())
            && spatial.links.as_ref().is_none_or(|links| links.is_empty())
        {
            return None;
        }
        let clear = Arc::new(RenderImage::new([Frame::new(RgbaImage::new(size.0, size.1))]));
        let mut overlay_state = spatial.clone();
        overlay_state.rotation_steps = 0;
        overlay_state.free_rotation_degrees = 0.0;
        overlay_state.is_mirrored = false;
        overlay_state.crop_rect_relative = None;
        overlay_state.adjustments = EditorImageAdjustments::default();
        overlay_state.story_canvas = None;
        let baked = EditorImageBake::render_edited_image(&clear, &overlay_state);
        Self::overlay(&baked, size)
    }

    /// baked frame as a straight rgba overlay of exactly the render size
    fn overlay(image: &RenderImage, size: (u32, u32)) -> Option<Overlay> {
        let bytes = image.as_bytes(0)?;
        let image_size = image.size(0);
        let width = u32::try_from(image_size.width.0).ok()?;
        let height = u32::try_from(image_size.height.0).ok()?;
        let count = (width as usize).checked_mul(height as usize)?;
        if count == 0 || bytes.len() < count * 4 {
            return None;
        }
        let mut rgba = Vec::with_capacity(count * 4);
        for bgra in bytes.chunks_exact(4).take(count) {
            rgba.extend_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
        }
        let mut frame = RgbaImage::from_raw(width, height, rgba)?;
        if (width, height) != size {
            frame = resize(&frame, size.0, size.1, FilterType::Triangle);
        }
        Some(Overlay {
            width: size.0,
            height: size.1,
            rgba: Arc::from(frame.into_raw()),
        })
    }
}
