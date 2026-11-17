use std::collections::{HashMap, HashSet};
use std::ops::Range;

use gpui::Hsla;
use rise_canon::RiseDocumentInlineMark;

use super::rise_document::{RiseDocumentReference, RiseDocumentRun};
use super::rise_document_editor_row::RiseDocumentEditorRowKind;
use crate::rich_text::rich_text_markup::FontDescriptorSymbolicTraits;
use crate::swiftui::rich_text::{AttributeContainer, AttributedString, LineStyle, RichFont};

/// fonts and colors one editor row draws with
/// resolved once per theme change and handed to every row, never rebuilt inside a text view callback
#[derive(Clone, Debug, PartialEq)]
pub struct RiseDocumentTextStyle {
    pub body: RichFont,
    pub mono: RichFont,
    pub heading_fonts: HashMap<i64, RichFont>,
    pub text_color: Hsla,
    pub secondary_color: Hsla,
    pub link_color: Hsla,
    pub code_color: Hsla,
    pub code_background: Hsla,
    pub spoiler_background: Hsla,
}

impl RiseDocumentTextStyle {
    pub fn font(&self, kind: &RiseDocumentEditorRowKind) -> RichFont {
        match kind {
            RiseDocumentEditorRowKind::Heading { level } => {
                self.heading_fonts.get(level).or_else(|| self.heading_fonts.get(&1)).copied().unwrap_or(self.body)
            }
            RiseDocumentEditorRowKind::Code { .. } | RiseDocumentEditorRowKind::Formula => self.mono,
            _ => self.body,
        }
    }
}

/// marks that cant be a plain text attribute
/// carried alongside so reading the runs back gives exactly what was written
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseDocumentAttributes {
    pub rise_document_marks: Option<Vec<String>>,
    pub rise_document_url: Option<String>,
    pub rise_document_reference: Option<RiseDocumentReference>,
}

/// one attribute run of the attributed string
/// a byte range of its text with the drawn attributes and the document attributes
#[derive(Clone, Debug, PartialEq)]
pub struct RiseDocumentAttributedRun {
    pub range: Range<usize>,
    pub attributes: AttributeContainer,
    pub document: RiseDocumentAttributes,
}

/// attributed string as the block editor holds it, text plus contiguous attribute runs covering it
/// each run has both visual and document attributes
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiseDocumentAttributedString {
    pub string: String,
    pub runs: Vec<RiseDocumentAttributedRun>,
}

impl RiseDocumentAttributedString {
    pub fn with_attributes(string: impl Into<String>, attributes: AttributeContainer) -> Self {
        let string = string.into();
        let runs = if string.is_empty() {
            Vec::new()
        } else {
            vec![RiseDocumentAttributedRun {
                range: 0..string.len(),
                attributes,
                document: RiseDocumentAttributes::default(),
            }]
        };
        Self { string, runs }
    }

    pub fn append(&mut self, other: RiseDocumentAttributedString) {
        let base = self.string.len();
        self.string.push_str(&other.string);
        self.runs.extend(other.runs.into_iter().map(|run| RiseDocumentAttributedRun {
            range: run.range.start + base..run.range.end + base,
            ..run
        }));
    }

    /// true when empty, byte length decides emptiness and slicing here
    pub fn is_empty(&self) -> bool {
        self.string.is_empty()
    }

    pub fn attributed_string(&self) -> AttributedString {
        let mut out = AttributedString::new(self.string.clone());
        for run in &self.runs {
            let attributes = run.attributes.clone();
            out.update_byte_range(run.range.clone(), move |container| *container = attributes.clone());
        }
        out
    }
}

/// converts between the document run list and the attributed string a block text view edits
/// runs is the direction that matters
/// the text view holds the truth while typing so the model is derived from it
pub enum RiseDocumentAttributedText {}

impl RiseDocumentAttributedText {
    pub fn attributed(
        runs: &[RiseDocumentRun],
        kind: &RiseDocumentEditorRowKind,
        style: &RiseDocumentTextStyle,
    ) -> RiseDocumentAttributedString {
        let mut result = RiseDocumentAttributedString::default();
        let base_font = style.font(kind);
        for run in runs {
            if run.text.is_empty() {
                continue;
            }
            let mut attributes = AttributeContainer {
                font: Some(base_font),
                foreground_color: Some(style.text_color),
                ..AttributeContainer::default()
            };
            apply(&run.marks, &mut attributes, base_font, style);
            let mut document = RiseDocumentAttributes::default();
            if !run.marks.is_empty() {
                let mut wires: Vec<String> = run.marks.iter().map(|mark| mark.wire().to_owned()).collect();
                wires.sort();
                document.rise_document_marks = Some(wires);
            }
            if let Some(url) = run.url.as_ref().filter(|url| !url.is_empty()) {
                document.rise_document_url = Some(url.clone());
            }
            if let Some(reference) = &run.reference {
                document.rise_document_reference = Some(reference.clone());
            }
            result.append(RiseDocumentAttributedString {
                string: run.text.clone(),
                runs: vec![RiseDocumentAttributedRun { range: 0..run.text.len(), attributes, document }],
            });
        }
        result
    }

    /// reads the model back out of the text view
    /// neighbour runs with equal marks merge so the same visible text gives the same run list and
    /// wire bytes
    pub fn runs(attributed: &RiseDocumentAttributedString) -> Vec<RiseDocumentRun> {
        let mut out: Vec<RiseDocumentRun> = Vec::new();
        for run in &attributed.runs {
            let Some(text) = attributed.string.get(run.range.clone()) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let mut marks: HashSet<RiseDocumentInlineMark> = HashSet::new();
            if let Some(wires) = &run.document.rise_document_marks {
                for wire in wires {
                    if let Some(mark) = RiseDocumentInlineMark::from_wire(wire) {
                        marks.insert(mark);
                    }
                }
            }
            let url = run.document.rise_document_url.clone();
            let reference = run.document.rise_document_reference.clone();
            if let Some(last) = out.last_mut()
                && last.marks == marks
                && last.url == url
                && last.reference.is_none()
                && reference.is_none()
            {
                last.text.push_str(text);
            } else {
                out.push(RiseDocumentRun::with(text, marks, url, reference));
            }
        }
        out
    }
}

fn apply(
    marks: &HashSet<RiseDocumentInlineMark>,
    attributes: &mut AttributeContainer,
    base: RichFont,
    style: &RiseDocumentTextStyle,
) {
    let mut traits = FontDescriptorSymbolicTraits::default();
    if marks.contains(&RiseDocumentInlineMark::Bold) {
        traits = traits.union(FontDescriptorSymbolicTraits::TRAIT_BOLD);
    }
    if marks.contains(&RiseDocumentInlineMark::Italic) {
        traits = traits.union(FontDescriptorSymbolicTraits::TRAIT_ITALIC);
    }
    if marks.contains(&RiseDocumentInlineMark::Code) {
        attributes.font = Some(style.mono);
        attributes.foreground_color = Some(style.code_color);
        attributes.background_color = Some(style.code_background);
    } else if traits != FontDescriptorSymbolicTraits::default() {
        attributes.font = Some(base.with_traits(traits));
    }
    if marks.contains(&RiseDocumentInlineMark::Underline) {
        attributes.underline_style = Some(LineStyle::single());
    }
    if marks.contains(&RiseDocumentInlineMark::Strikethrough) {
        attributes.strikethrough_style = Some(LineStyle::single());
    }
    if marks.contains(&RiseDocumentInlineMark::Spoiler) {
        attributes.background_color = Some(style.spoiler_background);
    }
    if marks.contains(&RiseDocumentInlineMark::Link) {
        attributes.foreground_color = Some(style.link_color);
        attributes.underline_style = Some(LineStyle::single());
    }
}
