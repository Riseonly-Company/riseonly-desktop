use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, DispatchPhase, ElementId, Entity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, RenderOnce, Size, Task, Window, canvas, div, point, prelude::*, px, size,
};
use rise_core::foundation::foundation_date::TimeInterval;

pub type ChatComposerCaptureGestureAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type ChatComposerCaptureGestureDrag = Rc<dyn Fn(Size<Pixels>, &mut Window, &mut App)>;
/// second argument true means the system cancelled the press
/// window lost the pointer or went to background
/// or the button was released outside, not a normal release
pub type ChatComposerCaptureGestureEnd = Rc<dyn Fn(Size<Pixels>, bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ChatComposerCaptureGesturePlate {
    pub id: ElementId,
    pub is_enabled: bool,
    pub hold_delay: TimeInterval,
    pub tap_max_movement: f32,
    pub on_press_began: ChatComposerCaptureGestureAction,
    pub on_press_ended: ChatComposerCaptureGestureAction,
    pub on_hold_began: ChatComposerCaptureGestureAction,
    pub on_hold_drag: ChatComposerCaptureGestureDrag,
    pub on_hold_ended: ChatComposerCaptureGestureEnd,
    pub on_tap: ChatComposerCaptureGestureAction,
}

pub struct ChatComposerCaptureGesturePlateCoordinator {
    pub hold_delay: TimeInterval,
    pub tap_max_movement: f32,
    pub on_press_began: ChatComposerCaptureGestureAction,
    pub on_press_ended: ChatComposerCaptureGestureAction,
    pub on_hold_began: ChatComposerCaptureGestureAction,
    pub on_hold_drag: ChatComposerCaptureGestureDrag,
    pub on_hold_ended: ChatComposerCaptureGestureEnd,
    pub on_tap: ChatComposerCaptureGestureAction,

    touch_active: bool,
    hold_started: bool,
    moved_beyond_tap: bool,
    began_in_active_window: bool,
    start_point: Point<Pixels>,
    hold_work_item: Option<Task<()>>,
}

enum PlateFinish {
    HoldEnded(ChatComposerCaptureGestureEnd),
    PressEnded {
        on_press_ended: ChatComposerCaptureGestureAction,
        on_tap: Option<ChatComposerCaptureGestureAction>,
    },
}

impl ChatComposerCaptureGesturePlateCoordinator {
    fn new(plate: &ChatComposerCaptureGesturePlate) -> Self {
        Self {
            hold_delay: plate.hold_delay,
            tap_max_movement: plate.tap_max_movement,
            on_press_began: plate.on_press_began.clone(),
            on_press_ended: plate.on_press_ended.clone(),
            on_hold_began: plate.on_hold_began.clone(),
            on_hold_drag: plate.on_hold_drag.clone(),
            on_hold_ended: plate.on_hold_ended.clone(),
            on_tap: plate.on_tap.clone(),
            touch_active: false,
            hold_started: false,
            moved_beyond_tap: false,
            began_in_active_window: false,
            start_point: Point::default(),
            hold_work_item: None,
        }
    }

    fn update_from(&mut self, plate: &ChatComposerCaptureGesturePlate) {
        self.hold_delay = plate.hold_delay;
        self.tap_max_movement = plate.tap_max_movement;
        self.on_press_began = plate.on_press_began.clone();
        self.on_press_ended = plate.on_press_ended.clone();
        self.on_hold_began = plate.on_hold_began.clone();
        self.on_hold_drag = plate.on_hold_drag.clone();
        self.on_hold_ended = plate.on_hold_ended.clone();
        self.on_tap = plate.on_tap.clone();
    }

    fn translation(&self, current: Point<Pixels>) -> Size<Pixels> {
        size(current.x - self.start_point.x, current.y - self.start_point.y)
    }

    pub fn start_hold_if_needed(coordinator: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let on_hold_began = coordinator.update(cx, |coordinator, _| {
            if !coordinator.touch_active || coordinator.hold_started {
                return None;
            }
            coordinator.hold_started = true;
            Some(coordinator.on_hold_began.clone())
        });
        if let Some(on_hold_began) = on_hold_began {
            on_hold_began(window, cx);
        }
    }

    pub fn finish(
        coordinator: &Entity<Self>,
        translation: Size<Pixels>,
        cancelled: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let finish = coordinator.update(cx, |coordinator, _| {
            coordinator.hold_work_item = None;
            if !coordinator.touch_active {
                return None;
            }
            coordinator.touch_active = false;
            let finish = if coordinator.hold_started {
                PlateFinish::HoldEnded(coordinator.on_hold_ended.clone())
            } else {
                PlateFinish::PressEnded {
                    on_press_ended: coordinator.on_press_ended.clone(),
                    on_tap: (!cancelled && !coordinator.moved_beyond_tap).then(|| coordinator.on_tap.clone()),
                }
            };
            x484_mark(|| {
                format!(
                    "finish cancelled={cancelled} hold={} moved={}",
                    coordinator.hold_started, coordinator.moved_beyond_tap
                )
            });
            coordinator.hold_started = false;
            coordinator.moved_beyond_tap = false;
            Some(finish)
        });
        match finish {
            Some(PlateFinish::HoldEnded(on_hold_ended)) => on_hold_ended(translation, cancelled, window, cx),
            Some(PlateFinish::PressEnded { on_press_ended, on_tap }) => {
                on_press_ended(window, cx);
                if let Some(on_tap) = on_tap {
                    on_tap(window, cx);
                }
            }
            None => {}
        }
    }

    fn touches_began(coordinator: &Entity<Self>, location: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let began_in_active_window = window.is_window_active();
        x484_mark(|| format!("began window_active={began_in_active_window}"));
        let (on_press_began, hold_delay) = coordinator.update(cx, |coordinator, _| {
            coordinator.hold_work_item = None;
            coordinator.touch_active = true;
            coordinator.began_in_active_window = began_in_active_window;
            coordinator.hold_started = false;
            coordinator.moved_beyond_tap = false;
            coordinator.start_point = location;
            (coordinator.on_press_began.clone(), coordinator.hold_delay)
        });
        on_press_began(window, cx);

        let weak = coordinator.downgrade();
        let delay = Duration::from_secs_f64(hold_delay.max(0.0));
        let work = window.spawn(cx, async move |cx| {
            cx.background_executor().timer(delay).await;
            let _ = cx.update(|window, cx| {
                if let Some(coordinator) = weak.upgrade() {
                    Self::start_hold_if_needed(&coordinator, window, cx);
                }
            });
        });
        coordinator.update(cx, |coordinator, _| coordinator.hold_work_item = Some(work));
    }

    fn touches_moved(coordinator: &Entity<Self>, location: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let on_hold_drag = coordinator.update(cx, |coordinator, _| {
            if !coordinator.touch_active {
                return None;
            }
            let translation = coordinator.translation(location);
            let distance = f32::from(translation.width).hypot(f32::from(translation.height));
            if distance > coordinator.tap_max_movement {
                coordinator.moved_beyond_tap = true;
            }
            coordinator
                .hold_started
                .then(|| (coordinator.on_hold_drag.clone(), translation))
        });
        if let Some((on_hold_drag, translation)) = on_hold_drag {
            on_hold_drag(translation, window, cx);
        }
    }

    fn touches_finished(
        coordinator: &Entity<Self>,
        location: Option<Point<Pixels>>,
        cancelled: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let translation = match location {
            Some(location) => coordinator.read(cx).translation(location),
            None => Size::default(),
        };
        Self::finish(coordinator, translation, cancelled, window, cx);
    }
}

fn is_inside_window(location: Point<Pixels>, window: &Window) -> bool {
    let viewport = Bounds::new(point(px(0.), px(0.)), window.viewport_size());
    viewport.contains(&location)
}

impl RenderOnce for ChatComposerCaptureGesturePlate {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let coordinator = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            ChatComposerCaptureGesturePlateCoordinator::new(&self)
        });
        coordinator.update(cx, |coordinator, _| coordinator.update_from(&self));

        let is_enabled = self.is_enabled;
        let press_coordinator = coordinator.clone();
        let tracking_coordinator = coordinator.clone();

        div()
            .id(self.id)
            .relative()
            .size_full()
            .when(is_enabled, move |plate| {
                plate.on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    ChatComposerCaptureGesturePlateCoordinator::touches_began(
                        &press_coordinator,
                        event.position,
                        window,
                        cx,
                    );
                })
            })
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, (), window, cx| {
                        let tracking = tracking_coordinator.read(cx);
                        let touch_active = tracking.touch_active;
                        let began_in_active_window = tracking.began_in_active_window;
                        if !is_enabled && !touch_active {
                            return;
                        }
                        if touch_active && began_in_active_window && !window.is_window_active() {
                            let coordinator = tracking_coordinator.clone();
                            window.defer(cx, move |window, cx| {
                                ChatComposerCaptureGesturePlateCoordinator::touches_finished(
                                    &coordinator,
                                    None,
                                    true,
                                    window,
                                    cx,
                                );
                            });
                            return;
                        }
                        let moved = tracking_coordinator.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble || !moved.read(cx).touch_active {
                                return;
                            }
                            if event.pressed_button != Some(MouseButton::Left) {
                                ChatComposerCaptureGesturePlateCoordinator::touches_finished(
                                    &moved,
                                    Some(event.position),
                                    true,
                                    window,
                                    cx,
                                );
                                return;
                            }
                            ChatComposerCaptureGesturePlateCoordinator::touches_moved(
                                &moved,
                                event.position,
                                window,
                                cx,
                            );
                        });
                        let released = tracking_coordinator.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble
                                || event.button != MouseButton::Left
                                || !released.read(cx).touch_active
                            {
                                return;
                            }
                            let cancelled = !is_inside_window(event.position, window);
                            ChatComposerCaptureGesturePlateCoordinator::touches_finished(
                                &released,
                                Some(event.position),
                                cancelled,
                                window,
                                cx,
                            );
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}

fn x484_mark(message: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X484", "{}", message());
}
