use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

use gpui::{FontWeight, Hsla, Rgba};
use rise_core::foundation::foundation_character::{Character, characters};
use rise_core::foundation::foundation_text::{character_boundaries, trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_core::foundation::ns_cache::NSCache;
use rise_core::foundation::ns_range::{self, NSRange, length_utf16};

use crate::swiftui::rich_text::{AttributedString, LineStyle, RichFont};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RichInlineStyle {
    Bold,
    Italic,
    Strikethrough,
    Spoiler,
    Code,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RichInlineSpan {
    pub range: NSRange,
    pub style: RichInlineStyle,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RichBlock {
    Paragraph(String),
    Code { language: Option<String>, code: String },
}

impl RichBlock {
    /// content derived key for a keyed list
    /// keying by position gives block N of one message the same identity as block N of the next
    /// so a reusing list hands over the old views
    pub fn identity(&self) -> String {
        match self {
            RichBlock::Paragraph(text) => format!("p\u{1F}{text}"),
            RichBlock::Code { language, code } => {
                format!("c\u{1F}{}\u{1F}{code}", language.as_deref().unwrap_or(""))
            }
        }
    }
}

/// bounded by count and by utf8 bytes, texts past the byte ceiling are split on every call
struct RichBlockCache {
    storage: NSCache<String, Arc<Vec<RichBlock>>>,
}

impl RichBlockCache {
    const MAXIMUM_CACHEABLE_UTF8_COUNT: usize = 65_536;

    fn new() -> Self {
        let storage = NSCache::new();
        storage.set_count_limit(768);
        storage.set_total_cost_limit(4 * 1_024 * 1_024);
        Self { storage }
    }

    fn blocks(&self, text: &str, split: impl Fn(&str) -> Vec<RichBlock>) -> Arc<Vec<RichBlock>> {
        let byte_count = text.len();
        if byte_count > Self::MAXIMUM_CACHEABLE_UTF8_COUNT {
            return Arc::new(split(text));
        }
        let key = text.to_owned();
        if let Some(cached) = self.storage.object(&key) {
            return cached;
        }
        let blocks = Arc::new(split(text));
        self.storage.set_object_cost(blocks.clone(), key, 128 + byte_count * 2);
        blocks
    }
}

static BLOCK_CACHE: LazyLock<RichBlockCache> = LazyLock::new(RichBlockCache::new);

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RichEditorStyle {
    pub font: RichFont,
    pub text_color: Hsla,
    pub marker_color: Hsla,
    pub code_font: RichFont,
    pub code_color: Hsla,
    pub code_background: Hsla,
    pub code_block_background: Hsla,
    pub code_block_foreground: Hsla,
}

impl RichEditorStyle {
    /// new style with the two code block colors left at defaults
    pub fn new(
        font: RichFont,
        text_color: Hsla,
        marker_color: Hsla,
        code_font: RichFont,
        code_color: Hsla,
        code_background: Hsla,
    ) -> Self {
        Self {
            font,
            text_color,
            marker_color,
            code_font,
            code_color,
            code_background,
            code_block_background: Rgba {
                r: 23.0 / 255.0,
                g: 26.0 / 255.0,
                b: 33.0 / 255.0,
                a: 1.0,
            }
            .into(),
            code_block_foreground: Rgba {
                r: 230.0 / 255.0,
                g: 230.0 / 255.0,
                b: 234.0 / 255.0,
                a: 1.0,
            }
            .into(),
        }
    }
}

/// the chars of a string, indexable
/// with byte boundaries so a run slices back out of the source without copying
pub(crate) struct CharacterArray<'a> {
    text: &'a str,
    bounds: Vec<usize>,
}

impl<'a> CharacterArray<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Self {
            text,
            bounds: character_boundaries(text),
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.bounds.len() - 1
    }

    pub(crate) fn at(&self, i: usize) -> &'a str {
        &self.text[self.bounds[i]..self.bounds[i + 1]]
    }

    pub(crate) fn slice(&self, from: usize, to: usize) -> &'a str {
        &self.text[self.bounds[from]..self.bounds[to]]
    }
}

fn split_on_newline_character(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut offset = 0;
    for character in rise_core::foundation::foundation_text::characters(text) {
        if character == "\n" {
            out.push(&text[start..offset]);
            start = offset + character.len();
        }
        offset += character.len();
    }
    out.push(&text[start..]);
    out
}

pub enum RichTextMarkup {}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RichTextMarkupInlineResult {
    pub display: String,
    pub spans: Vec<RichInlineSpan>,
    pub removed_ranges: Vec<NSRange>,
}

#[derive(Default)]
struct InlineAccumulator {
    display: String,
    spans: Vec<RichInlineSpan>,
    removed: Vec<NSRange>,
    display_u16: usize,
    raw_u16: usize,
}

enum InlineToken<'a> {
    Text(String),
    Styled {
        open: &'static str,
        inner: &'a str,
        close: &'static str,
        style: RichInlineStyle,
    },
}

struct FenceRange {
    full: NSRange,
    open_line: NSRange,
    close_line: Option<NSRange>,
}

/// the two font traits the editor adds
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct FontDescriptorSymbolicTraits {
    bits: u32,
}

impl FontDescriptorSymbolicTraits {
    pub const TRAIT_ITALIC: Self = Self { bits: 1 << 0 };
    pub const TRAIT_BOLD: Self = Self { bits: 1 << 1 };

    pub const fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    pub const fn contains(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }
}

impl RichFont {
    pub fn symbolic_traits(&self) -> FontDescriptorSymbolicTraits {
        let mut traits = FontDescriptorSymbolicTraits::default();
        if self.weight >= FontWeight::BOLD {
            traits = traits.union(FontDescriptorSymbolicTraits::TRAIT_BOLD);
        }
        if self.italic {
            traits = traits.union(FontDescriptorSymbolicTraits::TRAIT_ITALIC);
        }
        traits
    }

    /// same face and size with the traits added to its own
    pub fn with_traits(self, traits: FontDescriptorSymbolicTraits) -> Self {
        let combined = self.symbolic_traits().union(traits);
        let mut out = self;
        if combined.contains(FontDescriptorSymbolicTraits::TRAIT_BOLD) && out.weight < FontWeight::BOLD {
            out.weight = FontWeight::BOLD;
        }
        if combined.contains(FontDescriptorSymbolicTraits::TRAIT_ITALIC) {
            out.italic = true;
        }
        out
    }
}

/// emphasis stretch, where bold and italic overlap there is one combined font
pub type RichEmphasisRun = (NSRange, bool, bool);

impl RichTextMarkup {
    const MAXIMUM_INLINE_NESTING: usize = 3;

    const SPOILER_VEIL: char = '\u{2022}';

    pub fn has_code_block(text: &str) -> bool {
        if !text.contains("```") {
            return false;
        }
        Self::blocks(text)
            .iter()
            .any(|block| matches!(block, RichBlock::Code { .. }))
    }

    pub fn has_any_markup(text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        if text.contains("```") {
            return true;
        }
        !Self::parse_inline(text).spans.is_empty()
    }

    pub fn preview_line(text: &str) -> String {
        for block in Self::blocks(text).iter() {
            let source = match block {
                RichBlock::Paragraph(p) => p.as_str(),
                RichBlock::Code { code, .. } => code.as_str(),
            };
            for line in split_on_newline_character(source) {
                if !trimming_whitespaces(line).is_empty() {
                    return line.to_owned();
                }
            }
        }
        trimming_whitespaces_and_newlines(text).to_owned()
    }

    /// cheap byte scan, false means parse_inline and plain_text return the text unchanged
    pub fn contains_delimiter(text: &str) -> bool {
        text.bytes()
            .any(|b| b == 42 || b == 95 || b == 96 || b == 124 || b == 126)
    }

    /// text as a bubble draws it without delimiters, for previews drawn without styling
    pub fn plain_text(text: &str) -> String {
        if !Self::contains_delimiter(text) {
            return text.to_owned();
        }
        if !text.contains("```") {
            return Self::parse_inline(text).display;
        }
        Self::blocks(text)
            .iter()
            .map(|block| match block {
                RichBlock::Paragraph(paragraph) => Self::parse_inline(paragraph).display,
                RichBlock::Code { code, .. } => code.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn flattening_code_blocks(text: &str) -> String {
        if !text.contains("```") {
            return text.to_owned();
        }
        Self::blocks(text)
            .iter()
            .map(|block| match block {
                RichBlock::Paragraph(paragraph) => paragraph.clone(),
                RichBlock::Code { code, .. } => code.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// like plain_text but spoiler letters become dots so a list row doesnt leak hidden text
    pub fn preview_text(text: &str) -> String {
        if !Self::contains_delimiter(text) {
            return text.to_owned();
        }
        if !text.contains("```") {
            return Self::veiling_spoilers(&Self::parse_inline(text));
        }
        Self::blocks(text)
            .iter()
            .map(|block| match block {
                RichBlock::Paragraph(paragraph) => Self::veiling_spoilers(&Self::parse_inline(paragraph)),
                RichBlock::Code { code, .. } => code.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// one dot per visible char keeps the word shapes, whitespace stays
    fn veiling_spoilers(inline: &RichTextMarkupInlineResult) -> String {
        let veiled: Vec<NSRange> = inline
            .spans
            .iter()
            .filter(|span| span.style == RichInlineStyle::Spoiler)
            .map(|span| span.range)
            .collect();
        if veiled.is_empty() {
            return inline.display.clone();
        }
        let mut out = String::with_capacity(inline.display.len());
        let mut offset = 0;
        for character in characters(&inline.display) {
            let hidden = veiled
                .iter()
                .any(|range| offset >= range.location && offset < range.upper_bound());
            if hidden && !character.is_whitespace() {
                out.push(Self::SPOILER_VEIL);
            } else {
                out.push_str(character.as_str());
            }
            offset += length_utf16(character.as_str());
        }
        out
    }

    /// memoized, RichTextView and chat bubbles call this from render on every pass
    pub fn blocks(text: &str) -> Arc<Vec<RichBlock>> {
        if text.is_empty() {
            return Arc::new(Vec::new());
        }
        BLOCK_CACHE.blocks(text, Self::split_blocks)
    }

    fn split_blocks(text: &str) -> Vec<RichBlock> {
        let mut out: Vec<RichBlock> = Vec::new();
        let mut paragraph: Vec<&str> = Vec::new();
        let mut in_fence = false;
        let mut fence_lang: Option<String> = None;
        let mut code_lines: Vec<&str> = Vec::new();

        fn flush_paragraph(paragraph: &mut Vec<&str>, out: &mut Vec<RichBlock>) {
            if paragraph.is_empty() {
                return;
            }
            let joined = paragraph.join("\n");
            let joined = joined.trim_matches('\n');
            if !trimming_whitespaces_and_newlines(joined).is_empty() {
                out.push(RichBlock::Paragraph(joined.to_owned()));
            }
            paragraph.clear();
        }

        for line in text.split('\n') {
            let trimmed = trimming_whitespaces(line);
            if !in_fence && trimmed.starts_with("```") {
                flush_paragraph(&mut paragraph, &mut out);
                in_fence = true;
                let lang = trimming_whitespaces(&trimmed[3..]);
                fence_lang = (!lang.is_empty()).then(|| lang.to_owned());
                code_lines.clear();
            } else if in_fence && trimmed == "```" {
                out.push(RichBlock::Code {
                    language: fence_lang.take(),
                    code: code_lines.join("\n"),
                });
                in_fence = false;
                code_lines.clear();
            } else if in_fence {
                code_lines.push(line);
            } else {
                paragraph.push(line);
            }
        }
        if in_fence {
            out.push(RichBlock::Code {
                language: fence_lang,
                code: code_lines.join("\n"),
            });
        } else {
            flush_paragraph(&mut paragraph, &mut out);
        }
        out
    }

    pub fn parse_inline(raw: &str) -> RichTextMarkupInlineResult {
        if !Self::contains_delimiter(raw) {
            return RichTextMarkupInlineResult {
                display: raw.to_owned(),
                spans: Vec::new(),
                removed_ranges: Vec::new(),
            };
        }
        let mut acc = InlineAccumulator::default();
        Self::append_inline(raw, 0, &mut acc);
        RichTextMarkupInlineResult {
            display: acc.display,
            spans: acc.spans,
            removed_ranges: acc.removed,
        }
    }

    pub fn parse_inline_keeping_code_blocks_literal(raw: &str) -> RichTextMarkupInlineResult {
        if !raw.contains("```") {
            return Self::parse_inline(raw);
        }
        struct State {
            display: String,
            display_u16: usize,
            spans: Vec<RichInlineSpan>,
            removed: Vec<NSRange>,
            paragraph: String,
            paragraph_start: usize,
            last_kept_raw_end: usize,
        }
        fn flush_paragraph(state: &mut State) {
            if state.paragraph.is_empty() {
                return;
            }
            let parsed = RichTextMarkup::parse_inline(&state.paragraph);
            for span in &parsed.spans {
                state.spans.push(RichInlineSpan {
                    range: NSRange::new(span.range.location + state.display_u16, span.range.length),
                    style: span.style,
                });
            }
            for range in &parsed.removed_ranges {
                state
                    .removed
                    .push(NSRange::new(range.location + state.paragraph_start, range.length));
            }
            state.display.push_str(&parsed.display);
            state.display_u16 += length_utf16(&parsed.display);
            state.last_kept_raw_end = state.paragraph_start + length_utf16(&state.paragraph);
            state.paragraph.clear();
        }
        let mut state = State {
            display: String::new(),
            display_u16: 0,
            spans: Vec::new(),
            removed: Vec::new(),
            paragraph: String::new(),
            paragraph_start: 0,
            last_kept_raw_end: 0,
        };
        let mut raw_u16 = 0;
        let mut in_fence = false;
        let lines: Vec<&str> = raw.split('\n').collect();
        let last_index = lines.len() - 1;
        for (index, line) in lines.iter().enumerate() {
            let piece = if index == last_index { (*line).to_owned() } else { format!("{line}\n") };
            let piece_u16 = length_utf16(&piece);
            let trimmed = trimming_whitespaces(line);
            if !in_fence && trimmed.starts_with("```") {
                flush_paragraph(&mut state);
                state.removed.push(NSRange::new(raw_u16, piece_u16));
                in_fence = true;
            } else if in_fence && trimmed == "```" {
                state.removed.push(NSRange::new(raw_u16, piece_u16));
                in_fence = false;
            } else if in_fence {
                let line_u16 = length_utf16(line);
                if !trimmed.is_empty() {
                    state.spans.push(RichInlineSpan {
                        range: NSRange::new(state.display_u16, line_u16),
                        style: RichInlineStyle::Code,
                    });
                }
                state.display.push_str(&piece);
                state.display_u16 += piece_u16;
                state.last_kept_raw_end = raw_u16 + piece_u16;
            } else {
                if state.paragraph.is_empty() {
                    state.paragraph_start = raw_u16;
                }
                state.paragraph.push_str(&piece);
            }
            raw_u16 += piece_u16;
        }
        flush_paragraph(&mut state);

        while state.display.ends_with('\n') && !state.display.ends_with("\r\n") && state.last_kept_raw_end > 0 {
            state.display.pop();
            state.last_kept_raw_end -= 1;
            state.removed.push(NSRange::new(state.last_kept_raw_end, 1));
        }
        RichTextMarkupInlineResult {
            display: state.display,
            spans: state.spans,
            removed_ranges: state.removed,
        }
    }

    fn append_inline(raw: &str, depth: usize, acc: &mut InlineAccumulator) {
        for token in Self::tokenize_inline(raw) {
            match token {
                InlineToken::Text(s) => {
                    let len = length_utf16(&s);
                    acc.display.push_str(&s);
                    acc.display_u16 += len;
                    acc.raw_u16 += len;
                }
                InlineToken::Styled {
                    open,
                    inner,
                    close,
                    style,
                } => {
                    let open_len = length_utf16(open);
                    let close_len = length_utf16(close);
                    if open_len > 0 {
                        acc.removed.push(NSRange::new(acc.raw_u16, open_len));
                    }
                    acc.raw_u16 += open_len;
                    let start = acc.display_u16;
                    let span_index = acc.spans.len();
                    if style != RichInlineStyle::Code && depth < Self::MAXIMUM_INLINE_NESTING {
                        Self::append_inline(inner, depth + 1, acc);
                    } else {
                        let inner_len = length_utf16(inner);
                        acc.display.push_str(inner);
                        acc.display_u16 += inner_len;
                        acc.raw_u16 += inner_len;
                    }
                    if close_len > 0 {
                        acc.removed.push(NSRange::new(acc.raw_u16, close_len));
                    }
                    acc.raw_u16 += close_len;
                    let inner_display_len = acc.display_u16 - start;
                    if inner_display_len > 0 {
                        acc.spans.insert(
                            span_index,
                            RichInlineSpan {
                                range: NSRange::new(start, inner_display_len),
                                style,
                            },
                        );
                    }
                }
            }
        }
    }

    /// stretches where bold and italic overlap get one combined font
    /// renderers set fonts from these runs so a nested span keeps the outer weight or slant
    pub fn emphasis_runs(spans: &[RichInlineSpan]) -> Vec<RichEmphasisRun> {
        let emphasis: Vec<&RichInlineSpan> = spans
            .iter()
            .filter(|span| span.style == RichInlineStyle::Bold || span.style == RichInlineStyle::Italic)
            .collect();
        if emphasis.is_empty() {
            return Vec::new();
        }
        if let [only] = emphasis.as_slice() {
            return vec![(
                only.range,
                only.style == RichInlineStyle::Bold,
                only.style == RichInlineStyle::Italic,
            )];
        }
        let mut bounds = BTreeSet::new();
        for span in &emphasis {
            bounds.insert(span.range.location);
            bounds.insert(span.range.location + span.range.length);
        }
        let sorted: Vec<usize> = bounds.into_iter().collect();
        let mut runs: Vec<RichEmphasisRun> = Vec::new();
        for pair in sorted.windows(2) {
            let lower = pair[0];
            let upper = pair[1];
            let mut bold = false;
            let mut italic = false;
            for span in emphasis
                .iter()
                .filter(|span| span.range.location <= lower && span.range.location + span.range.length >= upper)
            {
                if span.style == RichInlineStyle::Bold {
                    bold = true;
                } else {
                    italic = true;
                }
            }
            if !(bold || italic) {
                continue;
            }
            let extends = runs
                .last()
                .is_some_and(|last| last.1 == bold && last.2 == italic && last.0.location + last.0.length == lower);
            if !extends {
                runs.push((NSRange::new(lower, upper - lower), bold, italic));
            } else if let Some(last) = runs.last_mut() {
                last.0.length += upper - lower;
            }
        }
        runs
    }

    pub fn display_location(for_raw: usize, removed: &[NSRange]) -> usize {
        let mut shift = 0;
        for r in removed {
            if r.location + r.length <= for_raw {
                shift += r.length;
            } else if r.location < for_raw {
                shift += for_raw - r.location;
            } else {
                break;
            }
        }
        for_raw - shift
    }

    pub fn display_range(for_raw: NSRange, removed: &[NSRange]) -> Option<NSRange> {
        let start = Self::display_location(for_raw.location, removed);
        let end = Self::display_location(for_raw.location + for_raw.length, removed);
        if end <= start {
            return None;
        }
        Some(NSRange::new(start, end - start))
    }

    fn tokenize_inline(raw: &str) -> Vec<InlineToken<'_>> {
        let chars = CharacterArray::new(raw);
        let n = chars.count();
        let mut tokens: Vec<InlineToken<'_>> = Vec::new();
        let mut buf = String::new();
        fn flush(buf: &mut String, tokens: &mut Vec<InlineToken<'_>>) {
            if !buf.is_empty() {
                tokens.push(InlineToken::Text(std::mem::take(buf)));
            }
        }

        let doubles: [(&str, &'static str, RichInlineStyle); 3] = [
            ("*", "**", RichInlineStyle::Bold),
            ("~", "~~", RichInlineStyle::Strikethrough),
            ("|", "||", RichInlineStyle::Spoiler),
        ];

        let mut i = 0;
        'scan: while i < n {
            let c = chars.at(i);

            if c == "`"
                && let Some(close) = Self::find_single_close(&chars, "`", i + 1, n)
                && close > i + 1
            {
                let inner = chars.slice(i + 1, close);
                flush(&mut buf, &mut tokens);
                tokens.push(InlineToken::Styled {
                    open: "`",
                    inner,
                    close: "`",
                    style: RichInlineStyle::Code,
                });
                i = close + 1;
                continue;
            }

            for (single, delimiter, style) in doubles {
                if c == single
                    && i + 1 < n
                    && chars.at(i + 1) == single
                    && let Some(close) = Self::find_double_close(&chars, single, i + 2, n)
                {
                    let inner = chars.slice(i + 2, close);
                    if Self::is_valid_emphasis(inner) {
                        flush(&mut buf, &mut tokens);
                        tokens.push(InlineToken::Styled {
                            open: delimiter,
                            inner,
                            close: delimiter,
                            style,
                        });
                        i = close + 2;
                        continue 'scan;
                    }
                }
            }

            if c == "_"
                && Self::can_open_underscore(&chars, i, n)
                && let Some(close) = Self::find_underscore_close(&chars, i + 1, n)
            {
                let inner = chars.slice(i + 1, close);
                if Self::is_valid_emphasis(inner) {
                    flush(&mut buf, &mut tokens);
                    tokens.push(InlineToken::Styled {
                        open: "_",
                        inner,
                        close: "_",
                        style: RichInlineStyle::Italic,
                    });
                    i = close + 1;
                    continue;
                }
            }

            buf.push_str(c);
            i += 1;
        }
        flush(&mut buf, &mut tokens);
        tokens
    }

    fn is_valid_emphasis(inner: &str) -> bool {
        if inner.is_empty() || inner.contains('\n') {
            return false;
        }
        !inner.starts_with(' ') && !inner.ends_with(' ')
    }

    fn is_word_char(c: &str) -> bool {
        Character::new(c).is_some_and(|c| c.is_letter() || c.is_number()) || c == "_"
    }

    fn can_open_underscore(chars: &CharacterArray<'_>, i: usize, n: usize) -> bool {
        let prev_ok = i == 0 || !Self::is_word_char(chars.at(i - 1));
        let next_ok = i + 1 < n && {
            let next = chars.at(i + 1);
            next != " " && next != "\n" && next != "_"
        };
        prev_ok && next_ok
    }

    fn find_single_close(chars: &CharacterArray<'_>, ch: &str, from: usize, n: usize) -> Option<usize> {
        let mut j = from;
        while j < n {
            if chars.at(j) == "\n" {
                return None;
            }
            if chars.at(j) == ch {
                return Some(j);
            }
            j += 1;
        }
        None
    }

    fn find_double_close(chars: &CharacterArray<'_>, ch: &str, from: usize, n: usize) -> Option<usize> {
        let mut j = from;
        while j + 1 < n {
            if chars.at(j) == "\n" {
                return None;
            }
            if chars.at(j) == ch && chars.at(j + 1) == ch {
                return Some(j);
            }
            j += 1;
        }
        None
    }

    fn find_underscore_close(chars: &CharacterArray<'_>, from: usize, n: usize) -> Option<usize> {
        let mut j = from;
        while j < n {
            if chars.at(j) == "\n" {
                return None;
            }
            if chars.at(j) == "_" {
                let prev_ok = j >= 1 && chars.at(j - 1) != " ";
                let next_ok = j + 1 >= n || !Self::is_word_char(chars.at(j + 1));
                if prev_ok && next_ok {
                    return Some(j);
                }
            }
            j += 1;
        }
        None
    }

    pub fn editor_attributed(raw: &str, style: &RichEditorStyle) -> AttributedString {
        let mut result = AttributedString::new(raw);
        let font = style.font;
        let text_color = style.text_color;
        result.update_all(|a| {
            a.font = Some(font);
            a.foreground_color = Some(text_color);
        });
        let length = length_utf16(raw);
        if length == 0 {
            return result;
        }
        let full = NSRange::new(0, length);

        let fences = Self::code_fence_ranges(raw);
        for fence in &fences {
            let code_font = style.code_font;
            let block_background = style.code_block_background;
            let block_foreground = style.code_block_foreground;
            result.update(fence.full, |a| {
                a.font = Some(code_font);
                a.background_color = Some(block_background);
                a.foreground_color = Some(block_foreground);
            });
            let dim = block_foreground.alpha(0.5);
            result.update(fence.open_line, |a| a.foreground_color = Some(dim));
            if let Some(close) = fence.close_line {
                result.update(close, |a| a.foreground_color = Some(dim));
            }
        }

        let fence_ranges: Vec<NSRange> = fences.iter().map(|fence| fence.full).collect();
        for region in Self::non_fence_regions(full, &fence_ranges) {
            Self::style_inline_region(&mut result, raw, region, style, 0);
        }
        result
    }

    fn style_inline_region(
        result: &mut AttributedString,
        ns: &str,
        region: NSRange,
        style: &RichEditorStyle,
        depth: usize,
    ) {
        let Some(sub) = ns_range::substring(ns, region) else {
            return;
        };
        let tokens = Self::tokenize_inline(sub);
        let mut offset = region.location;
        for token in tokens {
            match token {
                InlineToken::Text(s) => {
                    offset += length_utf16(&s);
                }
                InlineToken::Styled {
                    open,
                    inner,
                    close,
                    style: kind,
                } => {
                    let open_len = length_utf16(open);
                    let inner_len = length_utf16(inner);
                    let close_len = length_utf16(close);
                    let open_r = NSRange::new(offset, open_len);
                    let inner_r = NSRange::new(offset + open_len, inner_len);
                    let close_r = NSRange::new(offset + open_len + inner_len, close_len);
                    let marker = style.marker_color;
                    result.update(open_r, |a| a.foreground_color = Some(marker));
                    result.update(close_r, |a| a.foreground_color = Some(marker));
                    Self::apply_editor_inner_style(kind, result, inner_r, style);
                    if kind != RichInlineStyle::Code && depth < Self::MAXIMUM_INLINE_NESTING && inner_len > 0 {
                        Self::style_inline_region(result, ns, inner_r, style, depth + 1);
                    }
                    offset += open_len + inner_len + close_len;
                }
            }
        }
    }

    fn add_editor_trait(
        traits: FontDescriptorSymbolicTraits,
        result: &mut AttributedString,
        range: NSRange,
        style: &RichEditorStyle,
    ) {
        let base = style.font;
        result.update(range, |a| {
            a.font = Some(a.font.unwrap_or(base).with_traits(traits));
        });
    }

    fn apply_editor_inner_style(
        kind: RichInlineStyle,
        result: &mut AttributedString,
        range: NSRange,
        style: &RichEditorStyle,
    ) {
        if range.length == 0 {
            return;
        }
        match kind {
            RichInlineStyle::Bold => {
                Self::add_editor_trait(FontDescriptorSymbolicTraits::TRAIT_BOLD, result, range, style);
            }
            RichInlineStyle::Italic => {
                Self::add_editor_trait(FontDescriptorSymbolicTraits::TRAIT_ITALIC, result, range, style);
            }
            RichInlineStyle::Strikethrough => {
                let text_color = style.text_color;
                result.update(range, |a| {
                    a.strikethrough_style = Some(LineStyle::single_color(text_color));
                });
            }
            RichInlineStyle::Spoiler => {
                let background = style.code_background;
                result.update(range, |a| a.background_color = Some(background));
            }
            RichInlineStyle::Code => {
                let code_font = style.code_font;
                let code_color = style.code_color;
                let background = style.code_background;
                result.update(range, |a| {
                    a.font = Some(code_font);
                    a.foreground_color = Some(code_color);
                    a.background_color = Some(background);
                });
            }
        }
    }

    /// each line without its terminator
    /// cR LF ends one line and a trailing terminator opens no empty last line
    fn line_ranges(ns: &str) -> Vec<NSRange> {
        let mut out = Vec::new();
        let mut start = 0usize;
        let mut offset = 0usize;
        let mut scalars = ns.chars().peekable();
        while let Some(c) = scalars.next() {
            let width = c.len_utf16();
            if matches!(c, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}') {
                out.push(NSRange::new(start, offset - start));
                offset += width;
                if c == '\r' && scalars.peek() == Some(&'\n') {
                    scalars.next();
                    offset += 1;
                }
                start = offset;
            } else {
                offset += width;
            }
        }
        if start < offset {
            out.push(NSRange::new(start, offset - start));
        }
        out
    }

    fn code_fence_ranges(ns: &str) -> Vec<FenceRange> {
        let line_ranges = Self::line_ranges(ns);
        let mut out = Vec::new();
        let mut open_idx: Option<usize> = None;
        for (i, r) in line_ranges.iter().enumerate() {
            let line = trimming_whitespaces(ns_range::substring(ns, *r).unwrap_or(""));
            match open_idx {
                None => {
                    if line.starts_with("```") {
                        open_idx = Some(i);
                    }
                }
                Some(open) => {
                    if line == "```" {
                        let open_r = line_ranges[open];
                        out.push(FenceRange {
                            full: NSRange::new(open_r.location, r.location + r.length - open_r.location),
                            open_line: open_r,
                            close_line: Some(*r),
                        });
                        open_idx = None;
                    }
                }
            }
        }
        if let Some(open) = open_idx {
            let open_r = line_ranges[open];
            let end_loc = length_utf16(ns);
            out.push(FenceRange {
                full: NSRange::new(open_r.location, end_loc - open_r.location),
                open_line: open_r,
                close_line: None,
            });
        }
        out
    }

    fn non_fence_regions(full: NSRange, fences: &[NSRange]) -> Vec<NSRange> {
        if fences.is_empty() {
            return vec![full];
        }
        let mut sorted = fences.to_vec();
        sorted.sort_by_key(|range| range.location);
        let mut out = Vec::new();
        let mut cursor = full.location;
        for f in sorted {
            if f.location > cursor {
                out.push(NSRange::new(cursor, f.location - cursor));
            }
            cursor = cursor.max(f.location + f.length);
        }
        let end = full.location + full.length;
        if cursor < end {
            out.push(NSRange::new(cursor, end - cursor));
        }
        out
    }

    pub fn apply_format(format: RichTextMarkupFormat, to: &str, selection: NSRange) -> (String, NSRange) {
        let raw = to;
        let length = length_utf16(raw);
        let safe_loc = selection.location.min(length);
        let safe_len = selection.length.min(length - safe_loc);
        let sel = NSRange::new(safe_loc, safe_len);
        let (open, close) = format.delimiters();
        let open_len = length_utf16(open);
        let close_len = length_utf16(close);

        let Some(bytes) = ns_range::utf16_to_byte_range(raw, sel) else {
            return (raw.to_owned(), sel);
        };
        let replacing = |with: &str| {
            let mut out = String::with_capacity(raw.len() + with.len());
            out.push_str(&raw[..bytes.start]);
            out.push_str(with);
            out.push_str(&raw[bytes.end..]);
            out
        };

        if sel.length == 0 {
            let new_text = replacing(&format!("{open}{close}"));
            return (new_text, NSRange::new(sel.location + open_len, 0));
        }

        let selected = &raw[bytes.clone()];
        let selected_len = length_utf16(selected);
        let already_wrapped =
            selected.starts_with(open) && selected.ends_with(close) && selected_len >= open_len + close_len;
        if already_wrapped {
            let inner = &selected[open.len()..selected.len() - close.len()];
            let new_text = replacing(inner);
            return (new_text, NSRange::new(sel.location, length_utf16(inner)));
        }

        let replacement = format!("{open}{selected}{close}");
        let new_text = replacing(&replacement);
        if format == RichTextMarkupFormat::CodeBlock {
            return (new_text, NSRange::new(sel.location + 3, 0));
        }
        (new_text, NSRange::new(sel.location, length_utf16(&replacement)))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RichTextMarkupFormat {
    Bold,
    Italic,
    Strikethrough,
    Spoiler,
    Code,
    CodeBlock,
}

impl RichTextMarkupFormat {
    pub const ALL_CASES: [RichTextMarkupFormat; 6] = [
        RichTextMarkupFormat::Bold,
        RichTextMarkupFormat::Italic,
        RichTextMarkupFormat::Strikethrough,
        RichTextMarkupFormat::Spoiler,
        RichTextMarkupFormat::Code,
        RichTextMarkupFormat::CodeBlock,
    ];

    pub fn delimiters(self) -> (&'static str, &'static str) {
        match self {
            RichTextMarkupFormat::Bold => ("**", "**"),
            RichTextMarkupFormat::Italic => ("_", "_"),
            RichTextMarkupFormat::Strikethrough => ("~~", "~~"),
            RichTextMarkupFormat::Spoiler => ("||", "||"),
            RichTextMarkupFormat::Code => ("`", "`"),
            RichTextMarkupFormat::CodeBlock => ("```\n", "\n```"),
        }
    }

    pub fn title_key(self) -> &'static str {
        match self {
            RichTextMarkupFormat::Bold => "format_bold",
            RichTextMarkupFormat::Italic => "format_italic",
            RichTextMarkupFormat::Strikethrough => "format_strikethrough",
            RichTextMarkupFormat::Spoiler => "doc_mark_spoiler",
            RichTextMarkupFormat::Code => "format_monospace",
            RichTextMarkupFormat::CodeBlock => "format_code_block",
        }
    }

    pub fn sf_symbol(self) -> &'static str {
        match self {
            RichTextMarkupFormat::Bold => "bold",
            RichTextMarkupFormat::Italic => "italic",
            RichTextMarkupFormat::Strikethrough => "strikethrough",
            RichTextMarkupFormat::Spoiler => "eye.slash",
            RichTextMarkupFormat::Code => "chevron.left.forwardslash.chevron.right",
            RichTextMarkupFormat::CodeBlock => "curlybraces",
        }
    }
}

/// a rich block paired with its position, keyed for a keyed list
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseIndexedRichBlock {
    pub id: String,
    pub block: RichBlock,
}

impl RiseIndexedRichBlock {
    pub fn list(blocks: &[RichBlock]) -> Vec<RiseIndexedRichBlock> {
        blocks
            .iter()
            .enumerate()
            .map(|(offset, block)| RiseIndexedRichBlock {
                id: format!("{offset}\u{1F}{}", block.identity()),
                block: block.clone(),
            })
            .collect()
    }
}
