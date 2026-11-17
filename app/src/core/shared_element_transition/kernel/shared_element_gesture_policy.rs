use gpui::{Pixels, Point, point};

use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementGeometry, SharedElementGesturePolicy, SharedElementGesturePolicyAxis,
    SharedElementGesturePolicyDecision, SharedElementGesturePolicyFrame, SharedElementGesturePolicyInput,
};

impl SharedElementGesturePolicy {
    /// finger follow frame for one pan sample, all in screen space
    pub fn sample(input: &SharedElementGesturePolicyInput) -> SharedElementGesturePolicyFrame {
        let width = f32::from(input.viewport.width);
        let height = f32::from(input.viewport.height);
        let (dx, dy) = (f32::from(input.translation.x), f32::from(input.translation.y));
        let distance = dx.hypot(dy);

        let reference = SharedElementMotion::DRAG_REFERENCE_FRACTION * width.min(height);
        let ratio = if reference > 0.0 { distance / reference } else { 0.0 };
        let damped = ratio / (1.0 + ratio);
        let scale_floor = SharedElementMotion::SCALE_FLOOR;
        let scale = scale_floor.max(1.0 - (1.0 - scale_floor) * damped);

        let finger = point(input.touch_start.x + input.translation.x, input.touch_start.y + input.translation.y);
        let source_center = input.source.center();
        let center = point(
            finger.x - (input.touch_start.x - source_center.x) * scale,
            finger.y - (input.touch_start.y - source_center.y) * scale,
        );
        let clip = SharedElementGeometry::rect(
            center,
            gpui::size(input.source.size.width * scale, input.source.size.height * scale),
        );

        let fade_distance = SharedElementMotion::CHROME_FADE_DISTANCE_FRACTION * width;
        let chrome_alpha = if fade_distance > 0.0 {
            1.0 - SharedElementGeometry::smoothstep((distance / fade_distance).min(1.0))
        } else {
            1.0
        };
        let progress = match input.axis {
            SharedElementGesturePolicyAxis::Horizontal => {
                if width > 0.0 {
                    (Self::layout_translation(input) / width).max(0.0).min(1.0)
                } else {
                    0.0
                }
            }
            SharedElementGesturePolicyAxis::Vertical => {
                if height > 0.0 {
                    (dy / height).max(0.0).min(1.0)
                } else {
                    0.0
                }
            }
        };

        SharedElementGesturePolicyFrame { clip, scale, chrome_alpha, progress }
    }

    pub fn decision(input: &SharedElementGesturePolicyInput) -> SharedElementGesturePolicyDecision {
        match input.axis {
            SharedElementGesturePolicyAxis::Horizontal => Self::horizontal_decision(input),
            SharedElementGesturePolicyAxis::Vertical => Self::vertical_decision(input),
        }
    }

    fn vertical_decision(input: &SharedElementGesturePolicyInput) -> SharedElementGesturePolicyDecision {
        let ty = f32::from(input.translation.y);
        let vy = f32::from(input.velocity.y);
        let vx = f32::from(input.velocity.x);
        if vy < SharedElementMotion::CANCEL_VELOCITY {
            return SharedElementGesturePolicyDecision::Cancel;
        }
        if ty > SharedElementMotion::DISMISS_COMMIT_DISTANCE_FRACTION * f32::from(input.viewport.height) {
            return SharedElementGesturePolicyDecision::Finish;
        }
        if vy > SharedElementMotion::COMMIT_VELOCITY && vy > vx.abs() {
            return SharedElementGesturePolicyDecision::Finish;
        }
        let distance = f32::from(input.translation.x).hypot(ty);
        if distance > SharedElementMotion::CHROME_FADE_DISTANCE_FRACTION * f32::from(input.viewport.width)
            && vy > SharedElementMotion::CANCEL_VELOCITY
            && ty > 0.0
        {
            return SharedElementGesturePolicyDecision::Finish;
        }
        SharedElementGesturePolicyDecision::Cancel
    }

    fn horizontal_decision(input: &SharedElementGesturePolicyInput) -> SharedElementGesturePolicyDecision {
        let width = f32::from(input.viewport.width);
        let tx = Self::layout_translation(input);
        let vx = Self::layout_velocity(input);
        let distance = f32::from(input.translation.x).hypot(f32::from(input.translation.y));

        if vx < SharedElementMotion::CANCEL_VELOCITY {
            return SharedElementGesturePolicyDecision::Cancel;
        }
        if tx > SharedElementMotion::COMMIT_DISTANCE_FRACTION * width {
            return SharedElementGesturePolicyDecision::Finish;
        }
        if vx > SharedElementMotion::COMMIT_VELOCITY && vx > f32::from(input.velocity.y).abs() {
            return SharedElementGesturePolicyDecision::Finish;
        }
        if distance > SharedElementMotion::CHROME_FADE_DISTANCE_FRACTION * width
            && vx > SharedElementMotion::CANCEL_VELOCITY
        {
            return SharedElementGesturePolicyDecision::Finish;
        }
        SharedElementGesturePolicyDecision::Cancel
    }

    /// initial spring velocity relative to the remaining distance, capped at MAX_RELATIVE_VELOCITY
    pub fn relative_spring_velocity(velocity: Point<Pixels>, from: Point<Pixels>, to: Point<Pixels>) -> f32 {
        let dx = f32::from(to.x - from.x);
        let dy = f32::from(to.y - from.y);
        let length = dx.hypot(dy);
        if !(length >= 1.0) {
            return 0.0;
        }
        let along = (f32::from(velocity.x) * dx + f32::from(velocity.y) * dy) / length;
        let relative = along / length;
        if !relative.is_finite() {
            return 0.0;
        }
        relative.max(0.0).min(SharedElementMotion::MAX_RELATIVE_VELOCITY)
    }

    fn layout_sign(input: &SharedElementGesturePolicyInput) -> f32 {
        if input.layout_sign < 0.0 { -1.0 } else { 1.0 }
    }

    fn layout_translation(input: &SharedElementGesturePolicyInput) -> f32 {
        f32::from(input.translation.x) * Self::layout_sign(input)
    }

    fn layout_velocity(input: &SharedElementGesturePolicyInput) -> f32 {
        f32::from(input.velocity.x) * Self::layout_sign(input)
    }
}
