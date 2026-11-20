use gpui::{AnyElement, App, ElementId, Window, div, prelude::*};

/// full post rows in the feed column in the middle of a wide centre like the posts tab
/// a post page or picks feed opened with the panel closed ran its rows over the whole centre
#[derive(IntoElement)]
pub struct PostFeedColumn {
    id: ElementId,
    content: AnyElement,
}

impl PostFeedColumn {
    pub fn new(id: impl Into<ElementId>, content: impl IntoElement) -> Self {
        Self { id: id.into(), content: content.into_any_element() }
    }
}

impl RenderOnce for PostFeedColumn {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let width = rise_ui::theme(cx).feed.content_width;
        div()
            .id(self.id)
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .justify_center()
            .wheel_into_children()
            .child(div().relative().h_full().w_full().min_w_0().max_w(width).child(self.content))
    }
}
