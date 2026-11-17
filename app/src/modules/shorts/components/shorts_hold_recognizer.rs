use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, ElementId, Entity, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, RenderOnce, Styled, Task, Window, div,
};

/// true when a hold begins, false when it ends
pub type ShortsHoldRecognizerOnChanged = Rc<dyn Fn(bool, &mut App)>;

/// held button on the riff media,
/// sits behind the media so a long press doesnt eat the quick tap that pauses
#[derive(IntoElement)]
pub struct ShortsHoldRecognizerHost {
    id: ElementId,
    is_enabled: bool,
    minimum_duration: f64,
    on_changed: ShortsHoldRecognizerOnChanged,
}

impl ShortsHoldRecognizerHost {
    pub fn new(
        id: impl Into<ElementId>,
        is_enabled: bool,
        minimum_duration: f64,
        on_changed: impl Fn(bool, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), is_enabled, minimum_duration, on_changed: Rc::new(on_changed) }
    }

    fn apply(&self, view: &Entity<ShortsHoldRecognizerView>, cx: &mut App) {
        let on_changed = self.on_changed.clone();
        let minimum_duration = self.minimum_duration;
        let is_enabled = self.is_enabled;
        view.update(cx, |view, cx| {
            view.on_changed = Some(on_changed);
            view.minimum_duration = minimum_duration;
            view.set_hold_enabled(is_enabled, cx);
        });
    }
}

/// transparent to the pointer, only measures and takes presses that land inside it
pub struct ShortsHoldRecognizerView {
    on_changed: Option<ShortsHoldRecognizerOnChanged>,
    minimum_duration: f64,
    is_hold_enabled: bool,
    is_holding: bool,
    press_origin: Option<Point<Pixels>>,
    press_generation: u64,
    hold: Option<Task<()>>,
}

impl Default for ShortsHoldRecognizerView {
    fn default() -> Self {
        Self::new()
    }
}

impl ShortsHoldRecognizerView {
    pub const ALLOWABLE_MOVEMENT: f32 = 12.0;
    pub const DEFAULT_MINIMUM_DURATION: f64 = 0.22;

    pub fn new() -> Self {
        Self {
            on_changed: None,
            minimum_duration: Self::DEFAULT_MINIMUM_DURATION,
            is_hold_enabled: true,
            is_holding: false,
            press_origin: None,
            press_generation: 0,
            hold: None,
        }
    }

    pub fn is_holding(&self) -> bool {
        self.is_holding
    }

    pub fn set_hold_enabled(&mut self, is_enabled: bool, cx: &mut App) {
        self.is_hold_enabled = is_enabled;
        if is_enabled || !self.is_holding {
            return;
        }
        self.is_holding = false;
        self.notify_after_pass(false, cx);
    }

    pub fn uninstall(&mut self, cx: &mut App) {
        self.cancel_press();
        if self.is_holding {
            self.is_holding = false;
            self.notify_after_pass(false, cx);
        }
    }

    fn notify_after_pass(&self, holding: bool, cx: &mut App) {
        let Some(on_changed) = self.on_changed.clone() else {
            return;
        };
        cx.defer(move |cx| on_changed(holding, cx));
    }

    fn should_receive(&self) -> bool {
        self.is_hold_enabled
    }

    fn cancel_press(&mut self) {
        self.press_origin = None;
        self.hold = None;
        self.press_generation += 1;
    }

    fn press_down(&mut self, origin: Point<Pixels>) -> u64 {
        self.cancel_press();
        self.press_origin = Some(origin);
        self.press_generation
    }

    fn moved_too_far(&self, position: Point<Pixels>) -> bool {
        self.press_origin.is_some_and(|origin| {
            let dx = f32::from(position.x - origin.x);
            let dy = f32::from(position.y - origin.y);
            (dx * dx + dy * dy).sqrt() > Self::ALLOWABLE_MOVEMENT
        })
    }

    fn press_moved(&mut self, position: Point<Pixels>) {
        if !self.is_holding && self.moved_too_far(position) {
            self.cancel_press();
        }
    }

    fn hold_began(&mut self, generation: u64) -> Option<ShortsHoldRecognizerOnChanged> {
        if generation != self.press_generation || self.press_origin.is_none() || self.is_holding {
            return None;
        }
        self.hold = None;
        self.is_holding = true;
        self.on_changed.clone()
    }

    /// press ended, returns the callback to notify if a live hold ended with it
    fn press_ended(&mut self) -> Option<ShortsHoldRecognizerOnChanged> {
        self.cancel_press();
        if !self.is_holding {
            return None;
        }
        self.is_holding = false;
        self.on_changed.clone()
    }
}

fn start_hold(view: &Entity<ShortsHoldRecognizerView>, origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let (generation, minimum_duration) = view.update(cx, |view, _| (view.press_down(origin), view.minimum_duration));
    let duration = Duration::from_secs_f64(minimum_duration.max(0.0));
    let weak = view.downgrade();
    let executor = cx.background_executor().clone();
    let task = window.spawn(cx, async move |window_cx| {
        executor.timer(duration).await;
        let _ = window_cx.update(|_, cx| {
            let began = weak.update(cx, |view, _| view.hold_began(generation)).ok().flatten();
            if let Some(on_changed) = began {
                on_changed(true, cx);
            }
        });
    });
    view.update(cx, |view, _| {
        if view.press_generation == generation {
            view.hold = Some(task);
        }
    });
}

fn end_press(view: &Entity<ShortsHoldRecognizerView>, cx: &mut App) {
    if let Some(on_changed) = view.update(cx, |view, _| view.press_ended()) {
        on_changed(false, cx);
    }
}

impl RenderOnce for ShortsHoldRecognizerHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            cx.on_release(|view: &mut ShortsHoldRecognizerView, cx| view.uninstall(cx)).detach();
            ShortsHoldRecognizerView::new()
        });
        self.apply(&view, cx);

        let down_view = view.clone();
        let move_view = view.clone();
        let up_view = view.clone();
        let up_out_view = view;
        div()
            .absolute()
            .inset_0()
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                if down_view.read(cx).should_receive() {
                    start_hold(&down_view, event.position, window, cx);
                }
            })
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    move_view.update(cx, |view, _| view.press_moved(event.position));
                }
            })
            .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, _, cx| end_press(&up_view, cx))
            .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, _, cx| end_press(&up_out_view, cx))
    }
}
