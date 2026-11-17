use gpui::{AnyElement, App, Div, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonCircle, SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::frame::{EdgeSet, SwiftUiAlignedFrame, SwiftUiFrame};
use rise_ui::swiftui::stacks::{Alignment, HorizontalAlignment, StackAlign, hstack, spacer_min_w, vstack};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PostCardSkeletonVariant {
    #[default]
    Text,
    Media {
        aspect: f32,
    },
}

#[derive(IntoElement)]
pub struct PostCardSkeleton {
    variant: PostCardSkeletonVariant,
    alternation_index: i64,
    shimmers: bool,
}

impl Default for PostCardSkeleton {
    fn default() -> Self {
        Self { variant: PostCardSkeletonVariant::Text, alternation_index: 0, shimmers: true }
    }
}

impl PostCardSkeleton {
    pub fn variant(mut self, variant: PostCardSkeletonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn alternation_index(mut self, alternation_index: i64) -> Self {
        self.alternation_index = alternation_index;
        self
    }

    pub fn shimmers(mut self, shimmers: bool) -> Self {
        self.shimmers = shimmers;
        self
    }

    fn card(&self) -> Div {
        let bottom = if self.variant == PostCardSkeletonVariant::Text { 6.0 } else { 8.0 };
        let mut card = vstack(0.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(
                self.header()
                    .padding_edges(EdgeSet::HORIZONTAL, 12.0)
                    .padding_edges(EdgeSet::TOP, 10.0)
                    .padding_edges(EdgeSet::BOTTOM, 6.0),
            )
            .child(
                self.text_block()
                    .padding_edges(EdgeSet::HORIZONTAL, 12.0)
                    .padding_edges(EdgeSet::TOP, 5.0)
                    .padding_edges(EdgeSet::BOTTOM, bottom),
            );

        if let PostCardSkeletonVariant::Media { aspect } = self.variant {
            card = card.child(
                div().w_full().pb(px(6.0)).child(
                    div()
                        .relative()
                        .w_full()
                        .aspect_ratio(aspect)
                        .child(div().absolute().inset_0().child(SkeletonBlock::new(None::<f32>, None::<f32>, 0.0))),
                ),
            );
        }

        card.child(
            self.actions_bar()
                .padding_edges(EdgeSet::HORIZONTAL, 12.0)
                .padding_edges(EdgeSet::TOP, 6.0)
                .padding_edges(EdgeSet::BOTTOM, 8.0),
        )
    }

    fn header(&self) -> Div {
        hstack(10.0)
            .w_full()
            .child(SkeletonCircle::new(40.0))
            .child(
                vstack(8.0)
                    .align(HorizontalAlignment::Leading)
                    .child(SkeletonLine::new(self.alt([120.0, 96.0, 148.0]), 10.0))
                    .child(SkeletonLine::new(self.alt([72.0, 96.0, 56.0]), 8.0)),
            )
            .child(spacer_min_w(0.0))
    }

    fn text_block(&self) -> Div {
        let block = vstack(0.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(SkeletonLine::new(self.alt([190.0, 230.0, 160.0]), 12.0));
        if self.variant != PostCardSkeletonVariant::Text {
            return block;
        }
        block.child(
            vstack(11.0)
                .w_full()
                .align(HorizontalAlignment::Leading)
                .child(SkeletonLine::default())
                .child(SkeletonLine::default())
                .child(SkeletonLine::new(self.alt([210.0, 150.0, 250.0]), 10.0))
                .padding_edges(EdgeSet::TOP, 16.0),
        )
    }

    fn actions_bar(&self) -> Div {
        hstack(5.0).w_full().children((0..4).map(|_| {
            div()
                .flex_1()
                .min_h(px(30.0))
                .flex()
                .items_center()
                .justify_center()
                .child(hstack(6.0).child(SkeletonCircle::new(18.0)).child(SkeletonLine::new(24.0, 8.0)))
        }))
    }

    fn alt(&self, values: [f32; 3]) -> f32 {
        values[self.alternation_index.rem_euclid(values.len() as i64) as usize]
    }
}

impl RenderOnce for PostCardSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if self.shimmers {
            self.card().skeleton_shimmer(None).into_any_element()
        } else {
            self.card().into_any_element()
        }
    }
}

#[derive(IntoElement)]
pub struct PostFeedSkeleton {
    card_count: i64,
    top_padding: f32,
    fixed_variant: Option<PostCardSkeletonVariant>,
}

impl Default for PostFeedSkeleton {
    fn default() -> Self {
        Self { card_count: 6, top_padding: 5.0, fixed_variant: None }
    }
}

impl PostFeedSkeleton {
    pub fn card_count(mut self, card_count: i64) -> Self {
        self.card_count = card_count;
        self
    }

    pub fn top_padding(mut self, top_padding: f32) -> Self {
        self.top_padding = top_padding;
        self
    }

    pub fn fixed_variant(mut self, fixed_variant: impl Into<Option<PostCardSkeletonVariant>>) -> Self {
        self.fixed_variant = fixed_variant.into();
        self
    }

    fn feed_variant(index: i64) -> PostCardSkeletonVariant {
        if index % 2 == 0 {
            return PostCardSkeletonVariant::Media { aspect: if index % 4 == 0 { 1.25 } else { 1.5 } };
        }
        PostCardSkeletonVariant::Text
    }
}

impl RenderOnce for PostFeedSkeleton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let border = rise_ui::theme(cx).border._100;
        let fixed_variant = self.fixed_variant;
        let rows = (0..self.card_count).flat_map(move |index| -> [AnyElement; 2] {
            [
                PostCardSkeleton::default()
                    .variant(fixed_variant.unwrap_or_else(|| Self::feed_variant(index)))
                    .alternation_index(index)
                    .shimmers(false)
                    .into_any_element(),
                div().w_full().h(px(1.0)).bg(border).into_any_element(),
            ]
        });
        vstack(5.0)
            .w_full()
            .children(rows)
            .padding_edges(EdgeSet::TOP, self.top_padding)
            .skeleton_shimmer(None)
            .frame_max(Alignment::Top)
            .overflow_hidden()
    }
}

#[derive(IntoElement)]
pub struct PostGridSkeleton {
    columns: i64,
    spacing: f32,
    cell_height: Option<f32>,
    rows: i64,
}

impl Default for PostGridSkeleton {
    fn default() -> Self {
        Self { columns: 3, spacing: 2.0, cell_height: None, rows: 4 }
    }
}

impl PostGridSkeleton {
    pub fn columns(mut self, columns: i64) -> Self {
        self.columns = columns;
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn cell_height(mut self, cell_height: impl Into<Option<f32>>) -> Self {
        self.cell_height = cell_height.into();
        self
    }

    pub fn rows(mut self, rows: i64) -> Self {
        self.rows = rows;
        self
    }

    fn cell(cell_height: Option<f32>) -> Div {
        let cell = div().flex_1().min_w(px(0.0));
        match cell_height {
            Some(cell_height) => cell.child(SkeletonBlock::new(None::<f32>, cell_height, 0.0)),
            None => cell
                .relative()
                .aspect_ratio(1.0)
                .child(div().absolute().inset_0().child(SkeletonBlock::new(None::<f32>, None::<f32>, 0.0))),
        }
    }
}

impl RenderOnce for PostGridSkeleton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Self { columns, spacing, cell_height, rows } = self;
        let grid_rows = (0..rows).map(move |_| {
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(spacing))
                .children((0..columns).map(move |_| Self::cell(cell_height)))
        });
        vstack(spacing)
            .w_full()
            .children(grid_rows)
            .skeleton_shimmer(None)
            .frame_max(Alignment::Top)
            .min_h(px(0.0))
            .overflow_hidden()
    }
}
