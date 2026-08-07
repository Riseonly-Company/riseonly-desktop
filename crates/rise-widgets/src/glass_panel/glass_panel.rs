use gpui::{App, Div, canvas, div, prelude::*};
use rise_platform::materials::{GlassRegion, Material, MaterialBacking};
use rise_ui::theme;

use crate::glass_panel::glass_host::{GlassHost, region_rect};

/// A surface that asks the theme for a material and lets the platform decide
/// what that material is.
///
/// This is the only component allowed to know that native materials exist, and
/// even here it never names glass. `docs/PLATFORM_GLASS.md`: if a component
/// could ask for glass directly, every such call site would become a hole on
/// Linux and nobody would notice until a screenshot arrived.
///
/// The two branches are genuinely different drawings, not one drawing with a
/// flag:
///
/// - **Native.** An AppKit view sits *below* the Metal layer, so this element
///   must draw nothing at all in its own rectangle — a fill here would land on
///   top of the material and hide it. It only reports where it is.
/// - **Painted.** An ordinary element with a translucent fill, a hairline border
///   and no platform bookkeeping whatsoever.
pub struct GlassPanel;

impl GlassPanel {
    /// `key` names the surface — "rail", "sidebar", "composer" — and must be the
    /// same string for the lifetime of that surface, because the platform side
    /// moves the view it already has for an id it has seen before.
    pub fn surface(key: &'static str, material: Material, cx: &mut App) -> Div {
        let backing = GlassHost::backing(material, cx);

        match backing {
            MaterialBacking::Painted => Self::painted(material, cx),
            _ => Self::native(key, material, cx),
        }
    }

    fn painted(material: Material, cx: &App) -> Div {
        let painted = theme(cx).painted_material(material);

        div()
            .bg(painted.fill)
            .border_1()
            .border_color(painted.border)
            .rounded(painted.corner_radius)
    }

    /// The radius a native region carries, in logical pixels.
    ///
    /// It is the painted form's radius on purpose. A native backing cannot be
    /// clipped by GPUI drawing a rounded mask over it — the view is below the
    /// Metal layer, so the mask would land on top — so the radius has to travel
    /// with the region, and if the two tiers disagreed they would read as two
    /// different designs on two different machines.
    pub fn corner_radius(theme: &rise_theme::AppTheme, material: Material) -> f32 {
        f32::from(theme.painted_material(material).corner_radius)
    }

    fn native(key: &'static str, material: Material, cx: &mut App) -> Div {
        let id = GlassHost::region_id(key, cx);
        let corner_radius = Self::corner_radius(theme(cx), material);

        div().relative().child(
            canvas(
                move |bounds, _, cx: &mut App| {
                    // Prepaint, not paint: this is the layout pass, and a
                    // rectangle that moved during a render pass desynchronises
                    // from the content drawn around it.
                    let rect = region_rect(bounds);
                    if rect.has_area() {
                        GlassHost::record(
                            GlassRegion::new(id, material, rect).with_corner_radius(corner_radius),
                            cx,
                        );
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_theme::{AppTheme, Material as ThemeMaterial};

    #[test]
    fn the_painted_form_exists_for_every_material_at_both_appearances() {
        for theme in [AppTheme::dark(), AppTheme::light()] {
            for material in ThemeMaterial::ALL {
                let painted = theme.painted_material(material);
                assert!(painted.fill.a > 0.0);
                assert!(painted.fill.a < 1.0);
            }
        }
    }

    #[test]
    fn the_radius_a_native_region_carries_is_the_painted_one() {
        for theme in [AppTheme::dark(), AppTheme::light()] {
            for material in ThemeMaterial::ALL {
                assert_eq!(
                    GlassPanel::corner_radius(&theme, material),
                    f32::from(theme.painted_material(material).corner_radius),
                    "{material:?} would look like a different design on a machine \
                     that hosts it natively"
                );
            }
        }
    }

    #[test]
    fn the_surfaces_that_run_into_a_window_edge_are_square() {
        // Rounding a surface that meets the window corner leaves a wedge of
        // desktop showing through it.
        let theme = AppTheme::dark();
        assert_eq!(GlassPanel::corner_radius(&theme, Material::Chrome), 0.0);
        assert_eq!(GlassPanel::corner_radius(&theme, Material::Panel), 0.0);
        assert!(GlassPanel::corner_radius(&theme, Material::Overlay) > 0.0);
    }

    #[test]
    fn a_material_never_resolves_above_its_own_ceiling() {
        for material in Material::ALL {
            assert!(
                MaterialBacking::LiquidGlass
                    .capped_at(material.ceiling())
                    .tier()
                    <= material.ceiling().tier()
            );
        }
    }
}
