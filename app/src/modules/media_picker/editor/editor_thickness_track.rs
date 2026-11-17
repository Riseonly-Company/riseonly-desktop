use gpui::{
    App, BoxShadow, Hsla, IntoElement, ParentElement, RenderOnce, Styled, Window, div, hsla, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::transition::Transition;
use std::rc::Rc;

fn white(opacity: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, opacity)
}

fn black(opacity: f32) -> Hsla {
    hsla(0.0, 0.0, 0.0, opacity)
}

fn drop_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: black(0.35),
        offset: point(px(0.0), px(1.0)),
        blur_radius: px(3.0),
        spread_radius: px(0.0),
        inset: false,
    }]
}

struct EditorThicknessTrackDrag {
    is_dragging: bool,
    knob_scale: Animated<f32>,
}

impl EditorThicknessTrackDrag {
    fn set_dragging(&mut self, is_dragging: bool, cx: &App) {
        self.is_dragging = is_dragging;
        let target = if is_dragging { 1.18 } else { 1.0 };
        self.knob_scale.set_in(target, spring(0.25, 0.78), RiseClock::monotonic(), cx);
    }
}

#[derive(IntoElement)]
pub struct EditorThicknessTrack {
    id: gpui::ElementId,
    fraction: Binding<f32>,
    track_height: f32,
    knob_size: f32,
    on_interaction_change: Rc<dyn Fn(bool, &mut Window, &mut App)>,
}

impl EditorThicknessTrack {
    pub fn new(
        id: impl Into<gpui::ElementId>,
        fraction: Binding<f32>,
        track_height: f32,
        knob_size: f32,
        on_interaction_change: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            fraction,
            track_height,
            knob_size,
            on_interaction_change: Rc::new(on_interaction_change),
        }
    }
}

impl RenderOnce for EditorThicknessTrack {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let drag = window.use_keyed_state(self.id.clone(), cx, |_, _| EditorThicknessTrackDrag {
            is_dragging: false,
            knob_scale: Animated::new(1.0),
        });
        let now = RiseClock::monotonic();
        let knob_scale = drag.read(cx).knob_scale.frame_value(now, window);

        let track_height = self.track_height;
        let knob_size = self.knob_size;
        let fraction = self.fraction.get(cx);
        let width = knob_size.max(28.0);
        let drawn_knob = knob_size * knob_scale;

        let track = div()
            .absolute()
            .bottom_0()
            .left(px((width - 4.0) / 2.0))
            .w(px(4.0))
            .h(px(track_height))
            .capsule()
            .bg(white(0.12));
        let filled = div()
            .absolute()
            .bottom_0()
            .left(px((width - 4.0) / 2.0))
            .w(px(4.0))
            .h(px((track_height * fraction).max(2.0)))
            .capsule()
            .bg(white(1.0));
        let knob = div()
            .absolute()
            .left(px((width - drawn_knob) / 2.0))
            .top(px(track_height * (1.0 - fraction) - drawn_knob / 2.0))
            .w(px(drawn_knob))
            .h(px(drawn_knob))
            .circle()
            .bg(white(1.0))
            .shadow(drop_shadow())
            .child(centred_stroke(black(0.2), 0.5, Shape::Circle));

        let changed = {
            let drag = drag.clone();
            let binding = self.fraction.clone();
            let on_interaction_change = self.on_interaction_change.clone();
            move |value: &rise_ui::swiftui::drag_gesture::DragGestureValue,
                  window: &mut Window,
                  cx: &mut App| {
                if !drag.read(cx).is_dragging {
                    drag.update(cx, |drag, cx| {
                        drag.set_dragging(true, cx);
                        cx.notify();
                    });
                    on_interaction_change(true, window, cx);
                }
                let local = track_height - f32::from(value.location.y);
                let frac = (local / track_height).clamp(0.0, 1.0);
                binding.set(frac, window, cx);
            }
        };
        let ended = {
            let drag = drag.clone();
            let on_interaction_change = self.on_interaction_change.clone();
            move |_: &rise_ui::swiftui::drag_gesture::DragGestureValue,
                  window: &mut Window,
                  cx: &mut App| {
                drag.update(cx, |drag, cx| {
                    drag.set_dragging(false, cx);
                    cx.notify();
                });
                on_interaction_change(false, window, cx);
            }
        };

        if drag.read(cx).knob_scale.is_running(now) {
            window.request_animation_frame();
        }

        div()
            .relative()
            .w(px(width))
            .h(px(track_height))
            .child(track)
            .child(filled)
            .child(knob)
            .gesture(self.id, DragGesture::new(0.0).on_changed(changed).on_ended(ended))
    }
}

#[derive(IntoElement)]
pub struct EditorBrushPreviewCircle {
    width_px: f32,
    color: Hsla,
}

impl EditorBrushPreviewCircle {
    pub fn new(width_px: f32, color: Hsla) -> Self {
        Self { width_px, color }
    }

    /// transition for the circle, fade plus scale from 0.7
    /// set by the parent that shows it
    pub fn transition() -> Transition {
        Transition::opacity().combined(Transition::scale(0.7))
    }
}

impl RenderOnce for EditorBrushPreviewCircle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .w(px(self.width_px))
            .h(px(self.width_px))
            .circle()
            .bg(self.color.opacity(0.9))
            .shadow(drop_shadow())
            .child(centred_stroke(white(0.95), 1.0, Shape::Circle))
    }
}
