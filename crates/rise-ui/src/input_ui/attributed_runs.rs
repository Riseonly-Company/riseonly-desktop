use std::any::Any;
use std::fmt;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{Bounds, Pixels, RenderImage};

use crate::swiftui::rich_text::AttributeContainer;

pub const ATTACHMENT_CHARACTER: char = '\u{FFFC}';

static NEXT_ATTACHMENT_ID: AtomicU64 = AtomicU64::new(1);

/// which attachment object a run holds
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TextAttachmentId(u64);

/// text attachment with an image, drawn fitted and centered in bounds
/// which advances the pen by its width, origin y is measured up from the baseline
#[derive(Clone)]
pub struct TextAttachment {
    pub image: Option<Arc<RenderImage>>,
    pub bounds: Bounds<Pixels>,
    id: TextAttachmentId,
}

impl TextAttachment {
    pub fn new(image: Option<Arc<RenderImage>>, bounds: Bounds<Pixels>) -> Self {
        Self {
            image,
            bounds,
            id: TextAttachmentId(NEXT_ATTACHMENT_ID.fetch_add(1, Ordering::Relaxed)),
        }
    }

    pub fn id(&self) -> TextAttachmentId {
        self.id
    }
}

impl PartialEq for TextAttachment {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl fmt::Debug for TextAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TextAttachment")
            .field("id", &self.id)
            .field("has_image", &self.image.is_some())
            .field("bounds", &self.bounds)
            .finish()
    }
}

trait RunTagValue: Any {
    fn as_any(&self) -> &dyn Any;
    fn equals(&self, other: &dyn RunTagValue) -> bool;
}

impl<T: Any + PartialEq> RunTagValue for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn equals(&self, other: &dyn RunTagValue) -> bool {
        other
            .as_any()
            .downcast_ref::<T>()
            .is_some_and(|other| other == self)
    }
}

/// one custom attribute value of any PartialEq type
/// tags are equal only with the same type and equal values, which decides if neighbour runs merge
#[derive(Clone)]
pub struct RunTag(Rc<dyn RunTagValue>);

impl RunTag {
    pub fn new<T: Any + PartialEq>(value: T) -> Self {
        Self(Rc::new(value))
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        let value: &dyn RunTagValue = self.0.as_ref();
        value.as_any().downcast_ref::<T>()
    }
}

impl PartialEq for RunTag {
    fn eq(&self, other: &Self) -> bool {
        let value: &dyn RunTagValue = self.0.as_ref();
        value.equals(other.0.as_ref())
    }
}

impl fmt::Debug for RunTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RunTag(..)")
    }
}

/// attributes of one run, typing attributes have the same shape
#[derive(Clone, PartialEq, Debug, Default)]
pub struct RunAttributes {
    pub attributes: AttributeContainer,
    pub tag: Option<RunTag>,
    pub attachment: Option<TextAttachment>,
}

impl RunAttributes {
    pub fn new(attributes: AttributeContainer) -> Self {
        Self {
            attributes,
            tag: None,
            attachment: None,
        }
    }

    pub fn tagged<T: Any + PartialEq>(attributes: AttributeContainer, tag: T) -> Self {
        Self {
            attributes,
            tag: Some(RunTag::new(tag)),
            attachment: None,
        }
    }

    pub fn with_attachment(self, attachment: TextAttachment) -> Self {
        Self {
            attachment: Some(attachment),
            ..self
        }
    }

    /// what typing attributes keep of these, typing never uses an attachment
    pub fn without_attachment(self) -> Self {
        Self {
            attachment: None,
            ..self
        }
    }

    /// the custom attribute when it is a T
    pub fn tag<T: Any>(&self) -> Option<&T> {
        self.tag.as_ref().and_then(RunTag::downcast_ref::<T>)
    }
}

#[derive(Clone, PartialEq, Debug)]
struct Run {
    len: usize,
    attributes: RunAttributes,
}

/// runs over a text in utf8 bytes
/// they cover the text end to end with no empty run and equal neighbours are merged
#[derive(Clone, PartialEq, Debug, Default)]
pub struct AttributedRuns {
    runs: Vec<Run>,
}

impl AttributedRuns {
    /// one run of attributes over len bytes, no run at all for an empty text
    pub fn new(len: usize, attributes: RunAttributes) -> Self {
        Self::from_pieces([(len, attributes)])
    }

    /// consecutive pieces as byte length plus attributes
    pub fn from_pieces(pieces: impl IntoIterator<Item = (usize, RunAttributes)>) -> Self {
        let mut runs = Self {
            runs: pieces
                .into_iter()
                .map(|(len, attributes)| Run { len, attributes })
                .collect(),
        };
        runs.coalesce();
        runs
    }

    /// byte length the runs cover
    pub fn len(&self) -> usize {
        self.runs.iter().map(|run| run.len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// each run as byte range plus attributes, in order
    pub fn runs(&self) -> impl Iterator<Item = (Range<usize>, &RunAttributes)> {
        let mut offset = 0;
        self.runs.iter().map(move |run| {
            let start = offset;
            offset += run.len;
            (start..offset, &run.attributes)
        })
    }

    /// every attachment drawn over the text, byte range of each U+FFFC inside a run that carries one
    /// text is the string the runs cover
    pub fn attachments<'a>(
        &'a self,
        text: &'a str,
    ) -> impl Iterator<Item = (Range<usize>, &'a TextAttachment)> + 'a {
        self.runs().flat_map(move |(range, attributes)| {
            let attachment = attributes.attachment.as_ref();
            let start = range.start;
            text.get(range)
                .into_iter()
                .flat_map(|piece| piece.char_indices())
                .filter(|(_, character)| *character == ATTACHMENT_CHARACTER)
                .filter_map(move |(at, character)| {
                    let end = start + at + character.len_utf8();
                    attachment.map(|attachment| (start + at..end, attachment))
                })
        })
    }

    /// tallest attachment, a row must be at least this tall
    pub fn tallest_attachment(&self, text: &str) -> Option<Pixels> {
        self.attachments(text)
            .map(|(_, attachment)| attachment.bounds.size.height)
            .reduce(|tallest, height| tallest.max(height))
    }

    /// attributes of the char starting at byte, none at or past the end
    pub fn attributes_at(&self, byte: usize) -> Option<&RunAttributes> {
        self.runs()
            .find(|(range, _)| range.contains(&byte))
            .map(|(_, attributes)| attributes)
    }

    /// what text typed over range takes when no typing attributes are set, first replaced char
    /// else the char before the caret, else the first char of the text
    pub fn inherited_for(&self, range: &Range<usize>) -> Option<RunAttributes> {
        let byte = if !range.is_empty() || range.start == 0 {
            range.start
        } else {
            range.start - 1
        };
        self.attributes_at(byte)
            .or_else(|| self.runs.last().map(|run| &run.attributes))
            .cloned()
            .map(RunAttributes::without_attachment)
    }

    /// replaces the bytes in range with inserted_len bytes of attributes
    /// runs after the edit shift and runs across its edges are cut
    pub fn splice(&mut self, range: Range<usize>, inserted_len: usize, attributes: RunAttributes) {
        let total = self.len();
        let start = range.start.min(total);
        let end = range.end.clamp(start, total);
        self.split_at(start);
        self.split_at(end);

        let mut before = Vec::with_capacity(self.runs.len() + 1);
        let mut after = Vec::new();
        let mut offset = 0;
        for run in std::mem::take(&mut self.runs) {
            let run_start = offset;
            offset += run.len;
            if offset <= start {
                before.push(run);
            } else if run_start >= end {
                after.push(run);
            }
        }
        before.push(Run {
            len: inserted_len,
            attributes,
        });
        before.extend(after);
        self.runs = before;
        self.coalesce();
    }

    /// rewrites the attributes of the bytes in range, cutting runs at its edges
    pub fn update(&mut self, range: Range<usize>, f: impl Fn(&mut RunAttributes)) {
        let total = self.len();
        let start = range.start.min(total);
        let end = range.end.clamp(start, total);
        if start == end {
            return;
        }
        self.split_at(start);
        self.split_at(end);
        let mut offset = 0;
        for run in &mut self.runs {
            let run_start = offset;
            offset += run.len;
            if run_start >= start && offset <= end {
                f(&mut run.attributes);
            }
        }
        self.coalesce();
    }

    pub fn set(&mut self, range: Range<usize>, attributes: &RunAttributes) {
        self.update(range, |run| *run = attributes.clone());
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
        let tail = Run {
            len: self.runs[index].len - head,
            attributes: self.runs[index].attributes.clone(),
        };
        self.runs[index].len = head;
        self.runs.insert(index + 1, tail);
    }

    fn coalesce(&mut self) {
        let mut merged: Vec<Run> = Vec::with_capacity(self.runs.len());
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

#[cfg(test)]
#[path = "attributed_runs_tests.rs"]
mod tests;
