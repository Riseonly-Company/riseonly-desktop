use std::ops::Range;

/// location and length in utf-16 units
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NSRange {
    pub location: usize,
    pub length: usize,
}

impl NSRange {
    pub const fn new(location: usize, length: usize) -> Self {
        Self { location, length }
    }

    /// range covering the whole string
    pub fn full(text: &str) -> Self {
        Self::new(0, length_utf16(text))
    }

    /// start of the range
    pub const fn lower_bound(&self) -> usize {
        self.location
    }

    /// end of the range, location plus length
    pub const fn upper_bound(&self) -> usize {
        self.location + self.length
    }

    /// true if location is inside the range
    pub const fn contains(&self, location: usize) -> bool {
        location >= self.location && location < self.upper_bound()
    }

    /// overlap of two ranges, none if empty (touching counts as empty)
    pub fn intersection(&self, other: NSRange) -> Option<NSRange> {
        let lower = self.location.max(other.location);
        let upper = self.upper_bound().min(other.upper_bound());
        if lower >= upper {
            return None;
        }
        Some(NSRange::new(lower, upper - lower))
    }

    /// smallest range covering both
    pub fn union(&self, other: NSRange) -> NSRange {
        let lower = self.location.min(other.location);
        let upper = self.upper_bound().max(other.upper_bound());
        NSRange::new(lower, upper - lower)
    }

    /// the range as a utf-16 offset range
    pub const fn utf16_range(&self) -> Range<usize> {
        self.location..self.upper_bound()
    }
}

impl From<Range<usize>> for NSRange {
    fn from(r: Range<usize>) -> Self {
        NSRange::new(r.start, r.end.saturating_sub(r.start))
    }
}

/// length of text in utf-16 units
pub fn length_utf16(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// utf-16 offset of a byte offset
/// a byte inside a scalar rounds down to that scalar start
pub fn byte_to_utf16_offset(text: &str, byte: usize) -> usize {
    let mut end = byte.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    length_utf16(&text[..end])
}

/// byte offset of a utf-16 offset, none past the end or between surrogate halves
pub fn utf16_to_byte_offset(text: &str, utf16: usize) -> Option<usize> {
    let mut units = 0usize;
    for (i, c) in text.char_indices() {
        if units == utf16 {
            return Some(i);
        }
        units += c.len_utf16();
        if units > utf16 {
            return None;
        }
    }
    (units == utf16).then_some(text.len())
}

/// byte range of a utf-16 range, none if out of bounds or not found or it cuts a surrogate pair
pub fn utf16_to_byte_range(text: &str, range: NSRange) -> Option<Range<usize>> {
    let start = utf16_to_byte_offset(text, range.location)?;
    let end = utf16_to_byte_offset(text, range.location.checked_add(range.length)?)?;
    Some(start..end)
}

/// utf-16 range of a byte range of text
pub fn byte_to_utf16_range(text: &str, range: Range<usize>) -> NSRange {
    let start = byte_to_utf16_offset(text, range.start);
    let end = byte_to_utf16_offset(text, range.end.max(range.start));
    NSRange::new(start, end - start)
}

/// substring of the range, none where foundation would raise
pub fn substring(text: &str, range: NSRange) -> Option<&str> {
    utf16_to_byte_range(text, range).map(|r| &text[r])
}

/// first match of needle, canonically equivalent spellings match (e plus U+0301 finds U+00E9)
/// none if not found, an empty needle is never found
pub fn range_of(text: &str, needle: &str) -> Option<NSRange> {
    crate::foundation::foundation_compare::ns_range_of(text, needle, crate::foundation::foundation_compare::CompareOptions::NONE, NSRange::full(text))
}
