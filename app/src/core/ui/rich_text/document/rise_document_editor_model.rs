use std::collections::{HashMap, HashSet};

use gpui::Context;
use rise_canon::{RiseDocumentInlineMark, RiseDocumentListStyle};
use rise_core::foundation::foundation_text::drop_last_characters;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_range::{NSRange, length_utf16, utf16_to_byte_offset};
use rise_ui::rich_text::document::{
    RiseDocument, RiseDocumentBlock, RiseDocumentEditorRow, RiseDocumentEditorRowKind, RiseDocumentFocus,
    RiseDocumentProjection, RiseDocumentReference, RiseDocumentRowMapper, RiseDocumentRun, RiseDocumentTable,
    RiseDocumentTableCell, RiseDocumentTableRow,
};
use rise_validate::{MESSAGE_CONTENT, Unit};

use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    CustomEmojiEntityWire, MessageEntitiesCodec,
};
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

const HISTORY_LIMIT: usize = 64;

/// editing state of one document
/// ui state only, no server data transport or cache
/// so its a plain observable model and not a feature store
pub struct RiseDocumentEditorModel {
    rows: Vec<RiseDocumentEditorRow>,
    pub focus: Option<RiseDocumentFocus>,
    pub selection: NSRange,
    pub is_keyboard_suspended: bool,

    pub allows_embedded_media: bool,

    undo_stack: Vec<Vec<RiseDocumentEditorRow>>,
    redo_stack: Vec<Vec<RiseDocumentEditorRow>>,

    attached_media: Vec<MediaPickerItem>,

    custom_emoji_items: HashMap<String, StickerItem>,
}

impl RiseDocumentEditorModel {
    /// new model from a document, allows_embedded_media says if embedded media is ok
    pub fn new(document: &RiseDocument, allows_embedded_media: bool) -> Self {
        let rows = RiseDocumentRowMapper::rows(document);
        let focus = rows.first().map(|row| RiseDocumentFocus::Row(row.id));
        Self {
            rows,
            focus,
            selection: NSRange::new(0, 0),
            is_keyboard_suspended: false,
            allows_embedded_media,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            attached_media: Vec::new(),
            custom_emoji_items: HashMap::new(),
        }
    }

    pub fn rows(&self) -> &[RiseDocumentEditorRow] {
        &self.rows
    }

    pub fn attached_media(&self) -> &[MediaPickerItem] {
        &self.attached_media
    }

    pub fn custom_emoji_items(&self) -> &HashMap<String, StickerItem> {
        &self.custom_emoji_items
    }

    pub fn focused_row_id(&self) -> Option<Uuid> {
        self.focus.map(|focus| focus.row_id())
    }

    pub fn set_focused_row_id(&mut self, row_id: Option<Uuid>, cx: &mut Context<Self>) {
        self.focus = row_id.map(RiseDocumentFocus::Row);
        cx.notify();
    }

    pub fn set_focus(&mut self, focus: Option<RiseDocumentFocus>, cx: &mut Context<Self>) {
        self.focus = focus;
        cx.notify();
    }

    pub fn set_selection(&mut self, selection: NSRange, cx: &mut Context<Self>) {
        self.selection = selection;
        cx.notify();
    }

    pub fn set_is_keyboard_suspended(&mut self, is_keyboard_suspended: bool, cx: &mut Context<Self>) {
        self.is_keyboard_suspended = is_keyboard_suspended;
        cx.notify();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.iter().all(RiseDocumentEditorRow::is_empty)
    }

    pub fn focused_row(&self) -> Option<&RiseDocumentEditorRow> {
        let focused_row_id = self.focused_row_id()?;
        self.rows.iter().find(|row| row.id == focused_row_id)
    }

    pub fn has_selection(&self) -> bool {
        self.selection.length > 0 && self.focused_runs().is_some()
    }

    pub fn document(&self) -> RiseDocument {
        RiseDocumentRowMapper::document(&self.rows)
    }

    pub fn projection_ceiling() -> (usize, Unit) {
        MESSAGE_CONTENT.maximum().unwrap_or((usize::MAX, Unit::Chars))
    }

    pub fn projection_overflow(&self) -> usize {
        let projected = RiseDocumentProjection::project(&self.document()).trimmed_for_wire().content;
        let (limit, unit) = Self::projection_ceiling();
        unit.count(&projected).saturating_sub(limit)
    }

    /// runs of whatever the caret is in, a row or one cell of a table
    pub fn focused_runs(&self) -> Option<Vec<RiseDocumentRun>> {
        let focus = self.focus?;
        let row = self.rows.iter().find(|row| row.id == focus.row_id())?;
        match focus {
            RiseDocumentFocus::Row(_) => (!row.kind.is_verbatim()).then(|| row.runs.clone()),
            RiseDocumentFocus::TableCell { row: r, column: c, .. } => {
                let RiseDocumentEditorRowKind::Table(table) = &row.kind else {
                    return None;
                };
                let cell = table.rows.get(r)?.cells.get(c)?;
                Some(Self::cell_runs(cell))
            }
        }
    }

    pub fn set_focused_runs(&mut self, runs: Vec<RiseDocumentRun>, cx: &mut Context<Self>) {
        self.apply_focused_runs(runs);
        cx.notify();
    }

    fn apply_focused_runs(&mut self, runs: Vec<RiseDocumentRun>) {
        let Some(focus) = self.focus else {
            return;
        };
        let Some(index) = self.rows.iter().position(|row| row.id == focus.row_id()) else {
            return;
        };
        match focus {
            RiseDocumentFocus::Row(_) => {
                if self.rows[index].runs == runs {
                    return;
                }
                self.rows[index].runs = runs;
            }
            RiseDocumentFocus::TableCell { row: r, column: c, .. } => {
                self.update_table_cell(index, r, c, runs);
            }
        }
    }

    pub fn cell_runs(cell: &RiseDocumentTableCell) -> Vec<RiseDocumentRun> {
        for block in &cell.blocks {
            if let RiseDocumentBlock::Paragraph { runs } = block {
                return runs.clone();
            }
        }
        Vec::new()
    }

    /// marks under the caret or selection so the toolbar can show what is already on
    pub fn active_marks(&self) -> HashSet<RiseDocumentInlineMark> {
        let Some(runs) = self.focused_runs() else {
            return HashSet::new();
        };
        let range = self.selection;
        let mut found: Option<HashSet<RiseDocumentInlineMark>> = None;
        let mut cursor = 0;
        for run in &runs {
            let length = length_utf16(&run.text);
            let run_range = NSRange::new(cursor, length);
            cursor += length;
            if run_range.intersection(range).is_none() && !(range.length == 0 && run_range.contains(range.location)) {
                continue;
            }
            found = Some(match found {
                Some(found) => found.intersection(&run.marks).copied().collect(),
                None => run.marks.clone(),
            });
        }
        found.unwrap_or_default()
    }

    pub fn replace_runs(&mut self, runs: Vec<RiseDocumentRun>, row_id: Uuid, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(row_id) else {
            return;
        };
        if self.rows[index].runs == runs {
            return;
        }
        self.rows[index].runs = runs;
        cx.notify();
    }

    pub fn replace_verbatim(&mut self, text: String, row_id: Uuid, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(row_id) else {
            return;
        };
        if self.rows[index].verbatim == text {
            return;
        }
        self.rows[index].verbatim = text;
        cx.notify();
    }

    /// sets the row kind, row_id none is the focused row
    pub fn set_kind(&mut self, kind: RiseDocumentEditorRowKind, row_id: Option<Uuid>, cx: &mut Context<Self>) {
        let Some(id) = row_id.or_else(|| self.focused_row_id()) else {
            return;
        };
        let Some(index) = self.index_of(id) else {
            return;
        };
        self.record_undo();
        let mut row = self.rows[index].clone();
        if kind.is_verbatim() && !row.kind.is_verbatim() {
            row.verbatim = row.plain_text();
            row.runs = Vec::new();
        } else if !kind.is_verbatim() && row.kind.is_verbatim() {
            row.runs = if row.verbatim.is_empty() { Vec::new() } else { vec![RiseDocumentRun::new(row.verbatim.clone())] };
            row.verbatim = String::new();
        }
        row.kind = kind;
        self.rows[index] = row;
        cx.notify();
    }

    /// toggles a mark on the selection, url is for link marks
    pub fn toggle_mark(&mut self, mark: RiseDocumentInlineMark, url: Option<String>, cx: &mut Context<Self>) {
        let Some(runs) = self.focused_runs() else {
            return;
        };
        if self.selection.length == 0 {
            return;
        }
        self.record_undo();
        let is_on = self.active_marks().contains(&mark);
        let applied = Self::applying(mark, !is_on, url, &runs, self.selection);
        self.apply_focused_runs(applied);
        cx.notify();
    }

    pub fn insert_row(&mut self, kind: RiseDocumentEditorRowKind, cx: &mut Context<Self>) {
        self.record_undo();
        let row = RiseDocumentEditorRow::new(kind, Vec::new(), String::new());
        let row_id = row.id;
        match self.focused_row_id().and_then(|id| self.index_of(id)) {
            Some(index) => self.rows.insert(index + 1, row),
            None => self.rows.push(row),
        }
        self.focus = Some(RiseDocumentFocus::Row(row_id));
        self.selection = NSRange::new(0, 0);
        cx.notify();
    }

    pub fn split_row(&mut self, row_id: Uuid, offset: usize, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(row_id) else {
            return;
        };
        self.record_undo();
        let row = self.rows[index].clone();
        let (head, tail) = Self::split(&row.runs, offset);
        self.rows[index].runs = head;
        let next_kind = match &row.kind {
            RiseDocumentEditorRowKind::Quote
            | RiseDocumentEditorRowKind::PullQuote
            | RiseDocumentEditorRowKind::CollapsibleBody => row.kind.clone(),
            RiseDocumentEditorRowKind::ListItem { style, .. } => {
                RiseDocumentEditorRowKind::ListItem { style: *style, is_done: false }
            }
            _ => RiseDocumentEditorRowKind::Paragraph,
        };
        let next = RiseDocumentEditorRow::new(next_kind, tail, String::new());
        let next_id = next.id;
        self.rows.insert(index + 1, next);
        self.focus = Some(RiseDocumentFocus::Row(next_id));
        self.selection = NSRange::new(0, 0);
        cx.notify();
    }

    pub fn backspace_at_start(&mut self, row_id: Uuid, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(row_id) else {
            return;
        };
        if !matches!(self.rows[index].kind, RiseDocumentEditorRowKind::Paragraph) {
            self.set_kind(RiseDocumentEditorRowKind::Paragraph, Some(row_id), cx);
            return;
        }
        if index == 0 {
            return;
        }
        self.record_undo();
        let row = self.rows.remove(index);
        let previous = self.rows[index - 1].clone();
        let caret = length_utf16(&previous.plain_text());
        if previous.kind.is_verbatim() {
            self.rows[index - 1].verbatim.push_str(&row.plain_text());
        } else {
            let mut merged = previous.runs.clone();
            merged.extend(row.runs);
            self.rows[index - 1].runs = Self::normalized(merged);
        }
        self.focus = Some(RiseDocumentFocus::Row(self.rows[index - 1].id));
        self.selection = NSRange::new(caret, 0);
        cx.notify();
    }

    pub fn delete_row(&mut self, row_id: Uuid, cx: &mut Context<Self>) {
        if self.rows.len() <= 1 {
            return;
        }
        let Some(index) = self.index_of(row_id) else {
            return;
        };
        self.record_undo();
        self.rows.remove(index);
        let focused = self.rows[index.saturating_sub(1)].id;
        self.focus = Some(RiseDocumentFocus::Row(focused));
        cx.notify();
    }

    pub fn toggle_task(&mut self, row_id: Uuid, cx: &mut Context<Self>) {
        let Some(index) = self.index_of(row_id) else {
            return;
        };
        let RiseDocumentEditorRowKind::ListItem { style, is_done } = self.rows[index].kind else {
            return;
        };
        if style != RiseDocumentListStyle::Task {
            return;
        }
        self.record_undo();
        self.rows[index].kind = RiseDocumentEditorRowKind::ListItem { style, is_done: !is_done };
        cx.notify();
    }

    pub fn insert_table(&mut self, columns: usize, row_count: usize, cx: &mut Context<Self>) {
        let table = Self::empty_table(columns, row_count);
        self.insert_row(RiseDocumentEditorRowKind::Table(table), cx);
        if let Some(id) = self.focused_row_id() {
            self.focus = Some(RiseDocumentFocus::TableCell { row_id: id, row: 0, column: 0 });
            self.selection = NSRange::new(0, 0);
        }
        cx.notify();
    }

    pub fn empty_table(columns: usize, row_count: usize) -> RiseDocumentTable {
        let cell = RiseDocumentTableCell { blocks: vec![RiseDocumentBlock::Paragraph { runs: Vec::new() }] };
        let row = RiseDocumentTableRow { cells: vec![cell; columns] };
        RiseDocumentTable { column_count: columns, has_header_row: false, column_widths: None, rows: vec![row; row_count] }
    }

    pub fn table(&self, row_id: Uuid) -> Option<&RiseDocumentTable> {
        let row = self.rows.iter().find(|row| row.id == row_id)?;
        match &row.kind {
            RiseDocumentEditorRowKind::Table(table) => Some(table),
            _ => None,
        }
    }

    pub fn focus_table_cell(&mut self, row_id: Uuid, row: usize, column: usize, cx: &mut Context<Self>) {
        self.focus = Some(RiseDocumentFocus::TableCell { row_id, row, column });
        cx.notify();
    }

    fn update_table_cell(&mut self, row_index: usize, r: usize, c: usize, runs: Vec<RiseDocumentRun>) {
        let RiseDocumentEditorRowKind::Table(table) = &mut self.rows[row_index].kind else {
            return;
        };
        let Some(cell) = table.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) else {
            return;
        };
        if Self::cell_runs(cell) == runs {
            return;
        }
        *cell = RiseDocumentTableCell { blocks: vec![RiseDocumentBlock::Paragraph { runs }] };
    }

    pub fn insert_table_row(&mut self, row_id: Uuid, position: usize, cx: &mut Context<Self>) {
        self.mutate_table(row_id, |table| {
            let cell = RiseDocumentTableCell { blocks: vec![RiseDocumentBlock::Paragraph { runs: Vec::new() }] };
            let inserted = RiseDocumentTableRow { cells: vec![cell; table.column_count.max(1)] };
            let at = (position + 1).min(table.rows.len());
            table.rows.insert(at, inserted);
        });
        cx.notify();
    }

    pub fn insert_table_column(&mut self, row_id: Uuid, position: usize, cx: &mut Context<Self>) {
        self.mutate_table(row_id, |table| {
            let at = (position + 1).min(table.column_count);
            for row in &mut table.rows {
                let cell = RiseDocumentTableCell { blocks: vec![RiseDocumentBlock::Paragraph { runs: Vec::new() }] };
                let bounded = at.min(row.cells.len());
                row.cells.insert(bounded, cell);
            }
            table.column_count += 1;
            table.column_widths = None;
        });
        cx.notify();
    }

    pub fn delete_table_row(&mut self, row_id: Uuid, position: usize, cx: &mut Context<Self>) {
        self.mutate_table(row_id, |table| {
            if table.rows.len() <= 1 || position >= table.rows.len() {
                return;
            }
            table.rows.remove(position);
        });
        self.clamp_table_focus(row_id);
        cx.notify();
    }

    pub fn delete_table_column(&mut self, row_id: Uuid, position: usize, cx: &mut Context<Self>) {
        self.mutate_table(row_id, |table| {
            if table.column_count <= 1 {
                return;
            }
            for row in &mut table.rows {
                if position < row.cells.len() {
                    row.cells.remove(position);
                }
            }
            table.column_count -= 1;
            table.column_widths = None;
        });
        self.clamp_table_focus(row_id);
        cx.notify();
    }

    pub fn toggle_table_header(&mut self, row_id: Uuid, cx: &mut Context<Self>) {
        self.mutate_table(row_id, |table| {
            table.has_header_row = !table.has_header_row;
        });
        cx.notify();
    }

    fn mutate_table(&mut self, row_id: Uuid, body: impl FnOnce(&mut RiseDocumentTable)) {
        let Some(index) = self.index_of(row_id) else {
            return;
        };
        let RiseDocumentEditorRowKind::Table(table) = &self.rows[index].kind else {
            return;
        };
        let mut table = table.clone();
        self.record_undo();
        body(&mut table);
        self.rows[index].kind = RiseDocumentEditorRowKind::Table(table);
    }

    /// after a removal the caret may point at a cell that is gone
    /// move it to the nearest one that still exists
    fn clamp_table_focus(&mut self, row_id: Uuid) {
        let Some(RiseDocumentFocus::TableCell { row_id: id, row: r, column: c }) = self.focus else {
            return;
        };
        if id != row_id {
            return;
        }
        let Some(table) = self.table(row_id) else {
            return;
        };
        let row = r.min(table.rows.len().saturating_sub(1));
        let column = c.min(table.column_count.saturating_sub(1));
        self.focus = Some(RiseDocumentFocus::TableCell { row_id, row, column });
        self.selection = NSRange::new(0, 0);
    }

    pub fn attach(&mut self, items: Vec<MediaPickerItem>, cx: &mut Context<Self>) {
        if !self.allows_embedded_media || items.is_empty() {
            return;
        }
        for item in items {
            let index = self.attached_media.len();
            self.attached_media.push(item);
            self.insert_row(RiseDocumentEditorRowKind::Media { index }, cx);
        }
        self.insert_row(RiseDocumentEditorRowKind::Paragraph, cx);
    }

    pub fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        let Some(focus) = self.focus else {
            return;
        };
        if let RiseDocumentFocus::Row(row_id) = focus
            && let Some(index) = self.index_of(row_id)
            && self.rows[index].kind.is_verbatim()
        {
            self.record_undo();
            let source = self.rows[index].verbatim.clone();
            let source_length = length_utf16(&source);
            let at = self.selection.location.min(source_length);
            let length = self.selection.length.min(source_length - at);
            let start = byte_at_utf16(&source, at);
            let end = byte_at_utf16(&source, at + length).max(start);
            let mut replaced = String::with_capacity(source.len() + text.len());
            replaced.push_str(&source[..start]);
            replaced.push_str(text);
            replaced.push_str(&source[end..]);
            self.rows[index].verbatim = replaced;
            self.selection = NSRange::new(at + length_utf16(text), 0);
            cx.notify();
            return;
        }
        let marks = self.active_marks();
        self.insert(RiseDocumentRun::with(text, marks, None, None));
        cx.notify();
    }

    pub fn insert_custom_emoji(&mut self, item: StickerItem, cx: &mut Context<Self>) {
        let pack_id = (!item.pack_id.is_empty()).then(|| item.pack_id.clone());
        let reference = RiseDocumentReference::CustomEmoji { id: item.id.clone(), pack_id };
        self.custom_emoji_items.insert(item.id.clone(), item);
        self.insert(RiseDocumentRun::with(
            RiseDocumentProjection::CUSTOM_EMOJI_PLACEHOLDER,
            HashSet::new(),
            None,
            Some(reference),
        ));
        cx.notify();
    }

    /// backspace from the emoji panel, which has no keyboard behind it
    pub fn delete_backward(&mut self, cx: &mut Context<Self>) {
        let Some(existing) = self.focused_runs() else {
            return;
        };
        let at = self.selection.location.min(Self::length(&existing));
        if at == 0 {
            return;
        }
        self.record_undo();
        let (head, tail) = Self::split(&existing, at);
        let mut trimmed = head.clone();
        if let Some(mut last) = trimmed.last().cloned() {
            if last.reference.is_some() || length_utf16(&last.text) <= 1 {
                trimmed.pop();
            } else {
                last.text = drop_last_characters(&last.text, 1).to_owned();
                let end = trimmed.len() - 1;
                trimmed[end] = last;
            }
        }
        let removed = Self::length(&head) - Self::length(&trimmed);
        trimmed.extend(tail);
        self.apply_focused_runs(Self::normalized(trimmed));
        self.selection = NSRange::new(at.saturating_sub(removed), 0);
        cx.notify();
    }

    fn insert(&mut self, run: RiseDocumentRun) {
        let Some(existing) = self.focused_runs() else {
            return;
        };
        self.record_undo();
        let at = self.selection.location.min(Self::length(&existing));
        let (mut head, tail) = Self::split(&existing, at);
        let inserted = length_utf16(&run.text);
        head.push(run);
        head.extend(tail);
        self.apply_focused_runs(Self::normalized(head));
        self.selection = NSRange::new(at + inserted, 0);
    }

    pub fn length(runs: &[RiseDocumentRun]) -> usize {
        runs.iter().map(|run| length_utf16(&run.text)).sum()
    }

    pub fn projected_entities_json(&self) -> Option<String> {
        let references = RiseDocumentProjection::project(&self.document()).trimmed_for_wire().references;
        let emojis: Vec<CustomEmojiEntityWire> = references
            .iter()
            .filter_map(|reference| {
                let RiseDocumentReference::CustomEmoji { id, .. } = &reference.reference else {
                    return None;
                };
                let item = self.custom_emoji_items.get(id)?;
                Some(CustomEmojiEntityWire {
                    offset: reference.offset as i64,
                    length: reference.length as i64,
                    custom_emoji_id: item.id.clone(),
                    file_url: item.file_url.clone(),
                    asset_type: item.asset_type.wire().to_owned(),
                    width: item.width,
                    height: item.height,
                    pack_id: item.pack_id.clone(),
                })
            })
            .collect();
        if emojis.is_empty() {
            return None;
        }
        MessageEntitiesCodec::encode(&emojis, None)
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(previous) = self.undo_stack.pop() else {
            return;
        };
        let current = std::mem::replace(&mut self.rows, previous);
        self.redo_stack.push(current);
        self.focus = self.rows.first().map(|row| RiseDocumentFocus::Row(row.id));
        cx.notify();
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        let Some(next) = self.redo_stack.pop() else {
            return;
        };
        let current = std::mem::replace(&mut self.rows, next);
        self.undo_stack.push(current);
        self.focus = self.rows.first().map(|row| RiseDocumentFocus::Row(row.id));
        cx.notify();
    }

    fn record_undo(&mut self) {
        self.undo_stack.push(self.rows.clone());
        if self.undo_stack.len() > HISTORY_LIMIT {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    fn index_of(&self, row_id: Uuid) -> Option<usize> {
        self.rows.iter().position(|row| row.id == row_id)
    }

    pub fn normalized(runs: Vec<RiseDocumentRun>) -> Vec<RiseDocumentRun> {
        let mut out: Vec<RiseDocumentRun> = Vec::new();
        for run in runs {
            if run.text.is_empty() && run.reference.is_none() {
                continue;
            }
            if let Some(last) = out.last_mut()
                && last.marks == run.marks
                && last.url == run.url
                && last.reference.is_none()
                && run.reference.is_none()
            {
                last.text.push_str(&run.text);
            } else {
                out.push(run);
            }
        }
        out
    }

    pub fn split(runs: &[RiseDocumentRun], offset: usize) -> (Vec<RiseDocumentRun>, Vec<RiseDocumentRun>) {
        let mut head: Vec<RiseDocumentRun> = Vec::new();
        let mut tail: Vec<RiseDocumentRun> = Vec::new();
        let mut cursor = 0;
        for run in runs {
            let end = cursor + length_utf16(&run.text);
            if end <= offset {
                head.push(run.clone());
            } else if cursor >= offset {
                tail.push(run.clone());
            } else {
                let cut = byte_at_utf16(&run.text, offset - cursor);
                let mut left = run.clone();
                left.text = run.text[..cut].to_owned();
                let mut right = run.clone();
                right.text = run.text[cut..].to_owned();
                right.reference = None;
                head.push(left);
                tail.push(right);
            }
            cursor = end;
        }
        (Self::normalized(head), Self::normalized(tail))
    }

    pub fn applying(
        mark: RiseDocumentInlineMark,
        enabled: bool,
        url: Option<String>,
        runs: &[RiseDocumentRun],
        range: NSRange,
    ) -> Vec<RiseDocumentRun> {
        let mut out: Vec<RiseDocumentRun> = Vec::new();
        let mut cursor = 0;
        for run in runs {
            let length = length_utf16(&run.text);
            let run_range = NSRange::new(cursor, length);
            cursor += length;
            let Some(overlap) = run_range.intersection(range) else {
                out.push(run.clone());
                continue;
            };
            let local_start = overlap.location - run_range.location;
            let local_end = local_start + overlap.length;
            let start = byte_at_utf16(&run.text, local_start);
            let end = byte_at_utf16(&run.text, local_end).max(start);
            if local_start > 0 {
                let mut before = run.clone();
                before.text = run.text[..start].to_owned();
                out.push(before);
            }
            let mut middle = run.clone();
            middle.text = run.text[start..end].to_owned();
            if enabled {
                middle.marks.insert(mark);
                if mark == RiseDocumentInlineMark::Link {
                    middle.url = url.clone();
                }
            } else {
                middle.marks.remove(&mark);
                if mark == RiseDocumentInlineMark::Link {
                    middle.url = None;
                }
            }
            out.push(middle);
            if local_end < length {
                let mut after = run.clone();
                after.text = run.text[end..].to_owned();
                after.reference = None;
                out.push(after);
            }
        }
        Self::normalized(out)
    }
}

/// byte offset of a utf16 offset into text
/// an offset between the halves of a surrogate pair rounds down to the scalar start since a
/// string cant hold half
fn byte_at_utf16(text: &str, offset: usize) -> usize {
    utf16_to_byte_offset(text, offset)
        .or_else(|| offset.checked_sub(1).and_then(|lower| utf16_to_byte_offset(text, lower)))
        .unwrap_or(text.len())
}
