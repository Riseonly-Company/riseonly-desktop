pub enum HoldContextMenuMetrics {}

impl HoldContextMenuMetrics {
    pub const BACKDROP_FADE_IN_DURATION: f64 = 0.52;
    pub const BACKDROP_FADE_OUT_DURATION: f64 = 0.40;
    pub const CLOSED_TOUCH_BLOCK_TAIL: f64 = 0.30;
    pub const CLOSE_TOUCH_BLOCK_LIMIT: f64 = 0.25;

    pub const LIFTED_SCALE: f32 = 1.0;
    pub const LIFTED_SHADOW_RADIUS: f32 = 24.0;
    pub const LIFTED_SHADOW_OPACITY: f64 = 0.28;
    pub const LIFTED_SHADOW_OFFSET_Y: f32 = 8.0;

    pub const MENU_TO_SOURCE_GAP: f32 = 8.0;
    pub const EMOJI_TO_SOURCE_GAP: f32 = 18.0;
    pub const EMOJI_OVERLAP_INSET: f32 = -10.0;
    pub const EDGE_INSET: f32 = 10.0;
    pub const SCROLL_VERTICAL_INSET: f32 = 32.0;

    pub const MENU_MIN_WIDTH: f32 = 240.0;
    pub const MENU_MAX_WIDTH: f32 = 270.0;
    pub const MENU_SUBMENU_MAX_WIDTH: f32 = 300.0;
    pub const MENU_DESKTOP_WIDTH_SLACK: f32 = 40.0;
    pub const MENU_TITLE_MAX_WIDTH: f32 = 340.0;
    pub const MENU_CORNER_RADIUS: f32 = 24.0;
    pub const MENU_ROW_HEIGHT: f32 = 48.0;
    pub const MENU_ROW_HORIZONTAL_PADDING: f32 = 16.0;
    pub const MENU_ROW_ICON_SIZE: f32 = 18.0;
    pub const MENU_ROW_ICON_SLOT_WIDTH: f32 = 24.0;
    pub const MENU_ROW_FONT_SIZE: f32 = 17.0;
    pub const MENU_SEPARATOR_OPACITY: f64 = 0.16;
    pub const MENU_SECTION_SEPARATOR_INSET: f32 = 14.0;

    pub const MENU_SECTION_SEPARATOR_GAP: f32 = 6.0;
    pub const MENU_SECTION_SEPARATOR_OPACITY: f64 = 0.24;
    pub const MENU_ROW_HIGHLIGHT_OPACITY: f64 = 0.18;
    pub const PANEL_STROKE_OPACITY: f64 = 0.07;

    pub const EMOJI_SIZE: f32 = 26.0;
    pub const EMOJI_SLOT_WIDTH: f32 = 36.0;
    pub const EMOJI_SLOT_HEIGHT: f32 = 36.0;
    pub const EMOJI_BAR_HORIZONTAL_PADDING: f32 = 4.0;
    pub const EMOJI_BAR_VERTICAL_PADDING: f32 = 3.0;
    pub const EMOJI_HOVER_SCALE: f32 = 1.40;
    pub const EMOJI_PICK_SCALE: f32 = 1.75;
    pub const EMOJI_EXPAND_BUTTON_SIZE: f32 = 30.0;

    pub const PICKER_CORNER_RADIUS: f32 = 20.0;
    pub const PICKER_MAX_WIDTH: f32 = 480.0;
    pub const PICKER_MAX_HEIGHT: f32 = 440.0;
    pub const PICKER_MIN_HEIGHT: f32 = 240.0;
    pub const PICKER_TO_SOURCE_GAP: f32 = 18.0;

    pub const PRESS_SQUISH_AMOUNT: f32 = 0.30;
    pub const PRESS_SHRINK_DISTANCE: f32 = 15.0;
    pub const REDUCED_MOTION_PRESS_SCALE: f32 = 0.98;
    pub const PRESS_SQUISH_RESPONSE: f64 = 0.26;
    pub const PRESS_SQUISH_DAMPING: f64 = 0.74;
    pub const PRESS_RELEASE_RESPONSE: f64 = 0.34;
    pub const PRESS_RELEASE_DAMPING: f64 = 0.78;
    pub const PRESS_RAMP_DURATION: f64 = 0.20;

    pub const ANTICIPATION_DURATION: f64 = 0.12;
    pub const CONFIRM_DURATION: f64 = 0.30;
    pub const CANCEL_MOVEMENT: f32 = 12.0;

    pub const OPEN_RESPONSE: f64 = 0.30;
    pub const OPEN_DAMPING: f64 = 0.86;
    pub const CLOSE_RESPONSE: f64 = 0.28;
    pub const CLOSE_DAMPING: f64 = 0.84;
    pub const HIGHLIGHT_RESPONSE: f64 = 0.16;
    pub const HIGHLIGHT_DAMPING: f64 = 0.86;
    pub const INITIAL_PANEL_SCALE: f32 = 0.94;

    pub const HIGHLIGHT_TRAVEL_RESPONSE: f64 = 0.22;
    pub const HIGHLIGHT_TRAVEL_DAMPING: f64 = 0.88;

    pub const HIGHLIGHT_FADE_DURATION: f64 = 0.12;

    pub const ROW_SNAP_TOLERANCE: f32 = 8.0;

    pub const EMOJI_SNAP_TOLERANCE: f32 = 14.0;

    pub const SELECTION_AIM_TRAVEL: f32 = 10.0;

    pub const RELEASE_ARM_DELAY: f64 = 0.14;

    pub const SOURCE_RESTORE_DELAY_FACTOR: f64 = 1.45;
}

pub enum HoldContextMenuSourceLiftPolicy {}

impl HoldContextMenuSourceLiftPolicy {
    /// squish scale for the press activation
    /// rust has no default args so callers pass the press squish amount and reduce_motion
    pub fn activation_scale(source_width: f32, maximum_squish: f32, reduce_motion: bool) -> f32 {
        if !(source_width.is_finite() && source_width > 1.0) {
            return 1.0;
        }
        if reduce_motion {
            return HoldContextMenuMetrics::REDUCED_MOTION_PRESS_SCALE;
        }

        let minimum_scale = f32::max(0.7, 1.0 - f32::min(0.30, f32::max(0.0, maximum_squish)));
        f32::min(
            1.0,
            f32::max(
                minimum_scale,
                (source_width - HoldContextMenuMetrics::PRESS_SHRINK_DISTANCE) / source_width,
            ),
        )
    }
}
