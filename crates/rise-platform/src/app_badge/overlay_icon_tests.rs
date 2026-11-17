use super::app_badge::badge_text;
use super::overlay_icon::*;

fn ink(pixels: &[u32]) -> usize {
    pixels.iter().filter(|pixel| **pixel == 0xFFFF_FFFF).count()
}

#[test]
fn the_picture_fills_the_requested_square_with_clear_corners() {
    let pixels = overlay_icon_pixels("7");
    assert_eq!(pixels.len(), OVERLAY_ICON_SIZE * OVERLAY_ICON_SIZE);
    assert_eq!(pixels[0], 0, "the corner is cut");
    assert_eq!(pixels[OVERLAY_ICON_SIZE * OVERLAY_ICON_SIZE - 1], 0);
    assert_eq!(pixels[OVERLAY_ICON_SIZE * 8], 0xFFFF_3B30, "the edge's middle is filled");
}

#[test]
fn every_badge_text_is_drawn_without_losing_a_pixel_at_the_edge() {
    let widest = badge_text(u32::MAX).expect("over the cap reads 999+");
    assert_eq!(ink(&overlay_icon_pixels(&widest)), 12 * 3 + 5);
    assert_eq!(ink(&overlay_icon_pixels("88")), 13 * 2 * 4);
}

#[test]
fn one_digit_is_drawn_centred() {
    let pixels = overlay_icon_pixels("8");
    let size = OVERLAY_ICON_SIZE;
    let lit_columns: Vec<usize> = (0..size)
        .filter(|column| (0..size).any(|row| pixels[row * size + column] == 0xFFFF_FFFF))
        .collect();
    assert_eq!(lit_columns.first(), Some(&5));
    assert_eq!(lit_columns.last(), Some(&10));
}

#[test]
fn a_character_without_a_glyph_is_left_out() {
    assert_eq!(ink(&overlay_icon_pixels("x")), 0);
}
