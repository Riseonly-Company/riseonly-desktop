use gpui::Pixels;

use crate::tokens::density::Density;

/// Padding and gap values. Use these rather than gpui's `.gap_4()`/`.px_5()`,
/// which bake Tailwind's scale and do not follow density.
#[derive(Clone, Copy, PartialEq, Debug)]
#[allow(non_snake_case)]
pub struct SpacingValues {
    pub _100: Pixels,
    pub _200: Pixels,
    pub _300: Pixels,
    pub _400: Pixels,
    pub _500: Pixels,
    pub _600: Pixels,
    pub _700: Pixels,
    pub _800: Pixels,
    pub _900: Pixels,
}

impl SpacingValues {
    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            _100: l(2.0),
            _200: l(4.0),
            _300: l(6.0),
            _400: l(8.0),
            _500: l(10.0),
            _600: l(12.0),
            _700: l(14.0),
            _800: l(16.0),
            _900: l(24.0),
        }
    }
}

impl Default for SpacingValues {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn the_scale_is_the_reference_distribution_at_default_density() {
        let spacing = SpacingValues::default();
        assert_eq!(spacing._100, px(2.0));
        assert_eq!(spacing._400, px(8.0));
        assert_eq!(spacing._800, px(16.0));
        assert_eq!(spacing._900, px(24.0));
    }

    #[test]
    fn the_scale_is_strictly_ascending() {
        let s = SpacingValues::default();
        let steps = [
            s._100, s._200, s._300, s._400, s._500, s._600, s._700, s._800, s._900,
        ];
        for pair in steps.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} is not below {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn density_scales_every_step() {
        let dense = SpacingValues::new(Density::new(1.25));
        assert_eq!(dense._800, px(16.0 * 1.25));
    }
}
