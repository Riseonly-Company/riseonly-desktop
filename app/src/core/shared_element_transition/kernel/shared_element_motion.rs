use gpui::Point;

use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementCubicTiming;

pub enum SharedElementMotion {}

impl SharedElementMotion {
    pub const MOVE_DURATION: f64 = 0.42;
    pub const CURVE: SharedElementCubicTiming = SharedElementCubicTiming {
        c1: Point { x: 0.22, y: 0.82 },
        c2: Point { x: 0.12, y: 1.0 },
    };

    pub const COVER_FADE_START_PROGRESS: f32 = 0.4;
    pub const COVER_KEYFRAME_SAMPLES: usize = 8;

    pub const OPEN_TARGET_WAIT_TICKS: usize = 6;
    pub const OPEN_TARGET_STABLE_DELTA: f32 = 0.5;
    pub const FULL_IMAGE_HOLD: f64 = 0.6;
    pub const FULL_IMAGE_CROSSFADE: f64 = 0.15;
    pub const OPEN_FALLBACK_FADE: f64 = 0.22;
    pub const OPEN_FALLBACK_SCALE: f32 = 0.96;

    pub const EXIT_RESOLVE_TIMEOUT: f64 = 0.25;
    pub const STOP_FRAME_BUDGET: f64 = 0.08;
    pub const RETARGET_CHECK_DELAY: f64 = 0.05;
    pub const LANDING_MISMATCH: f32 = 1.0;
    pub const LANDING_CROSSFADE: f64 = 0.12;
    pub const EXIT_FALLBACK_FADE: f64 = 0.25;
    pub const EXIT_FALLBACK_SCALE: f32 = 0.92;
    pub const STOCK_FADE_SCALE: f32 = 0.92;

    pub const LIVE_VIDEO_READY_BUDGET: f64 = 0.12;

    pub const DRAG_REFERENCE_FRACTION: f32 = 0.5;
    pub const SCALE_FLOOR: f32 = 0.45;
    pub const CHROME_FADE_DISTANCE_FRACTION: f32 = 0.35;
    pub const COMMIT_DISTANCE_FRACTION: f32 = 0.3;
    pub const DISMISS_COMMIT_DISTANCE_FRACTION: f32 = 0.15;
    pub const COMMIT_VELOCITY: f32 = 800.0;
    pub const CANCEL_VELOCITY: f32 = -200.0;
    pub const FINISH_DAMPING: f32 = 0.88;
    pub const CANCEL_DAMPING: f32 = 1.0;
    pub const CANCEL_DURATION: f64 = 0.34;
    pub const MAX_RELATIVE_VELOCITY: f32 = 12.0;

    pub const INTENT_TTL: f64 = 0.5;
    pub const WATCHDOG: f64 = 1.5;

    pub const REDUCE_MOTION_FADE: f64 = 0.2;

    pub const OVERLAY_CURTAIN_FADE: f64 = 0.2;
    pub const OVERLAY_CHROME_FADE: f64 = 0.18;

    pub const BACKDROP_PIXEL_BUCKET: i64 = 96;
    pub const BACKDROP_BLUR: f32 = 18.0;
    pub const BACKDROP_DIM: f32 = 0.22;

    pub const REVEAL_CHUNK: usize = 1;
    pub const REVEAL_ABOVE_VIEWPORTS: f32 = 3.0;
}
