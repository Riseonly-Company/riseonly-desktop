use gpui::{Hsla, TextAlign, black, blue, green, hsla, red, white, yellow};

use crate::main_text::{
    MainText, TextAlignT, line_spacing, margin_edges, monospaced_digit_features, text_size,
};
use crate::secondary_text::SecondaryText;
use crate::swiftui::environment::LayoutDirection;
use crate::swiftui::stacks::Alignment;
use crate::swiftui::text::TextAlignment;

const PRIMARY_100: Hsla = red();
const TEXT_PRIMARY: Hsla = black();
const TEXT_SECONDARY: Hsla = blue();
const FILL_INK: Hsla = yellow();

fn resolve(text: &MainText, fill: Option<Hsla>) -> Hsla {
    text.resolved_color(PRIMARY_100, TEXT_PRIMARY, TEXT_SECONDARY, fill)
}

#[test]
fn color_defaults_to_theme_text_primary() {
    assert_eq!(resolve(&MainText::new("a"), None), TEXT_PRIMARY);
}

#[test]
fn primary_wins_over_secondary_colour_and_fill() {
    let text = MainText::new("a")
        .primary(true)
        .secondary(true)
        .color(green());
    assert_eq!(resolve(&text, Some(FILL_INK)), PRIMARY_100);
}

#[test]
fn secondary_wins_over_colour_and_fill() {
    let text = MainText::new("a").secondary(true).color(green());
    assert_eq!(resolve(&text, Some(FILL_INK)), TEXT_SECONDARY);
}

#[test]
fn fill_ink_replaces_missing_colour() {
    assert_eq!(resolve(&MainText::new("a"), Some(FILL_INK)), FILL_INK);
}

#[test]
fn fill_ink_replaces_plain_white() {
    let text = MainText::new("a").color(white());
    assert_eq!(resolve(&text, Some(FILL_INK)), FILL_INK);
}

#[test]
fn own_colour_survives_on_a_fill() {
    let text = MainText::new("a").color(green());
    assert_eq!(resolve(&text, Some(FILL_INK)), green());
}

#[test]
fn own_colour_without_fill() {
    let text = MainText::new("a").color(green());
    assert_eq!(resolve(&text, None), green());
}

#[test]
fn white_without_fill_stays_white() {
    let text = MainText::new("a").color(white());
    assert_eq!(resolve(&text, None), white());
}

#[test]
fn text_delta_is_added_to_px() {
    assert_eq!(text_size(16.0, 2.5, false), 18.5);
    assert_eq!(text_size(16.0, -3.0, false), 13.0);
}

#[test]
fn ignores_text_size_delta_keeps_px() {
    assert_eq!(text_size(16.0, 2.5, true), 16.0);
}

#[test]
fn text_size_never_drops_below_one() {
    assert_eq!(text_size(2.0, -3.0, false), 1.0);
    assert_eq!(text_size(0.0, 0.0, true), 1.0);
}

#[test]
fn line_height_becomes_line_spacing() {
    assert_eq!(line_spacing(None, 16.0), 0.0);
    assert_eq!(line_spacing(Some(22.0), 16.0), 6.0);
    assert_eq!(line_spacing(Some(12.0), 16.0), 0.0);
}

#[test]
fn number_of_lines_zero_leaves_the_limit_to_the_caller() {
    assert_eq!(MainText::new("a").effective_line_limit(), None);
    assert_eq!(MainText::new("a").line_limit(1).effective_line_limit(), Some(1));
}

#[test]
fn number_of_lines_wins_over_outer_line_limit() {
    let text = MainText::new("a").number_of_lines(3).line_limit(1);
    assert_eq!(text.effective_line_limit(), Some(3));
}

#[test]
fn text_align_t_maps_like_ios() {
    assert_eq!(TextAlignT::Auto.text_alignment(), TextAlignment::Leading);
    assert_eq!(TextAlignT::Left.text_alignment(), TextAlignment::Leading);
    assert_eq!(TextAlignT::Center.text_alignment(), TextAlignment::Center);
    assert_eq!(TextAlignT::Right.text_alignment(), TextAlignment::Trailing);
    assert_eq!(TextAlignT::Auto.frame_alignment(), Alignment::Leading);
    assert_eq!(TextAlignT::Left.frame_alignment(), Alignment::Leading);
    assert_eq!(TextAlignT::Center.frame_alignment(), Alignment::Center);
    assert_eq!(TextAlignT::Right.frame_alignment(), Alignment::Trailing);
}

#[test]
fn leading_and_trailing_follow_the_writing_direction() {
    let ltr = LayoutDirection::LeftToRight;
    let rtl = LayoutDirection::RightToLeft;
    assert_eq!(TextAlignment::Leading.resolve(ltr), TextAlign::Left);
    assert_eq!(TextAlignment::Leading.resolve(rtl), TextAlign::Right);
    assert_eq!(TextAlignment::Trailing.resolve(ltr), TextAlign::Right);
    assert_eq!(TextAlignment::Trailing.resolve(rtl), TextAlign::Left);
    assert_eq!(TextAlignment::Center.resolve(rtl), TextAlign::Center);
}

#[test]
fn leading_margin_swaps_side_right_to_left() {
    let ltr = margin_edges(1.0, 2.0, 3.0, 4.0, LayoutDirection::LeftToRight);
    assert_eq!((ltr.top, ltr.left, ltr.right, ltr.bottom), (1.0, 2.0, 3.0, 4.0));
    let rtl = margin_edges(1.0, 2.0, 3.0, 4.0, LayoutDirection::RightToLeft);
    assert_eq!((rtl.top, rtl.left, rtl.right, rtl.bottom), (1.0, 3.0, 2.0, 4.0));
    assert!(margin_edges(0.0, 0.0, 0.0, 0.0, LayoutDirection::LeftToRight).is_zero());
}

#[test]
fn monospaced_digits_turn_on_tabular_figures() {
    let features = monospaced_digit_features();
    assert_eq!(features.tag_value_list(), &[("tnum".to_string(), 1)]);
}

#[test]
fn secondary_text_defaults_to_theme_text_secondary() {
    assert_eq!(SecondaryText::new("a").resolved_color(TEXT_SECONDARY), TEXT_SECONDARY);
    let tinted = SecondaryText::new("a").color(hsla(0.5, 0.5, 0.5, 1.0));
    assert_eq!(tinted.resolved_color(TEXT_SECONDARY), hsla(0.5, 0.5, 0.5, 1.0));
}
