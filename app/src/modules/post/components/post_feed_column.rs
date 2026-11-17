use gpui::{AnyElement, App, ElementId, ScrollWheelEvent, Window, div, point, prelude::*, px};
use rise_ui::swiftui::measure::measure;

use crate::core::widgets::navigations::main_tab_scroll_to_top::MainTabScrollToTop;

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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let width = rise_ui::theme(cx).feed.content_width;
        let half_width = f32::from(width) / 2.0;
        let mid_x = window.use_keyed_state(self.id.clone(), cx, |_, _| None::<f32>);
        let wheel_mid_x = mid_x.clone();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .justify_center()
            .child(div().absolute().top_0().left_0().size_full().on_scroll_wheel(
                move |event: &ScrollWheelEvent, window, cx| {
                    let Some(mid_x) = *wheel_mid_x.read(cx) else {
                        return;
                    };
                    if (f32::from(event.position.x) - mid_x).abs() <= half_width {
                        return;
                    }
                    MainTabScrollToTop::forward_wheel(point(px(mid_x), event.position.y), event, window, cx);
                },
            ))
            .child(div().relative().h_full().w_full().min_w_0().max_w(width).child(self.content))
            .child(measure(self.id, move |bounds, _window, cx| {
                let centre = f32::from(bounds.center().x);
                mid_x.update(cx, |value, _| *value = Some(centre));
            }))
    }
}
