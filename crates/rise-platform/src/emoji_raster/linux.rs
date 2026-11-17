use std::sync::{Mutex, OnceLock};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight};
use swash::scale::Source;

use super::emoji_raster::*;
use super::glyph_canvas::{composite_glyph, GlyphPixels, GlyphPlacement};

pub(super) struct NativeEmojiRaster;

impl EmojiRasterSource for NativeEmojiRaster {
    fn render(&self, emoji: &str, point_size: f32) -> Result<EmojiRaster, EmojiRasterError> {
        self.render_weighted(emoji, point_size, TextWeight::Regular)
    }

    fn render_weighted(
        &self,
        text: &str,
        point_size: f32,
        weight: TextWeight,
    ) -> Result<EmojiRaster, EmojiRasterError> {
        let mut stack = text_stack().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let TextStack { fonts, glyphs } = &mut *stack;
        draw(fonts, glyphs, text, point_size, face_weight(weight))
    }
}

/// opentype weight class of a seam weight
/// fontdb matches the nearest face and cosmic-text sets the wght axis of a variable face
/// no semibold face or axis gives the nearest weight, never fake bold
fn face_weight(weight: TextWeight) -> Weight {
    match weight {
        TextWeight::Regular => Weight::NORMAL,
        TextWeight::Semibold => Weight::SEMIBOLD,
    }
}

/// font database and glyph cache, built once
/// loading the fontconfig set reads every system face header, way too slow per sticker
struct TextStack {
    fonts: FontSystem,
    glyphs: SwashCache,
}

fn text_stack() -> &'static Mutex<TextStack> {
    static STACK: OnceLock<Mutex<TextStack>> = OnceLock::new();
    STACK.get_or_init(|| {
        Mutex::new(TextStack {
            fonts: FontSystem::new(),
            glyphs: SwashCache::new(),
        })
    })
}

/// one glyph of the shaped string with the baseline of its line
struct PlacedGlyph {
    glyph: cosmic_text::LayoutGlyph,
    baseline: f32,
}

fn draw(
    fonts: &mut FontSystem,
    glyphs: &mut SwashCache,
    text: &str,
    point_size: f32,
    weight: Weight,
) -> Result<EmojiRaster, EmojiRasterError> {
    let mut buffer = Buffer::new(fonts, Metrics::new(point_size, point_size));
    buffer.set_size(None, None);
    buffer.set_text(text, &Attrs::new().family(Family::SansSerif).weight(weight), Shaping::Advanced, None);
    buffer.shape_until_scroll(fonts, false);

    let mut width = 0.0f32;
    let mut height = 0.0f32;
    let mut placed = Vec::new();
    for run in buffer.layout_runs() {
        let mut ascent = 0.0f32;
        let mut below = 0.0f32;
        for glyph in run.glyphs {
            let Some(font) = fonts.get_font(glyph.font_id, glyph.font_weight) else {
                continue;
            };
            let metrics = font.as_swash().metrics(&[]).scale(glyph.font_size);
            ascent = ascent.max(metrics.ascent);
            below = below.max(metrics.descent + metrics.leading);
        }
        let baseline = height + ascent;
        for glyph in run.glyphs {
            placed.push(PlacedGlyph {
                glyph: glyph.clone(),
                baseline,
            });
        }
        width = width.max(run.line_w);
        height += ascent + below;
    }
    if placed.is_empty() {
        return Err(failed("the system fonts shaped no glyph for the string"));
    }
    let Some((box_width, box_height)) = pixel_box(f64::from(width), f64::from(height)) else {
        return Err(failed(format!("the measured size {width}x{height} has no pixel box")));
    };

    let mut canvas = vec![0u8; box_width as usize * box_height as usize * 4];
    let mut drawn = false;
    for PlacedGlyph { glyph, baseline } in &placed {
        let physical = glyph.physical((0.0, *baseline), 1.0);
        let Some(image) = glyphs.get_image_uncached(fonts, physical.cache_key) else {
            continue;
        };
        let placement = GlyphPlacement {
            left: physical.x + image.placement.left,
            top: physical.y - image.placement.top,
            width: image.placement.width,
            height: image.placement.height,
        };
        let pixels = match (image.content, image.source) {
            (SwashContent::Mask, _) => GlyphPixels::Coverage(&image.data),
            (SwashContent::Color, Source::ColorOutline(_)) => GlyphPixels::Premultiplied(&image.data),
            (SwashContent::Color, _) => GlyphPixels::Straight(&image.data),
            (SwashContent::SubpixelMask, _) => continue,
        };
        drawn |= composite_glyph(&mut canvas, box_width, box_height, placement, pixels);
    }
    if !drawn {
        return Err(failed("no glyph of the string had an image"));
    }
    unpremultiply_rgba(&mut canvas);
    Ok(EmojiRaster {
        width: box_width,
        height: box_height,
        rgba: canvas,
    })
}

fn failed(reason: impl Into<String>) -> EmojiRasterError {
    EmojiRasterError::Failed {
        reason: reason.into(),
    }
}
