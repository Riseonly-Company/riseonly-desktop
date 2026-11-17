use rise_theme::RiseThemeAppearance;

use crate::liquid_chrome::{LiquidChrome, gradient_stroke_color};
use crate::swiftui::material::{Material, UIBlurEffectStyle};

#[test]
fn blur_style_follows_the_intensity_bands_of_ios() {
    let light = |t| LiquidChrome::nav_glass_ui_blur_effect_style(t, RiseThemeAppearance::Light);
    assert_eq!(light(0.), None);
    assert_eq!(light(-1.), None);
    assert_eq!(light(0.1), Some(UIBlurEffectStyle::SystemUltraThinMaterialLight));
    assert_eq!(light(0.2), Some(UIBlurEffectStyle::SystemThinMaterialLight));
    assert_eq!(light(0.5), Some(UIBlurEffectStyle::SystemMaterialLight));
    assert_eq!(light(0.79), Some(UIBlurEffectStyle::SystemThickMaterialLight));
    assert_eq!(light(1.), Some(UIBlurEffectStyle::SystemChromeMaterialLight));
    assert_eq!(light(4.), Some(UIBlurEffectStyle::SystemChromeMaterialLight));
    assert_eq!(
        LiquidChrome::nav_glass_ui_blur_effect_style(0.3, RiseThemeAppearance::Dark),
        Some(UIBlurEffectStyle::SystemThinMaterialDark)
    );
}

#[test]
fn nav_glass_material_matches_the_swiftui_bands() {
    assert_eq!(LiquidChrome::nav_glass_material(0.), Material::UltraThin);
    assert_eq!(LiquidChrome::nav_glass_material(0.3), Material::Thin);
    assert_eq!(LiquidChrome::nav_glass_material(0.6), Material::Thick);
    assert_eq!(LiquidChrome::nav_glass_material(LiquidChrome::DEFAULT_NAV_GLASS_BLUR_INTENSITY), Material::UltraThick);
}

#[test]
fn chrome_constants_are_the_ios_values() {
    assert_eq!(LiquidChrome::nav_glass_border_width(), 0.65);
    assert_eq!(LiquidChrome::material_darken_opacity(), 0.26);
    assert_eq!(LiquidChrome::bar_material(), Material::Thin);
}

#[test]
fn gradient_stroke_is_white_at_the_mean_opacity() {
    let color = gradient_stroke_color(0.14, 0.06);
    assert!((color.a - 0.10).abs() < 1e-6);
    assert_eq!(color.l, 1.);
}
