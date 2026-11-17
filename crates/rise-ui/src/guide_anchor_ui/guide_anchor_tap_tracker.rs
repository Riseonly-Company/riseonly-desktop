use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, Bounds, DispatchPhase, MouseDownEvent, MouseUpEvent, Pixels, Point, Window};

use crate::guide_overlay_ui::guide_overlay_window::GuideOverlayWindow;

type OnTap = Rc<dyn Fn(&mut App)>;

struct Expectation {
    rect: Bounds<Pixels>,
    on_tap: OnTap,
}

#[derive(Default)]
struct TapTrackerState {
    expectation: Option<Expectation>,
    began: Option<Point<Pixels>>,
}

thread_local! {
    static TAP_TRACKER: RefCell<TapTrackerState> = RefCell::new(TapTrackerState::default());
}

pub enum GuideAnchorTapTracker {}

impl GuideAnchorTapTracker {
    const TAP_SLOP: f32 = 12.0;

    pub fn install(window: &mut Window) {
        window.on_mouse_event(|event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture {
                let overlay_claims = GuideOverlayWindow::hit_test(event.position, window.window_handle(), cx);
                Self::touches_began(event.position, overlay_claims);
            }
        });
        window.on_mouse_event(|event: &MouseUpEvent, phase, _window, cx| {
            if phase == DispatchPhase::Capture {
                Self::touches_ended(event.position, cx);
            }
        });
    }

    /// starts waiting for a click inside rect
    /// replaces any previous expectation, only one step waits at a time
    pub fn expect(rect: Bounds<Pixels>, on_tap: impl Fn(&mut App) + 'static) {
        TAP_TRACKER.with(|state| {
            state.borrow_mut().expectation = Some(Expectation { rect, on_tap: Rc::new(on_tap) });
        });
    }

    pub fn stop_expecting() {
        TAP_TRACKER.with(|state| state.borrow_mut().expectation = None);
    }

    fn touches_began(position: Point<Pixels>, overlay_claims: bool) {
        TAP_TRACKER.with(|state| {
            let mut state = state.borrow_mut();
            state.began = (state.expectation.is_some() && !overlay_claims).then_some(position);
        });
    }

    fn touches_ended(ended: Point<Pixels>, cx: &mut App) {
        let on_tap = TAP_TRACKER.with(|state| {
            let mut state = state.borrow_mut();
            let began = state.began.take()?;
            let expectation = state.expectation.as_ref()?;
            let dx = f32::from(ended.x) - f32::from(began.x);
            let dy = f32::from(ended.y) - f32::from(began.y);
            if dx.hypot(dy) > Self::TAP_SLOP {
                return None;
            }
            if !expectation.rect.contains(&ended) || !expectation.rect.contains(&began) {
                return None;
            }
            state.expectation.take().map(|expectation| expectation.on_tap)
        });
        if let Some(on_tap) = on_tap {
            on_tap(cx);
        }
    }
}
