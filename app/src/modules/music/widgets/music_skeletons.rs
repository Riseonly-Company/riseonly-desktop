use gpui::{App, Div, IntoElement, RenderOnce, Window, div, px};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::prelude::*;

#[derive(IntoElement, Default)]
pub struct TrackRowSkeleton {
    alternation_index: usize,
}

impl TrackRowSkeleton {
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

impl RenderOnce for TrackRowSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        hstack(12.0)
            .w_full()
            .py(px(8.0))
            .child(SkeletonBlock::new(48.0, 48.0, 8.0))
            .child(
                vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .child(SkeletonLine::new(self.alt([140.0, 180.0, 110.0, 160.0]), 9.0))
                    .child(SkeletonLine::new(self.alt([90.0, 60.0, 120.0, 75.0]), 7.0)),
            )
            .child(spacer_min_w(0.0))
            .child(SkeletonLine::new(28.0, 7.0))
            .child(SkeletonCircle::new(15.0))
    }
}

#[derive(IntoElement)]
pub struct TrackListSkeleton {
    rows: usize,
    horizontal_padding: f32,
}

impl Default for TrackListSkeleton {
    fn default() -> Self {
        Self { rows: 7, horizontal_padding: 16.0 }
    }
}

impl TrackListSkeleton {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }

    pub fn horizontal_padding(mut self, horizontal_padding: f32) -> Self {
        self.horizontal_padding = horizontal_padding;
        self
    }
}

impl RenderOnce for TrackListSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let padding = self.horizontal_padding;
        let list = vstack(0.0).w_full().children((0..self.rows).map(|index| {
            div()
                .w_full()
                .px(px(padding))
                .child(TrackRowSkeleton::new().alternation_index(index))
        }));

        div().w_full().child(list.skeleton_shimmer(None))
    }
}

fn two_column_grid(spacing: f32) -> Div {
    div().grid().grid_cols(2).gap(px(spacing)).w_full().px(px(16.0))
}

#[derive(IntoElement)]
pub struct MusicRecentGridSkeleton {
    cells: usize,
}

impl Default for MusicRecentGridSkeleton {
    fn default() -> Self {
        Self { cells: 6 }
    }
}

impl MusicRecentGridSkeleton {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cells(mut self, cells: usize) -> Self {
        self.cells = cells;
        self
    }
}

impl RenderOnce for MusicRecentGridSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        two_column_grid(10.0)
            .children((0..self.cells).map(|_| div().w_full().child(SkeletonBlock::new(None, 52.0, 8.0))))
            .skeleton_shimmer(None)
    }
}

#[derive(IntoElement)]
pub struct MusicArtistsStripSkeleton {
    count: usize,
}

impl Default for MusicArtistsStripSkeleton {
    fn default() -> Self {
        Self { count: 4 }
    }
}

impl MusicArtistsStripSkeleton {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }
}

impl RenderOnce for MusicArtistsStripSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        hstack(16.0)
            .align(VerticalAlignment::Top)
            .w_full()
            .overflow_hidden()
            .px(px(16.0))
            .children((0..self.count).map(|_| {
                vstack(8.0)
                    .w(px(100.0))
                    .flex_none()
                    .child(SkeletonCircle::new(100.0))
                    .child(SkeletonLine::new(60.0, 7.0))
            }))
            .skeleton_shimmer(None)
    }
}

#[derive(IntoElement)]
pub struct AlbumCardGridSkeleton {
    cells: usize,
}

impl Default for AlbumCardGridSkeleton {
    fn default() -> Self {
        Self { cells: 4 }
    }
}

impl AlbumCardGridSkeleton {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cells(mut self, cells: usize) -> Self {
        self.cells = cells;
        self
    }
}

impl RenderOnce for AlbumCardGridSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        two_column_grid(16.0)
            .children((0..self.cells).map(|index| {
                let is_even = index.is_multiple_of(2);
                vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(div().w_full().aspect_square().child(SkeletonBlock::new(None, None, 12.0)))
                    .child(
                        vstack(5.0)
                            .align(HorizontalAlignment::Leading)
                            .child(SkeletonLine::new(if is_even { 100.0 } else { 130.0 }, 8.0))
                            .child(SkeletonLine::new(if is_even { 70.0 } else { 50.0 }, 6.0)),
                    )
            }))
            .skeleton_shimmer(None)
    }
}

#[derive(IntoElement)]
pub struct MusicLibraryListSkeleton {
    rows: usize,
}

impl Default for MusicLibraryListSkeleton {
    fn default() -> Self {
        Self { rows: 6 }
    }
}

impl MusicLibraryListSkeleton {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows;
        self
    }
}

impl RenderOnce for MusicLibraryListSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let list = vstack(0.0).w_full().children((0..self.rows).map(|index| {
            let is_even = index.is_multiple_of(2);
            hstack(14.0)
                .w_full()
                .px(px(16.0))
                .py(px(10.0))
                .child(SkeletonBlock::new(54.0, 54.0, 8.0))
                .child(
                    vstack(7.0)
                        .align(HorizontalAlignment::Leading)
                        .child(SkeletonLine::new(if is_even { 130.0 } else { 170.0 }, 9.0))
                        .child(SkeletonLine::new(if is_even { 80.0 } else { 60.0 }, 7.0)),
                )
                .child(spacer_min_w(0.0))
        }));

        div().w_full().child(list.skeleton_shimmer(None))
    }
}

#[derive(IntoElement, Default)]
pub struct PlaylistHeroSkeleton;

impl PlaylistHeroSkeleton {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for PlaylistHeroSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        vstack(16.0)
            .w_full()
            .py(px(24.0))
            .child(SkeletonBlock::new(180.0, 180.0, 16.0))
            .child(
                vstack(8.0)
                    .child(SkeletonLine::new(140.0, 11.0))
                    .child(SkeletonLine::new(90.0, 8.0)),
            )
            .child(SkeletonCircle::new(56.0))
            .skeleton_shimmer(None)
    }
}

/// sound page header before the track arrives
/// same geometry as the real one so the post grid below doesnt jump
#[derive(IntoElement, Default)]
pub struct AudioPageHeaderSkeleton;

impl AudioPageHeaderSkeleton {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for AudioPageHeaderSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let button_height = f32::from(rise_ui::theme(cx).button.height_300);

        vstack(12.0)
            .w_full()
            .child(SkeletonBlock::new(160.0, 160.0, 16.0))
            .child(
                vstack(4.0)
                    .child(SkeletonLine::new(170.0, 13.0))
                    .child(SkeletonLine::new(110.0, 10.0)),
            )
            .child(SkeletonBlock::new(220.0, button_height, button_height / 2.0))
            .skeleton_shimmer(None)
    }
}
