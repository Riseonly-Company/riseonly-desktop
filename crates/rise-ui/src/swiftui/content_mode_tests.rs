use gpui::ObjectFit;

use super::content_mode::ContentMode;

#[test]
fn fill_covers_and_fit_contains_as_swiftui_aspect_ratio_does() {
    assert!(matches!(ContentMode::Fill.object_fit(), ObjectFit::Cover));
    assert!(matches!(ContentMode::Fit.object_fit(), ObjectFit::Contain));
}

#[test]
fn the_default_is_fill_as_clever_image_ui_declares() {
    assert_eq!(ContentMode::default(), ContentMode::Fill);
}
