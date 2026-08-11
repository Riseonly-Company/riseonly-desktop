use gpui::Pixels;

use crate::tokens::density::Density;

/// Lengths for the profile header and the tabbed content under it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ProfileMetrics {
    /// How wide the profile column may grow, whatever the window does.
    pub content_width: Pixels,

    /// The cover image, and the radius its bottom corners are cut with.
    pub banner_height: Pixels,
    pub banner_radius: Pixels,

    /// The face, how far it hangs below the banner, and the ring drawn round it.
    pub avatar_size: Pixels,
    pub avatar_hang: Pixels,
    pub avatar_border: Pixels,

    /// The inset every block of the header shares, and the gap between blocks.
    pub padding_x: Pixels,
    pub block_gap: Pixels,
    /// Between the avatar and the name block, and inside the name block.
    pub identity_gap: Pixels,
    pub identity_row_gap: Pixels,
    /// How far the name block sits below the top of the avatar row.
    pub identity_top: Pixels,
    pub badge_size: Pixels,

    /// The capsule surfaces: the stats bar, the pills and the round buttons.
    pub surface_radius: Pixels,
    pub surface_border: Pixels,
    pub stats_height: Pixels,
    pub stats_divider_height: Pixels,
    pub stats_value_gap: Pixels,
    pub pill_padding_x: Pixels,
    pub pill_padding_y: Pixels,
    pub pill_gap: Pixels,
    pub pill_row_gap: Pixels,
    pub action_height: Pixels,
    pub action_gap: Pixels,
    pub circle_button: Pixels,

    /// The about block: between its label, the bio and what follows.
    pub about_gap: Pixels,
    pub about_inset: Pixels,

    /// The tab bar over the content, and the space above and below it.
    pub tabs_top: Pixels,
    pub tabs_bottom: Pixels,

    /// A goal or plan row.
    pub row_padding_x: Pixels,
    pub row_padding_y: Pixels,
    pub row_gap: Pixels,
    pub row_radius: Pixels,
    pub progress_height: Pixels,
}

impl ProfileMetrics {
    /// Type sizes for the profile header; not steps of the typography ramp.
    pub const NAME_SIZE: f32 = 22.0;
    pub const TAG_SIZE: f32 = 13.0;
    pub const LABEL_SIZE: f32 = 13.0;
    pub const BIO_SIZE: f32 = 15.0;
    pub const STAT_VALUE_SIZE: f32 = 16.0;
    pub const STAT_LABEL_SIZE: f32 = 10.5;
    pub const PILL_VALUE_SIZE: f32 = 13.0;
    pub const PILL_LABEL_SIZE: f32 = 11.0;
    pub const ACTION_SIZE: f32 = 14.0;
    pub const ROW_TITLE_SIZE: f32 = 15.0;
    pub const ROW_META_SIZE: f32 = 12.0;

    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            content_width: l(600.0),

            banner_height: l(200.0),
            banner_radius: l(26.0),

            avatar_size: l(88.0),
            avatar_hang: l(46.0),
            avatar_border: l(2.0),

            padding_x: l(16.0),
            block_gap: l(14.0),
            identity_gap: l(14.0),
            identity_row_gap: l(3.0),
            identity_top: l(30.0),
            badge_size: l(20.0),

            surface_radius: l(999.0),
            surface_border: l(1.0),
            stats_height: l(56.0),
            stats_divider_height: l(24.0),
            stats_value_gap: l(2.0),
            pill_padding_x: l(12.0),
            pill_padding_y: l(7.0),
            pill_gap: l(5.0),
            pill_row_gap: l(8.0),
            action_height: l(33.0),
            action_gap: l(8.0),
            circle_button: l(32.0),

            about_gap: l(8.0),
            about_inset: l(4.0),

            tabs_top: l(10.0),
            tabs_bottom: l(6.0),

            row_padding_x: l(14.0),
            row_padding_y: l(12.0),
            row_gap: l(8.0),
            row_radius: l(12.0),
            progress_height: l(6.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_and_the_face_keep_the_references_proportions() {
        let metrics = ProfileMetrics::new(Density::NORMAL);

        assert_eq!(metrics.banner_height, gpui::px(200.0));
        assert_eq!(metrics.avatar_size, gpui::px(88.0));
        assert_eq!(
            metrics.avatar_hang,
            gpui::px(46.0),
            "the face hangs by roughly half its height, which is what leaves room for the name"
        );
    }

    #[test]
    fn every_length_moves_with_density() {
        let comfortable = ProfileMetrics::new(Density::COMFORTABLE);
        let compact = ProfileMetrics::new(Density::COMPACT);

        assert!(compact.banner_height < comfortable.banner_height);
        assert!(compact.stats_height < comfortable.stats_height);
        assert!(compact.padding_x < comfortable.padding_x);
    }
}
