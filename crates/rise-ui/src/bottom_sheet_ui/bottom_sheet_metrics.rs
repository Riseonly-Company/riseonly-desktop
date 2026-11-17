pub enum BottomSheetMetrics {}

impl BottomSheetMetrics {
    pub const HANDLE_WIDTH: f32 = 40.0;
    pub const HANDLE_HEIGHT: f32 = 4.0;
    pub const HANDLE_TOP_PADDING: f32 = 8.0;
    pub const HANDLE_BOTTOM_PADDING: f32 = 6.0;
    pub const ATTACHED_FOOTER_TOP_PADDING: f32 = 10.0;

    pub const fn native_sheet_grabber_clearance() -> f32 {
        Self::HANDLE_TOP_PADDING + Self::HANDLE_HEIGHT + Self::HANDLE_BOTTOM_PADDING + 4.0
    }

    pub const TOP_CORNER_RADIUS: f32 = 26.0;
    pub const EXPERIMENTAL_TOP_CORNER_RADIUS: f32 = 38.0;
    pub const SHADOW_RADIUS: f32 = 22.0;
    pub const SHADOW_OPACITY: f64 = 0.22;
    pub const SHADOW_OFFSET_Y: f32 = -6.0;

    pub const BACKDROP_OPACITY: f64 = 0.54;

    pub const SCROLL_TOP_SLACK: f32 = 1.5;
    pub const SHEET_PULL_COMMIT_THRESHOLD: f32 = 6.0;
    pub const SHEET_PULL_REVERT_THRESHOLD: f32 = 2.0;
    pub const SHEET_EXPAND_COMMIT_THRESHOLD: f32 = 8.0;
    pub const SHEET_EXPAND_REVERT_THRESHOLD: f32 = -2.0;

    pub const PRESENT_DURATION: f64 = 0.32;
    pub const DISMISS_DURATION: f64 = 0.28;
    pub const SNAP_DURATION: f64 = 0.28;
    pub const BACKDROP_FADE_DURATION: f64 = 0.24;
    pub const SNAP_RESPONSE: f64 = 0.34;
    pub const SNAP_DAMPING: f64 = 0.88;
    pub const SHEET_APPEAR_RESPONSE: f64 = 0.38;
    pub const SHEET_APPEAR_DAMPING: f64 = 0.86;
    pub const APPEAR_RESPONSE: f64 = Self::SHEET_APPEAR_RESPONSE;
    pub const APPEAR_DAMPING: f64 = Self::SHEET_APPEAR_DAMPING;

    pub const PAGE_TRANSITION_DURATION: f64 = Self::SNAP_DURATION;
    pub const PAGE_MINIMUM_TRANSITION_DURATION: f64 = 0.2;
    pub const PAGE_COMMIT_DISTANCE_FRACTION: f32 = 0.24;
    pub const PAGE_COMMIT_PROJECTION_FRACTION: f32 = 0.46;
    pub const PAGE_FLICK_PROJECTION_SECONDS: f64 = 0.12;
    pub const PAGE_DRAG_EPSILON: f32 = 0.5;

    pub const CAROUSEL_PAGE_SPACING: f32 = 20.0;
    pub const CAROUSEL_SETTLE_DURATION: f64 = 0.42;
    pub const CAROUSEL_SETTLE_DAMPING: f32 = 0.9;
    pub const CAROUSEL_EDGE_RESISTANCE: f32 = 0.3;
    pub const CAROUSEL_EDGE_MAXIMUM_OVERSHOOT: f32 = 0.18;

    pub const FLICK_VELOCITY: f32 = 2_000.0;
    pub const MIN_DRAG_FOR_VELOCITY_DISMISS: f32 = 50.0;
    pub const MIN_DRAG_FOR_VELOCITY_EXPAND: f32 = 40.0;
    pub const COMMIT_DISTANCE_FRACTION: f32 = 0.28;

    pub const RUBBER_BAND: f32 = 0.42;
    pub const DISMISS_FRACTION: f32 = 0.62;

    pub const PRESENTATION_RETRY_WINDOW: f64 = 1.2;
    pub const PRESENTATION_RETRY_INTERVAL: f64 = 0.05;
}
