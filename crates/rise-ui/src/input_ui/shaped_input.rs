use std::ops::Range;

use gpui::{Font, Hsla, Pixels, ShapedLine, SharedString, TextRun, UnderlineStyle, Window, px};
use smallvec::SmallVec;

use crate::input_ui::display_text::DisplayText;

/// Everything a shaped result depends on.
#[derive(Clone, PartialEq, Debug)]
pub struct ShapeKey {
    pub revision: u64,
    pub font_size: Pixels,
    pub font: Font,
    pub color: Hsla,
    pub marked: Option<Range<usize>>,
    pub visible: Range<usize>,
}

/// The visible lines, shaped.
///
/// `visible` is a range of hard-line indices, not the whole document; every row
/// is exactly one line height tall.
pub struct ShapedInput {
    key: ShapeKey,
    lines: Vec<ShapedLine>,
}

impl ShapedInput {
    pub fn shape(
        key: ShapeKey,
        display: &DisplayText,
        underline: UnderlineStyle,
        window: &Window,
    ) -> Self {
        let mut lines = Vec::with_capacity(key.visible.len());
        for line_index in key.visible.clone() {
            let Some(line) = display.lines.get(line_index) else {
                break;
            };
            let text: SharedString = display.text[line.display.clone()].to_owned().into();
            let runs = runs_for_line(text.len(), display, line_index, &key, underline);
            lines.push(
                window
                    .text_system()
                    .shape_line(text, key.font_size, &runs, None),
            );
        }

        Self { key, lines }
    }

    pub fn matches(&self, key: &ShapeKey) -> bool {
        self.key == *key
    }

    pub fn visible(&self) -> Range<usize> {
        self.key.visible.clone()
    }

    pub fn line(&self, line_index: usize) -> Option<&ShapedLine> {
        line_index
            .checked_sub(self.key.visible.start)
            .and_then(|index| self.lines.get(index))
    }

    pub fn x_for(&self, line_index: usize, offset_in_line: usize) -> Pixels {
        self.line(line_index)
            .map_or(px(0.0), |line| line.x_for_index(offset_in_line))
    }

    pub fn width(&self, line_index: usize) -> Pixels {
        self.line(line_index).map_or(px(0.0), ShapedLine::width)
    }

    /// The byte offset within `line_index` closest to `x`.
    pub fn offset_for_x(&self, line_index: usize, x: Pixels) -> usize {
        self.line(line_index)
            .map_or(0, |line| line.closest_index_for_x(x))
    }
}

fn runs_for_line(
    len: usize,
    display: &DisplayText,
    line_index: usize,
    key: &ShapeKey,
    underline: UnderlineStyle,
) -> SmallVec<[TextRun; 3]> {
    let base = TextRun {
        len,
        font: key.font.clone(),
        color: key.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };

    let marked = key
        .marked
        .as_ref()
        .and_then(|marked| display.marked_within_line(line_index, marked))
        .filter(|marked| marked.end <= len);

    let Some(marked) = marked else {
        return SmallVec::from_iter([base].into_iter().filter(|run| run.len > 0));
    };

    SmallVec::from_iter(
        [
            TextRun {
                len: marked.start,
                ..base.clone()
            },
            TextRun {
                len: marked.end - marked.start,
                underline: Some(underline),
                ..base.clone()
            },
            TextRun {
                len: len - marked.end,
                ..base
            },
        ]
        .into_iter()
        .filter(|run| run.len > 0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{FontStyle, FontWeight, SharedString, hsla};

    fn key(marked: Option<Range<usize>>) -> ShapeKey {
        ShapeKey {
            revision: 0,
            font_size: px(14.0),
            font: Font {
                family: SharedString::new_static("Inter"),
                features: gpui::FontFeatures::default(),
                fallbacks: None,
                weight: FontWeight::NORMAL,
                style: FontStyle::Normal,
            },
            color: hsla(0.0, 0.0, 1.0, 1.0),
            marked,
            visible: 0..1,
        }
    }

    fn underline() -> UnderlineStyle {
        UnderlineStyle {
            thickness: px(1.0),
            color: None,
            wavy: false,
        }
    }

    #[test]
    fn an_unmarked_line_is_one_run() {
        let display = DisplayText::new("привет", &SharedString::default(), false, false);
        let runs = runs_for_line(display.text.len(), &display, 0, &key(None), underline());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len, display.text.len());
        assert!(runs[0].underline.is_none());
    }

    #[test]
    fn a_marked_range_becomes_its_own_underlined_run() {
        let display = DisplayText::new("にほんご", &SharedString::default(), false, false);
        let runs = runs_for_line(
            display.text.len(),
            &display,
            0,
            &key(Some(3..9)),
            underline(),
        );
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].len, 3);
        assert_eq!(runs[1].len, 6);
        assert_eq!(runs[2].len, 3);
        assert!(runs[1].underline.is_some());
        assert_eq!(
            runs.iter().map(|run| run.len).sum::<usize>(),
            display.text.len(),
            "runs must cover the line exactly or shaping silently drops glyphs"
        );
    }

    #[test]
    fn a_mark_that_starts_at_zero_leaves_no_empty_leading_run() {
        let display = DisplayText::new("abc", &SharedString::default(), false, false);
        let runs = runs_for_line(3, &display, 0, &key(Some(0..3)), underline());
        assert_eq!(runs.len(), 1);
        assert!(runs[0].underline.is_some());
    }

    #[test]
    fn a_mark_on_another_line_does_not_underline_this_one() {
        let display = DisplayText::new("one\ntwo", &SharedString::default(), false, true);
        let runs = runs_for_line(3, &display, 0, &key(Some(4..7)), underline());
        assert_eq!(runs.len(), 1);
        assert!(runs[0].underline.is_none());
    }
}
