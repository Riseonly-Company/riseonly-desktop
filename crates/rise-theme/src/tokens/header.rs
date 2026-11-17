use gpui::Pixels;

use crate::tokens::density::Density;

/// page header bar with leading, centre and trailing slots
/// the centre is an overlay
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HeaderMetrics {
    pub height: Pixels,
    pub padding_x: Pixels,
    pub button_size: Pixels,
    pub title_gap: Pixels,
    pub title_max_width: Pixels,
}

impl HeaderMetrics {
    pub const ICON_SIZE: f32 = 17.0;
    pub const BACK_ICON_SIZE: f32 = 16.0;
    pub const TITLE_SIZE: f32 = 15.0;
    pub const SUBTITLE_SIZE: f32 = 11.0;
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
