use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Context, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement, RenderOnce, Styled, Window,
    div,
};

use crate::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};

pub type ReorderGripOnChange = Rc<dyn Fn(f32, &mut Window, &mut App)>;
pub type ReorderGripOnEnd = Rc<dyn Fn(&mut Window, &mut App)>;

/// reorder grip of a settings row, only marks where a drag may start
/// on_change gets the vertical travel since the press began
#[derive(IntoElement)]
pub struct ReorderGripHandle {
    id: ElementId,
    pub on_change: ReorderGripOnChange,
    pub on_end: ReorderGripOnEnd,
}

impl ReorderGripHandle {
    pub fn new(id: impl Into<ElementId>, on_change: ReorderGripOnChange, on_end: ReorderGripOnEnd) -> Self {
        Self {
            id: id.into(),
            on_change,
            on_end,
        }
    }
}

/// whether the press has traveled far enough to lift the row
#[derive(Default)]
struct ReorderGripDriver {
    moved: Rc<Cell<bool>>,
}

impl ReorderGripDriver {
    const LIFT_THRESHOLD: f32 = 2.0;
}

impl RenderOnce for ReorderGripHandle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let driver = window.use_keyed_state(
            ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "reorder-grip-driver".into()),
            cx,
            |_: &mut Window, _: &mut Context<ReorderGripDriver>| ReorderGripDriver::default(),
        );
        let moved = driver.read(cx).moved.clone();
        let changed_moved = moved.clone();
        let on_change = self.on_change.clone();
        let on_end = self.on_end.clone();
        let gesture = DragGesture::new(0.0)
            .on_changed(move |value: &DragGestureValue, window: &mut Window, cx: &mut App| {
                let translation = f32::from(value.translation.height);
                if !changed_moved.get() && translation.abs() < ReorderGripDriver::LIFT_THRESHOLD {
                    return;
                }
                changed_moved.set(true);
                on_change(translation, window, cx);
            })
            .on_ended(move |_: &DragGestureValue, window: &mut Window, cx: &mut App| {
                if moved.replace(false) {
                    on_end(window, cx);
                }
            });
        div()
            .size_full()
            .child(
                div()
                    .id(self.id.clone())
                    .size_full()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
            )
            .gesture(self.id, gesture)
    }
}
