use gpui::{Div, IntoElement, div, prelude::*};
use rise_theme::AppTheme;
use rise_ui::animation::ShimmerBand;
use rise_ui::{SkeletonShape, SkeletonUi};

pub struct PostFeedSkeleton;

impl PostFeedSkeleton {
    const CARDS: usize = 5;

    pub fn render(theme: &AppTheme) -> impl IntoElement {
        let theme = theme.clone();

        SkeletonUi::group("feed.skeleton", move |band| {
            let mut column = div().size_full().flex().flex_col().overflow_hidden();

            for index in 0..Self::CARDS {
                column = column
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .child(Self::card(&theme, band, index)),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(theme.feed.separator_height)
                            .bg(theme.border._100),
                    );
            }

            column
        })
    }

    fn card(theme: &AppTheme, band: ShimmerBand, index: usize) -> Div {
        let metrics = theme.feed;
        let has_media = index.is_multiple_of(2);

        let mut card = div()
            .w_full()
            .max_w(metrics.content_width)
            .flex()
            .flex_col()
            .child(
                div()
                    .px(metrics.card_padding_x)
                    .pt(metrics.header_top)
                    .pb(metrics.header_bottom)
                    .child(Self::header(theme, band, index)),
            )
            .child(
                div()
                    .px(metrics.card_padding_x)
                    .pt(metrics.copy_top_after_header)
                    .pb(metrics.copy_bottom)
                    .child(Self::copy(theme, band, index, has_media)),
            );

        if has_media {
            let aspect = if index.is_multiple_of(4) { 1.25 } else { 1.5 };
            card = card.child(
                div().w_full().aspect_ratio(aspect).child(
                    SkeletonUi::shape(
                        theme,
                        band,
                        SkeletonShape::Rect {
                            width: metrics.content_width,
                            height: metrics.content_width / aspect,
                            radius: gpui::Pixels::ZERO,
                        },
                    )
                    .size_full(),
                ),
            );
        }

        card.child(
            div()
                .px(metrics.card_padding_x)
                .pt(metrics.actions_top)
                .pb(metrics.actions_bottom)
                .child(Self::actions(theme, band)),
        )
    }

    fn header(theme: &AppTheme, band: ShimmerBand, index: usize) -> Div {
        let metrics = theme.feed;
        let alternate = |values: [f32; 3]| values[index % values.len()];

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.header_gap)
            .child(SkeletonUi::circle(theme, band, theme.avatar.post_header))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(metrics.banner_gap)
                    .child(SkeletonUi::line(
                        theme,
                        band,
                        metrics.content_width * alternate([0.20, 0.16, 0.25]),
                    ))
                    .child(SkeletonUi::line(
                        theme,
                        band,
                        metrics.content_width * alternate([0.12, 0.16, 0.09]),
                    )),
            )
    }

    fn copy(theme: &AppTheme, band: ShimmerBand, index: usize, has_media: bool) -> Div {
        let metrics = theme.feed;
        let alternate = |values: [f32; 3]| values[index % values.len()];

        let mut column = div().w_full().flex().flex_col().gap(theme.spacing._600);

        column = column.child(SkeletonUi::line(
            theme,
            band,
            metrics.content_width * alternate([0.32, 0.38, 0.27]),
        ));

        if !has_media {
            column = column
                .child(SkeletonUi::line(theme, band, metrics.content_width * 0.92))
                .child(SkeletonUi::line(theme, band, metrics.content_width * 0.92))
                .child(SkeletonUi::line(
                    theme,
                    band,
                    metrics.content_width * alternate([0.35, 0.25, 0.42]),
                ));
        }

        column
    }

    fn actions(theme: &AppTheme, band: ShimmerBand) -> Div {
        let metrics = theme.feed;
        let mut row = div().w_full().flex().flex_row().gap(metrics.actions_gap);

        for _ in 0..4 {
            row = row.child(
                div()
                    .flex_1()
                    .h(metrics.actions_height)
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(metrics.banner_gap)
                    .child(SkeletonUi::circle(theme, band, theme.icon.regular))
                    .child(SkeletonUi::line(
                        theme,
                        band,
                        metrics.actions_count_min_width,
                    )),
            );
        }

        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skeleton_card_is_built_from_the_same_tokens_the_real_one_is() {
        let theme = AppTheme::dark();
        assert_eq!(theme.feed.actions_height, theme.feed.actions_hit_size);
        assert!(theme.feed.content_width > theme.avatar.post_header);
    }

    #[test]
    fn the_feed_draws_the_reference_number_of_placeholder_cards() {
        assert_eq!(PostFeedSkeleton::CARDS, 5);
    }

    #[test]
    fn the_media_card_reserves_the_height_the_clamped_gallery_will_use() {
        let theme = AppTheme::dark();
        for aspect in [1.25_f32, 1.5] {
            assert_eq!(theme.feed.gallery_aspect(Some(aspect)), aspect);
        }
    }
}
