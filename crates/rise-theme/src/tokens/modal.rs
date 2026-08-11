use gpui::Pixels;

use crate::tokens::density::Density;

/// Every modal in the product, dimensioned once.
///
/// The invariant these preserve: a dialog is narrower than the content behind
/// it, never taller than the window, and leaves scrim visible on all four sides.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ModalMetrics {
    /// A confirmation: one sentence and two buttons.
    pub width_small: Pixels,
    /// The default. A form, a short list, an explanation with an action.
    pub width_medium: Pixels,
    /// A picker or anything with a search field over a list.
    pub width_large: Pixels,

    /// The share of the window's height a modal may take before it scrolls
    /// inside itself.
    pub max_height_fraction: f32,
    /// …and never taller than this, however big the display is.
    pub max_height: Pixels,

    /// Around the panel's content.
    pub padding: Pixels,
    /// Between the header, the body and the actions.
    pub section_gap: Pixels,
    /// Between two action buttons.
    pub action_gap: Pixels,
    /// The height of an action button, and of the close control in the header.
    pub action_height: Pixels,
    pub close_size: Pixels,
}

impl ModalMetrics {
    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            width_small: l(360.0),
            width_medium: l(460.0),
            width_large: l(560.0),
            max_height_fraction: 0.8,
            max_height: l(640.0),
            padding: l(20.0),
            section_gap: l(16.0),
            action_gap: l(8.0),
            action_height: l(50.0),
            close_size: l(32.0),
        }
    }

    /// What a modal may actually be, given the window it is in: both
    /// [`Self::max_height_fraction`] and [`Self::max_height`] apply.
    pub fn height_ceiling(&self, viewport_height: Pixels) -> Pixels {
        (viewport_height * self.max_height_fraction).min(self.max_height)
    }
}

impl Default for ModalMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_widths_ascend() {
        let metrics = ModalMetrics::default();
        assert!(metrics.width_small < metrics.width_medium);
        assert!(metrics.width_medium < metrics.width_large);
    }

    /// A modal taller than its window has no outside to click to dismiss.
    #[test]
    fn a_modal_never_fills_its_window() {
        let metrics = ModalMetrics::default();
        for height in [px(400.0), px(800.0), px(1440.0), px(2160.0)] {
            assert!(
                metrics.height_ceiling(height) < height,
                "a {height:?} window left no scrim"
            );
        }
    }

    #[test]
    fn the_absolute_ceiling_wins_on_a_tall_display() {
        let metrics = ModalMetrics::default();
        assert_eq!(metrics.height_ceiling(px(4000.0)), metrics.max_height);
    }

    #[test]
    fn every_density_keeps_the_relationships() {
        for density in [Density::COMPACT, Density::NORMAL, Density::COMFORTABLE] {
            let metrics = ModalMetrics::new(density);
            assert!(metrics.width_small < metrics.width_large);
            assert!(metrics.padding > metrics.action_gap);
            assert!(metrics.height_ceiling(px(900.0)) < px(900.0));
        }
    }
}
