use gpui::{App, Edges};
use rise_ui::swiftui::environment::{EnvKey, env};

/// top inset of the call screens set by the window that hosts them
/// a screen pushed into the main window column keeps
/// zero since the traffic lights sit over the rail
pub struct RiseRTCScreenTopInsetKey;

impl EnvKey for RiseRTCScreenTopInsetKey {
    type Value = f32;

    fn default_value() -> f32 {
        0.0
    }
}

/// safe area insets for the call screens
/// desktop window has no notch or system bars in its content so sides and bottom are zero
/// top is not zero, the title bar is transparent with traffic lights over the canvas
pub enum RiseRTCScreenSafeArea {}

impl RiseRTCScreenSafeArea {
    pub const DOCK_CONTENT_GAP: f32 = 18.0;

    /// read it while rendering, the environment is empty in a click handler
    pub fn top(cx: &App) -> f32 {
        env::<RiseRTCScreenTopInsetKey>(cx)
    }

    pub fn insets() -> Edges<f32> {
        Edges::default()
    }

    /// max of both sides, callers pad one side (leading or trailing)
    /// which swap in right to left locales while left and right dont
    pub fn horizontal() -> f32 {
        let insets = Self::insets();
        insets.left.max(insets.right)
    }

    /// dock bottom inset shared by both call screens
    /// the floor of 18 matters, with a zero bottom inset the dock would touch the edge
    pub fn dock_bottom_padding() -> f32 {
        f32::max(18.0, Self::insets().bottom + 10.0)
    }
}
