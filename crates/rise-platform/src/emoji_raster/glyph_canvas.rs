/// pixels of one glyph image, rows top to bottom with no padding
#[derive(Clone, Copy, Debug)]
pub enum GlyphPixels<'a> {
    Coverage(&'a [u8]),
    Straight(&'a [u8]),
    Premultiplied(&'a [u8]),
}

/// where a glyph image lands on the canvas, its top left pixel and its size
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GlyphPlacement {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

/// composites pixels at placement over the premultiplied rgba canvas, pixels outside are clipped
/// returns false and draws nothing when the glyph buffer or canvas is shorter than claimed
pub fn composite_glyph(
    canvas: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    placement: GlyphPlacement,
    pixels: GlyphPixels<'_>,
) -> bool {
    let canvas_len = u64::from(canvas_width) * u64::from(canvas_height) * 4;
    if (canvas.len() as u64) < canvas_len {
        return false;
    }
    let area = u64::from(placement.width) * u64::from(placement.height);
    let (data, channels) = match pixels {
        GlyphPixels::Coverage(data) => (data, 1u64),
        GlyphPixels::Straight(data) | GlyphPixels::Premultiplied(data) => (data, 4u64),
    };
    if (data.len() as u64) < area * channels {
        return false;
    }
    let canvas_width = i64::from(canvas_width);
    let canvas_height = i64::from(canvas_height);
    for row in 0..i64::from(placement.height) {
        let y = i64::from(placement.top) + row;
        if y < 0 || y >= canvas_height {
            continue;
        }
        for column in 0..i64::from(placement.width) {
            let x = i64::from(placement.left) + column;
            if x < 0 || x >= canvas_width {
                continue;
            }
            let source_index = (row * i64::from(placement.width) + column) as usize;
            let source = premultiplied_source(pixels, source_index);
            let target_index = ((y * canvas_width + x) * 4) as usize;
            blend_over(&mut canvas[target_index..target_index + 4], source);
        }
    }
    true
}

fn premultiplied_source(pixels: GlyphPixels<'_>, index: usize) -> [u8; 4] {
    match pixels {
        GlyphPixels::Coverage(data) => [0, 0, 0, data[index]],
        GlyphPixels::Premultiplied(data) => {
            let at = index * 4;
            [data[at], data[at + 1], data[at + 2], data[at + 3]]
        }
        GlyphPixels::Straight(data) => {
            let at = index * 4;
            let alpha = data[at + 3];
            [
                premultiply(data[at], alpha),
                premultiply(data[at + 1], alpha),
                premultiply(data[at + 2], alpha),
                alpha,
            ]
        }
    }
}

/// channel * alpha / 255, rounded
pub fn premultiply(channel: u8, alpha: u8) -> u8 {
    let product = u16::from(channel) * u16::from(alpha);
    ((product + 127) / 255) as u8
}

fn blend_over(target: &mut [u8], source: [u8; 4]) {
    let keep = 255 - u16::from(source[3]);
    for channel in 0..4 {
        let kept = (u16::from(target[channel]) * keep + 127) / 255;
        target[channel] = (u16::from(source[channel]) + kept).min(255) as u8;
    }
}
