use std::ops::Range;

use gpui::SharedString;

use crate::input_ui::text_boundaries;

/// One bullet per grapheme, never per char: an emoji with a skin-tone modifier
/// is four chars and must not leak its length through four bullets.
pub const SECURE_BULLET: char = '\u{2022}';

const BULLET_LEN: usize = SECURE_BULLET.len_utf8();

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DisplayLine {
    pub source: Range<usize>,
    pub display: Range<usize>,
}

/// What the element actually shapes, and how to get back to the document.
///
/// Three things can make the shaped string differ from the field's text: an
/// empty field shows its placeholder, a secure field shows bullets, and a
/// multiline field is shaped one hard line at a time. Every offset the rest of
/// the component holds is a source offset; this type is the only place that
/// knows how to project one onto the string that was shaped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DisplayText {
    pub text: SharedString,
    pub is_placeholder: bool,
    pub lines: Vec<DisplayLine>,
    source_len: usize,
    grapheme_offsets: Vec<usize>,
}

impl DisplayText {
    pub fn new(source: &str, placeholder: &SharedString, secure: bool, multiline: bool) -> Self {
        if source.is_empty() {
            let text = placeholder.clone();
            let lines = if placeholder.is_empty() {
                vec![DisplayLine {
                    source: 0..0,
                    display: 0..0,
                }]
            } else {
                split_lines(&text, |_| 0..0)
            };
            return Self {
                text,
                is_placeholder: true,
                lines,
                source_len: 0,
                grapheme_offsets: Vec::new(),
            };
        }

        if secure {
            let grapheme_offsets: Vec<usize> = text_boundaries::graphemes(source)
                .map(|(offset, _)| offset)
                .collect();
            let text: SharedString = std::iter::repeat_n(SECURE_BULLET, grapheme_offsets.len())
                .collect::<String>()
                .into();
            let lines = vec![DisplayLine {
                source: 0..source.len(),
                display: 0..text.len(),
            }];
            return Self {
                text,
                is_placeholder: false,
                lines,
                source_len: source.len(),
                grapheme_offsets,
            };
        }

        let text: SharedString = source.to_owned().into();
        let lines = if multiline {
            split_lines(&text, |range| range)
        } else {
            vec![DisplayLine {
                source: 0..source.len(),
                display: 0..source.len(),
            }]
        };

        Self {
            text,
            is_placeholder: false,
            lines,
            source_len: source.len(),
            grapheme_offsets: Vec::new(),
        }
    }

    pub fn is_secure(&self) -> bool {
        !self.grapheme_offsets.is_empty()
    }

    pub fn display_offset(&self, source_offset: usize) -> usize {
        if self.is_placeholder {
            return 0;
        }
        if self.grapheme_offsets.is_empty() {
            return source_offset.min(self.text.len());
        }
        self.grapheme_offsets
            .partition_point(|offset| *offset < source_offset)
            * BULLET_LEN
    }

    pub fn source_offset(&self, display_offset: usize) -> usize {
        if self.is_placeholder {
            return 0;
        }
        if self.grapheme_offsets.is_empty() {
            return display_offset.min(self.source_len);
        }
        self.grapheme_offsets
            .get(display_offset / BULLET_LEN)
            .copied()
            .unwrap_or(self.source_len)
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line_index_for_source(&self, source_offset: usize) -> usize {
        if self.is_placeholder {
            return 0;
        }
        self.lines
            .iter()
            .position(|line| source_offset <= line.source.end)
            .unwrap_or(self.lines.len().saturating_sub(1))
    }

    /// The display offset of `source_offset` measured from the start of its own
    /// line, which is what a `ShapedLine` indexes by.
    pub fn offset_within_line(&self, line_index: usize, source_offset: usize) -> usize {
        let Some(line) = self.lines.get(line_index) else {
            return 0;
        };
        self.display_offset(source_offset)
            .clamp(line.display.start, line.display.end)
            - line.display.start
    }

    pub fn source_offset_within_line(&self, line_index: usize, offset_in_line: usize) -> usize {
        let Some(line) = self.lines.get(line_index) else {
            return 0;
        };
        let display = (line.display.start + offset_in_line).min(line.display.end);
        self.source_offset(display)
            .clamp(line.source.start, line.source.end)
    }

    pub fn line_text(&self, line_index: usize) -> &str {
        self.lines
            .get(line_index)
            .map_or("", |line| &self.text[line.display.clone()])
    }

    /// `marked`, in source offsets, projected onto `line_index`'s own indices.
    pub fn marked_within_line(
        &self,
        line_index: usize,
        marked: &Range<usize>,
    ) -> Option<Range<usize>> {
        if self.is_placeholder {
            return None;
        }
        let line = self.lines.get(line_index)?;
        let start = self
            .display_offset(marked.start)
            .clamp(line.display.start, line.display.end)
            - line.display.start;
        let end = self
            .display_offset(marked.end)
            .clamp(line.display.start, line.display.end)
            - line.display.start;
        (end > start).then_some(start..end)
    }
}

fn split_lines(text: &str, source_for: impl Fn(Range<usize>) -> Range<usize>) -> Vec<DisplayLine> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (index, _) in text.match_indices('\n') {
        lines.push(DisplayLine {
            source: source_for(start..index),
            display: start..index,
        });
        start = index + 1;
    }
    lines.push(DisplayLine {
        source: source_for(start..text.len()),
        display: start..text.len(),
    });
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholder() -> SharedString {
        SharedString::new_static("Type here")
    }

    #[test]
    fn an_empty_field_shows_its_placeholder_and_maps_every_offset_to_zero() {
        let display = DisplayText::new("", &placeholder(), false, false);
        assert!(display.is_placeholder);
        assert_eq!(display.text.as_ref(), "Type here");
        assert_eq!(display.display_offset(4), 0);
        assert_eq!(display.source_offset(4), 0);
    }

    #[test]
    fn a_plain_single_line_field_maps_offsets_through_unchanged() {
        let display = DisplayText::new("Привет", &placeholder(), false, false);
        assert!(!display.is_placeholder);
        assert_eq!(display.text.as_ref(), "Привет");
        assert_eq!(display.display_offset(6), 6);
        assert_eq!(display.source_offset(6), 6);
        assert_eq!(display.line_count(), 1);
    }

    #[test]
    fn a_secure_field_shows_one_bullet_per_grapheme() {
        let display = DisplayText::new("a👍🏻б", &placeholder(), true, false);
        assert_eq!(display.text.as_ref(), "•••");
        assert_eq!(display.display_offset(0), 0);
        assert_eq!(display.display_offset(1), BULLET_LEN);
        assert_eq!(display.display_offset("a👍🏻".len()), 2 * BULLET_LEN);
        assert_eq!(display.display_offset("a👍🏻б".len()), 3 * BULLET_LEN);
    }

    #[test]
    fn a_secure_field_maps_a_bullet_back_to_its_grapheme() {
        let source = "a👍🏻б";
        let display = DisplayText::new(source, &placeholder(), true, false);
        assert_eq!(display.source_offset(0), 0);
        assert_eq!(display.source_offset(BULLET_LEN), 1);
        assert_eq!(display.source_offset(2 * BULLET_LEN), "a👍🏻".len());
        assert_eq!(display.source_offset(9 * BULLET_LEN), source.len());
        for offset in 0..=display.text.len() {
            assert!(source.is_char_boundary(display.source_offset(offset)));
        }
    }

    #[test]
    fn a_multiline_field_splits_on_hard_newlines_only() {
        let display = DisplayText::new("one\ntwo\n", &placeholder(), false, true);
        assert_eq!(display.line_count(), 3);
        assert_eq!(display.lines[0].source, 0..3);
        assert_eq!(display.lines[1].source, 4..7);
        assert_eq!(display.lines[2].source, 8..8);
        assert_eq!(display.line_text(1), "two");
    }

    #[test]
    fn a_single_line_field_never_splits_even_if_the_text_has_a_newline() {
        let display = DisplayText::new("one\ntwo", &placeholder(), false, false);
        assert_eq!(display.line_count(), 1);
    }

    #[test]
    fn an_offset_resolves_to_the_line_that_contains_it() {
        let display = DisplayText::new("one\ntwo", &placeholder(), false, true);
        assert_eq!(display.line_index_for_source(0), 0);
        assert_eq!(display.line_index_for_source(3), 0);
        assert_eq!(display.line_index_for_source(4), 1);
        assert_eq!(display.line_index_for_source(7), 1);
        assert_eq!(display.offset_within_line(1, 6), 2);
        assert_eq!(display.source_offset_within_line(1, 2), 6);
    }

    #[test]
    fn a_marked_range_is_clipped_to_the_line_it_falls_on() {
        let display = DisplayText::new("one\ntwo", &placeholder(), false, true);
        assert_eq!(display.marked_within_line(0, &(0..3)), Some(0..3));
        assert_eq!(display.marked_within_line(1, &(0..3)), None);
        assert_eq!(display.marked_within_line(1, &(4..6)), Some(0..2));
    }

    #[test]
    fn a_secure_field_never_marks_a_range_off_a_bullet_boundary() {
        let display = DisplayText::new("日本語", &placeholder(), true, false);
        let marked = display.marked_within_line(0, &(0..3)).unwrap();
        assert_eq!(marked, 0..BULLET_LEN);
        assert!(display.text.is_char_boundary(marked.end));
    }
}
