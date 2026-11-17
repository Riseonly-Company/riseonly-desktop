use gpui::Pixels;

use crate::tokens::density::Density;

/// the segmented tab bar, header and pager must measure tabs from these same numbers
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TabsMetrics {
    pub bar_height: Pixels,
    pub bar_inner_padding: Pixels,
    pub tab_padding_x: Pixels,
    pub tab_min_width: Pixels,
    pub indicator_radius: Pixels,
    pub tab_font_size: f32,
}

impl TabsMetrics {
    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            bar_height: l(44.0),
            bar_inner_padding: l(4.0),
            tab_padding_x: l(14.0),
            tab_min_width: l(56.0),
            indicator_radius: l(18.0),
            tab_font_size: 15.0,
        }
    }

    /// indicator height, the bar minus its inset on both sides
    pub fn indicator_height(&self) -> Pixels {
        self.bar_height - self.bar_inner_padding * 2.0
    }
}

impl Default for TabsMetrics {
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
        let tabs = TabsMetrics::default();
        assert_eq!(tabs.bar_height, px(44.0));
        assert_eq!(tabs.bar_inner_padding, px(4.0));
        assert_eq!(tabs.tab_padding_x, px(14.0));
        assert_eq!(tabs.tab_min_width, px(56.0));
    }

    #[test]
    fn the_indicator_never_overhangs_the_bar_it_sits_in() {
        for multiplier in [0.85, 1.0, 1.25] {
            let tabs = TabsMetrics::new(Density::new(multiplier));
            assert!(tabs.indicator_height() < tabs.bar_height);

            let inset = f32::from(tabs.bar_height - tabs.indicator_height());
            let expected = f32::from(tabs.bar_inner_padding) * 2.0;
            assert!(
                (inset - expected).abs() < 0.001,
                "the indicator's inset is {inset}, not {expected}"
            );
        }
    }

    #[test]
    fn density_scales_the_bar() {
        assert_eq!(TabsMetrics::new(Density::new(1.25)).bar_height, px(55.0));
    }
}
