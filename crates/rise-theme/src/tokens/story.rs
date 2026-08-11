use gpui::Pixels;

use crate::tokens::avatar::AvatarMetrics;
use crate::tokens::density::Density;

/// The stories strip above the feed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StoryMetrics {
    /// Between two cells, and around the row.
    pub tray_gap: Pixels,
    pub tray_padding_x: Pixels,
    pub tray_padding_y: Pixels,
    /// Between a face and the name under it.
    pub cell_gap: Pixels,
    /// How wide a cell's label may be before it truncates.
    pub cell_width: Pixels,
    /// The "add a story" button on the user's own cell, and how far it hangs
    /// off the bottom-right of their face.
    pub compose_button: Pixels,
    pub compose_offset: Pixels,
    /// The ring that separates the compose button from the avatar behind it.
    pub compose_border: Pixels,
}

impl StoryMetrics {
    pub const NAME_SIZE: f32 = 12.0;
    pub const COMPOSE_GLYPH_SIZE: f32 = 13.0;

    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            tray_gap: l(14.0),
            tray_padding_x: l(14.0),
            tray_padding_y: l(10.0),
            cell_gap: l(7.0),
            cell_width: l(74.0),
            compose_button: l(26.0),
            compose_offset: l(5.0),
            compose_border: l(2.0),
        }
    }

    /// The strip's height, derived so it reserves exactly its content and never
    /// changes while the avatars load.
    ///
    /// `name_line_height` must come from the typography token the label is drawn
    /// with, or the reservation and the text disagree.
    pub fn tray_height(&self, avatar: &AvatarMetrics, name_line_height: Pixels) -> Pixels {
        self.tray_padding_y * 2.0 + avatar.story_ring + self.cell_gap + name_line_height
    }
}

impl Default for StoryMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::typography::Typography;
    use gpui::{FontWeight, px};

    #[test]
    fn the_cell_numbers_are_the_reference_ones() {
        let story = StoryMetrics::default();
        assert_eq!(story.tray_gap, px(14.0));
        assert_eq!(story.tray_padding_y, px(10.0));
        assert_eq!(story.cell_gap, px(7.0));
        assert_eq!(story.cell_width, px(74.0));
    }

    #[test]
    fn the_strip_reserves_its_content_at_every_density() {
        for multiplier in [0.85, 1.0, 1.25] {
            let density = Density::new(multiplier);
            let story = StoryMetrics::new(density);
            let avatar = AvatarMetrics::new(density);
            let label = Typography::new(density)
                .style(StoryMetrics::NAME_SIZE, FontWeight::MEDIUM)
                .line_height;

            let height = story.tray_height(&avatar, label);
            let content = avatar.story_ring + story.cell_gap + label;

            assert!(
                height >= content + story.tray_padding_y * 2.0,
                "a cell taller than the strip would clip the author's name"
            );
        }
    }

    #[test]
    fn the_strip_grows_with_density_rather_than_clipping_harder() {
        let of = |multiplier: f32| {
            let density = Density::new(multiplier);
            StoryMetrics::new(density).tray_height(
                &AvatarMetrics::new(density),
                Typography::new(density)
                    .style(StoryMetrics::NAME_SIZE, FontWeight::MEDIUM)
                    .line_height,
            )
        };

        assert!(of(0.85) < of(1.0));
        assert!(of(1.0) < of(1.25));
    }
}
