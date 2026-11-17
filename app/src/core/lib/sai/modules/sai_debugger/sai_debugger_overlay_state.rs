#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use gpui::{App, Context, Entity, Pixels, Point};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::{NotificationFeedbackGenerator, NotificationKind};

/// ten quick taps on the profile tab reveal the debugger orb
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SaiDebuggerUnlockPolicy {
    tap_count: i64,
    last_tap_timestamp: Option<f64>,
}

impl SaiDebuggerUnlockPolicy {
    pub const REQUIRED_TAP_COUNT: i64 = 10;
    pub const MAXIMUM_TAP_INTERVAL: f64 = 1.25;

    pub fn tap_count(&self) -> i64 {
        self.tap_count
    }

    pub fn last_tap_timestamp(&self) -> Option<f64> {
        self.last_tap_timestamp
    }

    /// registers a tap, timestamp is monotonic seconds
    pub fn register_tap(&mut self, at_timestamp: f64) -> bool {
        let timestamp = at_timestamp;
        if !timestamp.is_finite() {
            self.reset();
            return false;
        }

        match self.last_tap_timestamp {
            Some(last_tap_timestamp)
                if timestamp >= last_tap_timestamp
                    && timestamp - last_tap_timestamp <= Self::MAXIMUM_TAP_INTERVAL =>
            {
                self.tap_count += 1;
            }
            _ => self.tap_count = 1,
        }
        self.last_tap_timestamp = Some(timestamp);

        if self.tap_count < Self::REQUIRED_TAP_COUNT {
            return false;
        }
        self.reset();
        true
    }

    pub fn reset(&mut self) {
        self.tap_count = 0;
        self.last_tap_timestamp = None;
    }
}

/// when a fling of the floating debug surface throws it off screen
pub enum SaiFloatingDebugMotionPolicy {}

impl SaiFloatingDebugMotionPolicy {
    pub const HORIZONTAL_FLING_DISTANCE: f32 = 115.0;
    pub const HORIZONTAL_DOMINANCE_RATIO: f32 = 1.2;
    pub const PREDICTED_OVERSHOOT: f32 = 18.0;

    pub fn should_dismiss(
        current_center: Point<Pixels>,
        predicted_center: Point<Pixels>,
        container_width: Pixels,
        surface_width: Pixels,
    ) -> bool {
        let current_x = f32::from(current_center.x);
        let current_y = f32::from(current_center.y);
        let predicted_x = f32::from(predicted_center.x);
        let predicted_y = f32::from(predicted_center.y);
        let container_width = f32::from(container_width);
        let surface_width = f32::from(surface_width);

        if !(current_x.is_finite()
            && current_y.is_finite()
            && predicted_x.is_finite()
            && predicted_y.is_finite()
            && container_width.is_finite()
            && surface_width.is_finite()
            && container_width > 0.0
            && surface_width > 0.0)
        {
            return false;
        }

        let projected_x = predicted_x - current_x;
        let projected_y = predicted_y - current_y;
        let is_strong_horizontal_fling = projected_x.abs() >= Self::HORIZONTAL_FLING_DISTANCE
            && projected_x.abs() >= projected_y.abs() * Self::HORIZONTAL_DOMINANCE_RATIO;
        let is_projected_beyond_left =
            predicted_x <= -(surface_width / 2.0 + Self::PREDICTED_OVERSHOOT);
        let is_projected_beyond_right =
            predicted_x >= container_width + surface_width / 2.0 + Self::PREDICTED_OVERSHOOT;

        is_strong_horizontal_fling && (is_projected_beyond_left || is_projected_beyond_right)
    }
}

/// whether the orb and the performance hud are on screen
pub struct SaiDebuggerOverlayState {
    is_orb_visible: bool,
    is_performance_hud_visible: bool,
    unlock_policy: SaiDebuggerUnlockPolicy,
}

impl SaiDebuggerOverlayState {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            is_orb_visible: true,
            is_performance_hud_visible: false,
            unlock_policy: SaiDebuggerUnlockPolicy::default(),
        }
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    pub fn is_orb_visible(&self) -> bool {
        self.is_orb_visible
    }

    pub fn is_performance_hud_visible(&self) -> bool {
        self.is_performance_hud_visible
    }

    /// registers a profile tab tap with the current media time
    pub fn register_profile_tab_tap(&mut self, cx: &mut Context<Self>) {
        self.register_profile_tab_tap_at(RiseClock::monotonic().as_secs_f64(), cx);
    }

    pub fn register_profile_tab_tap_at(&mut self, timestamp: f64, cx: &mut Context<Self>) {
        if self.is_orb_visible {
            self.unlock_policy.reset();
            return;
        }
        if !self.unlock_policy.register_tap(timestamp) {
            return;
        }

        self.is_orb_visible = true;
        cx.notify();
        let feedback = NotificationFeedbackGenerator::new();
        let _ = feedback.prepare();
        let _ = feedback.notification_occurred(NotificationKind::Success);
    }

    pub fn show_orb(&mut self, cx: &mut Context<Self>) {
        self.is_orb_visible = true;
        self.unlock_policy.reset();
        cx.notify();
    }

    pub fn dismiss_orb(&mut self, cx: &mut Context<Self>) {
        self.is_orb_visible = false;
        self.unlock_policy.reset();
        cx.notify();
    }

    pub fn show_performance_hud(&mut self, cx: &mut Context<Self>) {
        self.is_performance_hud_visible = true;
        cx.notify();
    }

    pub fn dismiss_performance_hud(&mut self, cx: &mut Context<Self>) {
        self.is_performance_hud_visible = false;
        cx.notify();
    }
}
