use gpui::{Pixels, px};

use crate::input_ui::shaped_input::{logical_stops_from, wrap_starts_at};

fn mono_stops(text: &str) -> Vec<(usize, Pixels)> {
    (0..=text.len()).map(|index| (index, px(10.0 * index as f32))).collect()
}

#[test]
fn a_line_that_fits_is_not_wrapped() {
    let text = "one two";
    assert!(wrap_starts_at(text, &mono_stops(text), px(70.0)).is_empty());
}

#[test]
fn a_line_breaks_after_the_last_space_that_fits() {
    let text = "one two three";
    assert_eq!(wrap_starts_at(text, &mono_stops(text), px(75.0)), vec![8]);
}

#[test]
fn trailing_whitespace_hangs_past_the_edge() {
    let text = "abcd   efg";
    assert_eq!(wrap_starts_at(text, &mono_stops(text), px(40.0)), vec![7]);
}

#[test]
fn a_word_wider_than_the_row_breaks_between_characters() {
    let text = "abcdefghij";
    assert_eq!(wrap_starts_at(text, &mono_stops(text), px(40.0)), vec![4, 8]);
}

#[test]
fn a_long_word_after_a_space_moves_down_then_breaks() {
    let text = "ab cdefghij";
    assert_eq!(wrap_starts_at(text, &mono_stops(text), px(40.0)), vec![3, 7]);
}

#[test]
fn a_row_never_ends_empty_even_when_one_glyph_is_too_wide() {
    let text = "ab";
    assert_eq!(wrap_starts_at(text, &mono_stops(text), px(5.0)), vec![1]);
}

#[test]
fn logical_order_glyphs_have_no_logical_stops() {
    let glyphs = [(0, px(0.0)), (1, px(10.0)), (2, px(20.0))];
    assert!(logical_stops_from(&glyphs, 3, px(30.0)).is_none());
}

#[test]
fn right_to_left_glyphs_get_pen_stops_in_byte_order() {
    let glyphs = [(4, px(0.0)), (3, px(10.0)), (2, px(20.0)), (1, px(30.0)), (0, px(40.0))];
    let stops = logical_stops_from(&glyphs, 5, px(50.0)).unwrap_or_default();
    let expected: Vec<(usize, Pixels)> = (0..=5).map(|index| (index, px(10.0 * index as f32))).collect();
    assert_eq!(stops, expected);
}

#[test]
fn right_to_left_line_wraps_at_the_word_in_reading_order() {
    let text = "abc def";
    let glyphs: Vec<(usize, Pixels)> = (0..text.len()).rev().enumerate().map(|(at, index)| (index, px(10.0 * at as f32))).collect();
    let stops = logical_stops_from(&glyphs, text.len(), px(70.0)).unwrap_or_default();
    assert_eq!(wrap_starts_at(text, &stops, px(40.0)), vec![4]);
}
