use gpui::{App, Div, canvas, div, prelude::*};
use rise_platform::materials::{GlassRegion, Material, MaterialBacking};
use rise_ui::theme;

use crate::glass_panel::glass_host::{GlassHost, region_rect};

/// themed surface whose material the platform resolves, native backing draws nothing itself
pub struct GlassPanel;

impl GlassPanel {
    /// key names the surface and must stay the same string for its whole lifetime
    pub fn surface(key: &'static str, material: Material, cx: &mut App) -> Div {
        let radius = Self::corner_radius(theme(cx), material);
        Self::surface_rounded(key, material, radius, cx)
    }

    /// same surface at a chosen radius since gpui cant clip a native region
    /// chrome and panel have radius zero so a caller wanting rounded passes one
    pub fn surface_rounded(
        key: &'static str,
        material: Material,
        radius: f32,
        cx: &mut App,
    ) -> Div {
        match GlassHost::backing(material, cx) {
            MaterialBacking::Painted => Self::painted(material, cx).rounded(gpui::px(radius)),
            _ => Self::native(key, material, radius, cx),
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

    /// radius a native region carries in logical pixels, the painted form own
    pub fn corner_radius(theme: &rise_theme::AppTheme, material: Material) -> f32 {
        f32::from(theme.painted_material(material).corner_radius)
    }

    fn native(key: &'static str, material: Material, corner_radius: f32, cx: &mut App) -> Div {
        let id = GlassHost::region_id(key, cx);

        div().relative().child(
            canvas(
                move |bounds, _, cx: &mut App| {
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
