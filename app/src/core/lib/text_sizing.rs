use std::sync::LazyLock;

use gpui::{Font, FontWeight, Hsla, Pixels, SharedString, Size, TextRun, Window, font, px};
use rise_core::foundation::foundation_text::character_count;
use rise_core::foundation::ns_cache::NSCache;
use rise_theme::FONT_FAMILY;

pub fn grid_title_font_size(text: Option<&str>) -> Pixels {
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return px(30.0);
    };
    let length = character_count(text);
    if length <= 10 {
        return px(30.0);
    }
    if length >= 50 {
        return px(10.0);
    }
    px((30.0 - (length - 10) as f64 * (20.0 / 40.0)).round() as f32)
}

static CACHE: LazyLock<NSCache<String, f32>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(512);
    cache
});

/// shrinks the tile font until the widest word fits one line and the text fits the box
/// sizing by length alone left words that the layout broke mid word
pub struct GridTileTextFit;

impl GridTileTextFit {
    pub fn font_size(
        text: &str,
        preferred: Pixels,
        minimum: Pixels,
        weight: FontWeight,
        box_size: Size<Pixels>,
        window: &Window,
    ) -> Pixels {
        let preferred = f32::from(preferred);
        let minimum = f32::from(minimum);
        let width = f32::from(box_size.width).floor();
        let height = f32::from(box_size.height).floor();
        if text.is_empty() || preferred <= minimum || width <= 0.0 || height <= 0.0 {
            return px(preferred);
        }

        let key = format!("{preferred}|{minimum}|{width}|{height}|{}|{text}", weight.0);
        if let Some(cached) = CACHE.object(&key) {
            return px(cached);
        }

        let words: Vec<&str> = text.split(char::is_whitespace).filter(|word| !word.is_empty()).take(32).collect();
        let face = system_font(weight);
        let widest_word = |size: f32| -> f32 {
            words
                .iter()
                .map(|word| {
                    let runs = [text_run(word.len(), &face)];
                    f32::from(window.text_system().shape_line(SharedString::from(word.to_string()), px(size), &runs, None).width())
                })
                .fold(0.0, f32::max)
        };

        let limit = width - 2.0;
        let mut size = preferred;
        let mut widest = widest_word(size);
        for _ in 0..6 {
            if !(widest > limit && size > minimum) {
                break;
            }
            size = minimum.max((size - 1.0).min(size * limit / widest).floor());
            widest = widest_word(size);
        }

        for _ in 0..4 {
            if size <= minimum {
                break;
            }
            let needed = wrapped_height(text, size, width, &face, window);
            if needed <= height {
                break;
            }
            size = minimum.max((size * 0.5_f32.max((height / needed).sqrt())).floor());
        }

        CACHE.set_object(size, key);
        px(size)
    }
}

fn system_font(weight: FontWeight) -> Font {
    Font {
        weight,
        ..font(FONT_FAMILY)
    }
}

fn text_run(len: usize, face: &Font) -> TextRun {
    TextRun {
        len,
        font: face.clone(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn wrapped_height(text: &str, size: f32, width: f32, face: &Font, window: &Window) -> f32 {
    let runs = [text_run(text.len(), face)];
    let Ok(lines) = window
        .text_system()
        .shape_text(SharedString::from(text.to_string()), px(size), &runs, Some(px(width)), None)
    else {
        return 0.0;
    };
    lines
        .iter()
        .map(|line| {
            let line_height = f32::from(line.unwrapped_layout.ascent) + f32::from(line.unwrapped_layout.descent).abs();
            line_height * (line.wrap_boundaries.len() as f32 + 1.0)
        })
        .sum()
}
