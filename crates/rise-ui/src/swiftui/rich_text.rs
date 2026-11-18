use std::ops::{Add, AddAssign, Range};

use gpui::{
    App, ElementId, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveText, IntoElement,
    RenderOnce, SharedString, StrikethroughStyle, StyledText, UnderlineStyle, Window, px,
};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_range::{self, NSRange};
use rise_platform::text_capabilities::monospaced_family;

use super::open_url::open_url;

pub use crate::swiftui::font::FontDesign;

/// system font with size and weight and design, optionally italic
/// as an attribute value
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RichFont {
    pub size: f32,
    pub weight: FontWeight,
    pub design: FontDesign,
    pub italic: bool,
}

impl RichFont {
    pub fn system(size: f32) -> Self {
        Self::system_weight(size, FontWeight::NORMAL)
    }

    pub fn system_weight(size: f32, weight: FontWeight) -> Self {
        Self {
            size,
            weight,
            design: FontDesign::Default,
            italic: false,
        }
    }

    pub fn system_design(size: f32, design: FontDesign) -> Self {
        Self {
            design,
            ..Self::system(size)
        }
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
}

/// single line style, optionally with its own color
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct LineStyle {
    pub color: Option<Hsla>,
}

impl LineStyle {
    /// single line style
    pub const fn single() -> Self {
        Self { color: None }
    }

    pub const fn single_color(color: Hsla) -> Self {
        Self { color: Some(color) }
    }
}

/// attributes set on runs, font and colors and underline and strikethrough and link
/// none is unset so the run inherits from the enclosing text
#[derive(Clone, PartialEq, Debug, Default)]
pub struct AttributeContainer {
    pub font: Option<RichFont>,
    pub foreground_color: Option<Hsla>,
    pub background_color: Option<Hsla>,
    pub underline_style: Option<LineStyle>,
    pub strikethrough_style: Option<LineStyle>,
    pub link: Option<Url>,
}

impl AttributeContainer {
    /// gpui highlight of a run, a link takes the tint
    pub fn highlight_style(&self, tint: Option<Hsla>) -> HighlightStyle {
        let color = match (self.link.is_some(), tint) {
            (true, Some(tint)) => Some(tint),
            _ => self.foreground_color,
        };
        let fade_out = color.filter(|color| color.a < 1.0).map(|color| 1.0 - color.a.max(0.0));
        let color = color.map(|color| if color.a < 1.0 { Hsla { a: 1.0, ..color } } else { color });
        HighlightStyle {
            color,
            font_weight: self.font.map(|font| font.weight),
            font_style: self.font.map(|font| {
                if font.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                }
            }),
            background_color: self.background_color,
            underline: self.underline_style.map(|line| UnderlineStyle {
                thickness: px(1.0),
                color: line.color,
                wavy: false,
            }),
            strikethrough: self.strikethrough_style.map(|line| StrikethroughStyle {
                thickness: px(1.0),
                color: line.color,
            }),
            fade_out,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
struct AttributedRun {
    len: usize,
    attributes: AttributeContainer,
}

/// attributed string, runs cover the text end to end with no empty run
/// equal neighbours are merged
#[derive(Clone, PartialEq, Debug, Default)]
pub struct AttributedString {
    text: String,
    runs: Vec<AttributedRun>,
}

impl AttributedString {
    /// attributed string from plain text
    pub fn new(text: impl Into<String>) -> Self {
        Self::with_attributes(text, AttributeContainer::default())
    }

    pub fn with_attributes(text: impl Into<String>, attributes: AttributeContainer) -> Self {
        let text = text.into();
        let runs = if text.is_empty() {
            Vec::new()
        } else {
            vec![AttributedRun {
                len: text.len(),
                attributes,
            }]
        };
        Self { text, runs }
    }

    /// the plain text
    pub fn characters(&self) -> &str {
        &self.text
    }

    /// true if there is no text
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// runs with their byte range in the text and attributes
    pub fn runs(&self) -> impl Iterator<Item = (Range<usize>, &AttributeContainer)> {
        let mut offset = 0;
        self.runs.iter().map(move |run| {
            let range = offset..offset + run.len;
            offset += run.len;
            (range, &run.attributes)
        })
    }

    /// edits attributes in an NSRange, false if the range doesnt map onto the text
    /// out of bounds or cutting a surrogate pair, then nothing changes
    pub fn update(&mut self, range: NSRange, f: impl Fn(&mut AttributeContainer)) -> bool {
        let Some(bytes) = ns_range::utf16_to_byte_range(&self.text, range) else {
            return false;
        };
        self.update_bytes(bytes, f);
        true
    }

    /// edits attributes in a byte range, false when off a char boundary
    pub fn update_byte_range(&mut self, range: Range<usize>, f: impl Fn(&mut AttributeContainer)) -> bool {
        if range.start > range.end
            || !self.text.is_char_boundary(range.start)
            || !self.text.is_char_boundary(range.end)
        {
            return false;
        }
        self.update_bytes(range, f);
        true
    }

    /// edits attributes of every run
    pub fn update_all(&mut self, f: impl Fn(&mut AttributeContainer)) {
        self.update_bytes(0..self.text.len(), f);
    }

    /// range of needle in utf16 units, none if absent
    pub fn range_of(&self, needle: &str) -> Option<NSRange> {
        ns_range::range_of(&self.text, needle)
    }

    /// replaces a range, false if the range doesnt map onto the text
    pub fn replace_subrange(&mut self, range: NSRange, with: AttributedString) -> bool {
        let Some(bytes) = ns_range::utf16_to_byte_range(&self.text, range) else {
            return false;
        };
        self.split_at(bytes.start);
        self.split_at(bytes.end);
        let first = self.run_index_at(bytes.start);
        let last = self.run_index_at(bytes.end);
        self.text.replace_range(bytes, &with.text);
        self.runs.splice(first..last, with.runs);
        self.coalesce();
        true
    }

    pub fn append(&mut self, other: AttributedString) {
        self.text.push_str(&other.text);
        self.runs.extend(other.runs);
        self.coalesce();
    }

    /// highlight runs for StyledText, runs with nothing set are left out
    pub fn highlights(&self, tint: Option<Hsla>) -> Vec<(Range<usize>, HighlightStyle)> {
        self.runs()
            .map(|(range, attributes)| (range, attributes.highlight_style(tint)))
            .filter(|(_, style)| *style != HighlightStyle::default())
            .collect()
    }

    /// byte ranges and targets of the link runs, in text order
    pub fn link_ranges(&self) -> Vec<(Range<usize>, Url)> {
        self.runs()
            .filter_map(|(range, attributes)| attributes.link.clone().map(|url| (range, url)))
            .collect()
    }

    /// byte ranges of runs with a monospaced font
    pub fn monospaced_ranges(&self) -> Vec<Range<usize>> {
        self.runs()
            .filter(|(_, attributes)| {
                attributes
                    .font
                    .is_some_and(|font| font.design == FontDesign::Monospaced)
            })
            .map(|(range, _)| range)
            .collect()
    }

    fn update_bytes(&mut self, bytes: Range<usize>, f: impl Fn(&mut AttributeContainer)) {
        if bytes.is_empty() {
            return;
        }
        self.split_at(bytes.start);
        self.split_at(bytes.end);
        let mut offset = 0;
        for run in &mut self.runs {
            if offset >= bytes.start && offset + run.len <= bytes.end {
                f(&mut run.attributes);
            }
            offset += run.len;
        }
        self.coalesce();
    }

    fn split_at(&mut self, byte: usize) {
        let mut offset = 0;
        let found = self.runs.iter().enumerate().find_map(|(index, run)| {
            let start = offset;
            offset += run.len;
            (byte > start && byte < offset).then(|| (index, byte - start))
        });
        let Some((index, head)) = found else {
            return;
        };
        let tail = AttributedRun {
            len: self.runs[index].len - head,
            attributes: self.runs[index].attributes.clone(),
        };
        self.runs[index].len = head;
        self.runs.insert(index + 1, tail);
    }

    fn run_index_at(&self, byte: usize) -> usize {
        let mut offset = 0;
        for (index, run) in self.runs.iter().enumerate() {
            if offset >= byte {
                return index;
            }
            offset += run.len;
        }
        self.runs.len()
    }

    fn coalesce(&mut self) {
        let mut merged: Vec<AttributedRun> = Vec::with_capacity(self.runs.len());
        for run in self.runs.drain(..) {
            if run.len == 0 {
                continue;
            }
            match merged.last_mut() {
                Some(last) if last.attributes == run.attributes => last.len += run.len,
                _ => merged.push(run),
            }
        }
        self.runs = merged;
    }
}

impl From<&str> for AttributedString {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for AttributedString {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl AddAssign for AttributedString {
    fn add_assign(&mut self, rhs: AttributedString) {
        self.append(rhs);
    }
}

impl Add for AttributedString {
    type Output = AttributedString;

    fn add(mut self, rhs: AttributedString) -> AttributedString {
        self.append(rhs);
        self
    }
}

/// rich text from an attributed string, put it in a MainText for the base style
/// links open through the open url env read at render
#[derive(IntoElement)]
pub struct RichText {
    id: ElementId,
    text: AttributedString,
    tint: Option<Hsla>,
    monospaced_family: SharedString,
}

impl RichText {
    pub fn new(id: impl Into<ElementId>, text: AttributedString) -> Self {
        Self {
            id: id.into(),
            text,
            tint: None,
            monospaced_family: SharedString::new_static(monospaced_family()),
        }
    }

    /// color of link runs
    pub fn tint(mut self, tint: Option<Hsla>) -> Self {
        self.tint = tint;
        self
    }

    /// face used for monospaced runs, platform monospaced family unless overridden
    pub fn monospaced_family(mut self, family: impl Into<SharedString>) -> Self {
        self.monospaced_family = family.into();
        self
    }

    /// byte ranges of monospaced runs with the face they are drawn in
    pub fn font_family_overrides(&self) -> Vec<(Range<usize>, SharedString)> {
        self.text
            .monospaced_ranges()
            .into_iter()
            .map(|range| (range, self.monospaced_family.clone()))
            .collect()
    }
}

impl RenderOnce for RichText {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let characters: SharedString = self.text.characters().to_owned().into();
        let overrides = self.font_family_overrides();
        let mut styled = StyledText::new(characters).with_highlights(self.text.highlights(self.tint));
        if !overrides.is_empty() {
            styled = styled.with_font_family_overrides(overrides);
        }
        let links = self.text.link_ranges();
        if links.is_empty() {
            return styled.into_any_element();
        }
        let action = open_url(cx);
        let (ranges, urls): (Vec<Range<usize>>, Vec<Url>) = links.into_iter().unzip();
        InteractiveText::new(self.id, styled)
            .on_click(ranges, move |index, window, cx| {
                if let Some(url) = urls.get(index) {
                    action.call_as_function(url, window, cx);
                }
            })
            .into_any_element()
    }
}
