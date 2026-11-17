use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window, div, relative,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::swiftui::animated::Animated;
use crate::swiftui::motion::MotionSpec;

/// one character slot, old rolls out and new rolls in
/// equal sides stay put
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct NumericColumn {
    pub(crate) old: Option<char>,
    pub(crate) new: Option<char>,
}

/// slots of a change from old to new, aligned on the last character
pub(crate) fn numeric_columns(old: &str, new: &str) -> Vec<NumericColumn> {
    let mut old = old.chars().rev();
    let mut new = new.chars().rev();
    let mut columns = Vec::new();
    loop {
        let (old, new) = (old.next(), new.next());
        if old.is_none() && new.is_none() {
            break;
        }
        columns.push(NumericColumn { old, new });
    }
    columns.reverse();
    columns
}

struct NumericTextState {
    shown: SharedString,
    previous: SharedString,
    progress: Animated<f32>,
    animation_value: Option<u64>,
}

/// text whose changed characters roll
/// without an animation whose value changed the text just snaps
#[derive(IntoElement)]
pub struct NumericText {
    id: ElementId,
    value: SharedString,
    counts_down: bool,
    animation: Option<(MotionSpec, u64)>,
    text_element: Rc<dyn Fn(String) -> AnyElement>,
}

impl NumericText {
    pub fn new(id: impl Into<ElementId>, value: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            value: value.into(),
            counts_down: false,
            animation: None,
            text_element: Rc::new(|text: String| text.into_any_element()),
        }
    }

    /// the new character comes from above
    pub fn counts_down(mut self, counts_down: bool) -> Self {
        self.counts_down = counts_down;
        self
    }

    /// rolls a text change only in the render where value changed
    pub fn animation(mut self, spec: MotionSpec, value: impl Hash) -> Self {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        self.animation = Some((spec, hasher.finish()));
        self
    }

    /// how a run of text is drawn, by default it inherits the parent text style
    pub fn text_element(mut self, text_element: impl Fn(String) -> AnyElement + 'static) -> Self {
        self.text_element = Rc::new(text_element);
        self
    }
}

impl RenderOnce for NumericText {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, value, counts_down, animation, text_element } = self;
        let animation_value = animation.map(|(_, value)| value);
        let initial = value.clone();
        let state = window.use_keyed_state(id, cx, |_, _| NumericTextState {
            shown: initial,
            previous: SharedString::default(),
            progress: Animated::new(1.0_f32),
            animation_value,
        });
        let now = RiseClock::monotonic();
        let reduce_motion = cx.reduce_motion();
        state.update(cx, |state, _| {
            let value_changed = state.animation_value != animation_value;
            state.animation_value = animation_value;
            if state.shown == value {
                return;
            }
            state.previous = std::mem::replace(&mut state.shown, value.clone());
            state.progress.jump(0.0);
            let spec = animation.filter(|_| value_changed).map(|(spec, _)| spec);
            state.progress.set(1.0, spec, now, reduce_motion);
        });
        let (progress, is_running, previous) = {
            let state = state.read(cx);
            (
                state.progress.frame_value(now, window).clamp(0.0, 1.0),
                state.progress.is_running(now),
                state.previous.clone(),
            )
        };

        if !is_running {
            return div().whitespace_nowrap().child(text_element(value.to_string()));
        }
        let slide = if counts_down { -1.0_f32 } else { 1.0_f32 };
        div().flex().flex_row().whitespace_nowrap().children(
            numeric_columns(&previous, &value).into_iter().map(|column| {
                if column.old == column.new {
                    return div().children(column.new.map(|new| text_element(new.to_string())));
                }
                div()
                    .relative()
                    .children(column.new.map(|new| {
                        div().opacity(0.0).child(text_element(new.to_string()))
                    }))
                    .children(column.new.map(|new| {
                        div()
                            .absolute()
                            .left_0()
                            .top(relative(slide * (1.0 - progress)))
                            .opacity(progress)
                            .child(text_element(new.to_string()))
                    }))
                    .children(column.old.map(|old| {
                        div()
                            .absolute()
                            .left_0()
                            .top(relative(-slide * progress))
                            .opacity(1.0 - progress)
                            .child(text_element(old.to_string()))
                    }))
            }),
        )
    }
}
