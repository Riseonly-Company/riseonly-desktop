use gpui::Pixels;

use crate::tokens::density::Density;

/// The segmented tab bar.
///
/// The header and the pager must measure tabs from these same numbers: the
/// indicator interpolates between two tab rectangles while a drag is in flight.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TabsMetrics {
    /// The whole bar, pill included.
    pub bar_height: Pixels,
    /// Between the pill's edge and a tab label — also the indicator's inset,
    /// which is why it is one number and not two.
    pub bar_inner_padding: Pixels,
    /// Around a label when the bar scrolls rather than distributing.
    pub tab_padding_x: Pixels,
    /// The narrowest a tab may become when the bar distributes its width; below
    /// this the bar scrolls instead.
    pub tab_min_width: Pixels,
    pub indicator_radius: Pixels,
    /// The label size, unchanged when a tab activates — only the face gets
    /// heavier, so the pill's travel never moves a neighbour.
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

    /// The indicator's height: the bar minus its inset on both sides.
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

            // Tolerance, not equality: both sides are f32 products of the multiplier.
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
