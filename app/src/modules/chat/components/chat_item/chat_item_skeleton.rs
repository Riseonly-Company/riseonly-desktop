use gpui::{App, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use rise_ui::skeleton_ui::{SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::stacks::{hstack, spacer_min_w, vstack};

use crate::modules::chat::components::chat_item::chat_list_preview_row::ChatListRowMetrics;

#[derive(IntoElement)]
pub struct ChatItemSkeleton {
    alternation_index: i64,
}

impl ChatItemSkeleton {
    pub fn new(alternation_index: i64) -> Self {
        Self { alternation_index }
    }

    fn alt(&self, values: [f32; 4]) -> f32 {
        values[self.alternation_index.rem_euclid(values.len() as i64) as usize]
    }
}

impl Default for ChatItemSkeleton {
    fn default() -> Self {
        Self::new(0)
    }
}

impl RenderOnce for ChatItemSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        hstack(ChatListRowMetrics::LEADING_SPACING)
            .items_start()
            .w_full()
            .px(px(ChatListRowMetrics::HORIZONTAL_PADDING))
            .py(px(10.0))
            .child(SkeletonCircle::new(ChatListRowMetrics::AVATAR_SIZE))
            .child(
                vstack(9.0)
                    .items_start()
                    .flex_1()
                    .min_w_0()
                    .child(
                        hstack(4.0)
                            .w_full()
                            .child(SkeletonLine::new(self.alt([120.0, 90.0, 150.0, 105.0]), 10.0))
                            .child(spacer_min_w(8.0))
                            .child(SkeletonLine::new(32.0_f32, 8.0)),
                    )
                    .child(SkeletonLine::new(None::<f32>, 8.0))
                    .child(SkeletonLine::new(self.alt([180.0, 230.0, 130.0, 210.0]), 8.0)),
            )
    }
}

#[derive(IntoElement)]
pub struct ChatListSkeleton {
    rows: usize,
}

impl ChatListSkeleton {
    pub fn new() -> Self {
        Self { rows: 9 }
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }
}

impl Default for ChatListSkeleton {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for ChatListSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let separator = rise_ui::theme(cx).border._100;
        let rows = (0..self.rows).map(move |index| {
            div()
                .w_full()
                .child(ChatItemSkeleton::new(index as i64))
                .child(div().w_full().h(px(ChatListRowMetrics::SEPARATOR_HEIGHT)).bg(separator))
        });
        div().size_full().overflow_hidden().child(vstack(0.0).w_full().children(rows).skeleton_shimmer(None))
    }
}
