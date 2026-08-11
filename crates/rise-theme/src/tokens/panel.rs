use gpui::Pixels;

use crate::tokens::density::Density;

/// The resizable side panel: its width range and its dismiss gesture.
///
/// Dismissal must stay a deliberate, over-half drag, or a user aiming at the
/// resize grip loses their place every time they overshoot.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PanelMetrics {
    /// What the panel opens at.
    pub default_width: Pixels,
    /// The narrowest the user may drag it before letting go still restores it,
    /// and the widest it may grow.
    pub min_width: Pixels,
    pub max_width: Pixels,
    /// The grab strip along the panel's inner edge, wider than it looks.
    pub grip_width: Pixels,
    /// How far a two-finger horizontal swipe must travel before an inner page is
    /// popped, and how fast a shorter one has to be to count anyway.
    pub swipe_back_distance: Pixels,
    pub swipe_back_velocity: f32,

    /// Below this fraction of `default_width`, releasing closes the panel
    /// instead of snapping it back.
    pub dismiss_fraction: f32,
}

impl PanelMetrics {
    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            default_width: l(360.0),
            min_width: l(280.0),
            max_width: l(620.0),
            grip_width: l(6.0),
            swipe_back_distance: l(64.0),
            swipe_back_velocity: 520.0,
            dismiss_fraction: 0.5,
        }
    }

    /// The width below which letting go closes the panel.
    pub fn dismiss_width(&self) -> Pixels {
        self.default_width * self.dismiss_fraction
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
    fn dismissing_takes_a_deliberate_drag_past_half() {
        let panel = PanelMetrics::default();
        assert_eq!(panel.dismiss_width(), px(180.0));
        assert!(
            panel.dismiss_width() < panel.min_width,
            "the dismiss threshold has to be BELOW the resize floor, or the panel \
             would close every time somebody dragged it to its narrowest"
        );
    }

    #[test]
    fn density_scales_the_panel_and_its_threshold_together() {
        let dense = PanelMetrics::new(Density::new(1.25));
        assert_eq!(dense.default_width, px(450.0));
        assert_eq!(dense.dismiss_width(), px(225.0));
    }
}
