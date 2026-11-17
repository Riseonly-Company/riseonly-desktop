use std::rc::Rc;

use gpui::{
    App, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    RenderOnce, Styled, Window, div,
};

use crate::hold_context_menu_ui::rise_strict_tap_gesture_recognizer::{
    RiseScrollMotionProbe, RiseStrictTapGestureRecognizer, RiseTapPolicy,
};

#[derive(IntoElement)]
pub struct RiseStrictTapOverlay {
    id: ElementId,
    pub movement_slop: f32,
    pub governing_scroll_view: Option<RiseScrollMotionProbe>,
    pub action: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl RiseStrictTapOverlay {
    /// id keys the recognizer, which lives as long as the overlay is drawn
    pub fn new(id: impl Into<ElementId>, action: Rc<dyn Fn(&mut Window, &mut App)>) -> Self {
        Self {
            id: id.into(),
            movement_slop: RiseTapPolicy::MOVEMENT_SLOP,
            governing_scroll_view: None,
            action,
        }
    }

    pub fn movement_slop(mut self, movement_slop: f32) -> Self {
        self.movement_slop = movement_slop;
        self
    }

    pub fn governing_scroll_view(mut self, governing_scroll_view: Option<RiseScrollMotionProbe>) -> Self {
        self.governing_scroll_view = governing_scroll_view;
        self
    }
}

impl RenderOnce for RiseStrictTapOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let recognizer = window.use_keyed_state(self.id.clone(), cx, |_, _| RiseStrictTapGestureRecognizer::new());
        recognizer.update(cx, |recognizer, _| {
            recognizer.movement_slop = self.movement_slop;
            recognizer.governing_scroll_view = self.governing_scroll_view.clone();
        });

        let down = recognizer.clone();
        let moved = recognizer.clone();
        let action = self.action;
        div()
            .id(self.id)
            .absolute()
            .inset_0()
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                down.update(cx, |recognizer, _| recognizer.touches_began(event.position));
            })
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    moved.update(cx, |recognizer, _| recognizer.touches_moved(event.position));
                }
            })
            .on_mouse_up(MouseButton::Left, move |event: &MouseUpEvent, window, cx| {
                let is_tap = recognizer.update(cx, |recognizer, _| recognizer.touches_ended(event.position));
                if is_tap {
                    action(window, cx);
                }
            })
    }
}
