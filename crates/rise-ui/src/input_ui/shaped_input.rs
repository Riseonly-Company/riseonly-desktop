use std::ops::{Deref, DerefMut, Range};
use std::sync::Arc;

use gpui::{
    Font, Hsla, LineLayout, Pixels, ShapedGlyph, ShapedLine, ShapedRun, SharedString, TextRun,
    TruncateFrom, UnderlineStyle, Window, point, px, transparent_black,
};
use rise_platform::text_capabilities::monospaced_family;
use smallvec::SmallVec;

use crate::input_ui::attributed_runs::{
    ATTACHMENT_CHARACTER, AttributedRuns, RunAttributes, TextAttachment,
};
use crate::expandable_text_ui::first_strong_is_right_to_left;
use crate::input_ui::display_text::DisplayText;
use crate::swiftui::font::FontDesign;

/// everything a shaped result depends on
#[derive(Clone, PartialEq, Debug)]
pub struct ShapeKey {
    pub revision: u64,
    pub font_size: Pixels,
    pub font: Font,
    pub color: Hsla,
    pub marked: Option<Range<usize>>,
    pub visible: Range<usize>,
    pub wrap_width: Option<Pixels>,
    pub placeholder_width: Option<Pixels>,
}

const PLACEHOLDER_TRUNCATION_AFFIX: &str = "\u{2026}";

/// an attachment on a row, the U+FFFC at byte offset of the row draws attachment
#[derive(Clone, Debug)]
pub struct PlacedAttachment {
    pub offset: usize,
    pub attachment: TextAttachment,
}

/// one painted row, slice start..end of a display line in display bytes
pub struct ShapedRow {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub shaped: ShapedLine,
    pub attachments: Vec<PlacedAttachment>,
}

/// visible lines shaped into rows one line height tall
/// without wrapping row n is line n, with wrapping every line is shaped and rows count from the top
pub struct ShapedInput {
    key: ShapeKey,
    first_row: usize,
    rows: Vec<ShapedRow>,
}

impl ShapedInput {
    /// shapes the display text, attributed draws per run and a plain field passes none
    pub fn shape(
        key: ShapeKey,
        display: &DisplayText,
        attributed: Option<&AttributedRuns>,
        underline: UnderlineStyle,
        window: &Window,
    ) -> Self {
        debug_assert!(key.wrap_width.is_none() || key.visible.start == 0);
        let mut rows = Vec::with_capacity(key.visible.len());
        for line_index in key.visible.clone() {
            let Some(line) = display.lines.get(line_index) else {
                break;
            };
            let text: SharedString = display.text[line.display.clone()].to_owned().into();
            let attachments = line_attachments(display, attributed, line_index, &text);
            let runs = hide_attachment_glyphs(
                runs_for_line(
                    text.len(),
                    display,
                    attributed,
                    line_index,
                    &key,
                    underline,
                ),
                &attachments,
            );
            let (text, runs) = match key.placeholder_width.filter(|_| display.is_placeholder) {
                Some(width) => {
                    let mut wrapper = window.text_system().line_wrapper(key.font.clone(), key.font_size);
                    let (text, runs) =
                        wrapper.truncate_line(text, width, PLACEHOLDER_TRUNCATION_AFFIX, &runs, TruncateFrom::End);
                    (text, SmallVec::from_vec(runs.into_owned()))
                }
                None => (text, runs),
            };
            let mut shaped = window
                .text_system()
                .shape_line(text, key.font_size, &runs, None);
            advance_attachments(&mut shaped, &attachments);
            let logical = key.wrap_width.and_then(|_| logical_stops(&shaped));
            match (key.wrap_width, logical) {
                (Some(width), Some(stops)) => {
                    let starts = wrap_starts_at(&shaped.text, &stops, width);
                    reshaped_rows(line_index, &shaped.text, &runs, starts, &attachments, key.font_size, window, &mut rows);
                }
                _ => split_into_rows(line_index, shaped, key.wrap_width, &attachments, &mut rows),
            }
        }

        let first_row = if key.wrap_width.is_some() {
            0
        } else {
            key.visible.start
        };
        Self {
            key,
            first_row,
            rows,
        }
    }

    pub fn matches(&self, key: &ShapeKey) -> bool {
        self.key == *key
    }

    /// hard lines that were shaped
    pub fn visible(&self) -> Range<usize> {
        self.key.visible.clone()
    }

    pub fn wraps(&self) -> bool {
        self.key.wrap_width.is_some()
    }

    /// row indices this result holds
    pub fn row_range(&self) -> Range<usize> {
        self.first_row..self.first_row + self.rows.len()
    }

    pub fn wrapped_row_count(&self) -> Option<usize> {
        self.wraps().then_some(self.rows.len())
    }

    pub fn row(&self, row_index: usize) -> Option<&ShapedRow> {
        row_index
            .checked_sub(self.first_row)
            .and_then(|index| self.rows.get(index))
    }

    /// row a display offset of the line sits on
    /// a soft wrap boundary belongs to the row it starts
    pub fn row_for(&self, line_index: usize, offset_in_line: usize) -> Option<usize> {
        if !self.wraps() {
            return Some(line_index);
        }
        let after = self
            .rows
            .partition_point(|row| (row.line, row.start) <= (line_index, offset_in_line));
        let index = after.checked_sub(1)?;
        (self.rows[index].line == line_index).then_some(index)
    }

    /// x of a display offset within the row that holds it
    pub fn x_for(&self, line_index: usize, offset_in_line: usize) -> Pixels {
        self.row_for(line_index, offset_in_line)
            .and_then(|row_index| self.row(row_index))
            .map_or(px(0.0), |row| {
                row.shaped
                    .x_for_index(offset_in_line.saturating_sub(row.start))
            })
    }

    pub fn row_is_right_to_left(&self, row_index: usize) -> bool {
        let Some(line) = self.row(row_index).map(|row| row.line) else {
            return false;
        };
        self.rows
            .iter()
            .filter(|row| row.line == line)
            .find_map(|row| first_strong_is_right_to_left(&row.shaped.text))
            .unwrap_or(false)
    }

    pub fn row_width(&self, row_index: usize) -> Pixels {
        self.row(row_index)
            .map_or(px(0.0), |row| row.shaped.width())
    }

    /// closest line and offset to x on a row
    /// past the end of a soft wrapped row the caret stays before the last char, not on the next row
    pub fn offset_in_row(&self, row_index: usize, x: Pixels) -> Option<(usize, usize)> {
        let row = self.row(row_index)?;
        let mut index = row.shaped.closest_index_for_x(x).min(row.end - row.start);
        let continues = self
            .row(row_index + 1)
            .is_some_and(|next| next.line == row.line);
        if continues && index == row.end - row.start {
            index = row.shaped.text[..index]
                .char_indices()
                .next_back()
                .map_or(0, |(at, _)| at);
        }
        Some((row.line, row.start + index))
    }
}

fn split_into_rows(
    line_index: usize,
    shaped: ShapedLine,
    wrap_width: Option<Pixels>,
    attachments: &[(usize, TextAttachment)],
    rows: &mut Vec<ShapedRow>,
) {
    let starts = wrap_width.map_or_else(Vec::new, |width| wrap_starts(&shaped, width));
    push_split_rows(line_index, shaped, 0, &starts, attachments, rows);
}

fn push_split_rows(
    line_index: usize,
    shaped: ShapedLine,
    start: usize,
    starts: &[usize],
    attachments: &[(usize, TextAttachment)],
    rows: &mut Vec<ShapedRow>,
) {
    let middle = starts.len() / 2;
    let Some(&cut) = starts.get(middle) else {
        let end = start + shaped.len();
        rows.push(ShapedRow {
            line: line_index,
            start,
            end,
            shaped,
            attachments: attachments_in(attachments, start..end),
        });
        return;
    };
    let (head, tail) = shaped.split_at(cut - start);
    push_split_rows(line_index, head, start, &starts[..middle], attachments, rows);
    push_split_rows(line_index, tail, cut, &starts[middle + 1..], attachments, rows);
}

#[allow(clippy::too_many_arguments)]
fn reshaped_rows(
    line_index: usize,
    text: &SharedString,
    runs: &[TextRun],
    starts: Vec<usize>,
    attachments: &[(usize, TextAttachment)],
    font_size: Pixels,
    window: &Window,
    rows: &mut Vec<ShapedRow>,
) {
    let mut bounds = vec![0];
    bounds.extend(starts);
    bounds.push(text.len());
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let Some(piece) = text.get(start..end) else {
            continue;
        };
        let placed = attachments_in(attachments, start..end);
        let rebased: Vec<(usize, TextAttachment)> = placed
            .iter()
            .map(|placed| (placed.offset, placed.attachment.clone()))
            .collect();
        let mut shaped = window.text_system().shape_line(
            SharedString::from(piece.to_owned()),
            font_size,
            &runs_in(runs, start..end),
            None,
        );
        advance_attachments(&mut shaped, &rebased);
        rows.push(ShapedRow {
            line: line_index,
            start,
            end,
            shaped,
            attachments: placed,
        });
    }
}

fn runs_in(runs: &[TextRun], range: Range<usize>) -> Vec<TextRun> {
    let mut out = Vec::new();
    let mut offset = 0;
    for run in runs {
        let start = offset.max(range.start);
        let end = (offset + run.len).min(range.end);
        offset += run.len;
        if end > start {
            out.push(TextRun {
                len: end - start,
                ..run.clone()
            });
        }
    }
    out
}

fn logical_stops(line: &ShapedLine) -> Option<Vec<(usize, Pixels)>> {
    let glyphs: Vec<(usize, Pixels)> = line
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .map(|glyph| (glyph.index, glyph.position.x))
        .collect();
    logical_stops_from(&glyphs, line.len(), line.width())
}

pub(crate) fn logical_stops_from(
    glyphs: &[(usize, Pixels)],
    len: usize,
    width: Pixels,
) -> Option<Vec<(usize, Pixels)>> {
    let reordered = glyphs.windows(2).any(|pair| pair[1].0 < pair[0].0);
    if !reordered {
        return None;
    }
    let mut on_screen: Vec<(usize, Pixels)> = glyphs.to_vec();
    on_screen.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut advances: Vec<(usize, Pixels)> = on_screen
        .iter()
        .enumerate()
        .map(|(at, (index, x))| {
            let next = on_screen.get(at + 1).map_or(width, |(_, next_x)| *next_x);
            (*index, (next - *x).max(px(0.0)))
        })
        .collect();
    advances.sort_by_key(|(index, _)| *index);
    let mut stops: Vec<(usize, Pixels)> = Vec::with_capacity(advances.len() + 1);
    let mut pen = px(0.0);
    for (index, advance) in advances {
        match stops.last() {
            Some((last, _)) if *last == index => {}
            _ => stops.push((index, pen)),
        }
        pen += advance;
    }
    stops.push((len, pen));
    Some(stops)
}

fn attachments_in(
    attachments: &[(usize, TextAttachment)],
    row: Range<usize>,
) -> Vec<PlacedAttachment> {
    attachments
        .iter()
        .filter(|(offset, _)| row.contains(offset))
        .map(|(offset, attachment)| PlacedAttachment {
            offset: offset - row.start,
            attachment: attachment.clone(),
        })
        .collect()
}

/// every U+FFFC of a display line that carries an attachment, in order
/// a placeholder or secure field draws none
pub(crate) fn line_attachments(
    display: &DisplayText,
    attributed: Option<&AttributedRuns>,
    line_index: usize,
    line_text: &str,
) -> Vec<(usize, TextAttachment)> {
    let attributed = attributed.filter(|_| !display.is_placeholder && !display.is_secure());
    let Some(attributed) = attributed else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (range, attributes) in attributed.runs() {
        let Some(attachment) = attributes.attachment.as_ref() else {
            continue;
        };
        let Some(local) = display.marked_within_line(line_index, &range) else {
            continue;
        };
        let Some(piece) = line_text.get(local.clone()) else {
            continue;
        };
        for (at, character) in piece.char_indices() {
            if character == ATTACHMENT_CHARACTER {
                out.push((local.start + at, attachment.clone()));
            }
        }
    }
    out
}

fn hide_attachment_glyphs(
    runs: SmallVec<[TextRun; 3]>,
    attachments: &[(usize, TextAttachment)],
) -> SmallVec<[TextRun; 3]> {
    if attachments.is_empty() {
        return runs;
    }
    let width = ATTACHMENT_CHARACTER.len_utf8();
    let mut out = SmallVec::new();
    let mut offset = 0;
    for run in runs {
        let start = offset;
        let end = start + run.len;
        offset = end;
        let mut cursor = start;
        for (at, _) in attachments {
            let hidden = (*at).max(cursor)..(at + width).min(end);
            if hidden.end <= hidden.start {
                continue;
            }
            if hidden.start > cursor {
                out.push(TextRun {
                    len: hidden.start - cursor,
                    ..run.clone()
                });
            }
            out.push(TextRun {
                len: hidden.end - hidden.start,
                color: transparent_black(),
                ..run.clone()
            });
            cursor = hidden.end;
        }
        if end > cursor {
            out.push(TextRun {
                len: end - cursor,
                ..run
            });
        }
    }
    out
}

/// widens each attachment U+FFFC to its bounds width and shifts every glyph after it by the difference
fn advance_attachments(shaped: &mut ShapedLine, attachments: &[(usize, TextAttachment)]) {
    if attachments.is_empty() {
        return;
    }
    let current: Arc<LineLayout> = Arc::clone(Deref::deref(&*shaped));
    let stops: Vec<(usize, Pixels)> = current
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .map(|glyph| (glyph.index, glyph.position.x))
        .collect();
    let shifts = attachment_shifts(&stops, current.width, attachments);
    let shift_for = |index: usize| -> Pixels {
        shifts
            .iter()
            .filter(|(at, _)| *at < index)
            .fold(px(0.0), |sum, (_, delta)| sum + *delta)
    };
    let total = shifts.iter().fold(px(0.0), |sum, (_, delta)| sum + *delta);
    let runs = current
        .runs
        .iter()
        .map(|run| ShapedRun {
            font_id: run.font_id,
            glyphs: run
                .glyphs
                .iter()
                .map(|glyph| ShapedGlyph {
                    position: point(glyph.position.x + shift_for(glyph.index), glyph.position.y),
                    ..glyph.clone()
                })
                .collect(),
        })
        .collect();
    let widened = LineLayout {
        font_size: current.font_size,
        width: current.width + total,
        ascent: current.ascent,
        descent: current.descent,
        runs,
        len: current.len,
    };
    *DerefMut::deref_mut(shaped) = Arc::new(widened);
}

/// how far each attachment pushes what follows, its width minus the advance the font gave its U+FFFC
/// stops are byte index plus pen x of every glyph
pub(crate) fn attachment_shifts(
    stops: &[(usize, Pixels)],
    width: Pixels,
    attachments: &[(usize, TextAttachment)],
) -> Vec<(usize, Pixels)> {
    attachments
        .iter()
        .map(|(at, attachment)| {
            let advance = stops
                .iter()
                .find(|(index, _)| index == at)
                .map_or(px(0.0), |(_, x)| {
                    let next = stops
                        .iter()
                        .filter(|(index, _)| index > at)
                        .min_by_key(|(index, _)| *index)
                        .map_or(width, |(_, next_x)| *next_x);
                    (next - *x).max(px(0.0))
                });
            (*at, attachment.bounds.size.width - advance)
        })
        .collect()
}

fn wrap_starts(line: &ShapedLine, width: Pixels) -> Vec<usize> {
    let mut stops: Vec<(usize, Pixels)> = line
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .map(|glyph| (glyph.index, glyph.position.x))
        .collect();
    stops.sort_by_key(|(index, _)| *index);
    stops.dedup_by_key(|(index, _)| *index);
    stops.push((line.len(), line.width()));
    wrap_starts_at(&line.text, &stops, width)
}

pub(crate) fn wrap_starts_at(text: &str, stops: &[(usize, Pixels)], width: Pixels) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut row_start = 0;
    let mut row_x = px(0.0);
    let mut word_break: Option<(usize, Pixels)> = None;
    let mut previous: Option<(usize, Pixels)> = None;
    for &(index, x) in stops {
        if index <= row_start {
            previous = Some((index, x));
            continue;
        }
        let hangs = text
            .get(..index)
            .and_then(|head| head.chars().next_back())
            .is_some_and(char::is_whitespace);
        while x - row_x > width && !hangs {
            let cut = word_break
                .filter(|(at, _)| *at > row_start)
                .or(previous.filter(|(at, _)| *at > row_start));
            let Some((at, at_x)) = cut else {
                break;
            };
            starts.push(at);
            row_start = at;
            row_x = at_x;
            word_break = None;
        }
        if hangs {
            word_break = Some((index, x));
        }
        previous = Some((index, x));
    }
    starts
}

fn runs_for_line(
    len: usize,
    display: &DisplayText,
    attributed: Option<&AttributedRuns>,
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

    let styled = styled_segments(len, display, attributed, line_index, &base);

    let marked = key
        .marked
        .as_ref()
        .and_then(|marked| display.marked_within_line(line_index, marked))
        .filter(|marked| marked.end <= len);

    let Some(marked) = marked else {
        return styled.into_iter().filter(|run| run.len > 0).collect();
    };

    let mut out = SmallVec::new();
    let mut offset = 0;
    for run in styled {
        let start = offset;
        offset += run.len;
        for (piece, is_marked) in [
            (start..offset.min(marked.start).max(start), false),
            (start.max(marked.start)..offset.min(marked.end), true),
            (start.max(marked.end).min(offset)..offset, false),
        ] {
            if piece.end <= piece.start {
                continue;
            }
            let mut cut = TextRun {
                len: piece.end - piece.start,
                ..run.clone()
            };
            if is_marked {
                cut.underline = Some(underline);
            }
            out.push(cut);
        }
    }
    out
}

fn styled_segments(
    len: usize,
    display: &DisplayText,
    attributed: Option<&AttributedRuns>,
    line_index: usize,
    base: &TextRun,
) -> SmallVec<[TextRun; 3]> {
    let mut out: SmallVec<[TextRun; 3]> = SmallVec::new();
    let attributed = attributed.filter(|_| !display.is_placeholder && !display.is_secure());
    let Some(attributed) = attributed else {
        out.push(base.clone());
        return out;
    };
    let mut cursor = 0;
    for (range, attributes) in attributed.runs() {
        let Some(local) = display.marked_within_line(line_index, &range) else {
            continue;
        };
        let start = local.start.max(cursor);
        let end = local.end.min(len);
        if end <= start {
            continue;
        }
        if start > cursor {
            out.push(TextRun {
                len: start - cursor,
                ..base.clone()
            });
        }
        out.push(TextRun {
            len: end - start,
            ..styled_run(base, attributes)
        });
        cursor = end;
    }
    if cursor < len {
        out.push(TextRun {
            len: len - cursor,
            ..base.clone()
        });
    }
    out
}

fn styled_run(base: &TextRun, attributes: &RunAttributes) -> TextRun {
    let highlight = attributes.attributes.highlight_style(None);
    let mut font = base.font.clone();
    if let Some(weight) = highlight.font_weight {
        font.weight = weight;
    }
    if let Some(style) = highlight.font_style {
        font.style = style;
    }
    if attributes
        .attributes
        .font
        .is_some_and(|rich| rich.design == FontDesign::Monospaced)
    {
        font.family = SharedString::new_static(monospaced_family());
    }
    TextRun {
        len: base.len,
        font,
        color: highlight.color.unwrap_or(base.color),
        background_color: highlight.background_color,
        underline: highlight.underline,
        strikethrough: highlight.strikethrough,
    }
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
            wrap_width: None,
            placeholder_width: None,
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
        let display = DisplayText::new("\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}", &SharedString::default(), false, false);
        let runs = runs_for_line(
            display.text.len(),
            &display,
            None,
            0,
            &key(None),
            underline(),
        );
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
            None,
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
        let runs = runs_for_line(3, &display, None, 0, &key(Some(0..3)), underline());
        assert_eq!(runs.len(), 1);
        assert!(runs[0].underline.is_some());
    }

    #[test]
    fn a_mark_on_another_line_does_not_underline_this_one() {
        let display = DisplayText::new("one\ntwo", &SharedString::default(), false, true);
        let runs = runs_for_line(3, &display, None, 0, &key(Some(4..7)), underline());
        assert_eq!(runs.len(), 1);
        assert!(runs[0].underline.is_none());
    }

    fn bold() -> RunAttributes {
        RunAttributes::new(crate::swiftui::rich_text::AttributeContainer {
            font: Some(crate::swiftui::rich_text::RichFont::system_weight(
                14.0,
                FontWeight::BOLD,
            )),
            strikethrough_style: Some(crate::swiftui::rich_text::LineStyle::single()),
            ..Default::default()
        })
    }

    #[test]
    fn attribute_runs_style_their_slice_of_the_line() {
        let display = DisplayText::new("abcdef", &SharedString::default(), false, false);
        let attributed = AttributedRuns::from_pieces([
            (2, RunAttributes::default()),
            (2, bold()),
            (2, RunAttributes::default()),
        ]);
        let runs = runs_for_line(6, &display, Some(&attributed), 0, &key(None), underline());
        assert_eq!(runs.iter().map(|run| run.len).collect::<Vec<_>>(), vec![2, 2, 2]);
        assert_eq!(runs[1].font.weight, FontWeight::BOLD);
        assert!(runs[1].strikethrough.is_some());
        assert!(runs[0].strikethrough.is_none());
    }

    #[test]
    fn a_mark_cuts_through_attribute_runs_and_still_covers_the_line() {
        let display = DisplayText::new("ab\ncdef", &SharedString::default(), false, true);
        let attributed = AttributedRuns::from_pieces([(4, RunAttributes::default()), (3, bold())]);
        let runs = runs_for_line(4, &display, Some(&attributed), 1, &key(Some(4..6)), underline());
        assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), 4);
        assert_eq!(runs.iter().map(|run| run.len).collect::<Vec<_>>(), vec![1, 2, 1]);
        assert!(runs[0].underline.is_none());
        assert!(runs[1].underline.is_some());
        assert_eq!(runs[1].font.weight, FontWeight::BOLD);
        assert!(runs[2].underline.is_none());
        assert_eq!(runs[2].font.weight, FontWeight::BOLD);
    }
}
