use gpui::Pixels;

use crate::tokens::density::Density;

pub use crate::avatar_palette::AvatarPalette;

/// avatar sizes, one per place an avatar is drawn
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AvatarMetrics {
    pub post_header: Pixels,
    pub comment: Pixels,
    pub likes_summary: Pixels,
    pub likes_summary_step: Pixels,
    pub story: Pixels,
    pub story_ring: Pixels,
    pub story_cell: Pixels,
    pub ring_width: Pixels,
    pub toolbar: Pixels,
    pub border_width: Pixels,
}

impl AvatarMetrics {
    pub const LETTER_RATIO: f32 = 0.46;
    pub const LETTER_MIN: f32 = 11.0;

    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            post_header: l(40.0),
            comment: l(36.0),
            likes_summary: l(23.0),
            likes_summary_step: l(15.0),
            story: l(62.0),
            story_ring: l(68.0),
            story_cell: l(74.0),
            ring_width: l(2.5),
            toolbar: l(45.0),
            border_width: l(1.0),
        }
    }

    /// letter point size for a face of this diameter
    pub fn letter_size(diameter: Pixels) -> f32 {
        (f32::from(diameter) * Self::LETTER_RATIO).max(Self::LETTER_MIN)
    }
}

impl Default for AvatarMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_letter_never_shrinks_below_legibility() {
        assert_eq!(AvatarMetrics::letter_size(px(40.0)), 40.0 * 0.46);
        assert_eq!(
            AvatarMetrics::letter_size(px(20.0)),
            AvatarMetrics::LETTER_MIN,
            "a 20pt face would otherwise draw a 9pt letter"
        );
    }

    #[test]
    fn density_scales_every_face() {
        assert_eq!(AvatarMetrics::new(Density::new(1.25)).post_header, px(50.0));
    }
}
