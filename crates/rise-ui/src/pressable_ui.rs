use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, Entity, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, RenderOnce,
    StatefulInteractiveElement, Styled, Task, Window, div,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::{ImpactStyle, impact, selection_changed};
use smallvec::SmallVec;

use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::button_style::{ButtonAction, focus_visible_ring};
use crate::swiftui::motion::{MotionSpec, ease_out, spring};
use crate::swiftui::transition::TransitionModifiers;

pub const PRESS_RECOGNITION: Duration = Duration::from_micros(100);

pub const MAXIMUM_PRESS_DISTANCE: f32 = 10.0;

pub fn press_in_motion() -> MotionSpec {
    ease_out(0.1)
}

/// spring back to 1 after a press ends
pub fn press_out_motion() -> MotionSpec {
    spring(0.25, 0.7)
}

/// what a press has done so far, lives in element state while the content is drawn
pub struct PressableState {
    scale: Animated<f32>,
    did_long_press: bool,
    press_origin: Option<Point<Pixels>>,
    hold_generation: u64,
    hold: Option<Task<()>>,
}

impl Default for PressableState {
    fn default() -> Self {
        Self::new()
    }
}

impl PressableState {
    pub fn new() -> Self {
        Self { scale: Animated::new(1.0), did_long_press: false, press_origin: None, hold_generation: 0, hold: None }
    }

    pub fn scale(&self, now: MotionTime) -> f32 {
        self.scale.value(now)
    }

    pub fn did_long_press(&self) -> bool {
        self.did_long_press
    }

    pub fn press_began(&mut self, tap_scale: f32, now: MotionTime, reduce_motion: bool) {
        self.scale.set(tap_scale, press_in_motion(), now, reduce_motion);
        self.scale.set(1.0, press_out_motion(), now + PRESS_RECOGNITION, reduce_motion);
    }

    pub fn press_down(&mut self, origin: Point<Pixels>) {
        self.press_ended();
        self.press_origin = Some(origin);
    }

    /// identifies the current press for a hold timer started with it
    pub fn hold_generation(&self) -> u64 {
        self.hold_generation
    }

    /// pointer left the press, a pending hold no longer fires
    pub fn press_ended(&mut self) {
        self.press_origin = None;
        self.hold = None;
        self.hold_generation += 1;
    }

    pub fn moved_too_far(&self, position: Point<Pixels>) -> bool {
        self.press_origin.is_some_and(|origin| {
            let dx = f32::from(position.x - origin.x);
            let dy = f32::from(position.y - origin.y);
            (dx * dx + dy * dy).sqrt() > MAXIMUM_PRESS_DISTANCE
        })
    }

    /// the hold reached its duration, returns whether the long press fires
    pub fn hold_elapsed(&mut self, generation: u64, now: MotionTime, reduce_motion: bool) -> bool {
        if generation != self.hold_generation || self.press_origin.is_none() {
            return false;
        }
        self.did_long_press = true;
        self.scale.set(1.0, press_out_motion(), now, reduce_motion);
        true
    }

    /// tap ended, returns whether on_press runs
    /// not right after a long press, the tap closing it only clears
    pub fn tap_ended(&mut self) -> bool {
        if self.did_long_press {
            self.did_long_press = false;
            return false;
        }
        true
    }
}

#[derive(IntoElement)]
pub struct PressableUi {
    id: ElementId,
    tap_scale: f32,
    pub long_press_scale: f32,
    long_press_duration: f32,
    disabled: bool,
    on_press: Option<ButtonAction>,
    on_long_press: Option<ButtonAction>,
    children: SmallVec<[AnyElement; 2]>,
}

impl PressableUi {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            tap_scale: 0.98,
            long_press_scale: 0.92,
            long_press_duration: 0.5,
            disabled: false,
            on_press: None,
            on_long_press: None,
            children: SmallVec::new(),
        }
    }

    pub fn tap_scale(mut self, scale: f32) -> Self {
        self.tap_scale = scale;
        self
    }

    pub fn long_press_scale(mut self, scale: f32) -> Self {
        self.long_press_scale = scale;
        self
    }

    pub fn long_press_duration(mut self, seconds: f32) -> Self {
        self.long_press_duration = seconds;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_press(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_press = Some(Rc::new(action));
        self
    }

    pub fn on_long_press(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_long_press = Some(Rc::new(action));
        self
    }
}

impl ParentElement for PressableUi {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

fn start_hold(
    state: &Entity<PressableState>,
    duration: Duration,
    on_long_press: ButtonAction,
    window: &mut Window,
    cx: &mut App,
) {
    let generation = state.read(cx).hold_generation();
    let weak = state.downgrade();
    let executor = cx.background_executor().clone();
    let task = window.spawn(cx, async move |window_cx| {
        executor.timer(duration).await;
        let _ = window_cx.update(|window, cx| {
            let fired = weak
                .update(cx, |pressable, cx| {
                    let elapsed = pressable.hold_elapsed(generation, RiseClock::monotonic(), cx.reduce_motion());
                    cx.notify();
                    elapsed
                })
                .unwrap_or(false);
            if fired {
                let _ = impact(ImpactStyle::Medium);
                on_long_press(window, cx);
            }
        });
    });
    state.update(cx, |pressable, _| pressable.hold = Some(task));
}

impl RenderOnce for PressableUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| PressableState::new());
        let now = RiseClock::monotonic();
        let scale = {
            let pressable = state.read(cx);
            if pressable.scale.is_running(now) {
                window.request_animation_frame();
            }
            pressable.scale(now)
        };

        let content = div().children(self.children).scale_effect(scale);
        if self.disabled {
            return div().id(self.id).child(content);
        }

        let tap_scale = self.tap_scale;
        let hold_duration = Duration::from_secs_f32(self.long_press_duration.max(0.0));
        let hold = self.on_long_press.map(|action| (action, hold_duration));
        let context_press = hold.as_ref().map(|(action, _)| action.clone());
        let on_press = self.on_press;
        let down_state = state.clone();
        let move_state = state.clone();
        let up_state = state.clone();
        let up_out_state = state.clone();

        focus_visible_ring(div().id(self.id).tab_index(0))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                window.prevent_default();
                let reduce_motion = cx.reduce_motion();
                down_state.update(cx, |pressable, cx| {
                    pressable.press_down(event.position);
                    pressable.press_began(tap_scale, RiseClock::monotonic(), reduce_motion);
                    cx.notify();
                });
                if let Some((action, duration)) = hold.clone() {
                    start_hold(&down_state, duration, action, window, cx);
                }
            })
            .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, window, cx| {
                if let Some(action) = &context_press {
                    let _ = impact(ImpactStyle::Medium);
                    action(window, cx);
                }
            })
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) && move_state.read(cx).moved_too_far(event.position) {
                    move_state.update(cx, |pressable, _| pressable.press_ended());
                }
            })
            .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, _, cx| {
                up_state.update(cx, |pressable, _| pressable.press_ended());
            })
            .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, _, cx| {
                up_out_state.update(cx, |pressable, _| pressable.press_ended());
            })
            .on_click(move |_: &ClickEvent, window, cx| {
                if state.update(cx, |pressable, _| pressable.tap_ended())
                    && let Some(on_press) = &on_press
                {
                    let _ = selection_changed();
                    on_press(window, cx);
                }
            })
            .child(content)
    }
}
