use std::collections::HashSet;

use gpui::{px, Hsla, Pixels};
use rise_core::foundation::ns_range::{self, NSRange};
use rise_ui::swiftui::environment::{env, EnvKey};

/// words of one message the search backend matched, as utf16 ranges into the text
/// carried through render so the bubble can mark them
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatMessageSearchHighlight {
    pub message_id: String,
    pub ranges: Vec<NSRange>,
}

impl ChatMessageSearchHighlight {
    /// the matched words themselves
    /// hosted rows draw a projection with different offsets so they mark the words instead of
    /// raw ranges
    pub fn words(text: &str, ranges: &[NSRange]) -> Vec<String> {
        let source_length = ns_range::length_utf16(text);
        let mut seen: HashSet<&str> = HashSet::new();
        let mut words: Vec<String> = Vec::new();
        for range in ranges {
            if range.length == 0 || range.location + range.length > source_length {
                continue;
            }
            let Some(word) = ns_range::substring(text, *range) else {
                continue;
            };
            if seen.insert(word) {
                words.push(word.to_owned());
            }
        }
        words
    }
}

/// fixed mark not a theme field
/// it stays readable under white text on every wallpaper so it ignores the palette
pub enum ChatSearchHighlightPalette {}

impl ChatSearchHighlightPalette {
    pub const MATCH_FILL: Hsla = Hsla { h: 0.114_285_715, s: 0.777_777_8, l: 0.45, a: 1.0 };
    pub const CORNER_RADIUS: Pixels = px(4.0);
    pub const HORIZONTAL_PADDING: f32 = 1.5;
    pub const VERTICAL_PADDING: f32 = 1.0;

    pub fn match_fill_color() -> Hsla {
        Self::MATCH_FILL
    }
}

pub struct ChatSearchHighlightWordsKey;

impl EnvKey for ChatSearchHighlightWordsKey {
    type Value = Vec<String>;

    fn default_value() -> Self::Value {
        Vec::new()
    }
}

/// words a hosted bubble marks with the search plate, empty everywhere but the row the
/// reader jumped to
pub fn chat_search_highlight_words(cx: &gpui::App) -> Vec<String> {
    env::<ChatSearchHighlightWordsKey>(cx)
}
