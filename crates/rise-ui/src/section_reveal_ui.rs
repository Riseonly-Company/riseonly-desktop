use gpui::prelude::FluentBuilder;
use gpui::{AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use rise_core::foundation::foundation_date::RiseClock;

use crate::swiftui::animated::Animated;
use crate::swiftui::motion::{MotionSpec, spring};
use crate::swiftui::shapes::pass_through;

/// height reveal for a panel that grows out of a settings row
/// content is mounted at full height once and clipped
/// so expansion animates one container height instead of rebuilding layout every spring frame
#[derive(IntoElement)]
pub struct SectionRevealUi {
    id: ElementId,
    is_expanded: bool,
    expanded_height: f32,
    content: AnyElement,
    animated: bool,
}

struct SectionRevealState {
    reveal_progress: Animated<f32>,
    is_expanded: bool,
}

fn reveal_motion() -> MotionSpec {
    spring(0.38, 0.88)
}

impl SectionRevealUi {
    pub fn new(id: impl Into<ElementId>, is_expanded: bool, expanded_height: f32, content: impl IntoElement) -> Self {
        Self { id: id.into(), is_expanded, expanded_height, content: content.into_any_element(), animated: true }
    }

    /// off means the panel opens and folds in one frame
    /// a sheet in a deferred layer lays out the whole window every spring frame so a panel there
    /// grows at once
    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }
}

impl RenderOnce for SectionRevealUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_expanded = self.is_expanded;
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| SectionRevealState {
            reveal_progress: Animated::new(if is_expanded { 1.0 } else { 0.0 }),
            is_expanded,
        });
        let now = RiseClock::monotonic();
        let reduce_motion = cx.reduce_motion() || !self.animated;
        state.update(cx, |state, _| {
            if state.is_expanded != is_expanded {
                state.is_expanded = is_expanded;
                let target = if is_expanded { 1.0 } else { 0.0 };
                state.reveal_progress.set(target, reveal_motion(), now, reduce_motion);
            }
        });
        let reveal_progress = state.read(cx).reveal_progress.frame_value(now, window);

        let settled_closed = !is_expanded && reveal_progress <= 0.001;
        let revealed = div()
            .w_full()
            .h(px(self.expanded_height))
            .when(settled_closed, |panel| panel.invisible())
            .child(self.content);
        let height = px((self.expanded_height * reveal_progress).max(0.0));

        if is_expanded && reveal_progress > 0.98 {
            div().w_full().h(height).overflow_hidden().child(revealed).into_any_element()
        } else {
            pass_through().w_full().h(height).overflow_hidden().child(revealed).into_any_element()
        }
    }
}
