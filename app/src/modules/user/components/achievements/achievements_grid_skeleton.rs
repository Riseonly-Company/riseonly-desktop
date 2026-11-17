use gpui::{App, Window};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;

use crate::modules::user::components::achievements::achievement_card::AchievementSquareGeometry;

/// laid out from the same numbers as the card so content lands on the same pixels
/// and doesnt push the page down
#[derive(IntoElement)]
pub struct AchievementsGridSkeleton {
    side: f32,
    columns: usize,
    rows: usize,
    spacing: f32,
    row_spacing: f32,
}

impl AchievementsGridSkeleton {
    pub fn new(side: f32) -> Self {
        Self { side, columns: 4, rows: 3, spacing: 12.0, row_spacing: 16.0 }
    }

    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = columns;
        self
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn row_spacing(mut self, row_spacing: f32) -> Self {
        self.row_spacing = row_spacing;
        self
    }

    fn cell(side: f32) -> Div {
        vstack(8.0)
            .w(px(side))
            .child(SkeletonBlock::new(side, side, AchievementSquareGeometry::corner_radius(side)))
            .child(
                vstack(2.0)
                    .child(SkeletonBlock::new(side * 0.8, 12.0, 6.0))
                    .child(SkeletonBlock::new(side * 0.6, 12.0, 6.0))
                    .child(SkeletonBlock::new(side * 0.5, 10.0, 5.0)),
            )
    }
}

impl RenderOnce for AchievementsGridSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let columns = self.columns.max(1);
        let count = self.rows.max(1) * columns;
        let side = self.side;

        div()
            .w_full()
            .grid()
            .grid_cols(columns as u16)
            .gap_x(px(self.spacing))
            .gap_y(px(self.row_spacing))
            .children((0..count).map(move |_| div().flex().justify_center().child(Self::cell(side))))
            .skeleton_shimmer(None)
    }
}
