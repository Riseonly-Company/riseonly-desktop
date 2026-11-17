use gpui::{App, Window};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;

use crate::modules::user::components::user_relationship_list_contract::UserRelationshipRowMetrics;

#[derive(IntoElement, Default)]
pub struct UserRowSkeleton {
    alternation_index: usize,
}

impl UserRowSkeleton {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alternation_index(mut self, alternation_index: usize) -> Self {
        self.alternation_index = alternation_index;
        self
    }

    fn alt(&self, values: [f32; 4]) -> f32 {
        values[self.alternation_index % values.len()]
    }
}

impl RenderOnce for UserRowSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let button_height = UserRelationshipRowMetrics::relationship_button_minimum_height();
        hstack(UserRelationshipRowMetrics::CONTENT_SPACING)
            .w_full()
            .px(px(UserRelationshipRowMetrics::HORIZONTAL_PADDING))
            .py(px(UserRelationshipRowMetrics::VERTICAL_PADDING))
            .child(SkeletonCircle::new(UserRelationshipRowMetrics::AVATAR_SIZE))
            .child(
                vstack(7.0)
                    .align(HorizontalAlignment::Leading)
                    .child(SkeletonLine::new(self.alt([110.0, 80.0, 140.0, 95.0]), 9.0))
                    .child(SkeletonLine::new(self.alt([70.0, 90.0, 60.0, 100.0]), 7.0)),
            )
            .child(spacer_min_w(0.0))
            .child(SkeletonBlock::new(self.alt([88.0, 112.0, 96.0, 124.0]), button_height, button_height * 0.5))
    }
}

#[derive(IntoElement)]
pub struct UserListSkeleton {
    rows: usize,
    shows_separators: bool,
}

impl Default for UserListSkeleton {
    fn default() -> Self {
        Self { rows: 10, shows_separators: true }
    }
}

impl UserListSkeleton {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    pub fn shows_separators(mut self, shows_separators: bool) -> Self {
        self.shows_separators = shows_separators;
        self
    }
}

impl RenderOnce for UserListSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let shows_separators = self.shows_separators;
        let rows = (0..self.rows).map(|index| {
            vstack(0.0)
                .w_full()
                .child(UserRowSkeleton::new().alternation_index(index))
                .when(shows_separators, |row| row.child(div().w_full().h(px(0.5)).bg(theme.border._100)))
        });
        div()
            .size_full()
            .overflow_hidden()
            .child(vstack(if shows_separators { 0.0 } else { 1.0 }).w_full().children(rows).skeleton_shimmer(None))
    }
}
