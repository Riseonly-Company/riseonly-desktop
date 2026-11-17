use rise_canon::{RiseDocumentBlockKind, RiseDocumentInlineMark, RiseDocumentListStyle};
use rise_core::foundation::foundation_text::is_whitespace_or_newline;
use rise_core::foundation::ns_range::length_utf16;

use super::rise_document::{RiseDocument, RiseDocumentBlock, RiseDocumentListItem, RiseDocumentReference, RiseDocumentRun};

/// a reference that survived the projection
/// addressed by utf16 offset into the projected content so it can go out as a legacy message entity
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseDocumentProjectedReference {
    pub offset: usize,
    pub length: usize,
    pub reference: RiseDocumentReference,
}

/// flattens a document into message content text plus legacy entity references
/// old clients and previews render it, every client and the server must match bytes
/// affixes come from canon, spoiler text never enters
pub enum RiseDocumentProjection {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseDocumentProjectionOutput {
    pub content: String,
    pub references: Vec<RiseDocumentProjectedReference>,
}

impl RiseDocumentProjectionOutput {
    pub const EMPTY: RiseDocumentProjectionOutput =
        RiseDocumentProjectionOutput { content: String::new(), references: Vec::new() };
}

impl RiseDocumentProjection {
    pub const CUSTOM_EMOJI_PLACEHOLDER: &'static str = "\u{FFFC}";

    pub fn project(document: &RiseDocument) -> RiseDocumentProjectionOutput {
        let mut builder = Builder::default();
        builder.append_blocks(&document.blocks);
        builder.finish()
    }

    /// plain text only, for a preview or a length check
    pub fn plain_text(document: &RiseDocument) -> String {
        Self::project(document).content
    }
}

mod affix {
    use rise_canon::RiseDocumentProjectionAffix;

    pub fn block_separator() -> &'static str {
        RiseDocumentProjectionAffix::BlockSeparator.open_delimiter()
    }
    pub fn quote_line() -> &'static str {
        RiseDocumentProjectionAffix::QuoteLine.open_delimiter()
    }
    pub fn bulleted_item() -> &'static str {
        RiseDocumentProjectionAffix::BulletedItem.open_delimiter()
    }
    pub fn numbered_item() -> &'static str {
        RiseDocumentProjectionAffix::NumberedItem.open_delimiter()
    }
    pub fn task_item_open() -> &'static str {
        RiseDocumentProjectionAffix::TaskItemOpen.open_delimiter()
    }
    pub fn task_item_done() -> &'static str {
        RiseDocumentProjectionAffix::TaskItemDone.open_delimiter()
    }
    pub fn divider() -> &'static str {
        RiseDocumentProjectionAffix::Divider.open_delimiter()
    }
    pub fn table_cell_separator() -> &'static str {
        RiseDocumentProjectionAffix::TableCellSeparator.open_delimiter()
    }
    pub fn button_separator() -> &'static str {
        RiseDocumentProjectionAffix::ButtonSeparator.open_delimiter()
    }
    pub fn spoiler_mask() -> &'static str {
        RiseDocumentProjectionAffix::SpoilerMask.open_delimiter()
    }
    pub fn link_url_open() -> &'static str {
        RiseDocumentProjectionAffix::LinkUrl.open_delimiter()
    }
    pub fn link_url_close() -> &'static str {
        RiseDocumentProjectionAffix::LinkUrl.close_delimiter()
    }
    pub const NUMBER_TOKEN: &str = "{n}";
}

/// builder that keeps the utf16 length of text alongside it so every reference offset is O(1)
#[derive(Default)]
struct Builder {
    text: String,
    text_utf16: usize,
    references: Vec<RiseDocumentProjectedReference>,
}

impl Builder {
    fn push(&mut self, piece: &str) {
        self.text.push_str(piece);
        self.text_utf16 += length_utf16(piece);
    }

    fn append_blocks(&mut self, blocks: &[RiseDocumentBlock]) {
        for (index, block) in blocks.iter().enumerate() {
            if index > 0 {
                self.push(affix::block_separator());
            }
            self.append(block);
        }
    }

    fn append(&mut self, block: &RiseDocumentBlock) {
        match block {
            RiseDocumentBlock::Paragraph { runs } => self.append_runs(runs),

            RiseDocumentBlock::Heading { runs, .. } => {
                let format = RiseDocumentBlockKind::Heading.projection_format();
                self.push(format.map_or("", |format| format.open_delimiter()));
                self.append_runs(runs);
                self.push(format.map_or("", |format| format.close_delimiter()));
            }

            RiseDocumentBlock::Quote { blocks } | RiseDocumentBlock::PullQuote { blocks } => {
                let mut inner = Builder::default();
                inner.append_blocks(blocks);
                self.append_prefixed_lines(inner.finish(), affix::quote_line());
            }

            RiseDocumentBlock::Code { text: body, .. } => {
                let format = RiseDocumentBlockKind::Code.projection_format();
                self.push(format.map_or("", |format| format.open_delimiter()));
                self.push(body);
                self.push(format.map_or("", |format| format.close_delimiter()));
            }

            RiseDocumentBlock::Formula { text: body } => {
                let format = RiseDocumentBlockKind::Formula.projection_format();
                self.push(format.map_or("", |format| format.open_delimiter()));
                self.push(body);
                self.push(format.map_or("", |format| format.close_delimiter()));
            }

            RiseDocumentBlock::List { style, items } => {
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        self.push(affix::block_separator());
                    }
                    let marker = marker(*style, item, index + 1);
                    self.push(&marker);
                    let mut inner = Builder::default();
                    inner.append_blocks(&item.blocks);
                    self.append_nested(inner.finish(), false);
                }
            }

            RiseDocumentBlock::Collapsible { title, blocks, .. } => {
                self.append_runs(title);
                if !blocks.is_empty() {
                    self.push(affix::block_separator());
                    let mut inner = Builder::default();
                    inner.append_blocks(blocks);
                    self.append_nested(inner.finish(), false);
                }
            }

            RiseDocumentBlock::Table(table) => {
                for (row_index, row) in table.rows.iter().enumerate() {
                    if row_index > 0 {
                        self.push(affix::block_separator());
                    }
                    for (cell_index, cell) in row.cells.iter().enumerate() {
                        if cell_index > 0 {
                            self.push(affix::table_cell_separator());
                        }
                        let mut inner = Builder::default();
                        inner.append_blocks(&cell.blocks);
                        self.append_nested(inner.finish(), true);
                    }
                }
            }

            RiseDocumentBlock::ButtonRow { buttons } => {
                for (index, button) in buttons.iter().enumerate() {
                    if index > 0 {
                        self.push(affix::button_separator());
                    }
                    self.push(&button.label);
                }
            }

            RiseDocumentBlock::Media { .. } => {}

            RiseDocumentBlock::Divider => self.push(affix::divider()),

            RiseDocumentBlock::Unsupported { .. } => {}
        }
    }

    /// splices a projected fragment in, rebasing its references onto this builder offsets
    fn append_nested(&mut self, nested: RiseDocumentProjectionOutput, collapsing_newlines: bool) {
        let base = self.text_utf16;
        if collapsing_newlines {
            self.push(&nested.content.replace('\n', " "));
        } else {
            self.push(&nested.content);
        }
        for reference in nested.references {
            self.references.push(RiseDocumentProjectedReference {
                offset: base + reference.offset,
                length: reference.length,
                reference: reference.reference,
            });
        }
    }

    fn append_prefixed_lines(&mut self, nested: RiseDocumentProjectionOutput, prefix: &str) {
        let prefix_length = length_utf16(prefix);
        let lines: Vec<&str> = nested.content.split('\n').collect();
        let mut line_starts: Vec<usize> = Vec::with_capacity(lines.len());
        let mut cursor = 0;
        for line in &lines {
            line_starts.push(cursor);
            cursor += length_utf16(line) + 1;
        }
        let base = self.text_utf16;
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                self.push("\n");
            }
            self.push(prefix);
            self.push(line);
        }
        for reference in nested.references {
            let preceding_lines = line_starts.iter().filter(|start| **start <= reference.offset).count();
            self.references.push(RiseDocumentProjectedReference {
                offset: base + reference.offset + preceding_lines * prefix_length,
                length: reference.length,
                reference: reference.reference,
            });
        }
    }

    fn append_runs(&mut self, runs: &[RiseDocumentRun]) {
        for run in runs {
            self.append_run(run);
        }
    }

    fn append_run(&mut self, run: &RiseDocumentRun) {
        if let Some(reference) = &run.reference {
            let piece = match reference {
                RiseDocumentReference::CustomEmoji { .. } => RiseDocumentProjection::CUSTOM_EMOJI_PLACEHOLDER,
                RiseDocumentReference::Mention { .. } | RiseDocumentReference::RoleMention { .. } => {
                    run.text.as_str()
                }
            };
            let offset = self.text_utf16;
            self.push(piece);
            self.references.push(RiseDocumentProjectedReference {
                offset,
                length: length_utf16(piece),
                reference: reference.clone(),
            });
            return;
        }

        if run.marks.contains(&RiseDocumentInlineMark::Spoiler) {
            self.push(affix::spoiler_mask());
            return;
        }

        let mut piece = run.text.clone();
        for mark in RiseDocumentInlineMark::ALL.iter().filter(|mark| run.marks.contains(*mark)) {
            let Some(format) = mark.legacy_format() else {
                continue;
            };
            piece = format!("{}{}{}", format.open_delimiter(), piece, format.close_delimiter());
        }
        if run.marks.contains(&RiseDocumentInlineMark::Link)
            && let Some(url) = run.url.as_deref()
            && !url.is_empty()
            && url != run.text
        {
            piece.push_str(affix::link_url_open());
            piece.push_str(url);
            piece.push_str(affix::link_url_close());
        }
        self.push(&piece);
    }

    fn finish(self) -> RiseDocumentProjectionOutput {
        RiseDocumentProjectionOutput { content: self.text, references: self.references }
    }
}

fn marker(style: RiseDocumentListStyle, item: &RiseDocumentListItem, position: usize) -> String {
    match style {
        RiseDocumentListStyle::Bulleted => affix::bulleted_item().to_owned(),
        RiseDocumentListStyle::Numbered => affix::numbered_item().replace(affix::NUMBER_TOKEN, &position.to_string()),
        RiseDocumentListStyle::Task => {
            if item.is_done.unwrap_or(false) {
                affix::task_item_done().to_owned()
            } else {
                affix::task_item_open().to_owned()
            }
        }
    }
}

impl RiseDocumentProjectionOutput {
    /// the server trims content which would shift every reference offset
    /// trimming here and moving references to match keeps both sides on the same string
    pub fn trimmed_for_wire(&self) -> Self {
        let content = self.content.as_str();
        let after_leading = content.trim_start_matches(is_whitespace_or_newline);
        let trimmed = after_leading.trim_end_matches(is_whitespace_or_newline);
        if trimmed.len() == content.len() {
            return self.clone();
        }
        let start = length_utf16(&content[..content.len() - after_leading.len()]);
        let length = length_utf16(trimmed);
        let moved = self
            .references
            .iter()
            .filter_map(|reference| {
                let offset = reference.offset.checked_sub(start)?;
                (offset + reference.length <= length).then(|| RiseDocumentProjectedReference {
                    offset,
                    length: reference.length,
                    reference: reference.reference.clone(),
                })
            })
            .collect();
        Self { content: trimmed.to_owned(), references: moved }
    }
}
