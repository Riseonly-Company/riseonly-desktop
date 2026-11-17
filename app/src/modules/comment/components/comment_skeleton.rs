use gpui::{App, IntoElement, RenderOnce, Window};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;

use crate::modules::comment::components::native_list::comment_list_models::CommentRowMetrics;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CommentRowSkeletonStyle {
    #[default]
    Sheet,
    Profile,
}

#[derive(IntoElement, Default)]
pub struct CommentRowSkeleton {
    style: CommentRowSkeletonStyle,
    alternation_index: i64,
}

impl CommentRowSkeleton {
    pub fn style(mut self, style: CommentRowSkeletonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn alternation_index(mut self, alternation_index: i64) -> Self {
        self.alternation_index = alternation_index;
        self
    }

    fn alt(&self, values: [f32; 3]) -> f32 {
        values[self.alternation_index.rem_euclid(values.len() as i64) as usize]
    }
}

impl RenderOnce for CommentRowSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let row = hstack(8.0)
            .align(VerticalAlignment::Top)
            .w_full()
            .px(px(CommentRowMetrics::HORIZONTAL_PADDING))
            .py(px(10.0))
            .child(SkeletonCircle::new(35.0))
            .child(
                vstack(7.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w_0()
                    .child(
                        hstack(5.0)
                            .w_full()
                            .child(SkeletonLine::new(self.alt([90.0, 120.0, 70.0]), 8.0))
                            .child(spacer_min_w(8.0))
                            .child(SkeletonLine::new(24.0_f32, 6.0)),
                    )
                    .child(SkeletonLine::new(None::<f32>, 9.0))
                    .child(SkeletonLine::new(self.alt([200.0, 140.0, 240.0]), 9.0)),
            );

        let footer = match self.style {
            CommentRowSkeletonStyle::Sheet => hstack(8.0)
                .pl(px(CommentRowMetrics::HORIZONTAL_PADDING + 43.0))
                .pr(px(10.0))
                .pb(px(10.0))
                .child(SkeletonBlock::new(64.0_f32, 23.0_f32, 6.0))
                .child(SkeletonBlock::new(56.0_f32, 23.0_f32, 6.0))
                .into_any_element(),
            CommentRowSkeletonStyle::Profile => div()
                .w_full()
                .pl(px(CommentRowMetrics::HORIZONTAL_PADDING + 43.0))
                .pr(px(10.0))
                .pb(px(10.0))
                .child(SkeletonBlock::new(None::<f32>, 56.0_f32, 10.0))
                .into_any_element(),
        };

        vstack(2.0).align(HorizontalAlignment::Leading).w_full().child(row).child(footer)
    }
}

#[derive(IntoElement)]
pub struct CommentListSkeleton {
    style: CommentRowSkeletonStyle,
    rows: i64,
}

impl Default for CommentListSkeleton {
    fn default() -> Self {
        Self { style: CommentRowSkeletonStyle::Sheet, rows: 7 }
    }
}

impl CommentListSkeleton {
    pub fn style(mut self, style: CommentRowSkeletonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn rows(mut self, rows: i64) -> Self {
        self.rows = rows;
        self
    }
}

impl RenderOnce for CommentListSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let style = self.style;
        let rows = (0..self.rows).map(move |index| CommentRowSkeleton::default().style(style).alternation_index(index));
        div().size_full().overflow_hidden().child(vstack(0.0).w_full().children(rows).skeleton_shimmer(None))
    }
}
