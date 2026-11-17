use std::ops::Range;
use std::sync::Arc;

use gpui::{FontWeight, Hsla, Pixels, Point, bounds, point, px, size};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_range::{self, NSRange};
use rise_ui::rich_text::rich_text_markup::{FontDescriptorSymbolicTraits, RichBlock, RichInlineStyle, RichTextMarkup};
use rise_ui::swiftui::font::FontDesign;
use rise_ui::swiftui::rich_text::{AttributeContainer, AttributedString, LineStyle, RichFont};

use rise_i18n::tr;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::formatted_text_ui::{FormattedText, FormattedTextOptions, FormattedTextStyle};
use crate::core::ui::rich_text::post_text_layout::{PostTextLayout, PostTextLayoutBuilder};
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowCaptionLayout, PostRowLayoutSpec, PostRowPalette, PostRowParagraphLayout, PostRowTextBlockLayout,
    PostTextTokenKind,
};

/// every constant tying native caption text to the hosted caption
pub enum PostRichTextCalibration {}

impl PostRichTextCalibration {
    pub const TITLE_FONT_SIZE: f32 = 17.0;
    pub const TITLE_WEIGHT: FontWeight = FontWeight::SEMIBOLD;
    pub const TITLE_LINE_SPACING: f32 = 0.0;
    pub const PARAGRAPH_FONT_SIZE: f32 = 15.0;
    pub const PARAGRAPH_LINE_SPACING: f32 = 3.0;
    pub const CODE_FONT_SCALE: f32 = 0.95;
    pub const CODE_BACKGROUND_OPACITY: f32 = 0.16;
    pub const SUBTITLE_FONT_SIZE: f32 = 12.0;
    pub const SUBTITLE_LINE_SPACING: f32 = 2.0;
    pub const SUBTITLE_GAP: f32 = 4.0;
    pub const BLOCK_SPACING: f32 = 8.0;
    pub const CAPTION_CONTENT_LINE_LIMIT: usize = 3;
    pub const CAPTION_TOGGLE_GAP: f32 = 2.0;
    pub const HASHTAG_FONT_SIZE: f32 = 15.0;
    pub const HASHTAG_SPACING: f32 = 4.0;
    pub const EXTRA_LINE_HEIGHT: f32 = 0.0;
    pub const APPLIES_LINE_SPACING_AFTER_LAST_LINE: bool = false;
    pub const SNAPS_BASELINES_TO_PIXELS: bool = true;
    pub const STRIKETHROUGH_X_HEIGHT_OFFSET: f32 = 0.5;
    pub const STRIKETHROUGH_THICKNESS_EM: f32 = 1.0 / 15.0;
    pub const BOLD_TEXT_BUMPS_WEIGHT: bool = true;
    pub const FALLBACK_DISPLAY_SCALE: f32 = 3.0;
    pub const BITMAP_PIXEL_BUDGET: i64 = 3_000_000;
    pub const BITMAP_MAXIMUM_DIMENSION: i64 = 8_192;
    pub const MINIMUM_FONT_SIZE: f32 = 1.0;
}

/// concrete caption colors, safe to read off the main thread
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostRichTextColors {
    pub text: Hsla,
    pub secondary: Hsla,
    pub link: Hsla,
    pub hashtag: Hsla,
    pub code_foreground: Hsla,
    pub code_background: Hsla,
}

impl PostRichTextColors {
    pub fn new(
        text: Hsla,
        secondary: Hsla,
        link: Hsla,
        hashtag: Hsla,
        code_foreground: Hsla,
        code_background: Hsla,
    ) -> Self {
        Self {
            text,
            secondary,
            link,
            hashtag,
            code_foreground,
            code_background,
        }
    }

    pub fn from_palette(palette: &PostRowPalette) -> Self {
        let text = palette.text_primary;
        Self::new(
            text,
            palette.text_secondary,
            palette.link,
            palette.primary100,
            text,
            Hsla {
                a: text.a * PostRichTextCalibration::CODE_BACKGROUND_OPACITY,
                ..text
            },
        )
    }
}

/// everything a caption render depends on besides the text and the width
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostRichTextStyle {
    pub colors: PostRichTextColors,
    pub text_delta: f32,
    pub display_scale: f32,
    pub is_bold_text_enabled: bool,
}

impl PostRichTextStyle {
    pub fn new(colors: PostRichTextColors, text_delta: f32, display_scale: f32, is_bold_text_enabled: bool) -> Self {
        Self {
            colors,
            text_delta: if text_delta.is_finite() { text_delta } else { 0.0 },
            display_scale: if display_scale.is_finite() && display_scale > 0.0 {
                display_scale
            } else {
                PostRichTextCalibration::FALLBACK_DISPLAY_SCALE
            },
            is_bold_text_enabled,
        }
    }

    pub fn from_palette(palette: &PostRowPalette, spec: &PostRowLayoutSpec) -> Self {
        Self::new(
            PostRichTextColors::from_palette(palette),
            spec.text_delta,
            spec.display_scale,
            spec.is_bold_text_enabled,
        )
    }
}

/// a raw text range drawn semibold in color, never tappable
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostRichTextRoleMention {
    pub raw_range: NSRange,
    pub color: Hsla,
}

/// attribute keys the layout builder turns into tokens, plus the ellipsis mark
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostRichTextAttribute {
    Mention,
    Url,
    Hashtag,
    Truncation,
}

impl PostRichTextAttribute {
    pub const TOKEN_KEYS: [PostRichTextAttribute; 3] = [Self::Mention, Self::Hashtag, Self::Url];
}

/// one token per instance, separate spans keep adjacent tokens from merging during enumeration
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostRichTextTokenBox {
    pub kind: PostTextTokenKind,
}

impl PostRichTextTokenBox {
    pub fn new(kind: PostTextTokenKind) -> Self {
        Self { kind }
    }
}

/// paragraph style, the two values the line builder reads
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PostRichTextParagraphStyle {
    pub line_spacing: f32,
    pub paragraph_spacing: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct PostRichTextTokenSpan {
    key: PostRichTextAttribute,
    range: NSRange,
    value: Arc<PostRichTextTokenBox>,
}

/// a stretch of one font, color
/// kern and truncation mark as the line builder shapes it
#[derive(Clone, Debug, PartialEq)]
pub struct PostRichTextStyleRun {
    pub bytes: Range<usize>,
    pub font: RichFont,
    pub color: Hsla,
    pub kern: f32,
    pub is_truncation: bool,
}

/// attributed string as captions use it
/// kit string holds font and colors and line styles, the rest like kern and custom keys lives beside it
/// every string carries one paragraph style end to end
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostRichTextAttributedString {
    base: AttributedString,
    length: usize,
    paragraph_style: PostRichTextParagraphStyle,
    kerns: Vec<(NSRange, f32)>,
    tokens: Vec<PostRichTextTokenSpan>,
    truncations: Vec<NSRange>,
}

impl PostRichTextAttributedString {
    pub fn new(string: &str, attributes: AttributeContainer, paragraph_style: PostRichTextParagraphStyle) -> Self {
        Self {
            base: AttributedString::with_attributes(string, attributes),
            length: ns_range::length_utf16(string),
            paragraph_style,
            kerns: Vec::new(),
            tokens: Vec::new(),
            truncations: Vec::new(),
        }
    }

    pub fn string(&self) -> &str {
        self.base.characters()
    }

    /// length in utf16 units
    pub fn length(&self) -> usize {
        self.length
    }

    pub fn paragraph_style(&self) -> PostRichTextParagraphStyle {
        self.paragraph_style
    }

    pub fn attributes_at(&self, location: usize) -> Option<&AttributeContainer> {
        let byte = ns_range::utf16_to_byte_offset(self.string(), location)?;
        self.base.runs().find(|(range, _)| range.contains(&byte)).map(|(_, attributes)| attributes)
    }

    pub fn font_at(&self, location: usize) -> Option<RichFont> {
        self.attributes_at(location).and_then(|attributes| attributes.font)
    }

    pub fn foreground_color_at(&self, location: usize) -> Option<Hsla> {
        self.attributes_at(location).and_then(|attributes| attributes.foreground_color)
    }

    pub fn update(&mut self, range: NSRange, f: impl Fn(&mut AttributeContainer)) {
        self.base.update(range, f);
    }

    /// adds a token key over a range, the new value replaces what it covers
    pub fn add_token(&mut self, key: PostRichTextAttribute, value: PostRichTextTokenBox, range: NSRange) {
        self.remove_attribute(key, range);
        self.tokens.push(PostRichTextTokenSpan {
            key,
            range,
            value: Arc::new(value),
        });
    }

    /// removes a token key over a range, spans outside the range survive
    pub fn remove_attribute(&mut self, key: PostRichTextAttribute, range: NSRange) {
        let mut kept = Vec::with_capacity(self.tokens.len());
        for span in self.tokens.drain(..) {
            if span.key != key || span.range.intersection(range).is_none() {
                kept.push(span);
                continue;
            }
            if span.range.location < range.location {
                kept.push(PostRichTextTokenSpan {
                    key,
                    range: NSRange::new(span.range.location, range.location - span.range.location),
                    value: span.value.clone(),
                });
            }
            if span.range.upper_bound() > range.upper_bound() {
                kept.push(PostRichTextTokenSpan {
                    key,
                    range: NSRange::new(range.upper_bound(), span.range.upper_bound() - range.upper_bound()),
                    value: span.value.clone(),
                });
            }
        }
        self.tokens = kept;
    }

    pub fn add_kern(&mut self, range: NSRange, kern: f32) {
        self.kerns.push((range, kern));
    }

    pub fn add_truncation(&mut self, range: NSRange) {
        self.truncations.push(range);
    }

    pub fn append(&mut self, other: PostRichTextAttributedString) {
        let offset = self.length;
        if self.base.is_empty() {
            self.paragraph_style = other.paragraph_style;
        }
        self.base.append(other.base);
        self.length += other.length;
        let shift = |range: NSRange| NSRange::new(range.location + offset, range.length);
        self.kerns.extend(other.kerns.into_iter().map(|(range, kern)| (shift(range), kern)));
        self.truncations.extend(other.truncations.into_iter().map(shift));
        self.tokens.extend(other.tokens.into_iter().map(|span| PostRichTextTokenSpan {
            range: shift(span.range),
            ..span
        }));
    }

    /// substring of the range, none where the range cuts a surrogate pair
    pub fn attributed_substring(&self, range: NSRange) -> Option<PostRichTextAttributedString> {
        let bytes = ns_range::utf16_to_byte_range(self.string(), range)?;
        let mut base = AttributedString::default();
        for (run, attributes) in self.base.runs() {
            let start = run.start.max(bytes.start);
            let end = run.end.min(bytes.end);
            if start < end {
                base.append(AttributedString::with_attributes(&self.string()[start..end], attributes.clone()));
            }
        }
        let clip = |candidate: NSRange| {
            candidate
                .intersection(range)
                .map(|overlap| NSRange::new(overlap.location - range.location, overlap.length))
        };
        Some(PostRichTextAttributedString {
            base,
            length: range.length,
            paragraph_style: self.paragraph_style,
            kerns: self.kerns.iter().filter_map(|(kern_range, kern)| Some((clip(*kern_range)?, *kern))).collect(),
            tokens: self
                .tokens
                .iter()
                .filter_map(|span| {
                    Some(PostRichTextTokenSpan {
                        key: span.key,
                        range: clip(span.range)?,
                        value: span.value.clone(),
                    })
                })
                .collect(),
            truncations: self.truncations.iter().filter_map(|range| clip(*range)).collect(),
        })
    }

    pub fn token_spans(&self, key: PostRichTextAttribute) -> Vec<(NSRange, Arc<PostRichTextTokenBox>)> {
        let mut spans: Vec<(NSRange, Arc<PostRichTextTokenBox>)> = self
            .tokens
            .iter()
            .filter(|span| span.key == key)
            .map(|span| (span.range, span.value.clone()))
            .collect();
        spans.sort_by_key(|(range, _)| range.location);
        spans
    }

    pub fn background_runs(&self) -> Vec<(NSRange, Hsla)> {
        let mut result: Vec<(NSRange, Hsla)> = Vec::new();
        for (bytes, attributes) in self.base.runs() {
            let Some(color) = attributes.background_color else {
                continue;
            };
            let range = ns_range::byte_to_utf16_range(self.string(), bytes);
            match result.last_mut() {
                Some((last, last_color)) if *last_color == color && last.upper_bound() == range.location => {
                    last.length += range.length;
                }
                _ => result.push((range, color)),
            }
        }
        result
    }

    pub fn strikethrough_runs(&self) -> Vec<(NSRange, Option<RichFont>, Option<Hsla>)> {
        self.base
            .runs()
            .filter_map(|(bytes, attributes)| {
                let line = attributes.strikethrough_style?;
                Some((
                    ns_range::byte_to_utf16_range(self.string(), bytes),
                    attributes.font,
                    line.color.or(attributes.foreground_color),
                ))
            })
            .collect()
    }

    pub fn style_runs(&self, bytes: Range<usize>) -> Vec<PostRichTextStyleRun> {
        let text = self.string();
        let to_bytes = |range: &NSRange| ns_range::utf16_to_byte_range(text, *range);
        let kerns: Vec<(Range<usize>, f32)> =
            self.kerns.iter().filter_map(|(range, kern)| Some((to_bytes(range)?, *kern))).collect();
        let truncations: Vec<Range<usize>> = self.truncations.iter().filter_map(to_bytes).collect();

        let mut result = Vec::new();
        for (run, attributes) in self.base.runs() {
            let start = run.start.max(bytes.start);
            let end = run.end.min(bytes.end);
            if start >= end {
                continue;
            }
            let mut cuts = vec![start, end];
            for candidate in kerns.iter().map(|(range, _)| range).chain(truncations.iter()) {
                for cut in [candidate.start, candidate.end] {
                    if cut > start && cut < end {
                        cuts.push(cut);
                    }
                }
            }
            cuts.sort_unstable();
            cuts.dedup();
            for pair in cuts.windows(2) {
                let [from, to] = pair else {
                    continue;
                };
                let inside = |range: &Range<usize>| range.start <= *from && *to <= range.end;
                result.push(PostRichTextStyleRun {
                    bytes: *from..*to,
                    font: attributes
                        .font
                        .unwrap_or_else(|| RichFont::system(PostRichTextCalibration::PARAGRAPH_FONT_SIZE)),
                    color: attributes.foreground_color.unwrap_or_else(gpui::black),
                    kern: kerns.iter().find(|(range, _)| inside(range)).map_or(0.0, |(_, kern)| *kern),
                    is_truncation: truncations.iter().any(inside),
                });
            }
        }
        result
    }
}

/// a rich paragraph render string and the link subtitle shown under it
#[derive(Clone, Debug)]
pub struct PostRichTextParagraph {
    pub body: PostRichTextAttributedString,
    pub subtitle: Option<String>,
}

/// laid out paragraph and its optional subtitle line
#[derive(Clone)]
pub struct PostRichTextParagraphLayouts {
    pub body: Arc<PostTextLayout>,
    pub link_subtitle: Option<Arc<PostTextLayout>>,
}

pub enum PostRichTextRender {}

impl PostRichTextRender {
    /// system font at size, weight stepped up under bold text
    pub fn font(size: f32, weight: FontWeight, is_bold_text_enabled: bool) -> RichFont {
        RichFont::system_weight(
            size.max(PostRichTextCalibration::MINIMUM_FONT_SIZE),
            Self::legibility_weight(weight, is_bold_text_enabled),
        )
    }

    pub fn italic_font(size: f32, is_bold_text_enabled: bool) -> RichFont {
        let size = size.max(PostRichTextCalibration::MINIMUM_FONT_SIZE);
        if !is_bold_text_enabled || !PostRichTextCalibration::BOLD_TEXT_BUMPS_WEIGHT {
            return RichFont::system(size).italic();
        }
        Self::font(size, FontWeight::NORMAL, true).with_traits(FontDescriptorSymbolicTraits::TRAIT_ITALIC)
    }

    pub fn monospaced_font(size: f32, is_bold_text_enabled: bool) -> RichFont {
        RichFont {
            weight: Self::legibility_weight(FontWeight::NORMAL, is_bold_text_enabled),
            ..RichFont::system_design(size.max(PostRichTextCalibration::MINIMUM_FONT_SIZE), FontDesign::Monospaced)
        }
    }

    pub fn legibility_weight(weight: FontWeight, is_bold_text_enabled: bool) -> FontWeight {
        if !is_bold_text_enabled || !PostRichTextCalibration::BOLD_TEXT_BUMPS_WEIGHT {
            return weight;
        }
        if weight < FontWeight::NORMAL {
            FontWeight::NORMAL
        } else if weight < FontWeight::MEDIUM {
            FontWeight::SEMIBOLD
        } else if weight < FontWeight::BOLD {
            FontWeight::BOLD
        } else if weight < FontWeight::EXTRA_BOLD {
            FontWeight::EXTRA_BOLD
        } else {
            FontWeight::BLACK
        }
    }

    pub fn title_string(title: &str, style: &PostRichTextStyle) -> PostRichTextAttributedString {
        PostRichTextAttributedString::new(
            title,
            AttributeContainer {
                font: Some(Self::font(
                    PostRichTextCalibration::TITLE_FONT_SIZE + style.text_delta,
                    PostRichTextCalibration::TITLE_WEIGHT,
                    style.is_bold_text_enabled,
                )),
                foreground_color: Some(style.colors.text),
                ..AttributeContainer::default()
            },
            Self::paragraph_style(PostRichTextCalibration::TITLE_LINE_SPACING),
        )
    }

    /// one paragraph block with default options
    pub fn paragraph(
        text: &str,
        style: &PostRichTextStyle,
        role_mentions: &[PostRichTextRoleMention],
    ) -> PostRichTextParagraph {
        let font_size = (PostRichTextCalibration::PARAGRAPH_FONT_SIZE + style.text_delta)
            .max(PostRichTextCalibration::MINIMUM_FONT_SIZE);
        let preparation = FormattedText::interaction_preparation(
            text,
            &Self::geometry_style(font_size),
            &FormattedTextOptions {
                no_url_format: true,
                no_url_subtitle: true,
                ..FormattedTextOptions::default()
            },
            &[],
            None,
        );
        let display = preparation.geometry.display.as_str();
        let inline = RichTextMarkup::parse_inline(text);
        let inline_matches = inline.display == display;
        let colors = style.colors;
        let bold = style.is_bold_text_enabled;

        let mut body = PostRichTextAttributedString::new(
            display,
            AttributeContainer {
                font: Some(Self::font(font_size, FontWeight::NORMAL, bold)),
                foreground_color: Some(colors.text),
                ..AttributeContainer::default()
            },
            Self::paragraph_style(PostRichTextCalibration::PARAGRAPH_LINE_SPACING),
        );
        let length = body.length();

        if inline_matches {
            for (range, run_bold, run_italic) in RichTextMarkup::emphasis_runs(&inline.spans) {
                if !Self::is_valid(range, length) {
                    continue;
                }
                let run_font = match (run_bold, run_italic) {
                    (true, true) => Self::font(font_size, FontWeight::BOLD, bold)
                        .with_traits(FontDescriptorSymbolicTraits::TRAIT_ITALIC),
                    (true, false) => Self::font(font_size, FontWeight::BOLD, bold),
                    _ => Self::italic_font(font_size, bold),
                };
                body.update(range, |attributes| attributes.font = Some(run_font));
            }
            for span in inline.spans.iter().filter(|span| Self::is_valid(span.range, length)) {
                match span.style {
                    RichInlineStyle::Bold | RichInlineStyle::Italic => {}
                    RichInlineStyle::Strikethrough => {
                        body.update(span.range, |attributes| {
                            attributes.strikethrough_style = Some(LineStyle::single());
                        });
                    }
                    RichInlineStyle::Spoiler => {
                        body.update(span.range, |attributes| {
                            attributes.background_color = Some(colors.code_background);
                        });
                    }
                    RichInlineStyle::Code => {
                        let code_font = Self::monospaced_font(font_size * PostRichTextCalibration::CODE_FONT_SCALE, bold);
                        body.update(span.range, |attributes| {
                            attributes.font = Some(code_font);
                            attributes.foreground_color = Some(colors.code_foreground);
                            attributes.background_color = Some(colors.code_background);
                        });
                    }
                }
            }
        }

        for token in preparation.urls.iter().filter(|token| Self::is_valid(token.range, length)) {
            body.update(token.range, |attributes| attributes.foreground_color = Some(colors.link));
            if let Some(url) = Url::from_string(&token.tag) {
                body.add_token(
                    PostRichTextAttribute::Url,
                    PostRichTextTokenBox::new(PostTextTokenKind::Url(url)),
                    token.range,
                );
            }
        }

        for token in preparation.mentions.iter().filter(|token| Self::is_valid(token.range, length)) {
            body.update(token.range, |attributes| attributes.foreground_color = Some(colors.link));
            let link = RiseonlyPublicLink::PublicTag { tag: token.tag.clone() };
            if let Some(url) = link.web_url(AppEnvironment::current()) {
                body.add_token(
                    PostRichTextAttribute::Mention,
                    PostRichTextTokenBox::new(PostTextTokenKind::Mention {
                        tag: token.tag.clone(),
                        url,
                    }),
                    token.range,
                );
            }
        }

        let mut taken: Vec<NSRange> = preparation.urls.iter().chain(preparation.mentions.iter()).map(|token| token.range).collect();
        if inline_matches {
            taken.extend(inline.spans.iter().filter(|span| span.style == RichInlineStyle::Code).map(|span| span.range));
        }
        for token in FormattedText::hashtag_tokens(display, &taken) {
            if !Self::is_valid(token.range, length) {
                continue;
            }
            body.update(token.range, |attributes| attributes.foreground_color = Some(colors.hashtag));
            body.add_token(
                PostRichTextAttribute::Hashtag,
                PostRichTextTokenBox::new(PostTextTokenKind::Hashtag(token.tag.clone())),
                token.range,
            );
        }

        if inline_matches {
            for role in role_mentions {
                let Some(range) = RichTextMarkup::display_range(role.raw_range, &inline.removed_ranges) else {
                    continue;
                };
                if !Self::is_valid(range, length) {
                    continue;
                }
                let role_font = Self::font(font_size, FontWeight::SEMIBOLD, bold);
                body.update(range, |attributes| {
                    attributes.foreground_color = Some(role.color);
                    attributes.font = Some(role_font);
                });
                body.remove_attribute(PostRichTextAttribute::Mention, range);
                body.remove_attribute(PostRichTextAttribute::Url, range);
            }
        }

        PostRichTextParagraph { body, subtitle: None }
    }

    pub fn subtitle_string(subtitle: &str, style: &PostRichTextStyle) -> PostRichTextAttributedString {
        PostRichTextAttributedString::new(
            subtitle,
            AttributeContainer {
                font: Some(Self::font(
                    PostRichTextCalibration::SUBTITLE_FONT_SIZE,
                    FontWeight::NORMAL,
                    style.is_bold_text_enabled,
                )),
                foreground_color: Some(style.colors.secondary),
                ..AttributeContainer::default()
            },
            Self::paragraph_style(PostRichTextCalibration::SUBTITLE_LINE_SPACING),
        )
    }

    pub fn hashtags_string(hashtags: &[String], style: &PostRichTextStyle) -> PostRichTextAttributedString {
        let item_font = Self::font(
            PostRichTextCalibration::HASHTAG_FONT_SIZE + style.text_delta,
            FontWeight::NORMAL,
            style.is_bold_text_enabled,
        );
        let attributes = AttributeContainer {
            font: Some(item_font),
            foreground_color: Some(style.colors.hashtag),
            ..AttributeContainer::default()
        };
        let paragraph_style = Self::paragraph_style(0.0);
        let separator_kern = PostRichTextCalibration::HASHTAG_SPACING - Self::space_advance(&item_font);

        let mut result = PostRichTextAttributedString::default();
        for (index, tag) in hashtags.iter().enumerate() {
            if index > 0 {
                let mut separator = PostRichTextAttributedString::new(" ", attributes.clone(), paragraph_style);
                separator.add_kern(NSRange::new(0, 1), separator_kern);
                result.append(separator);
            }
            let mut item =
                PostRichTextAttributedString::new(&format!("#{tag}"), attributes.clone(), paragraph_style);
            let whole = NSRange::new(0, item.length());
            item.add_token(
                PostRichTextAttribute::Hashtag,
                PostRichTextTokenBox::new(PostTextTokenKind::Hashtag(tag.clone())),
                whole,
            );
            result.append(item);
        }
        result
    }

    pub fn title_layout(title: &str, style: &PostRichTextStyle, width: f32) -> Arc<PostTextLayout> {
        Arc::new(PostTextLayoutBuilder::layout(&Self::title_string(title, style), width, None, style.display_scale))
    }

    pub fn paragraph_layouts(
        text: &str,
        style: &PostRichTextStyle,
        width: f32,
        role_mentions: &[PostRichTextRoleMention],
        maximum_lines: Option<usize>,
    ) -> PostRichTextParagraphLayouts {
        let rendered = Self::paragraph(text, style, role_mentions);
        let body = PostTextLayoutBuilder::layout(&rendered.body, width, maximum_lines.map(|lines| lines as i64), style.display_scale);
        let link_subtitle = rendered.subtitle.map(|subtitle| {
            Arc::new(PostTextLayoutBuilder::layout(
                &Self::subtitle_string(&subtitle, style),
                width,
                None,
                style.display_scale,
            ))
        });
        PostRichTextParagraphLayouts {
            body: Arc::new(body),
            link_subtitle,
        }
    }

    pub fn caption_toggle_layout(is_expanded: bool, style: &PostRichTextStyle, width: f32) -> Arc<PostTextLayout> {
        let font_size = (PostRichTextCalibration::PARAGRAPH_FONT_SIZE + style.text_delta)
            .max(PostRichTextCalibration::MINIMUM_FONT_SIZE);
        let label = PostRichTextAttributedString::new(
            &tr(if is_expanded { "profile_bio_less" } else { "profile_bio_more" }),
            AttributeContainer {
                font: Some(Self::font(font_size, FontWeight::MEDIUM, style.is_bold_text_enabled)),
                foreground_color: Some(style.colors.secondary),
                ..AttributeContainer::default()
            },
            Self::paragraph_style(0.0),
        );
        Arc::new(PostTextLayoutBuilder::layout(&label, width, Some(1), style.display_scale))
    }

    /// single line, tail truncated when the tags overflow width
    pub fn hashtags_layout(hashtags: &[String], style: &PostRichTextStyle, width: f32) -> Arc<PostTextLayout> {
        Arc::new(PostTextLayoutBuilder::layout(
            &Self::hashtags_string(hashtags, style),
            width,
            Some(1),
            style.display_scale,
        ))
    }

    /// caption blocks stacked like the post copy, block frames are local and frame starts at origin
    /// none when nothing is written or the content has a code fence which goes to the hosted fallback
    /// line limit clamps content unless expanded, past it a more toggle shows and when expanded a less one
    #[allow(clippy::too_many_arguments)]
    pub fn caption_layout(
        title: &str,
        content: &str,
        hashtags: &[String],
        style: &PostRichTextStyle,
        width: f32,
        origin: Point<Pixels>,
        content_line_limit: Option<usize>,
        is_expanded: bool,
    ) -> Option<PostRowCaptionLayout> {
        if title.is_empty() && content.is_empty() && hashtags.is_empty() {
            return None;
        }
        let blocks = RichTextMarkup::blocks(content);
        if blocks.iter().any(|block| matches!(block, RichBlock::Code { .. })) {
            return None;
        }

        let width = if width.is_finite() { width.max(0.0) } else { 0.0 };
        let spacing = PostRichTextCalibration::BLOCK_SPACING;
        let mut cursor = 0.0_f32;
        let mut has_placed_child = false;
        let mut is_clamped = false;
        let mut toggle_block = None;

        let mut title_block = None;
        if !title.is_empty() {
            let layout = Self::title_layout(title, style, width);
            let height = f32::from(layout.size.height);
            title_block = Some(PostRowTextBlockLayout {
                frame: bounds(point(px(0.0), px(cursor)), size(px(width), px(height))),
                text: layout,
            });
            cursor += height;
            has_placed_child = true;
        }

        let mut paragraphs = Vec::new();
        if !content.is_empty() {
            if has_placed_child {
                cursor += spacing;
            }
            let mut has_placed_paragraph = false;
            let limit = content_line_limit.filter(|limit| *limit > 0);
            let mut remaining_lines = if is_expanded { None } else { limit };
            let mut placed_lines = 0;
            for block in blocks.iter() {
                let RichBlock::Paragraph(text) = block else {
                    continue;
                };
                if remaining_lines == Some(0) {
                    is_clamped = true;
                    break;
                }
                if has_placed_paragraph {
                    cursor += spacing;
                }
                let layouts = Self::paragraph_layouts(text, style, width, &[], remaining_lines);
                let line_count = layouts.body.lines.len();
                placed_lines += line_count;
                remaining_lines = remaining_lines.map(|remaining| remaining.saturating_sub(line_count));
                let body_height = f32::from(layouts.body.size.height);
                let body_frame = bounds(point(px(0.0), px(cursor)), size(px(width), px(body_height)));
                cursor += body_height;
                let mut subtitle_block = None;
                if layouts.body.is_truncated {
                    is_clamped = true;
                } else if let Some(subtitle) = layouts.link_subtitle {
                    let subtitle_height = f32::from(subtitle.size.height);
                    let top = cursor + PostRichTextCalibration::SUBTITLE_GAP;
                    subtitle_block = Some(PostRowTextBlockLayout {
                        frame: bounds(point(px(0.0), px(top)), size(px(width), px(subtitle_height))),
                        text: subtitle,
                    });
                    cursor = top + subtitle_height;
                }
                paragraphs.push(PostRowParagraphLayout {
                    body: PostRowTextBlockLayout {
                        frame: body_frame,
                        text: layouts.body,
                    },
                    link_subtitle: subtitle_block,
                });
                has_placed_paragraph = true;
            }
            let overflows_limit = is_expanded && limit.is_some_and(|limit| placed_lines > limit);
            if is_clamped || overflows_limit {
                let layout = Self::caption_toggle_layout(!is_clamped, style, width);
                let top = cursor + PostRichTextCalibration::CAPTION_TOGGLE_GAP;
                let toggle_size = layout.size;
                toggle_block = Some(PostRowTextBlockLayout {
                    frame: bounds(point(px(0.0), px(top)), toggle_size),
                    text: layout,
                });
                cursor = top + f32::from(toggle_size.height);
            }
            has_placed_child = true;
        }

        let mut hashtags_block = None;
        if !hashtags.is_empty() {
            if has_placed_child {
                cursor += spacing;
            }
            let layout = Self::hashtags_layout(hashtags, style, width);
            let height = f32::from(layout.size.height);
            hashtags_block = Some(PostRowTextBlockLayout {
                frame: bounds(point(px(0.0), px(cursor)), size(px(width), px(height))),
                text: layout,
            });
            cursor += height;
        }

        Some(PostRowCaptionLayout {
            frame: bounds(origin, size(px(width), px(cursor))),
            title: title_block,
            paragraphs,
            hashtags: hashtags_block,
            toggle: toggle_block,
        })
    }

    fn paragraph_style(line_spacing: f32) -> PostRichTextParagraphStyle {
        PostRichTextParagraphStyle {
            line_spacing,
            paragraph_spacing: 0.0,
        }
    }

    fn geometry_style(font_size: f32) -> FormattedTextStyle {
        let primary = gpui::black();
        let mut style = FormattedTextStyle::new(
            primary,
            FormattedTextStyle::standard_link_color(),
            Hsla { a: 0.6, ..primary },
        );
        style.font_size = font_size;
        style.subtitle_font_size = PostRichTextCalibration::SUBTITLE_FONT_SIZE;
        style.line_spacing = PostRichTextCalibration::PARAGRAPH_LINE_SPACING;
        style
    }

    fn space_advance(font: &RichFont) -> f32 {
        PostTextLayoutBuilder::text_width(" ", font)
    }

    fn is_valid(range: NSRange, length: usize) -> bool {
        range.length > 0 && range.upper_bound() <= length
    }
}
