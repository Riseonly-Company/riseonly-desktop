pub const MAIN_TAB_BAR_HEIGHT: f32 = 62.0;

pub struct MainTabChrome;

impl MainTabChrome {
    pub const FLOATING_BAR_HORIZONTAL_INSET: f32 = 15.0;
    pub const FLOATING_BAR_MIN_BOTTOM_GAP: f32 = Self::FLOATING_BAR_HORIZONTAL_INSET;
    pub const MINI_PLAYER_GAP_ABOVE_TAB_BAR: f32 = 10.0;
    pub const MINI_PLAYER_BLOCK_HEIGHT: f32 = 54.0;

    pub fn extra_underlap_when_mini_player_visible(visible: bool) -> f32 {
        if !visible {
            return 0.0;
        }
        Self::MINI_PLAYER_BLOCK_HEIGHT + Self::MINI_PLAYER_GAP_ABOVE_TAB_BAR
    }

    /// gap from the window bottom edge to the bar
    pub fn bar_bottom_gap(safe_bottom: f32) -> f32 {
        Self::FLOATING_BAR_MIN_BOTTOM_GAP.max(safe_bottom - 5.0)
    }

    pub fn content_bottom_underlap(safe_bottom: f32, mini_player_visible: bool) -> f32 {
        Self::bar_bottom_gap(safe_bottom)
            + MAIN_TAB_BAR_HEIGHT
            + Self::extra_underlap_when_mini_player_visible(mini_player_visible)
    }

    pub const BOTTOM_UNDERLAP: f32 = 78.0;
    pub const NATIVE_TAB_BAR_DEVICE_FUDGE: f32 = 12.0;
    pub const NATIVE_TAB_BAR_BLOCK_HEIGHT: f32 = 49.0;

    pub fn native_tab_bar_reserved_height(safe_bottom: f32) -> f32 {
        safe_bottom + Self::NATIVE_TAB_BAR_BLOCK_HEIGHT + Self::NATIVE_TAB_BAR_DEVICE_FUDGE
    }

    pub fn native_content_bottom_underlap(safe_bottom: f32, mini_player_visible: bool) -> f32 {
        Self::native_tab_bar_reserved_height(safe_bottom)
            + Self::extra_underlap_when_mini_player_visible(mini_player_visible)
    }
}
