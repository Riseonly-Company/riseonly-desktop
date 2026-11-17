use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, LazyLock, OnceLock};

use gpui::{App, Bounds, Font as GpuiFont, FontWeight, Hsla, Pixels, SharedString, TextRun, TextSystem, WindowTextSystem, bounds, point, px, size};
use rise_canon::RiseLimits;
use rise_core::foundation::ns_range::{self, NSRange};
use rise_ui::swiftui::font::{Font, FontDesign, NATURAL_LINE_HEIGHT};
use rise_ui::swiftui::rich_text::AttributedString;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::formatted_text_ui::{
    FormattedText, FormattedTextBoundedCache, FormattedTextMentionToken, FormattedTextPreparation,
    FormattedTextPreparationCachePolicy, FormattedTextPreparationQueue, FormattedTextPreparedGeometry,
    FormattedTextRenderSeed, FormattedTextStyle,
};

#[derive(Clone, Debug)]
pub struct ChatTextHitLayoutSeed {
    pub render: FormattedTextRenderSeed,
    pub width: f32,
    pub display_range: Option<NSRange>,
}

impl ChatTextHitLayoutSeed {
    pub fn new(render: FormattedTextRenderSeed, width: f32, display_range: Option<NSRange>) -> Self {
        Self {
            render,
            width: if width.is_finite() { width.max(1.0) } else { 1.0 },
            display_range,
        }
    }
}

impl PartialEq for ChatTextHitLayoutSeed {
    fn eq(&self, other: &Self) -> bool {
        self.render == other.render
            && self.width.to_bits() == other.width.to_bits()
            && self.display_range == other.display_range
    }
}

impl Eq for ChatTextHitLayoutSeed {}

impl Hash for ChatTextHitLayoutSeed {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.render.hash(state);
        self.width.to_bits().hash(state);
        self.display_range.hash(state);
    }
}

/// measurement result of the prepared text layout
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChatPreparedTextLayoutMeasurement {
    pub last_line_width: f32,
    pub total_height: f32,
    pub line_count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct ChatPreparedTextLayout {
    pub hits: Vec<MentionGlyphGeometryHit>,
    pub measurement: ChatPreparedTextLayoutMeasurement,
}

impl ChatPreparedTextLayout {
    fn empty() -> Self {
        Self::default()
    }
}

pub enum ChatPreparedTextLayoutCache {}

static COUNT_LIMIT: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("chat_prepared_text_layout_cache_entries"));
static TOTAL_COST_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("chat_prepared_text_layout_cache_bytes"));

static LAYOUTS: LazyLock<FormattedTextBoundedCache<ChatTextHitLayoutSeed, ChatPreparedTextLayout>> =
    LazyLock::new(|| FormattedTextBoundedCache::new(*COUNT_LIMIT, *TOTAL_COST_LIMIT));

static TEXT_SYSTEM: OnceLock<Arc<TextSystem>> = OnceLock::new();

impl ChatPreparedTextLayoutCache {
    pub const URL_MARKER: &'static str = "\u{1}url:";
    pub const SPOILER_MARKER: &'static str = "\u{1}spoiler:";

    /// hands the app text system to the layout worker, later calls do nothing
    pub fn install_text_system(cx: &App) {
        if TEXT_SYSTEM.get().is_none() {
            let _ = TEXT_SYSTEM.set(cx.text_system().clone());
        }
    }

    pub fn request(seed: &ChatTextHitLayoutSeed) -> Arc<FormattedTextPreparation<ChatPreparedTextLayout>> {
        if let Some(cached) = LAYOUTS.cached_value(seed) {
            return FormattedTextPreparation::with_value(cached);
        }
        let cost = FormattedTextPreparationCachePolicy::geometry_cost(&seed.render.text)
            + FormattedTextPreparationCachePolicy::role_mention_cost(&seed.render.role_mentions)
            + FormattedTextPreparationCachePolicy::option_cost(&seed.render.options);
        let key = seed.clone();
        let build_seed = seed.clone();
        FormattedTextPreparationQueue::shared().request(key, cost, move || Self::prepare(&build_seed, None))
    }

    /// callable from the presentation worker once the exact text width is known
    /// no shaped line or view crosses threads
    pub fn prepare(
        seed: &ChatTextHitLayoutSeed,
        geometry: Option<Arc<FormattedTextPreparedGeometry>>,
    ) -> ChatPreparedTextLayout {
        let Some(text_system) = TEXT_SYSTEM.get().cloned() else {
            return ChatPreparedTextLayout::empty();
        };
        LAYOUTS.value(
            seed.clone(),
            FormattedTextPreparationCachePolicy::line_measurement_cost(&seed.render.text)
                + FormattedTextPreparationCachePolicy::role_mention_cost(&seed.render.role_mentions)
                + FormattedTextPreparationCachePolicy::option_cost(&seed.render.options),
            FormattedTextPreparationCachePolicy::can_cache(&seed.render.text),
            |layout| {
                layout.hits.iter().fold(0_i64, |sum, hit| {
                    sum + std::mem::size_of::<MentionGlyphGeometryHit>() as i64 + hit.tag.len() as i64
                })
            },
            || {
                let input = FormattedText::interaction_preparation(
                    &seed.render.text,
                    &seed.render.style,
                    &seed.render.options,
                    &seed.render.role_mentions,
                    geometry,
                );
                let mut tokens: Vec<FormattedTextMentionToken> = input.mentions.clone();
                tokens.extend(input.urls.iter().map(|token| FormattedTextMentionToken {
                    tag: format!("{}{}", Self::URL_MARKER, token.tag),
                    range: token.range,
                }));
                tokens.extend(input.spoilers.iter().map(|token| FormattedTextMentionToken {
                    tag: format!("{}{}", Self::SPOILER_MARKER, token.tag),
                    range: token.range,
                }));
                let attributed = &input.geometry.attributed;
                let style = &seed.render.style;
                if let Some(range) = seed.display_range {
                    let length = ns_range::length_utf16(attributed.characters());
                    let bytes = (range.location <= length && range.length <= length - range.location)
                        .then(|| ns_range::utf16_to_byte_range(attributed.characters(), range))
                        .flatten();
                    let Some(bytes) = bytes else {
                        return ChatPreparedTextLayout::empty();
                    };
                    return Self::layout(
                        &text_system,
                        attributed,
                        bytes,
                        &Self::clipped_tokens(&tokens, range),
                        seed.width,
                        style,
                    );
                }
                let whole = 0..attributed.characters().len();
                Self::layout(&text_system, attributed, whole, &tokens, seed.width, style)
            },
        )
    }

    pub fn clipped_tokens(tokens: &[FormattedTextMentionToken], to: NSRange) -> Vec<FormattedTextMentionToken> {
        tokens
            .iter()
            .filter_map(|token| {
                let overlap = token.range.intersection(to)?;
                Some(FormattedTextMentionToken {
                    tag: token.tag.clone(),
                    range: NSRange::new(overlap.location - to.location, overlap.length),
                })
            })
            .collect()
    }

    /// lays out one fragment (byte range of attributed), token ranges are utf16 offsets into it
    /// same line height as the drawn text
    fn layout(
        text_system: &Arc<TextSystem>,
        attributed: &AttributedString,
        fragment: Range<usize>,
        tokens: &[FormattedTextMentionToken],
        width: f32,
        style: &FormattedTextStyle,
    ) -> ChatPreparedTextLayout {
        let characters = attributed.characters();
        let Some(text) = characters.get(fragment.clone()) else {
            return ChatPreparedTextLayout::empty();
        };
        if text.is_empty() || !width.is_finite() || width <= 0.0 {
            return ChatPreparedTextLayout::empty();
        }
        let runs = Self::text_runs(attributed, fragment, style.font_size);
        let line_height = (style.font_size * NATURAL_LINE_HEIGHT + style.line_spacing).round();
        let shaper = WindowTextSystem::new(text_system.clone());
        let Ok(lines) = shaper.shape_text(SharedString::from(text.to_owned()), px(style.font_size), &runs, Some(px(width)), None)
        else {
            return ChatPreparedTextLayout::empty();
        };

        let token_bytes: Vec<(&str, Range<usize>)> = tokens
            .iter()
            .filter_map(|token| Some((token.tag.as_str(), ns_range::utf16_to_byte_range(text, token.range)?)))
            .collect();

        let mut hits: Vec<MentionGlyphGeometryHit> = Vec::new();
        let mut row_count = 0_usize;
        let mut last_row_width = 0.0_f32;
        let mut paragraph_start = 0_usize;
        for line in lines.iter() {
            let layout = &line.unwrapped_layout;
            let ascent = f32::from(layout.ascent);
            let descent = f32::from(layout.descent).abs();
            let glyph_top = ((line_height - (ascent + descent)) / 2.0).max(0.0);
            let mut row_starts: Vec<(usize, f32)> = vec![(0, 0.0)];
            for boundary in line.wrap_boundaries.iter() {
                if let Some(glyph) = layout.runs.get(boundary.run_ix).and_then(|run| run.glyphs.get(boundary.glyph_ix)) {
                    row_starts.push((glyph.index, f32::from(glyph.position.x)));
                }
            }
            for (row, &(start, origin_x)) in row_starts.iter().enumerate() {
                let (end, end_x) = row_starts
                    .get(row + 1)
                    .copied()
                    .unwrap_or((layout.len, f32::from(layout.width)));
                let top = row_count as f32 * line_height;
                for (tag, bytes) in token_bytes.iter() {
                    let token_start = bytes.start.max(paragraph_start + start);
                    let token_end = bytes.end.min(paragraph_start + end);
                    if token_start >= token_end {
                        continue;
                    }
                    let start_x = f32::from(layout.x_for_index(token_start - paragraph_start)) - origin_x;
                    let end_x_token = f32::from(layout.x_for_index(token_end - paragraph_start)) - origin_x;
                    let hit_width = (end_x_token - start_x).abs();
                    if hit_width <= 0.5 {
                        continue;
                    }
                    hits.push(MentionGlyphGeometryHit {
                        tag: (*tag).to_owned(),
                        rect: bounds(
                            point(px(start_x.min(end_x_token)), px(top + glyph_top)),
                            size(px(hit_width), px(ascent + descent)),
                        ),
                    });
                }
                last_row_width = end_x - origin_x;
                row_count += 1;
            }
            paragraph_start += line.len() + 1;
        }
        if row_count == 0 {
            return ChatPreparedTextLayout::empty();
        }
        ChatPreparedTextLayout {
            hits,
            measurement: ChatPreparedTextLayoutMeasurement {
                last_line_width: last_row_width.ceil(),
                total_height: (line_height * row_count as f32).ceil(),
                line_count: row_count,
            },
        }
    }

    fn text_runs(attributed: &AttributedString, fragment: Range<usize>, font_size: f32) -> Vec<TextRun> {
        attributed
            .runs()
            .filter_map(|(range, attributes)| {
                let start = range.start.max(fragment.start);
                let end = range.end.min(fragment.end);
                if start >= end {
                    return None;
                }
                let rich = attributes.font;
                let face = Font {
                    size: font_size,
                    weight: rich.map_or(FontWeight::NORMAL, |font| font.weight),
                    design: rich.map_or(FontDesign::Default, |font| font.design),
                    italic: rich.is_some_and(|font| font.italic),
                    monospaced_digit: false,
                };
                Some(TextRun {
                    len: end - start,
                    font: GpuiFont {
                        family: face.family(),
                        features: face.features(),
                        fallbacks: None,
                        weight: face.weight,
                        style: face.style(),
                    },
                    color: Hsla::default(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                })
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MentionGlyphGeometryHit {
    pub tag: String,
    pub rect: Bounds<Pixels>,
}

pub enum MentionGlyphGeometry {}

impl MentionGlyphGeometry {
    pub fn rects(
        attributed: &AttributedString,
        tokens: &[FormattedTextMentionToken],
        width: f32,
        style: &FormattedTextStyle,
    ) -> Vec<MentionGlyphGeometryHit> {
        if tokens.is_empty() || width <= 1.0 {
            return Vec::new();
        }
        let Some(text_system) = TEXT_SYSTEM.get() else {
            return Vec::new();
        };
        let whole = 0..attributed.characters().len();
        ChatPreparedTextLayoutCache::layout(text_system, attributed, whole, tokens, width, style).hits
    }
}
