use std::f32::consts::PI;
use std::ops::RangeInclusive;

use gpui::{Point, Size};
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};

/// snapping rules for transformable canvas entities like text and the music plate
/// center magnetizes to the canvas midlines and rotation to right angles
/// reaches are small on purpose, an assist and not a grid
pub struct EditorEntitySnap;

impl EditorEntitySnap {
    pub const CENTER_REACH: f32 = 7.0;
    pub const ROTATION_REACH_DEGREES: f32 = 3.5;

    /// the center also stays inside the canvas
    /// the canvas clips to the photo so an entity dragged past its edge couldnt be seen or grabbed back
    /// returns the point and snapped_x and snapped_y
    pub fn snap_center(proposed: Point<f32>, canvas: Size<f32>) -> (Point<f32>, bool, bool) {
        if canvas.width <= 0.0 || canvas.height <= 0.0 {
            return (proposed, false, false);
        }
        let mut point = Point {
            x: proposed.x.max(0.0).min(canvas.width),
            y: proposed.y.max(0.0).min(canvas.height),
        };
        let snapped_x = (point.x - canvas.width / 2.0).abs() <= Self::CENTER_REACH;
        let snapped_y = (point.y - canvas.height / 2.0).abs() <= Self::CENTER_REACH;
        if snapped_x {
            point.x = canvas.width / 2.0;
        }
        if snapped_y {
            point.y = canvas.height / 2.0;
        }
        (point, snapped_x, snapped_y)
    }

    /// returns the angle and whether it snapped
    pub fn snap_rotation(angle: f32) -> (f32, bool) {
        let quarter = PI / 2.0;
        let nearest = (angle / quarter).round() * quarter;
        if (angle - nearest).abs() <= Self::ROTATION_REACH_DEGREES * PI / 180.0 {
            return (nearest, true);
        }
        (angle, false)
    }
}

/// side handle scales and rotates its entity around the entity center
/// so the handle stays under the pointer
/// angle follows the pointer turn and size its distance
pub struct EditorEntitySideHandle;

impl EditorEntitySideHandle {
    pub const SCALE_MIN: f32 = 0.3;
    pub const SCALE_MAX: f32 = 4.0;
    pub const SCALE_RANGE: RangeInclusive<f32> = Self::SCALE_MIN..=Self::SCALE_MAX;

    /// handle_offset is the handle center minus the entity center at gesture start
    /// returns the scale and the rotation
    pub fn scale_and_rotation(
        handle_offset: Point<f32>,
        translation: Point<f32>,
        start_scale: f32,
        start_rotation: f32,
    ) -> (f32, f32) {
        let current = Point {
            x: handle_offset.x + translation.x,
            y: handle_offset.y + translation.y,
        };
        let start_length = handle_offset.x.hypot(handle_offset.y);
        let length = current.x.hypot(current.y);
        if start_length <= 1.0 || length <= 1.0 {
            return (start_scale, start_rotation);
        }

        let scale = (start_scale * length / start_length).max(Self::SCALE_MIN).min(Self::SCALE_MAX);
        let mut turn = current.y.atan2(current.x) - handle_offset.y.atan2(handle_offset.x);
        if turn > PI {
            turn -= 2.0 * PI;
        } else if turn <= -PI {
            turn += 2.0 * PI;
        }
        (scale, start_rotation + turn)
    }
}

/// one light haptic per transition into a snapped state, never while resting inside it
pub struct EditorEntitySnapFeedback {
    generator: ImpactFeedbackGenerator,
    was_snapped_x: bool,
    was_snapped_y: bool,
    was_snapped_rotation: bool,
}

impl Default for EditorEntitySnapFeedback {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorEntitySnapFeedback {
    pub fn new() -> Self {
        Self {
            generator: ImpactFeedbackGenerator::new(ImpactStyle::Light),
            was_snapped_x: false,
            was_snapped_y: false,
            was_snapped_rotation: false,
        }
    }

    pub fn begin(&mut self) {
        self.generator.prepare();
        self.was_snapped_x = false;
        self.was_snapped_y = false;
        self.was_snapped_rotation = false;
    }

    pub fn update_center(&mut self, snapped_x: bool, snapped_y: bool) {
        if (snapped_x && !self.was_snapped_x) || (snapped_y && !self.was_snapped_y) {
            self.generator.impact_occurred_with_intensity(0.7);
        }
        self.was_snapped_x = snapped_x;
        self.was_snapped_y = snapped_y;
    }

    pub fn update_rotation(&mut self, snapped: bool) {
        if snapped && !self.was_snapped_rotation {
            self.generator.impact_occurred_with_intensity(0.7);
        }
        self.was_snapped_rotation = snapped;
    }
}
