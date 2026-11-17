use gpui::TestAppContext;
use rise_theme::RiseThemeAppearance;

use crate::swiftui::environment_values::{
    ColorScheme, accessibility_reduce_transparency, color_scheme, is_enabled,
    set_accessibility_reduce_transparency,
};

#[test]
fn color_scheme_follows_the_theme_appearance() {
    assert_eq!(ColorScheme::from_appearance(RiseThemeAppearance::Light), ColorScheme::Light);
    assert_eq!(ColorScheme::from_appearance(RiseThemeAppearance::Dark), ColorScheme::Dark);
}

#[gpui::test]
fn defaults_match_swiftui_outside_any_scope(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert!(is_enabled(cx));
        assert!(!accessibility_reduce_transparency(cx));
        assert_eq!(color_scheme(cx), ColorScheme::Light);
    });
}

#[gpui::test]
fn reduce_transparency_reads_what_the_shell_published(cx: &mut TestAppContext) {
    cx.update(|cx| {
        set_accessibility_reduce_transparency(true, cx);
        assert!(accessibility_reduce_transparency(cx));
        set_accessibility_reduce_transparency(false, cx);
        assert!(!accessibility_reduce_transparency(cx));
    });
}
