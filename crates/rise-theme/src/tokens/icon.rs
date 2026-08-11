use gpui::Pixels;

use crate::tokens::density::Density;

/// The sizes an icon is allowed to be: three steps, keyed to the type they
/// accompany — caption, body and title.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct IconMetrics {
    pub small: Pixels,
    pub regular: Pixels,
    pub large: Pixels,
}

impl IconMetrics {
    pub fn new(density: Density) -> Self {
        Self {
            small: density.scale(14.0),
            regular: density.scale(18.0),
            large: density.scale(22.0),
        }
    }
}

impl Default for IconMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_steps_are_ordered() {
        let metrics = IconMetrics::default();
        assert!(metrics.small < metrics.regular);
        assert!(metrics.regular < metrics.large);
    }

    #[test]
    fn density_scales_icons_with_everything_else() {
        assert_eq!(
            IconMetrics::new(Density::new(1.25)).regular,
            px(18.0 * 1.25)
        );
    }
}
