use std::sync::Arc;

use gpui::{AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::transition::UnitPoint;

const ENTER_DURATION: f64 = 0.28;
const ENTER_OFFSET_Y: f32 = 14.0;
const ENTER_START_OPACITY: f32 = 0.35;

struct ChatOutgoingMessageEnterState {
    progress: Animated<f32>,
}

#[derive(IntoElement)]
pub struct ChatOutgoingMessageEnterModifier {
    id: ElementId,
    is_enabled: bool,
    content: AnyElement,
}

impl ChatOutgoingMessageEnterModifier {
    pub fn new(id: impl Into<ElementId>, is_enabled: bool, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            is_enabled,
            content: content.into_any_element(),
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(name))
    }
}

impl RenderOnce for ChatOutgoingMessageEnterModifier {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| ChatOutgoingMessageEnterState {
            progress: Animated::new(0.0),
        });
        let progress = state.read(cx).progress.frame_value(RiseClock::monotonic(), window);
        let is_enabled = self.is_enabled;
        let opacity = if is_enabled { ENTER_START_OPACITY + (1.0 - ENTER_START_OPACITY) * progress } else { 1.0 };
        let offset_y = if is_enabled { ENTER_OFFSET_Y * (1.0 - progress) } else { 0.0 };
        let appear_id = self.child_id("appear");

        div()
            .relative()
            .top(px(offset_y))
            .opacity(opacity)
            .child(self.content)
            .on_appear(appear_id, move |_, cx| {
                state.update(cx, |state, cx| {
                    if !is_enabled {
                        state.progress.jump(1.0);
                    } else {
                        state.progress.jump(0.0);
                        state.progress.set_in(1.0, ease_out(ENTER_DURATION), RiseClock::monotonic(), cx);
                    }
                    cx.notify();
                });
            })
    }
}

pub trait ChatOutgoingMessageEnterModifiers: IntoElement + Sized {
    fn chat_outgoing_message_enter(
        self,
        id: impl Into<ElementId>,
        is_enabled: bool,
        _anchor: UnitPoint,
    ) -> ChatOutgoingMessageEnterModifier {
        ChatOutgoingMessageEnterModifier::new(id, is_enabled, self)
    }
}

impl<E: IntoElement> ChatOutgoingMessageEnterModifiers for E {}
