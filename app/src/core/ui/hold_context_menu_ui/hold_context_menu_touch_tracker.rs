use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, Bounds, DispatchPhase, IntoElement, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Styled, Window, WindowId, canvas,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::rise_strict_tap_gesture_recognizer::RiseTapPolicy;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;

const LOG_NAME: &str = "HoldMenu";

#[derive(Clone, Copy)]
struct TrackedTouch {
    start: Duration,
    start_location: Point<Pixels>,
}

#[derive(Default)]
struct WindowTrackers {
    tracker: bool,
    close_touch_blocker: bool,
    tracked_touch: Option<TrackedTouch>,
}

thread_local! {
    static WINDOWS: RefCell<HashMap<WindowId, WindowTrackers>> = RefCell::new(HashMap::new());
    static MENU_SCROLL_REGION: Cell<Option<Bounds<Pixels>>> = const { Cell::new(None) };
}

fn with_window<R>(window_id: WindowId, f: impl FnOnce(&mut WindowTrackers) -> R) -> R {
    WINDOWS.with(|windows| f(windows.borrow_mut().entry(window_id).or_default()))
}

pub enum HoldContextMenuTouchTracker {}

impl HoldContextMenuTouchTracker {
    pub fn install(in_window: &Window) {
        let window_id = in_window.window_handle().window_id();
        Self::attach(window_id);
        HoldContextMenuCloseTouchBlocker::attach(window_id);
    }

    pub fn install_in_menu_window(window: AnyWindowHandle) {
        Self::attach(window.window_id());
    }

    fn attach(window_id: WindowId) {
        with_window(window_id, |trackers| trackers.tracker = true);
    }

    /// scrollable part of the open menu in window coordinates, while its content is taller than its bounds
    /// a press that lands there scrolls the menu instead of choosing by drag
    pub fn set_menu_scroll_region(region: Option<Bounds<Pixels>>) {
        MENU_SCROLL_REGION.with(|cell| cell.set(region));
    }

    /// element the menu layer draws, registers this window listeners for the frame
    pub fn observer(window: AnyWindowHandle) -> impl IntoElement {
        let window_id = window.window_id();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (tracker, close_touch_blocker) =
                    with_window(window_id, |trackers| (trackers.tracker, trackers.close_touch_blocker));
                if close_touch_blocker {
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            HoldContextMenuCloseTouchBlocker::touches_began(event, cx);
                        }
                    });
                }
                if !tracker {
                    return;
                }
                window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        Self::touches_began(window_id, event.position, cx);
                    }
                });
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        Self::touches_moved(window_id, event, cx);
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        Self::touches_ended(window_id, event.position, cx);
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }

    fn touches_began(window_id: WindowId, location: Point<Pixels>, cx: &mut App) {
        let menu_is_open = HoldContextMenuPresenter::has_active_request(cx);
        let watched = with_window(window_id, |trackers| {
            if menu_is_open && trackers.tracked_touch.is_some() {
                return false;
            }
            trackers.tracked_touch = Some(TrackedTouch { start: RiseClock::monotonic(), start_location: location });
            true
        });
        if !watched || !menu_is_open {
            return;
        }
        let scrolls_menu = MENU_SCROLL_REGION.with(|cell| cell.get()).is_some_and(|region| region.contains(&location));
        HoldContextMenuPresenter::note_touch_began(location, !scrolls_menu, cx);
    }

    fn touches_moved(window_id: WindowId, event: &MouseMoveEvent, cx: &mut App) {
        if event.pressed_button.is_none() {
            with_window(window_id, |trackers| trackers.tracked_touch = None);
            return;
        }
        let tracked = with_window(window_id, |trackers| trackers.tracked_touch.is_some());
        if !tracked || !HoldContextMenuPresenter::has_active_request(cx) {
            return;
        }
        HoldContextMenuPresenter::update_drag(event.position, cx);
    }

    /// the slot is released whenever the tracked press lifts, menu or no menu
    /// only the report to the presenter is conditional
    fn touches_ended(window_id: WindowId, location: Point<Pixels>, cx: &mut App) {
        let Some(touch) = with_window(window_id, |trackers| trackers.tracked_touch.take()) else { return };
        if !HoldContextMenuPresenter::has_active_request(cx) {
            Self::log_hold_without_menu(touch, location, cx);
            return;
        }
        HoldContextMenuPresenter::release_at(Some(location), cx);
    }

    fn log_hold_without_menu(touch: TrackedTouch, end: Point<Pixels>, cx: &mut App) {
        let held = RiseClock::monotonic().saturating_sub(touch.start).as_secs_f64();
        if held < HoldContextMenuMetrics::CONFIRM_DURATION {
            return;
        }
        let dx = f32::from(end.x - touch.start_location.x);
        let dy = f32::from(end.y - touch.start_location.y);
        if dx.hypot(dy) > RiseTapPolicy::MOVEMENT_SLOP {
            return;
        }
        Sai::log(
            LOG_NAME,
            || format!("window hold without menu cancelled=false held={held:.2} at={end:?}"),
            SaiLogKind::Ui,
            cx,
        );
    }
}

/// source layer half of the close time press block
/// menu layer swallows presses while closing but a fast second click could still reach whats under
/// so such a press is claimed before any element sees it
pub enum HoldContextMenuCloseTouchBlocker {}

impl HoldContextMenuCloseTouchBlocker {
    pub fn attach(window_id: WindowId) {
        with_window(window_id, |trackers| trackers.close_touch_blocker = true);
    }

    fn touches_began(_event: &MouseDownEvent, cx: &mut App) {
        let blocks = HoldContextMenuPresenter::shared(cx).read(cx).blocks_source_window_touches();
        if blocks {
            tracing::warn!(target: "riseonly::X1161", "close touch blocker took a press");
            cx.stop_propagation();
        }
    }
}
