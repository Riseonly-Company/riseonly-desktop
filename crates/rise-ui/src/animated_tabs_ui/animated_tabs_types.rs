use std::rc::Rc;

use gpui::{px, AnyElement, App, Pixels, SharedString, Window};
use rise_canon::RiseGuideAnchor;
use rise_platform::haptics::SelectionFeedbackGenerator;

use crate::swiftui::motion::{timing_curve, CubicBezier, MotionSpec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimatedTabsHeaderPlacement {
    Top,
    Bottom,
}

pub type TabContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

#[derive(Clone)]
pub struct TabConfig {
    pub id: i64,
    pub text: SharedString,
    pub content: TabContent,
}

impl TabConfig {
    pub fn new(
        id: i64,
        text: impl Into<SharedString>,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id,
            text: text.into(),
            content: Rc::new(content),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabHeaderConfig {
    pub id: i64,
    pub text: SharedString,
    pub guide_anchor: Option<RiseGuideAnchor>,
    pub custom_emojis: Vec<TabHeaderCustomEmoji>,
}

/// custom emoji as the tab strip carries it
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct TabHeaderCustomEmoji {
    pub offset: i64,
    pub length: i64,
    pub custom_emoji_id: String,
    pub file_url: String,
    pub asset_type: String,
    pub width: i64,
    pub height: i64,
    pub pack_id: String,
}

impl TabHeaderConfig {
    pub fn new(id: i64, text: impl Into<SharedString>) -> Self {
        Self {
            id,
            text: text.into(),
            guide_anchor: None,
            custom_emojis: Vec::new(),
        }
    }

    pub fn custom_emojis(mut self, custom_emojis: Vec<TabHeaderCustomEmoji>) -> Self {
        self.custom_emojis = custom_emojis;
        self
    }

    pub fn guide_anchor(mut self, anchor: impl Into<Option<RiseGuideAnchor>>) -> Self {
        self.guide_anchor = anchor.into();
        self
    }
}

impl From<&TabConfig> for TabHeaderConfig {
    fn from(tab: &TabConfig) -> Self {
        Self::new(tab.id, tab.text.clone())
    }
}

pub struct AnimatedTabsMetrics;

impl AnimatedTabsMetrics {
    pub const BAR_HEIGHT: Pixels = px(44.0);
    pub const BAR_INNER_PADDING: Pixels = px(4.0);
    pub const BAR_OUTER_HORIZONTAL_PADDING: Pixels = px(0.0);
    pub const TAB_HORIZONTAL_PADDING: Pixels = px(14.0);
    pub const COMPACT_MIN_TAB_PADDING: Pixels = px(6.0);
    pub const COMPACT_ROUNDING_SLACK: Pixels = px(2.0);
    pub const TAB_FONT_SIZE: Pixels = px(15.0);
    pub const STRIP_EDGE_FADE_WIDTH: Pixels = px(40.0);
    pub const TAB_MIN_TAP_WIDTH: Pixels = px(56.0);

    pub const INDICATOR_CORNER_RADIUS: Pixels = px(18.0);

    pub const RELEASE_DURATION: f64 = 0.34;
    pub const RELEASE_CONTROL_POINTS: (f64, f64, f64, f64) = (0.2, 0.82, 0.22, 1.0);

    pub const PAGE_COMMIT_THRESHOLD: f32 = 0.30;
    pub const PAGE_VELOCITY_COMMIT_THRESHOLD: f32 = 380.0;

    pub const EDGE_RESISTANCE: f32 = 0.45;
    pub const EDGE_MAXIMUM_OVERSHOOT: f32 = 0.6;

    pub const BACK_BEYOND_FIRST_THRESHOLD: f32 = 0.32;
    pub const BACK_BEYOND_FIRST_VELOCITY: f32 = 500.0;

    pub const TAB_TAP_RESTORE_DELAY: f64 = 0.15;

    pub fn release_animation() -> MotionSpec {
        let (x1, y1, x2, y2) = Self::RELEASE_CONTROL_POINTS;
        timing_curve(x1, y1, x2, y2, Self::RELEASE_DURATION)
    }

    pub fn release_timing_function() -> CubicBezier {
        let (x1, y1, x2, y2) = Self::RELEASE_CONTROL_POINTS;
        CubicBezier(x1, y1, x2, y2)
    }
}

/// one prepared haptics generator for the whole component
/// one made at the call site pays the lazy engine setup at the first tab tap
pub enum AnimatedTabsHaptics {}

impl AnimatedTabsHaptics {
    const SELECTION: SelectionFeedbackGenerator = SelectionFeedbackGenerator::new();

    pub fn prepare() {
        let _ = Self::SELECTION.prepare();
    }

    pub fn selection_changed() {
        let _ = Self::SELECTION.selection_changed();
        let _ = Self::SELECTION.prepare();
    }
}
