use gpui::Pixels;

use crate::tokens::density::Density;

/// lengths and type sizes for a feed post card, preview and detail bring their own
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FeedMetrics {
    pub content_width: Pixels,

    pub card_padding_x: Pixels,
    pub card_gap: Pixels,
    pub separator_height: Pixels,

    pub header_gap: Pixels,
    pub header_top: Pixels,
    pub header_bottom: Pixels,
    pub header_name_gap: Pixels,
    pub header_meta_gap: Pixels,
    pub badge_size: Pixels,
    pub options_button: Pixels,

    pub copy_gap: Pixels,
    pub copy_top_after_media: Pixels,
    pub copy_top_after_header: Pixels,
    pub copy_bottom: Pixels,
    pub hashtag_gap: Pixels,

    pub banner_padding_x: Pixels,
    pub banner_padding_y: Pixels,
    pub banner_gap: Pixels,
    pub banner_radius: Pixels,
    pub banner_bottom: Pixels,

    pub actions_gap: Pixels,
    pub actions_height: Pixels,
    pub actions_hit_size: Pixels,
    pub actions_top: Pixels,
    pub actions_bottom: Pixels,
    pub actions_bottom_with_summary: Pixels,
    pub actions_count_min_width: Pixels,

    pub summary_padding_x: Pixels,
    pub summary_bottom: Pixels,

    pub gallery_min_aspect: f32,
    pub gallery_max_aspect: f32,
    pub gallery_dot_size: Pixels,
    pub gallery_dot_gap: Pixels,
    pub gallery_dots_padding_x: Pixels,
    pub gallery_dots_padding_y: Pixels,
    pub gallery_dots_bottom: Pixels,

    pub double_tap_heart: Pixels,

    pub repost_border_width: Pixels,
    pub repost_bottom: Pixels,
}

impl FeedMetrics {
    pub const TITLE_SIZE: f32 = 17.0;
    pub const CONTENT_SIZE: f32 = 15.0;
    pub const AUTHOR_SIZE: f32 = 16.0;
    pub const META_SIZE: f32 = 12.0;
    pub const ACTION_COUNT_SIZE: f32 = 11.0;
    pub const BANNER_SIZE: f32 = 12.0;

    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            content_width: l(600.0),

            card_padding_x: l(12.0),
            card_gap: l(5.0),
            separator_height: l(1.0),

            header_gap: l(10.0),
            header_top: l(10.0),
            header_bottom: l(6.0),
            header_name_gap: l(5.0),
            header_meta_gap: l(4.0),
            badge_size: l(18.0),
            options_button: l(28.0),

            copy_gap: l(8.0),
            copy_top_after_media: l(8.0),
            copy_top_after_header: l(5.0),
            copy_bottom: l(2.0),
            hashtag_gap: l(4.0),

            banner_padding_x: l(10.0),
            banner_padding_y: l(7.0),
            banner_gap: l(6.0),
            banner_radius: l(8.0),
            banner_bottom: l(6.0),

            actions_gap: l(5.0),
            actions_height: l(30.0),
            actions_hit_size: l(30.0),
            actions_top: l(4.0),
            actions_bottom: l(10.0),
            actions_bottom_with_summary: l(2.0),
            actions_count_min_width: l(22.0),

            summary_padding_x: l(7.0),
            summary_bottom: l(10.0),

            gallery_min_aspect: 0.8,
            gallery_max_aspect: 1.91,
            gallery_dot_size: l(6.0),
            gallery_dot_gap: l(5.0),
            gallery_dots_padding_x: l(8.0),
            gallery_dots_padding_y: l(5.0),
            gallery_dots_bottom: l(10.0),

            double_tap_heart: l(84.0),

            repost_border_width: l(1.0),
            repost_bottom: l(10.0),
        }
    }

    /// gallery aspect from the first image the server reports, decided once before the bitmap arrives
    pub fn gallery_aspect(&self, reported: Option<f32>) -> f32 {
        reported
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(1.0)
            .clamp(self.gallery_min_aspect, self.gallery_max_aspect)
    }
}

impl Default for FeedMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_paddings_are_the_reference_ones() {
        let feed = FeedMetrics::default();
        assert_eq!(feed.card_padding_x, px(12.0));
        assert_eq!(feed.header_top, px(10.0));
        assert_eq!(feed.header_bottom, px(6.0));
        assert_eq!(feed.actions_top, px(4.0));
        assert_eq!(feed.actions_bottom, px(10.0));
        assert_eq!(feed.card_gap, px(5.0));
    }

    #[test]
    fn an_unreported_aspect_falls_back_to_a_square_rather_than_to_zero() {
        let feed = FeedMetrics::default();
        assert_eq!(feed.gallery_aspect(None), 1.0);
    }

    #[test]
    fn a_panorama_and_a_tower_are_both_clamped_into_the_row() {
        let feed = FeedMetrics::default();
        assert_eq!(feed.gallery_aspect(Some(6.0)), feed.gallery_max_aspect);
        assert_eq!(feed.gallery_aspect(Some(0.1)), feed.gallery_min_aspect);
    }

    #[test]
    fn a_broken_aspect_never_reaches_the_layout() {
        let feed = FeedMetrics::default();
        assert_eq!(feed.gallery_aspect(Some(f32::NAN)), 1.0);
        assert_eq!(feed.gallery_aspect(Some(0.0)), 1.0);
        assert_eq!(feed.gallery_aspect(Some(f32::INFINITY)), 1.0);
        assert_eq!(feed.gallery_aspect(Some(-2.0)), 1.0);
    }

    #[test]
    fn the_summary_line_takes_the_bottom_margin_off_the_actions_row() {
        let feed = FeedMetrics::default();
        assert!(feed.actions_bottom_with_summary < feed.actions_bottom);
    }

    #[test]
    fn density_scales_the_card() {
        assert_eq!(
            FeedMetrics::new(Density::new(1.25)).card_padding_x,
            px(15.0)
        );
    }
}
