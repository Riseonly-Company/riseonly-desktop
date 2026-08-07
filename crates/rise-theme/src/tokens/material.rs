use gpui::{Hsla, Pixels, hsla};

use crate::tokens::density::Density;
use crate::tokens::palette::ThemePalette;
use crate::{Appearance, alpha, color};

pub use rise_platform::materials::{Material, MaterialBacking};

/// What a material looks like when nothing native is drawing it.
///
/// Tier 2 of `docs/PLATFORM_GLASS.md`: macOS 13 through 25, and Linux and
/// Windows always. A translucent fill, a hairline border and a soft inner
/// highlight, so the surface reads as a deliberate flat design rather than a
/// broken attempt at glass.
///
/// The alpha matters even where the window behind is opaque. With
/// `WindowMaterial::Blurred` granted the fill sits over a real system material;
/// without it the same fill sits over `bg._000` and simply reads as a lifted
/// panel. One set of numbers covers both, which is why there is no second
/// "opaque window" variant to keep in sync.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PaintedMaterial {
    pub fill: Hsla,
    pub border: Hsla,
    pub highlight: Hsla,
    pub corner_radius: Pixels,
}

/// The painted form of every [`Material`], resolved for one appearance and one
/// density.
///
/// A component never reads this directly and never asks for glass. It asks for a
/// [`Material`]; `rise-platform` decides whether that material is an AppKit view
/// or paint, and only the painted branch reaches these tokens.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MaterialTokens {
    pub chrome: PaintedMaterial,
    pub panel: PaintedMaterial,
    pub overlay: PaintedMaterial,
    /// What dims the window behind a modal overlay.
    ///
    /// Not a [`PaintedMaterial`]: a scrim has no border, no highlight and no
    /// radius, and it is never a native material — an AppKit view behind a hole
    /// in the Metal layer cannot darken what gpui drew on top of it.
    pub scrim: Hsla,
}

impl MaterialTokens {
    /// Chrome and panel span a window edge, so they are square: rounding a
    /// surface that runs into the window corner leaves a wedge of desktop
    /// showing through. Only the overlay floats, and only the overlay is round.
    pub fn new(palette: &ThemePalette, appearance: Appearance, density: Density) -> Self {
        let highlight = match appearance {
            Appearance::Dark => hsla(0.0, 0.0, 1.0, 0.05),
            Appearance::Light => hsla(0.0, 0.0, 1.0, 0.55),
        };

        Self {
            chrome: PaintedMaterial {
                fill: alpha(color(&palette.bg_100), 0.80),
                border: color(&palette.border_100),
                highlight,
                corner_radius: Pixels::ZERO,
            },
            panel: PaintedMaterial {
                fill: alpha(color(&palette.bg_200), 0.72),
                border: color(&palette.border_100),
                highlight,
                corner_radius: Pixels::ZERO,
            },
            overlay: PaintedMaterial {
                fill: alpha(color(&palette.bg_300), 0.92),
                border: color(&palette.border_200),
                highlight,
                corner_radius: density.scale(palette.radius_300),
            },
            scrim: match appearance {
                Appearance::Dark => alpha(color(&palette.bg_000), 0.55),
                Appearance::Light => alpha(color(&palette.bg_700), 0.25),
            },
        }
    }

    pub fn get(&self, material: Material) -> PaintedMaterial {
        match material {
            Material::Chrome => self.chrome,
            Material::Panel => self.panel,
            Material::Overlay => self.overlay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    fn tokens(appearance: Appearance) -> MaterialTokens {
        let palette = match appearance {
            Appearance::Dark => ThemePalette::default_dark(),
            Appearance::Light => ThemePalette::default_light(),
        };
        MaterialTokens::new(&palette, appearance, Density::NORMAL)
    }

    #[test]
    fn every_material_has_a_painted_form_at_both_appearances() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            let tokens = tokens(appearance);
            for material in Material::ALL {
                let painted = tokens.get(material);
                assert!(
                    painted.fill.a > 0.0,
                    "{material:?} at {appearance:?} would paint nothing"
                );
            }
        }
    }

    #[test]
    fn a_painted_material_is_translucent_so_the_window_material_can_show_through() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            let tokens = tokens(appearance);
            for material in Material::ALL {
                let painted = tokens.get(material);
                assert!(
                    painted.fill.a < 1.0,
                    "{material:?} at {appearance:?} is opaque, which hides the tier-0 blur"
                );
            }
        }
    }

    #[test]
    fn the_three_materials_are_visually_distinct() {
        let tokens = tokens(Appearance::Dark);
        assert_ne!(tokens.chrome.fill, tokens.panel.fill);
        assert_ne!(tokens.panel.fill, tokens.overlay.fill);
    }

    #[test]
    fn only_the_floating_material_is_rounded() {
        let tokens = tokens(Appearance::Dark);
        assert_eq!(tokens.chrome.corner_radius, Pixels::ZERO);
        assert_eq!(tokens.panel.corner_radius, Pixels::ZERO);
        assert_eq!(tokens.overlay.corner_radius, px(10.0));
    }

    #[test]
    fn density_scales_the_radius_and_leaves_the_colours_alone() {
        let palette = ThemePalette::default_dark();
        let normal = MaterialTokens::new(&palette, Appearance::Dark, Density::NORMAL);
        let dense = MaterialTokens::new(&palette, Appearance::Dark, Density::new(1.25));

        assert_eq!(
            dense.overlay.corner_radius,
            px(f32::from(normal.overlay.corner_radius) * 1.25)
        );
        assert_eq!(normal.overlay.fill, dense.overlay.fill);
    }

    #[test]
    fn light_and_dark_do_not_share_a_fill() {
        assert_ne!(
            tokens(Appearance::Dark).panel.fill,
            tokens(Appearance::Light).panel.fill
        );
    }
}
