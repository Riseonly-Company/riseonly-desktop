use gpui::Pixels;

use crate::tokens::density::Density;

/// the resizable columns of the frame, the right panel with its width range and collapse
/// and the list column beside a split centre which never collapses
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PanelMetrics {
    pub default_width: Pixels,
    pub min_width: Pixels,
    pub max_width: Pixels,
    pub grip_width: Pixels,
    pub list_min_width: Pixels,
    pub list_max_width: Pixels,
    pub collapse_slack: Pixels,
    pub swipe_back_distance: Pixels,
    pub swipe_back_velocity: f32,
}

impl PanelMetrics {
    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            default_width: l(375.0),
            min_width: l(300.0),
            max_width: l(960.0),
            grip_width: l(6.0),
            list_min_width: l(280.0),
            list_max_width: l(600.0),
            collapse_slack: l(24.0),
            swipe_back_distance: l(64.0),
            swipe_back_velocity: 520.0,
        }
    }

    /// width below which letting go closes the panel, below the minimum it goes
    pub fn dismiss_width(&self) -> Pixels {
        self.min_width - self.collapse_slack
    }
}

impl Default for PanelMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_default_width_sits_inside_the_range_it_can_be_dragged_through() {
        for multiplier in [0.85, 1.0, 1.25] {
            let panel = PanelMetrics::new(Density::new(multiplier));
            assert!(panel.min_width < panel.default_width);
            assert!(panel.default_width < panel.max_width);
        }
    }

    #[test]
    fn dismissing_takes_a_drag_past_the_floor() {
        let panel = PanelMetrics::default();
        assert_eq!(panel.dismiss_width(), px(276.0));
        assert!(
            panel.dismiss_width() < panel.min_width,
            "the dismiss threshold has to be BELOW the resize floor, or the panel \
             would close every time somebody dragged it to its narrowest"
        );
    }

    #[test]
    fn density_scales_the_panel_and_its_threshold_together() {
        let dense = PanelMetrics::new(Density::new(1.25));
        assert_eq!(dense.default_width, px(468.75));
        assert_eq!(dense.dismiss_width(), px(345.0));
    }

    #[test]
    fn the_list_range_holds_the_list_default_width() {
        for multiplier in [0.85, 1.0, 1.25] {
            let density = Density::new(multiplier);
            let panel = PanelMetrics::new(density);
            let list = crate::tokens::shell::ShellMetrics::new(density).sidebar_width;
            assert!(panel.list_min_width <= list && list <= panel.list_max_width);
        }
    }
}
