use gpui::Pixels;

use crate::tokens::density::Density;

/// modal dimensions, always narrower and shorter than the window so the scrim shows on all sides
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ModalMetrics {
    pub width_small: Pixels,
    pub width_medium: Pixels,
    pub width_large: Pixels,

    pub max_height_fraction: f32,
    pub max_height: Pixels,

    pub padding: Pixels,
    pub section_gap: Pixels,
    pub action_gap: Pixels,
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

    /// height ceiling for a viewport, both max_height_fraction and max_height apply
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
