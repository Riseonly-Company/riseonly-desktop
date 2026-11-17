use gpui::{
    App, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce, StatefulInteractiveElement, Styled,
    Window, div, px,
};

use crate::main_text::{MainText, TextAlignT};
use crate::swiftui::frame::SwiftUiFrame;
use crate::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};

pub const MAXIMUM_SHOWN_COUNT: i64 = 999;

#[derive(IntoElement)]
pub struct UnreadCountBadgeUi {
    count: i64,
    px: f32,
    fill: Option<Hsla>,
}

impl UnreadCountBadgeUi {
    pub fn new(count: i64) -> Self {
        Self { count, px: 10.0, fill: None }
    }

    pub fn px(mut self, px: f32) -> Self {
        self.px = px;
        self
    }

    pub fn fill(mut self, fill: impl Into<Option<Hsla>>) -> Self {
        self.fill = fill.into();
        self
    }

    /// text the capsule shows
    pub fn label(count: i64) -> String {
        if count > MAXIMUM_SHOWN_COUNT {
            format!("{MAXIMUM_SHOWN_COUNT}+")
        } else {
            count.to_string()
        }
    }
}

impl RenderOnce for UnreadCountBadgeUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.count <= 0 {
            return div().into_any_element();
        }
        let theme = crate::theme(cx);
        let fill = self.fill.unwrap_or(theme.semantic.error_100);
        let outline = theme.bg._100;
        let text = MainText::new(Self::label(self.count))
            .px(self.px)
            .tac(TextAlignT::Center)
            .font_weight(FontWeight::BOLD)
            .color(theme.content_on(fill))
            .number_of_lines(1);

        div()
            .id("unread-count-badge")
            .relative()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .px(px(4.0))
            .frame_min(self.px + 7.0, self.px + 7.0)
            .capsule()
            .bg(fill)
            .aria_label(self.count.to_string())
            .child(text)
            .child(centred_stroke(outline, 1.5, Shape::Capsule))
            .into_any_element()
    }
}
