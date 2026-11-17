use std::fmt::Debug;

use gpui::{Bounds, Edges, Point, Size, bounds, point, size};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::desktop_platform::rise_app_events::RiseNotificationName;
use crate::modules::media_picker::editor::editor_crop_auto_fit_log::EditorCropAutoFitLog;
use crate::modules::media_picker::editor::editor_image_rect::EditorImageRect;

pub const EDITOR_CROP_AUTO_FIT: RiseNotificationName = RiseNotificationName::new("editorCropAutoFit");
pub const EDITOR_CROP_AUTO_FIT_DID_FINISH: RiseNotificationName =
    RiseNotificationName::new("editorCropAutoFitDidFinish");
pub const EDITOR_CROP_AUTO_FIT_CANCEL: RiseNotificationName = RiseNotificationName::new("editorCropAutoFitCancel");
pub const EDITOR_CROP_AUTO_FIT_PRESENTATION_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("editorCropAutoFitPresentationDidChange");
pub const EDITOR_CROP_IMAGE_TRANSFORM_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("editorCropImageTransformDidChange");
pub const EDITOR_CROP_FRAME_IN_STAGE: RiseNotificationName = RiseNotificationName::new("editorCropFrameInStage");
pub const EDITOR_CROP_PINCH: RiseNotificationName = RiseNotificationName::new("editorCropPinch");
pub const EDITOR_CROP_RESET: RiseNotificationName = RiseNotificationName::new("editorCropReset");
pub const EDITOR_CROP_INTERACTION_CHANGED: RiseNotificationName =
    RiseNotificationName::new("editorCropInteractionChanged");
pub const EDITOR_CROP_GEOMETRY_REQUEST: RiseNotificationName = RiseNotificationName::new("editorCropGeometryRequest");
pub const EDITOR_CROP_GEOMETRY_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("editorCropGeometryDidChange");
pub const EDITOR_CROP_ORIENTATION_CHANGE: RiseNotificationName =
    RiseNotificationName::new("editorCropOrientationChange");
pub const EDITOR_CROP_ORIENTATION_DID_FINISH: RiseNotificationName =
    RiseNotificationName::new("editorCropOrientationDidFinish");
pub const EDITOR_CROP_FINE_ROTATION_INTERACTION: RiseNotificationName =
    RiseNotificationName::new("editorCropFineRotationInteraction");
pub const EDITOR_CROP_FINE_ROTATION_DID_FINISH: RiseNotificationName =
    RiseNotificationName::new("editorCropFineRotationDidFinish");

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCropAutoFitRequest {
    pub scope_id: Uuid,
    pub id: Uuid,
    pub source_rect: Bounds<f32>,
    pub target_rect: Bounds<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorCropAutoFitCompletionStatus {
    Completed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCropAutoFitCompletion {
    pub scope_id: Uuid,
    pub id: Uuid,
    pub status: EditorCropAutoFitCompletionStatus,
    pub projected_rect: Option<Bounds<f32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditorCropAutoFitCancel {
    pub scope_id: Uuid,
    pub id: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorCropAutoFitPresentationUpdatePhase {
    Began,
    Changed,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCropAutoFitPresentationUpdate {
    pub scope_id: Uuid,
    pub id: Uuid,
    pub phase: EditorCropAutoFitPresentationUpdatePhase,
    pub crop_frame: Bounds<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCropFrameUpdate {
    pub scope_id: Uuid,
    pub rect: Bounds<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorCropPinchUpdatePhase {
    Began,
    Changed,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCropPinchUpdate {
    pub scope_id: Uuid,
    pub phase: EditorCropPinchUpdatePhase,
    pub scale: f32,
    pub center: Point<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditorCropResetRequest {
    pub scope_id: Uuid,
    pub id: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditorCropInteractionUpdate {
    pub scope_id: Uuid,
    pub is_active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditorCropGeometryRequest {
    pub scope_id: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCropGeometryUpdate {
    pub scope_id: Uuid,
    pub image_layout: EditorImageRect,
    pub crop_frame: Bounds<f32>,
    pub completed_reset_id: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditorCropOrientationRequest {
    pub scope_id: Uuid,
    pub id: Uuid,
    pub rotation_steps: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditorCropOrientationCompletion {
    pub scope_id: Uuid,
    pub id: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCropFineRotationInteractionUpdate {
    pub scope_id: Uuid,
    pub is_active: bool,
    pub degrees: f64,
    pub completion_id: Option<Uuid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditorCropFineRotationCompletion {
    pub scope_id: Uuid,
    pub id: Uuid,
}

pub enum EditorCropAutoFit {}

impl EditorCropAutoFit {
    pub const DEBOUNCE_SECONDS: f64 = 0.55;
    pub const ANIMATION_DURATION: f64 = 0.36;
    pub const CROP_MINIMUM_ZOOM_SCALE: f32 = 1.0;
    pub const MIN_ZOOM_GAIN: f32 = 1.02;
    pub const MIN_CROP_EXTENT: f32 = 1.0;
    pub const CROP_MAXIMUM_ZOOM_SCALE: f32 = 512.0;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCropAutoFitTransitionSample {
    pub progress: f32,
    pub crop_frame: Bounds<f32>,
    pub zoom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorCropAutoFitTransition {
    pub source_frame: Bounds<f32>,
    pub target_frame: Bounds<f32>,
    pub source_zoom: f32,
    pub target_zoom: f32,
}

impl EditorCropAutoFitTransition {
    pub fn sample(&self, raw_progress: f32) -> EditorCropAutoFitTransitionSample {
        let t = raw_progress.clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        EditorCropAutoFitTransitionSample {
            progress: eased,
            crop_frame: EditorCropAutoFitPlanner::interpolate(self.source_frame, self.target_frame, eased),
            zoom: self.source_zoom + (self.target_zoom - self.source_zoom) * eased,
        }
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<f32> {
    bounds(point(x, y), size(width, height))
}

/// the smaller x edge, also for a negative width
fn min_x(r: &Bounds<f32>) -> f32 {
    r.origin.x.min(r.origin.x + r.size.width)
}

fn max_x(r: &Bounds<f32>) -> f32 {
    r.origin.x.max(r.origin.x + r.size.width)
}

fn min_y(r: &Bounds<f32>) -> f32 {
    r.origin.y.min(r.origin.y + r.size.height)
}

fn max_y(r: &Bounds<f32>) -> f32 {
    r.origin.y.max(r.origin.y + r.size.height)
}

fn mid_x(r: &Bounds<f32>) -> f32 {
    r.origin.x + r.size.width / 2.0
}

fn mid_y(r: &Bounds<f32>) -> f32 {
    r.origin.y + r.size.height / 2.0
}

/// width, always non negative
fn width(r: &Bounds<f32>) -> f32 {
    r.size.width.abs()
}

fn height(r: &Bounds<f32>) -> f32 {
    r.size.height.abs()
}

pub enum EditorCropAutoFitPlanner {}

impl EditorCropAutoFitPlanner {
    pub fn target_rect(crop: Bounds<f32>, available: Bounds<f32>, maximum_scale: f32) -> Option<Bounds<f32>> {
        let passes = width(&crop) >= EditorCropAutoFit::MIN_CROP_EXTENT && height(&crop) >= EditorCropAutoFit::MIN_CROP_EXTENT && width(&available) > 1.0 && height(&available) > 1.0 && maximum_scale >= EditorCropAutoFit::MIN_ZOOM_GAIN;
        if !passes {
            return None;
        }
        let scale = (width(&available) / width(&crop)).min(height(&available) / height(&crop)).min(maximum_scale);
        let scale_gains = scale >= EditorCropAutoFit::MIN_ZOOM_GAIN;
        if !scale_gains {
            return None;
        }
        let fitted: Size<f32> = size(width(&crop) * scale, height(&crop) * scale);
        Some(rect(
            mid_x(&available) - fitted.width / 2.0,
            mid_y(&available) - fitted.height / 2.0,
            fitted.width,
            fitted.height,
        ))
    }

    pub fn interpolate(source: Bounds<f32>, target: Bounds<f32>, progress: f32) -> Bounds<f32> {
        let t = progress.clamp(0.0, 1.0);
        rect(
            min_x(&source) + (min_x(&target) - min_x(&source)) * t,
            min_y(&source) + (min_y(&target) - min_y(&source)) * t,
            width(&source) + (width(&target) - width(&source)) * t,
            height(&source) + (height(&target) - height(&source)) * t,
        )
    }

    pub fn relative_rect(crop: Bounds<f32>, image: Bounds<f32>) -> Option<Bounds<f32>> {
        let passes = width(&image) > 0.0 && height(&image) > 0.0;
        if !passes {
            return None;
        }
        Some(rect(
            (min_x(&crop) - min_x(&image)) / width(&image),
            (min_y(&crop) - min_y(&image)) / height(&image),
            width(&crop) / width(&image),
            height(&crop) / height(&image),
        ))
    }

    pub fn project(relative: Bounds<f32>, image: Bounds<f32>) -> Bounds<f32> {
        rect(
            min_x(&image) + min_x(&relative) * width(&image),
            min_y(&image) + min_y(&relative) * height(&image),
            width(&relative) * width(&image),
            height(&relative) * height(&image),
        )
    }

    /// callers without an explicit floor pass CROP_MINIMUM_ZOOM_SCALE, the default of minimum_zoom
    pub fn minimum_zoom_covering(
        crop_size: Size<f32>,
        image_size: Size<f32>,
        rotation_radians: f32,
        minimum_zoom: f32,
    ) -> f32 {
        let passes = crop_size.width > 0.0 && crop_size.height > 0.0 && image_size.width > 0.0 && image_size.height > 0.0;
        if !passes {
            return minimum_zoom;
        }
        let cosine = rotation_radians.cos().abs();
        let sine = rotation_radians.sin().abs();
        let required_width = cosine * crop_size.width + sine * crop_size.height;
        let required_height = sine * crop_size.width + cosine * crop_size.height;
        minimum_zoom.max(required_width / image_size.width).max(required_height / image_size.height)
    }

    pub fn plan(
        crop_rect_in_stage: Bounds<f32>,
        image_rect_in_stage: Bounds<f32>,
        stage_bounds: Bounds<f32>,
        side_insets: &Edges<f32>,
        current_zoom: f32,
        maximum_zoom: f32,
    ) -> Option<f32> {
        let passes = width(&crop_rect_in_stage) >= EditorCropAutoFit::MIN_CROP_EXTENT && height(&crop_rect_in_stage) >= EditorCropAutoFit::MIN_CROP_EXTENT && width(&image_rect_in_stage) > 1.0 && height(&image_rect_in_stage) > 1.0 && width(&stage_bounds) > 1.0 && height(&stage_bounds) > 1.0;
        if !passes {
            return None;
        }

        let avail_rect = rect(
            min_x(&stage_bounds) + side_insets.left,
            min_y(&stage_bounds) + side_insets.top,
            width(&stage_bounds) - side_insets.left - side_insets.right,
            height(&stage_bounds) - side_insets.top - side_insets.bottom,
        );
        let avail_usable = width(&avail_rect) > 1.0 && height(&avail_rect) > 1.0;
        if !avail_usable {
            return None;
        }

        let crop_w = width(&crop_rect_in_stage);
        let crop_h = height(&crop_rect_in_stage);
        let avail_w = width(&avail_rect);
        if crop_w >= avail_w * 0.98 {
            EditorCropAutoFitLog::log("plan.rejected.alreadyFullWidth", &[("cropW", &crop_w as &dyn Debug), ("availW", &avail_w)]);
            return None;
        }

        let width_scale = avail_w / crop_w;
        let height_scale = height(&avail_rect) / crop_h;
        let scale_factor = width_scale.min(height_scale);
        let limit_reason = if width_scale <= height_scale { "width" } else { "height" };

        if scale_factor < EditorCropAutoFit::MIN_ZOOM_GAIN {
            EditorCropAutoFitLog::log(
                "plan.rejected.minGain",
                &[
                    ("currentZoom", &current_zoom as &dyn Debug),
                    ("scaleFactor", &scale_factor),
                    ("cropW", &crop_w),
                    ("cropH", &crop_h),
                    ("availW", &avail_w),
                    ("limit", &limit_reason),
                ],
            );
            return None;
        }

        let target_zoom = (current_zoom * scale_factor).min(maximum_zoom);

        if target_zoom < current_zoom * EditorCropAutoFit::MIN_ZOOM_GAIN {
            EditorCropAutoFitLog::log(
                "plan.rejected.capped",
                &[("currentZoom", &current_zoom as &dyn Debug), ("targetZoom", &target_zoom), ("maximumZoom", &maximum_zoom)],
            );
            return None;
        }

        EditorCropAutoFitLog::log(
            "plan.accepted",
            &[
                ("currentZoom", &current_zoom as &dyn Debug),
                ("targetZoom", &target_zoom),
                ("scaleFactor", &scale_factor),
                ("limit", &limit_reason),
                ("crop", &crop_rect_in_stage),
            ],
        );
        Some(target_zoom)
    }

    pub fn plan_expanded_crop_rect(
        crop: Bounds<f32>,
        clamp_bounds: Bounds<f32>,
        horizontal_slot: Bounds<f32>,
        top_limit: f32,
        toolbar_top: f32,
    ) -> Option<Bounds<f32>> {
        let passes = width(&crop) >= EditorCropAutoFit::MIN_CROP_EXTENT && height(&crop) >= EditorCropAutoFit::MIN_CROP_EXTENT && width(&clamp_bounds) > 1.0 && height(&clamp_bounds) > 1.0 && width(&horizontal_slot) > 1.0;
        if !passes {
            return None;
        }

        let left_limit = min_x(&horizontal_slot).max(min_x(&clamp_bounds));
        let right_limit = max_x(&horizontal_slot).min(max_x(&clamp_bounds));
        let target_width = right_limit - left_limit;
        let width_usable = target_width > EditorCropAutoFit::MIN_CROP_EXTENT;
        if !width_usable {
            return None;
        }

        let crop_w = width(&crop);
        let crop_h = height(&crop);
        if crop_w >= target_width * 0.98 {
            EditorCropAutoFitLog::log(
                "expand.rejected.alreadyFullWidth",
                &[("cropW", &crop_w as &dyn Debug), ("targetW", &target_width)],
            );
            return None;
        }

        let mut scale = target_width / crop_w;
        let mut new_h = crop_h * scale;
        let mid = mid_y(&crop);
        let mut half_h = new_h / 2.0;

        let bottom_limit = toolbar_top.min(max_y(&clamp_bounds));
        let top_bound = top_limit.max(min_y(&clamp_bounds));

        if mid - half_h < top_bound {
            let room = mid - top_bound;
            if room > 0.5 {
                scale = scale.min((room * 2.0) / crop_h);
            }
        }
        if mid + half_h > bottom_limit {
            let room = bottom_limit - mid;
            if room > 0.5 {
                scale = scale.min((room * 2.0) / crop_h);
            }
        }

        if scale < EditorCropAutoFit::MIN_ZOOM_GAIN {
            EditorCropAutoFitLog::log(
                "expand.rejected.minGain",
                &[("scale", &scale as &dyn Debug), ("crop", &crop), ("targetW", &target_width)],
            );
            return None;
        }

        new_h = crop_h * scale;
        half_h = new_h / 2.0;
        let mut target = rect(left_limit, mid - half_h, target_width, new_h);

        if min_y(&target) < min_y(&clamp_bounds) {
            target.origin.y = min_y(&clamp_bounds);
        }
        if max_y(&target) > max_y(&clamp_bounds) {
            target.origin.y = max_y(&clamp_bounds) - height(&target);
        }
        if min_x(&target) < min_x(&clamp_bounds) {
            target.origin.x = min_x(&clamp_bounds);
        }
        if max_x(&target) > max_x(&clamp_bounds) {
            target.origin.x = max_x(&clamp_bounds) - width(&target);
        }

        if width(&target) < crop_w * EditorCropAutoFit::MIN_ZOOM_GAIN {
            return None;
        }

        EditorCropAutoFitLog::log("expand.accepted", &[("from", &crop as &dyn Debug), ("to", &target), ("clamp", &clamp_bounds)]);
        Some(target)
    }

    pub fn is_effectively_full(relative: Option<Bounds<f32>>) -> bool {
        let Some(relative) = relative else {
            return true;
        };
        let epsilon: f32 = 0.000_000_1;
        min_x(&relative) <= epsilon
            && min_y(&relative) <= epsilon
            && width(&relative) >= 1.0 - epsilon
            && height(&relative) >= 1.0 - epsilon
    }

    pub fn rect_in_image_container(relative: Bounds<f32>, container_bounds: Bounds<f32>) -> Bounds<f32> {
        rect(
            relative.origin.x * width(&container_bounds),
            relative.origin.y * height(&container_bounds),
            relative.size.width * width(&container_bounds),
            relative.size.height * height(&container_bounds),
        )
    }

    pub fn crop_in_image_container(
        crop_rect_in_stage: Bounds<f32>,
        image_rect_in_stage: Bounds<f32>,
        container_bounds: Bounds<f32>,
    ) -> Bounds<f32> {
        let passes = width(&image_rect_in_stage) > 0.0 && height(&image_rect_in_stage) > 0.0;
        if !passes {
            return Bounds::default();
        }
        rect(
            (min_x(&crop_rect_in_stage) - min_x(&image_rect_in_stage)) / width(&image_rect_in_stage)
                * width(&container_bounds),
            (min_y(&crop_rect_in_stage) - min_y(&image_rect_in_stage)) / height(&image_rect_in_stage)
                * height(&container_bounds),
            width(&crop_rect_in_stage) / width(&image_rect_in_stage) * width(&container_bounds),
            height(&crop_rect_in_stage) / height(&image_rect_in_stage) * height(&container_bounds),
        )
    }
}
