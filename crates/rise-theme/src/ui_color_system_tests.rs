use gpui::{Hsla, Rgba, rgb};

use crate::Appearance;
use crate::ui_color_system::UIColor;

fn expected(hex: u32, alpha: f32) -> Hsla {
    let mut value = rgb(hex);
    value.a = alpha;
    value.into()
}

fn assert_resolves(color: UIColor, light: (u32, f32), dark: (u32, f32)) {
    assert_same(color.resolved_color(Appearance::Light), expected(light.0, light.1));
    assert_same(color.resolved_color(Appearance::Dark), expected(dark.0, dark.1));
}

fn assert_same(actual: Hsla, expected: Hsla) {
    let actual: Rgba = actual.into();
    let expected: Rgba = expected.into();
    assert!((actual.r - expected.r).abs() < 1e-3, "{actual:?} != {expected:?}");
    assert!((actual.g - expected.g).abs() < 1e-3, "{actual:?} != {expected:?}");
    assert!((actual.b - expected.b).abs() < 1e-3, "{actual:?} != {expected:?}");
    assert!((actual.a - expected.a).abs() < 1e-3, "{actual:?} != {expected:?}");
}

#[test]
fn the_colours_the_reference_names_resolve_to_the_ios_values() {
    assert_resolves(UIColor::label(), (0x000000, 1.0), (0xFFFFFF, 1.0));
    assert_resolves(UIColor::secondary_label(), (0x3C3C43, 0.6), (0xEBEBF5, 0.6));
    assert_resolves(UIColor::secondary_system_fill(), (0x787880, 0.16), (0x787880, 0.32));
    assert_resolves(UIColor::system_background(), (0xFFFFFF, 1.0), (0x000000, 1.0));
    assert_resolves(UIColor::secondary_system_background(), (0xF2F2F7, 1.0), (0x1C1C1E, 1.0));
    assert_resolves(UIColor::separator(), (0x3C3C43, 0.29), (0x545458, 0.6));
    assert_resolves(UIColor::system_red(), (0xFF383C, 1.0), (0xFF4245, 1.0));
    assert_resolves(UIColor::system_blue(), (0x0088FF, 1.0), (0x0091FF, 1.0));
}

#[test]
fn every_label_level_is_fainter_than_the_one_above_it() {
    let levels = [
        UIColor::label(),
        UIColor::secondary_label(),
        UIColor::tertiary_label(),
        UIColor::quaternary_label(),
    ];
    for appearance in [Appearance::Light, Appearance::Dark] {
        for pair in levels.windows(2) {
            let upper = pair[0].resolved_color(appearance).a;
            let lower = pair[1].resolved_color(appearance).a;
            assert!(lower < upper, "{appearance:?}: {lower} !< {upper}");
        }
    }
}

#[test]
fn every_fill_level_is_fainter_than_the_one_above_it() {
    let levels = [
        UIColor::system_fill(),
        UIColor::secondary_system_fill(),
        UIColor::tertiary_system_fill(),
        UIColor::quaternary_system_fill(),
    ];
    for appearance in [Appearance::Light, Appearance::Dark] {
        for pair in levels.windows(2) {
            let upper = pair[0].resolved_color(appearance).a;
            let lower = pair[1].resolved_color(appearance).a;
            assert!(lower < upper, "{appearance:?}: {lower} !< {upper}");
        }
    }
}

#[test]
fn backgrounds_are_opaque_in_both_appearances() {
    let backgrounds = [
        UIColor::system_background(),
        UIColor::secondary_system_background(),
        UIColor::tertiary_system_background(),
        UIColor::system_grouped_background(),
        UIColor::secondary_system_grouped_background(),
        UIColor::tertiary_system_grouped_background(),
        UIColor::opaque_separator(),
    ];
    for color in backgrounds {
        for appearance in [Appearance::Light, Appearance::Dark] {
            assert_same(
                color.resolved_color(appearance),
                Hsla { a: 1.0, ..color.resolved_color(appearance) },
            );
        }
    }
}

#[test]
fn the_grouped_backgrounds_swap_the_plain_ones_in_light_mode() {
    assert_eq!(
        UIColor::system_grouped_background().resolved_color(Appearance::Light),
        UIColor::secondary_system_background().resolved_color(Appearance::Light),
    );
    assert_eq!(
        UIColor::secondary_system_grouped_background().resolved_color(Appearance::Light),
        UIColor::system_background().resolved_color(Appearance::Light),
    );
}

#[test]
fn the_gray_ramp_inverts_between_appearances() {
    assert_resolves(UIColor::system_gray(), (0x8E8E93, 1.0), (0x8E8E93, 1.0));
    assert_resolves(UIColor::system_gray6(), (0xF2F2F7, 1.0), (0x1C1C1E, 1.0));
    let light_six = UIColor::system_gray6().resolved_color(Appearance::Light).l;
    let light_two = UIColor::system_gray2().resolved_color(Appearance::Light).l;
    let dark_six = UIColor::system_gray6().resolved_color(Appearance::Dark).l;
    let dark_two = UIColor::system_gray2().resolved_color(Appearance::Dark).l;
    assert!(light_six > light_two);
    assert!(dark_six < dark_two);
}
