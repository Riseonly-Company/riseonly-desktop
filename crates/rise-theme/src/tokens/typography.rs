use gpui::{Font, FontFeatures, FontStyle, FontWeight, Pixels, SharedString, px};

use crate::tokens::density::Density;

/// The family the product ships, and therefore the only family that measures the
/// same on all three platforms.
pub const FONT_FAMILY: &str = "Inter";

/// Every weight the bundle actually contains.
///
/// [`Typography::font`] snaps a requested weight to the nearest entry here; a
/// weight with no shipped face is substituted per-OS by the text stack.
pub const SHIPPED_WEIGHTS: [FontWeight; 6] = [
    FontWeight::LIGHT,
    FontWeight::NORMAL,
    FontWeight::MEDIUM,
    FontWeight::SEMIBOLD,
    FontWeight::BOLD,
    FontWeight::BLACK,
];

/// A size and weight already resolved for one density, ready to hand to gpui.
#[derive(Clone, PartialEq, Debug)]
pub struct TextStyleToken {
    pub size: Pixels,
    pub line_height: Pixels,
    pub font: Font,
}

/// Text tokens for one density.
///
/// Prefer the named steps; [`Typography::style`] is the only sanctioned way to
/// write a size down, because it applies density and pins the family.
#[derive(Clone, PartialEq, Debug)]
pub struct Typography {
    density: Density,
    family: SharedString,
}

impl Typography {
    const LINE_HEIGHT_RATIO: f32 = 1.3;

    pub fn new(density: Density) -> Self {
        Self {
            density,
            family: SharedString::new_static(FONT_FAMILY),
        }
    }

    pub fn family(&self) -> SharedString {
        self.family.clone()
    }

    /// A design size in points and a weight, scaled for this density.
    pub fn style(&self, size_pt: f32, weight: FontWeight) -> TextStyleToken {
        let size = self.density.scale(size_pt);

        TextStyleToken {
            size,
            line_height: px(f32::from(size) * Self::LINE_HEIGHT_RATIO),
            font: self.font(weight),
        }
    }

    pub fn font(&self, weight: FontWeight) -> Font {
        Font {
            family: self.family.clone(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight: Self::nearest_shipped_weight(weight),
            style: FontStyle::Normal,
        }
    }

    fn nearest_shipped_weight(requested: FontWeight) -> FontWeight {
        let mut best = SHIPPED_WEIGHTS[0];
        let mut best_distance = f32::INFINITY;

        for candidate in SHIPPED_WEIGHTS {
            let distance = (candidate.0 - requested.0).abs();
            if distance < best_distance {
                best = candidate;
                best_distance = distance;
            }
        }

        best
    }

    pub fn caption_small(&self) -> TextStyleToken {
        self.style(11.0, FontWeight::NORMAL)
    }

    pub fn caption(&self) -> TextStyleToken {
        self.style(12.0, FontWeight::SEMIBOLD)
    }

    pub fn footnote(&self) -> TextStyleToken {
        self.style(13.0, FontWeight::SEMIBOLD)
    }

    pub fn body(&self) -> TextStyleToken {
        self.style(15.0, FontWeight::NORMAL)
    }

    pub fn body_strong(&self) -> TextStyleToken {
        self.style(14.0, FontWeight::SEMIBOLD)
    }

    pub fn headline(&self) -> TextStyleToken {
        self.style(16.0, FontWeight::SEMIBOLD)
    }

    pub fn title(&self) -> TextStyleToken {
        self.style(18.0, FontWeight::SEMIBOLD)
    }

    pub fn large_title(&self) -> TextStyleToken {
        self.style(22.0, FontWeight::NORMAL)
    }

    /// The heading a full-window screen opens with — onboarding and the sign-in
    /// wizard.
    pub fn display(&self) -> TextStyleToken {
        self.style(32.0, FontWeight::BOLD)
    }

    pub fn display_small(&self) -> TextStyleToken {
        self.style(27.0, FontWeight::BOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_scales_type_but_never_changes_the_family() {
        let normal = Typography::new(Density::NORMAL);
        let comfortable = Typography::new(Density::COMFORTABLE);

        assert_eq!(normal.style(14.0, FontWeight::SEMIBOLD).size, px(14.0));
        assert_eq!(
            comfortable.style(14.0, FontWeight::SEMIBOLD).size,
            px(14.0 * Density::COMFORTABLE.multiplier())
        );
        assert_eq!(normal.family(), comfortable.family());
    }

    #[test]
    fn line_height_follows_the_scaled_size_not_the_design_size() {
        let comfortable = Typography::new(Density::COMFORTABLE);
        let token = comfortable.style(14.0, FontWeight::NORMAL);

        assert!(token.line_height > token.size);
        assert_eq!(token.line_height, px(f32::from(token.size) * 1.3));
    }

    #[test]
    fn an_unshipped_weight_snaps_to_a_face_the_bundle_actually_has() {
        let typography = Typography::new(Density::NORMAL);

        assert_eq!(
            typography.font(FontWeight::EXTRA_BOLD).weight,
            FontWeight::BOLD,
            "800 has no shipped face; substituting one costs a different family per OS"
        );
        assert_eq!(typography.font(FontWeight::THIN).weight, FontWeight::LIGHT);
        assert_eq!(
            typography.font(FontWeight::EXTRA_LIGHT).weight,
            FontWeight::LIGHT
        );
    }

    #[test]
    fn every_weight_the_reference_uses_is_shipped_unchanged() {
        let typography = Typography::new(Density::NORMAL);

        for weight in SHIPPED_WEIGHTS {
            assert_eq!(
                typography.font(weight).weight,
                weight,
                "{weight:?} is used by riseonly-ios and must not be snapped away"
            );
        }
    }

    #[test]
    fn the_named_steps_are_the_reference_sizes() {
        let typography = Typography::new(Density::NORMAL);

        assert_eq!(typography.body_strong().size, px(14.0));
        assert_eq!(typography.headline().size, px(16.0));
        assert_eq!(typography.large_title().size, px(22.0));
    }
}
