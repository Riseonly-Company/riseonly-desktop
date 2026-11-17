use super::glyph_canvas::*;

fn placement(left: i32, top: i32, width: u32, height: u32) -> GlyphPlacement {
    GlyphPlacement {
        left,
        top,
        width,
        height,
    }
}

#[test]
fn premultiply_rounds_to_nearest() {
    assert_eq!(premultiply(255, 255), 255);
    assert_eq!(premultiply(255, 0), 0);
    assert_eq!(premultiply(200, 128), 100);
    assert_eq!(premultiply(1, 128), 1);
}

#[test]
fn straight_pixel_is_premultiplied_on_an_empty_canvas() {
    let mut canvas = vec![0u8; 4];
    let glyph = [200u8, 100, 50, 128];
    assert!(composite_glyph(&mut canvas, 1, 1, placement(0, 0, 1, 1), GlyphPixels::Straight(&glyph)));
    assert_eq!(canvas, vec![100, 50, 25, 128]);
}

#[test]
fn premultiplied_pixel_lands_unchanged_on_an_empty_canvas() {
    let mut canvas = vec![0u8; 4];
    let glyph = [100u8, 50, 25, 128];
    assert!(composite_glyph(
        &mut canvas,
        1,
        1,
        placement(0, 0, 1, 1),
        GlyphPixels::Premultiplied(&glyph)
    ));
    assert_eq!(canvas, vec![100, 50, 25, 128]);
}

#[test]
fn coverage_draws_black_with_that_alpha() {
    let mut canvas = vec![0u8; 8];
    let glyph = [255u8, 64];
    assert!(composite_glyph(&mut canvas, 2, 1, placement(0, 0, 2, 1), GlyphPixels::Coverage(&glyph)));
    assert_eq!(canvas, vec![0, 0, 0, 255, 0, 0, 0, 64]);
}

#[test]
fn opaque_source_replaces_the_target() {
    let mut canvas = vec![10u8, 20, 30, 255];
    let glyph = [1u8, 2, 3, 255];
    assert!(composite_glyph(&mut canvas, 1, 1, placement(0, 0, 1, 1), GlyphPixels::Straight(&glyph)));
    assert_eq!(canvas, vec![1, 2, 3, 255]);
}

#[test]
fn half_source_over_opaque_target_mixes() {
    let mut canvas = vec![0u8, 0, 200, 255];
    let glyph = [128u8, 0, 0, 128];
    assert!(composite_glyph(
        &mut canvas,
        1,
        1,
        placement(0, 0, 1, 1),
        GlyphPixels::Premultiplied(&glyph)
    ));
    assert_eq!(canvas, vec![128, 0, 100, 255]);
}

#[test]
fn pixels_outside_the_canvas_are_clipped() {
    let mut canvas = vec![0u8; 4 * 4];
    let glyph = [255u8; 9];
    assert!(composite_glyph(&mut canvas, 2, 2, placement(-1, -1, 3, 3), GlyphPixels::Coverage(&glyph)));
    for pixel in canvas.chunks_exact(4) {
        assert_eq!(pixel, [0, 0, 0, 255]);
    }
    let mut far = vec![0u8; 4];
    assert!(composite_glyph(&mut far, 1, 1, placement(5, 5, 1, 1), GlyphPixels::Coverage(&[255])));
    assert_eq!(far, vec![0, 0, 0, 0]);
}

#[test]
fn short_buffers_draw_nothing() {
    let mut canvas = vec![0u8; 4];
    assert!(!composite_glyph(&mut canvas, 1, 1, placement(0, 0, 2, 1), GlyphPixels::Straight(&[255; 4])));
    assert!(!composite_glyph(&mut canvas, 2, 1, placement(0, 0, 1, 1), GlyphPixels::Coverage(&[255])));
    assert_eq!(canvas, vec![0, 0, 0, 0]);
}
