use gpui::{Pixels, px};

/// The length multiplier, applied once at the token layer so no component
/// carries its own idea of how big things are.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Density(f32);

impl Density {
    pub const COMPACT: Self = Self(0.9);
    pub const NORMAL: Self = Self(1.0);
    pub const COMFORTABLE: Self = Self(1.15);

    pub fn new(multiplier: f32) -> Self {
        Self(multiplier.clamp(0.75, 1.5))
    }

    pub fn multiplier(self) -> f32 {
        self.0
    }

    pub fn scale(self, value: f32) -> Pixels {
        px(value * self.0)
    }
}

impl Default for Density {
    fn default() -> Self {
        Self::NORMAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_density_is_the_ios_reference_scale() {
        assert_eq!(Density::default().scale(45.0), px(45.0));
    }

    #[test]
    fn extreme_multipliers_are_clamped_to_a_usable_range() {
        assert_eq!(Density::new(1000.0).multiplier(), 1.5);
        assert_eq!(Density::new(0.0).multiplier(), 0.75);
    }
}
