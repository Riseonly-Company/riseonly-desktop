use std::ops::RangeInclusive;

use gpui::{App, Bounds, FontWeight, Hsla, Pixels, Size, hsla, px};
use rise_canon::{RiseGuideCanon, RiseGuideSpotlightShape};
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_theme::AppTheme;

use crate::expandable_text_ui::EdgeInsets;
use crate::guide_overlay_ui::guide_overlay_layout::GuideOverlayLayoutMetrics;
use crate::message_bubble_geometry::MessageBubbleGeometry;
use crate::swiftui::motion::{CubicBezier, MotionSpec, ease_in_out};

/// the only place in the guide overlay that holds a number or color, other files read from here
/// durations come from canon, sizes weights and radii are local
pub enum GuideOverlayTheme {}

const fn seconds(milliseconds: i64) -> f64 {
    milliseconds as f64 / 1000.0
}

impl GuideOverlayTheme {
    pub const SPOTLIGHT_MOVE: f64 = seconds(RiseGuideCanon::TIMINGS_SPOTLIGHT_MOVE_MS);
    pub const PATH_DRAW: f64 = seconds(RiseGuideCanon::TIMINGS_PATH_DRAW_MS);
    pub const NARRATOR_TRANSITION: f64 = seconds(RiseGuideCanon::TIMINGS_NARRATOR_TRANSITION_MS);
    pub const SCRIM_FADE: f64 = seconds(RiseGuideCanon::TIMINGS_NARRATOR_TRANSITION_MS);
    pub const POSE_CROSSFADE: f64 = seconds(RiseGuideCanon::TIMINGS_NARRATOR_TRANSITION_MS) / 2.0;

    /// ease in ease out timing curve
    pub fn core_animation_timing() -> CubicBezier {
        CubicBezier::EASE_IN_OUT
    }

    pub fn move_animation(motion: GuideOverlayMotion) -> Option<MotionSpec> {
        motion.animates_transitions().then(|| ease_in_out(Self::SPOTLIGHT_MOVE))
    }

    pub fn fade_animation(motion: GuideOverlayMotion) -> Option<MotionSpec> {
        motion.animates_transitions().then(|| ease_in_out(Self::NARRATOR_TRANSITION))
    }

    /// scrim has to darken both palettes so this is the one color that is not from the palette
    pub fn scrim_color() -> Hsla {
        hsla(0.0, 0.0, 0.0, 0.62)
    }

    pub const CUTOUT_PADDING: f32 = 8.0;

    pub const ROUNDED_RECT_CUTOUT_RADIUS: f32 = 14.0;

    pub const MINIMUM_CORNER_RADIUS: f32 = 0.01;

    pub fn cutout_corner_radius(shape: RiseGuideSpotlightShape, rect: Bounds<Pixels>) -> f32 {
        let half = f32::from(rect.size.width.min(rect.size.height)) * 0.5;
        match shape {
            RiseGuideSpotlightShape::RoundedRect => {
                Self::MINIMUM_CORNER_RADIUS.max(Self::ROUNDED_RECT_CUTOUT_RADIUS.min(half))
            }
            RiseGuideSpotlightShape::Circle | RiseGuideSpotlightShape::Capsule => {
                Self::MINIMUM_CORNER_RADIUS.max(half)
            }
        }
    }

    /// edge of the hole and its echoes when a step arrives
    /// same color as the line between anchors, two accents would look like two systems
    pub fn attention_color(theme: &AppTheme) -> Hsla {
        theme.primary._100
    }
    pub const ATTENTION_LINE_WIDTH: f32 = 2.5;
    pub const ATTENTION_ECHO_SPREAD: f32 = 16.0;
    pub const ATTENTION_ECHO_COUNT: f32 = 2.0;

    pub const ATTENTION_PULSE: f64 = Self::PATH_DRAW;

    /// hint color drawn over anything in the hole, white with a soft dark edge reads on both themes
    pub fn gesture_color() -> Hsla {
        hsla(0.0, 0.0, 1.0, 1.0)
    }
    pub fn gesture_shadow_color() -> Hsla {
        hsla(0.0, 0.0, 0.0, 0.38)
    }
    pub const GESTURE_SHADOW_RADIUS: f32 = 3.0;
    pub const GESTURE_LINE_WIDTH: f32 = 3.5;

    pub const GESTURE_CHEVRON_WIDTH: f32 = 18.0;
    pub const GESTURE_CHEVRON_DEPTH: f32 = 8.0;
    pub const GESTURE_CHEVRON_SPACING: f32 = 11.0;
    pub const GESTURE_CHEVRON_COUNT: usize = 3;
    pub const GESTURE_TRAVEL_SHARE: f32 = 0.42;
    pub const GESTURE_TRAVEL_RANGE: RangeInclusive<f32> = 22.0..=70.0;

    pub const GESTURE_FINGER_SIDE: f32 = 44.0;
    pub const GESTURE_FINGER_FILL_ALPHA: f32 = 0.22;
    pub const GESTURE_HOLD_RING_SIDE: f32 = 62.0;
    pub const GESTURE_TAP_RIPPLE_SIDE: f32 = 58.0;

    pub const GESTURE_TRAVEL: f64 = Self::PATH_DRAW * 1.6;
    pub const GESTURE_STAGGER: f64 = Self::PATH_DRAW / 4.0;
    pub const GESTURE_REST: f64 = Self::PATH_DRAW / 2.0;
    pub const GESTURE_HOLD_FILL: f64 = Self::PATH_DRAW * 1.8;
    pub const GESTURE_TAP_RIPPLE: f64 = Self::PATH_DRAW * 0.8;

    pub fn path_color(theme: &AppTheme) -> Hsla {
        theme.primary._100
    }
    pub const PATH_LINE_WIDTH: f32 = 3.0;
    pub const PATH_CORNER_RADIUS: f32 = 18.0;
    pub const PATH_ANCHOR_GAP: f32 = 10.0;

    pub const NARRATOR_SIDE: f32 = 104.0;
    pub const NARRATOR_ENTER_SCALE: f32 = 0.88;

    pub const GROUP_SPACING: f32 = 4.0;
    pub const SURFACE_MAX_WIDTH: f32 = 320.0;
    pub const CAPTION_MAX_WIDTH: f32 = 260.0;
    pub const CARD_MAX_WIDTH: f32 = 340.0;

    pub const SURFACE_PADDING: EdgeInsets = EdgeInsets::new(14.0, 16.0, 14.0, 16.0);
    pub const CAPTION_PADDING: EdgeInsets = EdgeInsets::new(9.0, 13.0, 9.0, 13.0);
    pub const SURFACE_CONTENT_SPACING: f32 = 10.0;
    pub const SURFACE_ACTION_SPACING: f32 = 8.0;

    pub fn surface_background(theme: &AppTheme) -> Hsla {
        theme.bg._200
    }
    pub fn surface_border(theme: &AppTheme) -> Hsla {
        theme.border._100
    }
    pub fn surface_title_color(theme: &AppTheme) -> Hsla {
        theme.text.primary
    }
    pub fn surface_body_color(theme: &AppTheme) -> Hsla {
        theme.text.primary
    }
    pub fn surface_meta_color(theme: &AppTheme) -> Hsla {
        theme.text.secondary
    }
    pub fn accent_color(theme: &AppTheme) -> Hsla {
        theme.primary._100
    }
    pub const SURFACE_BORDER_WIDTH: f32 = 1.0;

    pub fn bubble_corner_radius(theme: &AppTheme) -> f32 {
        f32::from(theme.radius._100)
    }
    pub fn caption_corner_radius(theme: &AppTheme) -> f32 {
        f32::from(theme.radius._300)
    }
    pub fn card_corner_radius(theme: &AppTheme) -> f32 {
        f32::from(theme.radius._100)
    }
    pub fn video_corner_radius(theme: &AppTheme) -> f32 {
        f32::from(theme.radius._200)
    }

    pub const BUBBLE_TAIL_SIZE: Size<Pixels> = Size {
        width: MessageBubbleGeometry::TAIL_INSET,
        height: MessageBubbleGeometry::TAIL_INSET,
    };

    pub const VIDEO_ASPECT_RATIO: f32 = 16.0 / 9.0;

    pub const TITLE_SIZE: f32 = 18.0;
    pub const TITLE_WEIGHT: FontWeight = FontWeight::SEMIBOLD;
    pub const BODY_SIZE: f32 = 15.0;
    pub const BODY_LINE_HEIGHT: f32 = 21.0;
    pub const CAPTION_SIZE: f32 = 13.0;
    pub const CAPTION_LINE_HEIGHT: f32 = 18.0;
    pub const META_SIZE: f32 = 12.0;
    pub const ACTION_TITLE_SIZE: f32 = 15.0;
    pub const ACTION_TITLE_WEIGHT: FontWeight = FontWeight::SEMIBOLD;

    pub fn action_height(theme: &AppTheme) -> f32 {
        f32::from(theme.button.height_500)
    }
    pub const ACTION_HORIZONTAL_PADDING: f32 = 18.0;

    pub const EXIT_HEIGHT: f32 = 32.0;
    pub const EXIT_HORIZONTAL_PADDING: f32 = 14.0;
    pub const EXIT_TITLE_SIZE: f32 = 14.0;
    pub fn exit_title_color(theme: &AppTheme) -> Hsla {
        theme.text.secondary
    }

    pub const LAYOUT: GuideOverlayLayoutMetrics = GuideOverlayLayoutMetrics {
        screen_margin: px(16.0),
        cutout_clearance: px(12.0),
        group_spacing: px(Self::GROUP_SPACING),
        minimum_vertical_band: px(180.0),
        minimum_horizontal_band: px(300.0),
        minimum_surface_width: px(180.0),
    };
}

/// what is allowed to move right now
/// reduce motion and power saving remove motion and never content, the step still shows line
/// cutout and narrator
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuideOverlayMotion {
    pub reduce_motion: bool,
    pub conserves_energy: bool,
}

impl GuideOverlayMotion {
    pub fn new(reduce_motion: bool, conserves_energy: bool) -> Self {
        Self {
            reduce_motion,
            conserves_energy,
        }
    }

    /// current motion verdict from the system reduce motion setting plus the energy state
    pub fn current(cx: &mut App) -> Self {
        let reduce_motion = crate::swiftui::environment::reduce_motion(cx);
        Self::new(reduce_motion, Self::energy_is_scarce(cx))
    }

    pub fn conserving_energy(reduce_motion: bool, cx: &mut App) -> Self {
        Self::new(reduce_motion, Self::energy_is_scarce(cx))
    }

    pub fn energy_is_scarce(cx: &mut App) -> bool {
        RiseEnergySavingStore::shared(cx).read(cx).system_low_power_mode()
            || RiseEnergySavingStore::snapshot_reduces(RiseEnergySavingFeature::InterfaceEffects)
    }

    pub fn animates_transitions(&self) -> bool {
        !self.reduce_motion
    }

    /// true when the line may be drawn progressively, otherwise it just appears
    pub fn draws_path_progressively(&self) -> bool {
        !self.reduce_motion && !self.conserves_energy
    }

    pub fn plays_narrator(&self) -> bool {
        !self.reduce_motion
    }
}
