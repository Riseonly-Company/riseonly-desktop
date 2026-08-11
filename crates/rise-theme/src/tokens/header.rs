use gpui::Pixels;

use crate::tokens::density::Density;

/// The bar every screen opens with: leading, centre and trailing slots, where
/// the centre is an overlay so a long title cannot push the buttons off.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HeaderMetrics {
    pub height: Pixels,
    pub padding_x: Pixels,
    /// Both side slots, occupied or not. An empty slot still reserves it.
    pub button_size: Pixels,
    /// Between the title and the subtitle above it.
    pub title_gap: Pixels,
    /// The widest a centred title may grow before it truncates.
    pub title_max_width: Pixels,
}

impl HeaderMetrics {
    pub const ICON_SIZE: f32 = 17.0;
    pub const BACK_ICON_SIZE: f32 = 16.0;
    pub const TITLE_SIZE: f32 = 15.0;
    pub const SUBTITLE_SIZE: f32 = 11.0;
    /// A panel's own header title, a size up from a page header's.
    pub const PANEL_TITLE_SIZE: f32 = 16.0;

    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            height: l(56.0),
            padding_x: l(8.0),
            button_size: l(45.0),
            title_gap: l(1.0),
            title_max_width: l(220.0),
        }
    }
}

impl Default for HeaderMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_numbers_are_the_reference_ones() {
        let header = HeaderMetrics::default();
        assert_eq!(header.height, px(56.0));
        assert_eq!(header.button_size, px(45.0));
        assert_eq!(header.padding_x, px(8.0));
    }

    #[test]
    fn a_side_button_fits_inside_the_bar_it_sits_in() {
        for multiplier in [0.85, 1.0, 1.25] {
            let header = HeaderMetrics::new(Density::new(multiplier));
            assert!(header.button_size < header.height);
        }
    }
}
