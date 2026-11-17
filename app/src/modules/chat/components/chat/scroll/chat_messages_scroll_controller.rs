use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, Bounds, ElementId, IntoElement, RenderOnce, ScrollHandle, Styled, Window, div, point, px};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::motion::CubicBezier;

use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;

struct ChatComposerInsetAnimationState {
    last_duration: f64,
    last_options: CubicBezier,
    last_announced_at: Duration,
    last_send_at: Duration,
}

static INSET_ANIMATION: LazyLock<Mutex<ChatComposerInsetAnimationState>> = LazyLock::new(|| {
    Mutex::new(ChatComposerInsetAnimationState {
        last_duration: 0.25,
        last_options: CubicBezier::EASE_OUT,
        last_announced_at: Duration::ZERO,
        last_send_at: Duration::ZERO,
    })
});

const ANNOUNCEMENT_WINDOW: Duration = Duration::from_millis(350);
const SEND_HOLD_WINDOW: Duration = Duration::from_millis(800);

pub enum ChatComposerInsetAnimation {}

impl ChatComposerInsetAnimation {
    pub const SENT_ROW_HOLD_DURATION: Duration = Duration::from_millis(800);

    pub fn last_duration() -> f64 {
        INSET_ANIMATION.lock().last_duration
    }

    /// stores the last duration
    pub fn set_last_duration(duration: f64) {
        let mut state = INSET_ANIMATION.lock();
        state.last_duration = duration;
        state.last_options = CubicBezier::EASE_OUT;
        state.last_announced_at = RiseClock::monotonic();
    }

    /// curve of the composer change being applied now
    pub fn last_options() -> CubicBezier {
        INSET_ANIMATION.lock().last_options
    }

    pub fn set_last_options(options: CubicBezier) {
        INSET_ANIMATION.lock().last_options = options;
    }

    pub fn last_announced_at() -> Duration {
        INSET_ANIMATION.lock().last_announced_at
    }

    pub fn announces_animated_change() -> bool {
        Self::announces_animated_change_at(RiseClock::monotonic())
    }

    pub fn announces_animated_change_at(time: Duration) -> bool {
        time.saturating_sub(Self::last_announced_at()) <= ANNOUNCEMENT_WINDOW
    }

    pub fn last_send_at() -> Duration {
        INSET_ANIMATION.lock().last_send_at
    }

    pub fn announce_send() {
        INSET_ANIMATION.lock().last_send_at = RiseClock::monotonic();
    }

    pub fn announces_send() -> bool {
        Self::announces_send_at(RiseClock::monotonic())
    }

    pub fn announces_send_at(time: Duration) -> bool {
        time.saturating_sub(Self::last_send_at()) <= SEND_HOLD_WINDOW
    }
}

pub enum ChatComposerBottomPaddingPolicy {}

impl ChatComposerBottomPaddingPolicy {
    pub fn resolve(
        safe_bottom_inset: f32,
        _keyboard_bottom_inset: f32,
        picker_visible: bool,
        picker_panel_height: f32,
        is_picker_to_keyboard_transitioning: bool,
        top_padding: f32,
    ) -> f32 {
        let resting_padding = if safe_bottom_inset > 1.0 { safe_bottom_inset } else { top_padding };

        if is_picker_to_keyboard_transitioning {
            return resting_padding.max(picker_panel_height.max(0.0) + top_padding);
        }
        if picker_visible {
            return resting_padding.max(picker_panel_height.max(0.0) + top_padding);
        }

        resting_padding
    }
}

pub enum ChatPickerContentSafeAreaPolicy {}

impl ChatPickerContentSafeAreaPolicy {
    pub fn resolved_safe_bottom(shell_safe_bottom_inset: f32, window_safe_bottom_inset: f32) -> f32 {
        0.0_f32.max(shell_safe_bottom_inset).max(window_safe_bottom_inset)
    }
}

pub enum ChatPickerPanelHeightPolicy {}

impl ChatPickerPanelHeightPolicy {
    pub const DEFAULT_PANEL_HEIGHT: f32 = 336.0;

    pub fn resolve(last_keyboard_height: f32, handoff_start_height: Option<f32>) -> f32 {
        if let Some(handoff_start_height) = handoff_start_height {
            return handoff_start_height;
        }
        if last_keyboard_height > 100.0 {
            last_keyboard_height
        } else {
            Self::DEFAULT_PANEL_HEIGHT
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChatKeyboardLayoutState {
    pub height: f32,
    pub target_height: f32,
    pub is_settled: bool,
}

pub enum ChatKeyboardGeometryPolicy {}

impl ChatKeyboardGeometryPolicy {
    pub const SAMPLE_TOLERANCE: f32 = 8.0;

    pub fn maximum_plausible_height(viewport_height: f32, safe_top_inset: f32) -> f32 {
        if !viewport_height.is_finite() || viewport_height <= 0.0 {
            return 0.0;
        }
        let minimum_visible_app_height = 80.0_f32.max(safe_top_inset + 44.0);
        0.0_f32.max((viewport_height * 0.78).min(viewport_height - minimum_visible_app_height))
    }

    pub fn normalized_target_height(height: f32, viewport_height: f32, safe_top_inset: f32) -> Option<f32> {
        if !height.is_finite() || height < 0.0 {
            return None;
        }
        if height <= 1.0 {
            return Some(0.0);
        }
        if height > Self::maximum_plausible_height(viewport_height, safe_top_inset) + Self::SAMPLE_TOLERANCE {
            return None;
        }
        Some(height)
    }

    pub fn normalized_tracker_overlap(
        tracker_bottom: f32,
        viewport_height: f32,
        safe_top_inset: f32,
        safe_bottom_inset: f32,
    ) -> Option<f32> {
        if !tracker_bottom.is_finite()
            || !viewport_height.is_finite()
            || viewport_height <= 0.0
            || tracker_bottom <= 1.0
            || tracker_bottom > viewport_height + 1.0
        {
            return None;
        }

        let overlap = 0.0_f32.max(viewport_height - tracker_bottom);
        if overlap <= safe_bottom_inset + 0.5 {
            return Some(0.0);
        }
        Self::normalized_target_height(overlap, viewport_height, safe_top_inset)
    }

    pub fn accepted_guide_height(
        guide_height: Option<f32>,
        target_height: f32,
        last_confirmed_height: f32,
        fallback_height: f32,
    ) -> Option<f32> {
        let guide_height = guide_height.filter(|height| height.is_finite() && *height >= 0.0)?;
        if guide_height <= 1.0 {
            return Some(0.0);
        }

        let confirmed_ceiling = target_height.max(last_confirmed_height).max(fallback_height);
        if confirmed_ceiling <= 1.0 || guide_height > confirmed_ceiling + Self::SAMPLE_TOLERANCE {
            return None;
        }
        Some(guide_height)
    }

    pub fn should_stop_sampling(
        settled_frame_count: i64,
        height: f32,
        fallback_height: f32,
        target_height: f32,
        is_keyboard_session_active: bool,
    ) -> bool {
        if settled_frame_count < 3 {
            return false;
        }

        if (height - target_height).abs() <= 0.1 {
            return true;
        }

        height <= 0.1 && fallback_height <= 0.1 && target_height <= 0.1 && !is_keyboard_session_active
    }
}

pub type ChatKeyboardLayoutChange = Rc<dyn Fn(ChatKeyboardLayoutState, &mut Window, &mut App)>;

/// a desktop window has no on screen keyboard
/// so the reader reports the settled hidden state once drawn
#[derive(IntoElement)]
pub struct ChatKeyboardFrameReader {
    id: ElementId,
    reports_presentation_frames: bool,
    on_change: ChatKeyboardLayoutChange,
}

impl ChatKeyboardFrameReader {
    pub const HIDDEN_STATE: ChatKeyboardLayoutState = ChatKeyboardLayoutState {
        height: 0.0,
        target_height: 0.0,
        is_settled: true,
    };

    pub fn new(id: impl Into<ElementId>, on_change: impl Fn(ChatKeyboardLayoutState, &mut Window, &mut App) + 'static) -> Self {
        Self {
            id: id.into(),
            reports_presentation_frames: false,
            on_change: Rc::new(on_change),
        }
    }

    pub fn reports_presentation_frames(mut self, reports_presentation_frames: bool) -> Self {
        self.reports_presentation_frames = reports_presentation_frames;
        self
    }

    pub fn reports_presentation_frames_enabled(&self) -> bool {
        self.reports_presentation_frames
    }

    pub fn docked_keyboard_overlap(keyboard_frame_in_window: Bounds<f32>, window_bounds: Bounds<f32>) -> f32 {
        let keyboard = keyboard_frame_in_window;
        if window_bounds.size.height <= 0.0
            || !keyboard.origin.x.is_finite()
            || !keyboard.origin.y.is_finite()
            || !keyboard.size.width.is_finite()
            || !keyboard.size.height.is_finite()
            || keyboard.size.width <= 0.0
            || keyboard.size.height <= 0.0
            || keyboard.top() <= window_bounds.top() + 1.0
            || keyboard.bottom() < window_bounds.bottom() - 1.0
            || keyboard.top() >= window_bounds.bottom()
            || keyboard.right() <= window_bounds.left()
            || keyboard.left() >= window_bounds.right()
        {
            return 0.0;
        }
        window_bounds.size.height.min(0.0_f32.max(window_bounds.bottom() - keyboard.top()))
    }
}

impl RenderOnce for ChatKeyboardFrameReader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_change = self.on_change;
        div()
            .absolute()
            .size_0()
            .on_appear(self.id, move |window, cx| on_change(Self::HIDDEN_STATE, window, cx))
    }
}

#[derive(Default)]
pub struct ChatMessagesScrollController {
    pub scroll_view: Option<ScrollHandle>,
    pub chat_id: String,
}

impl ChatMessagesScrollController {
    pub fn is_near_bottom(&self) -> bool {
        let Some(scroll_view) = &self.scroll_view else {
            return true;
        };
        self.distance_from_bottom(scroll_view) <= ChatScrollPositionStore::NEAR_BOTTOM_THRESHOLD
    }

    pub fn bind(&mut self, scroll_view: ScrollHandle) {
        self.scroll_view = Some(scroll_view);
    }

    pub fn max_content_offset_y(&self, scroll_view: &ScrollHandle) -> f32 {
        0.0_f32.max(f32::from(scroll_view.max_offset().y))
    }

    pub fn distance_from_bottom(&self, scroll_view: &ScrollHandle) -> f32 {
        0.0_f32.max(self.max_content_offset_y(scroll_view) - Self::content_offset_y(scroll_view))
    }

    pub fn scroll_to_bottom(&self, _animated: bool) {
        let Some(scroll_view) = &self.scroll_view else {
            return;
        };
        let target_y = self.max_content_offset_y(scroll_view);
        if (Self::content_offset_y(scroll_view) - target_y).abs() <= 0.5 {
            return;
        }
        let offset = scroll_view.offset();
        scroll_view.set_offset(point(offset.x, px(-target_y)));
    }

    pub fn persist_scroll_position(&self, cx: &mut App) {
        let Some(scroll_view) = &self.scroll_view else {
            return;
        };
        if self.chat_id.is_empty() {
            return;
        }
        ChatScrollPositionStore::save_offset(Self::content_offset_y(scroll_view), &self.chat_id, self.is_near_bottom(), cx);
    }

    fn content_offset_y(scroll_view: &ScrollHandle) -> f32 {
        -f32::from(scroll_view.offset().y)
    }
}
