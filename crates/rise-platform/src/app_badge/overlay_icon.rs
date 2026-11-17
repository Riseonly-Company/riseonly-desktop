pub const OVERLAY_ICON_SIZE: usize = 16;

const FILL: u32 = 0xFFFF_3B30;
const INK: u32 = 0xFFFF_FFFF;
const CLEAR: u32 = 0;

const CORNER_RADIUS: usize = 3;
const GLYPH_WIDTH: usize = 3;
const GLYPH_HEIGHT: usize = 5;

/// rows of a glyph, the top bit of the low three is the leftmost pixel
const fn glyph(character: char) -> Option<[u8; GLYPH_HEIGHT]> {
    match character {
        '0' => Some([0b111, 0b101, 0b101, 0b101, 0b111]),
        '1' => Some([0b010, 0b110, 0b010, 0b010, 0b111]),
        '2' => Some([0b111, 0b001, 0b111, 0b100, 0b111]),
        '3' => Some([0b111, 0b001, 0b111, 0b001, 0b111]),
        '4' => Some([0b101, 0b101, 0b111, 0b001, 0b001]),
        '5' => Some([0b111, 0b100, 0b111, 0b001, 0b111]),
        '6' => Some([0b111, 0b100, 0b111, 0b101, 0b111]),
        '7' => Some([0b111, 0b001, 0b010, 0b010, 0b010]),
        '8' => Some([0b111, 0b101, 0b111, 0b101, 0b111]),
        '9' => Some([0b111, 0b101, 0b111, 0b001, 0b111]),
        '+' => Some([0b000, 0b010, 0b111, 0b010, 0b000]),
        _ => None,
    }
}

fn outside_rounded_square(x: usize, y: usize) -> bool {
    let last = OVERLAY_ICON_SIZE - 1;
    let (dx, dy) = (x.min(last - x), y.min(last - y));
    if dx >= CORNER_RADIUS || dy >= CORNER_RADIUS {
        return false;
    }
    let (rx, ry) = (CORNER_RADIUS - dx, CORNER_RADIUS - dy);
    rx * rx + ry * ry > CORNER_RADIUS * CORNER_RADIUS
}

/// top down rows of 0xAARRGGBB, icon size squared
/// one or two chars are drawn double size, 999+ at single size still fits
/// a char without a glyph is skipped
pub fn overlay_icon_pixels(text: &str) -> Vec<u32> {
    let size = OVERLAY_ICON_SIZE;
    let mut pixels: Vec<u32> = (0..size * size)
        .map(|index| if outside_rounded_square(index % size, index / size) { CLEAR } else { FILL })
        .collect();

    let glyphs: Vec<[u8; GLYPH_HEIGHT]> = text.chars().filter_map(glyph).collect();
    let scale = if glyphs.len() <= 2 { 2 } else { 1 };
    let width = (glyphs.len() * (GLYPH_WIDTH + 1)).saturating_sub(1) * scale;
    let left = size.saturating_sub(width) / 2;
    let top = (size - GLYPH_HEIGHT * scale) / 2;

    for (position, rows) in glyphs.iter().enumerate() {
        let glyph_left = left + position * (GLYPH_WIDTH + 1) * scale;
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..GLYPH_WIDTH {
                if bits & (1 << (GLYPH_WIDTH - 1 - column)) == 0 {
                    continue;
                }
                for y in 0..scale {
                    for x in 0..scale {
                        let (px, py) = (glyph_left + column * scale + x, top + row * scale + y);
                        if px < size && py < size {
                            pixels[py * size + px] = INK;
                        }
                    }
                }
            }
        }
    }
    pixels
}
