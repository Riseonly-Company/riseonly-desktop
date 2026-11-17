use rise_canon::{RiseDocumentBlockKind, RiseDocumentListStyle};
use rise_core::foundation::foundation_uuid::Uuid;

use super::rise_document::{
    RiseDocument, RiseDocumentBlock, RiseDocumentButton, RiseDocumentListItem, RiseDocumentRun, RiseDocumentTable,
};

/// one editable line of the editor, editor is flat while the document is a tree
/// so a three paragraph quote is three quote rows here and one quote block there
/// each row gets exactly one text view
#[derive(Clone, Debug, PartialEq)]
pub struct RiseDocumentEditorRow {
    pub id: Uuid,
    pub kind: RiseDocumentEditorRowKind,
    pub runs: Vec<RiseDocumentRun>,
    pub verbatim: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RiseDocumentEditorRowKind {
    Paragraph,
    Heading { level: i64 },
    Quote,
    PullQuote,
    Code { language: Option<String> },
    Formula,
    ListItem { style: RiseDocumentListStyle, is_done: bool },
    CollapsibleTitle { expanded: bool },
    CollapsibleBody,
    Divider,
    Media { index: usize },
    Buttons(Vec<RiseDocumentButton>),
    Table(RiseDocumentTable),
    Unsupported(RiseDocumentBlock),
}

impl RiseDocumentEditorRowKind {
    pub fn is_textual(&self) -> bool {
        match self {
            Self::Paragraph
            | Self::Heading { .. }
            | Self::Quote
            | Self::PullQuote
            | Self::Code { .. }
            | Self::Formula
            | Self::ListItem { .. }
            | Self::CollapsibleTitle { .. }
            | Self::CollapsibleBody => true,
            Self::Divider | Self::Media { .. } | Self::Buttons(_) | Self::Table(_) | Self::Unsupported(_) => false,
        }
    }

    /// true for rows holding plain text that refuse inline marks
    /// a bold run in a code block isnt code anymore
    pub fn is_verbatim(&self) -> bool {
        matches!(self, Self::Code { .. } | Self::Formula)
    }

    pub fn block_kind(&self) -> RiseDocumentBlockKind {
        match self {
            Self::Paragraph => RiseDocumentBlockKind::Paragraph,
            Self::Heading { .. } => RiseDocumentBlockKind::Heading,
            Self::Quote => RiseDocumentBlockKind::Quote,
            Self::PullQuote => RiseDocumentBlockKind::PullQuote,
            Self::Code { .. } => RiseDocumentBlockKind::Code,
            Self::Formula => RiseDocumentBlockKind::Formula,
            Self::ListItem { .. } => RiseDocumentBlockKind::List,
            Self::CollapsibleTitle { .. } | Self::CollapsibleBody => RiseDocumentBlockKind::Collapsible,
            Self::Divider => RiseDocumentBlockKind::Divider,
            Self::Media { .. } => RiseDocumentBlockKind::Media,
            Self::Buttons(_) => RiseDocumentBlockKind::ButtonRow,
            Self::Table(_) => RiseDocumentBlockKind::Table,
            Self::Unsupported(_) => RiseDocumentBlockKind::Paragraph,
        }
    }
}

impl Default for RiseDocumentEditorRow {
    fn default() -> Self {
        Self::new(RiseDocumentEditorRowKind::Paragraph, Vec::new(), String::new())
    }
}

impl RiseDocumentEditorRow {
    /// new row with a fresh id
    pub fn new(kind: RiseDocumentEditorRowKind, runs: Vec<RiseDocumentRun>, verbatim: String) -> Self {
        Self::with_id(Uuid::new_v4(), kind, runs, verbatim)
    }

    pub fn with_id(id: Uuid, kind: RiseDocumentEditorRowKind, runs: Vec<RiseDocumentRun>, verbatim: String) -> Self {
        Self { id, kind, runs, verbatim }
    }

    fn of_kind(kind: RiseDocumentEditorRowKind) -> Self {
        Self::new(kind, Vec::new(), String::new())
    }

    pub fn plain_text(&self) -> String {
        if self.kind.is_verbatim() {
            self.verbatim.clone()
        } else {
            self.runs.iter().map(|run| run.text.as_str()).collect()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.kind.is_textual() && self.plain_text().is_empty()
    }
}

/// what the caret is inside
/// a table cell edits like a paragraph so it is a focus target and not a special case
/// marks and the selection toolbar keep working
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseDocumentFocus {
    Row(Uuid),
    TableCell { row_id: Uuid, row: usize, column: usize },
}

impl RiseDocumentFocus {
    pub fn row_id(&self) -> Uuid {
        match self {
            Self::Row(id) => *id,
            Self::TableCell { row_id, .. } => *row_id,
        }
    }

    pub fn table_cell(&self) -> Option<(usize, usize)> {
        match self {
            Self::TableCell { row, column, .. } => Some((*row, *column)),
            Self::Row(_) => None,
        }
    }
}

pub enum RiseDocumentRowMapper {}

impl RiseDocumentRowMapper {
    pub fn rows(document: &RiseDocument) -> Vec<RiseDocumentEditorRow> {
        let mut out: Vec<RiseDocumentEditorRow> = Vec::new();
        append(&document.blocks, &mut out);
        if out.is_empty() {
            out.push(RiseDocumentEditorRow::default());
        }
        out
    }

    /// folds the flat rows back into a tree
    /// consecutive rows of the same container kind become one block, so a three line quote is one quote
    pub fn document(rows: &[RiseDocumentEditorRow]) -> RiseDocument {
        let mut blocks: Vec<RiseDocumentBlock> = Vec::new();
        let mut index = 0;
        while index < rows.len() {
            let row = &rows[index];
            match &row.kind {
                RiseDocumentEditorRowKind::Paragraph => {
                    blocks.push(RiseDocumentBlock::Paragraph { runs: row.runs.clone() });
                    index += 1;
                }
                RiseDocumentEditorRowKind::Heading { level } => {
                    blocks.push(RiseDocumentBlock::Heading { level: *level, runs: row.runs.clone() });
                    index += 1;
                }
                RiseDocumentEditorRowKind::Quote | RiseDocumentEditorRowKind::PullQuote => {
                    let is_pull = matches!(row.kind, RiseDocumentEditorRowKind::PullQuote);
                    let mut inner: Vec<RiseDocumentBlock> = Vec::new();
                    while index < rows.len() && same_container(&rows[index].kind, &row.kind) {
                        inner.push(RiseDocumentBlock::Paragraph { runs: rows[index].runs.clone() });
                        index += 1;
                    }
                    blocks.push(if is_pull {
                        RiseDocumentBlock::PullQuote { blocks: inner }
                    } else {
                        RiseDocumentBlock::Quote { blocks: inner }
                    });
                }
                RiseDocumentEditorRowKind::Code { language } => {
                    blocks.push(RiseDocumentBlock::Code { language: language.clone(), text: row.verbatim.clone() });
                    index += 1;
                }
                RiseDocumentEditorRowKind::Formula => {
                    blocks.push(RiseDocumentBlock::Formula { text: row.verbatim.clone() });
                    index += 1;
                }
                RiseDocumentEditorRowKind::ListItem { style, .. } => {
                    let style = *style;
                    let mut items: Vec<RiseDocumentListItem> = Vec::new();
                    while index < rows.len()
                        && let RiseDocumentEditorRowKind::ListItem { style: next_style, is_done } = &rows[index].kind
                        && *next_style == style
                    {
                        let is_task = style == RiseDocumentListStyle::Task;
                        items.push(RiseDocumentListItem {
                            id: is_task.then(|| task_identity(&rows[index])),
                            is_done: is_task.then_some(*is_done),
                            blocks: vec![RiseDocumentBlock::Paragraph { runs: rows[index].runs.clone() }],
                        });
                        index += 1;
                    }
                    blocks.push(RiseDocumentBlock::List { style, items });
                }
                RiseDocumentEditorRowKind::CollapsibleTitle { expanded } => {
                    let title = row.runs.clone();
                    let expanded = *expanded;
                    index += 1;
                    let mut inner: Vec<RiseDocumentBlock> = Vec::new();
                    while index < rows.len() && matches!(rows[index].kind, RiseDocumentEditorRowKind::CollapsibleBody) {
                        inner.push(RiseDocumentBlock::Paragraph { runs: rows[index].runs.clone() });
                        index += 1;
                    }
                    blocks.push(RiseDocumentBlock::Collapsible { title, expanded, blocks: inner });
                }
                RiseDocumentEditorRowKind::CollapsibleBody => {
                    blocks.push(RiseDocumentBlock::Paragraph { runs: row.runs.clone() });
                    index += 1;
                }
                RiseDocumentEditorRowKind::Divider => {
                    blocks.push(RiseDocumentBlock::Divider);
                    index += 1;
                }
                RiseDocumentEditorRowKind::Media { index: media_index } => {
                    blocks.push(RiseDocumentBlock::Media { index: *media_index });
                    index += 1;
                }
                RiseDocumentEditorRowKind::Buttons(buttons) => {
                    blocks.push(RiseDocumentBlock::ButtonRow { buttons: buttons.clone() });
                    index += 1;
                }
                RiseDocumentEditorRowKind::Table(table) => {
                    blocks.push(RiseDocumentBlock::Table(table.clone()));
                    index += 1;
                }
                RiseDocumentEditorRowKind::Unsupported(block) => {
                    blocks.push(block.clone());
                    index += 1;
                }
            }
        }
        blocks.retain(|block| !block.is_empty() || !is_droppable(block));
        RiseDocument::new(blocks)
    }
}

fn append(blocks: &[RiseDocumentBlock], out: &mut Vec<RiseDocumentEditorRow>) {
    for block in blocks {
        match block {
            RiseDocumentBlock::Paragraph { runs } => {
                out.push(RiseDocumentEditorRow::new(RiseDocumentEditorRowKind::Paragraph, runs.clone(), String::new()));
            }
            RiseDocumentBlock::Heading { level, runs } => {
                out.push(RiseDocumentEditorRow::new(
                    RiseDocumentEditorRowKind::Heading { level: *level },
                    runs.clone(),
                    String::new(),
                ));
            }
            RiseDocumentBlock::Quote { blocks } => {
                out.extend(textual_rows(blocks, &RiseDocumentEditorRowKind::Quote));
            }
            RiseDocumentBlock::PullQuote { blocks } => {
                out.extend(textual_rows(blocks, &RiseDocumentEditorRowKind::PullQuote));
            }
            RiseDocumentBlock::Code { language, text } => {
                out.push(RiseDocumentEditorRow::new(
                    RiseDocumentEditorRowKind::Code { language: language.clone() },
                    Vec::new(),
                    text.clone(),
                ));
            }
            RiseDocumentBlock::Formula { text } => {
                out.push(RiseDocumentEditorRow::new(RiseDocumentEditorRowKind::Formula, Vec::new(), text.clone()));
            }
            RiseDocumentBlock::List { style, items } => {
                for item in items {
                    out.push(RiseDocumentEditorRow::new(
                        RiseDocumentEditorRowKind::ListItem { style: *style, is_done: item.is_done.unwrap_or(false) },
                        first_paragraph_runs(&item.blocks),
                        String::new(),
                    ));
                }
            }
            RiseDocumentBlock::Collapsible { title, expanded, blocks } => {
                out.push(RiseDocumentEditorRow::new(
                    RiseDocumentEditorRowKind::CollapsibleTitle { expanded: *expanded },
                    title.clone(),
                    String::new(),
                ));
                out.extend(textual_rows(blocks, &RiseDocumentEditorRowKind::CollapsibleBody));
            }
            RiseDocumentBlock::Table(table) => {
                out.push(RiseDocumentEditorRow::of_kind(RiseDocumentEditorRowKind::Table(table.clone())));
            }
            RiseDocumentBlock::ButtonRow { buttons } => {
                out.push(RiseDocumentEditorRow::of_kind(RiseDocumentEditorRowKind::Buttons(buttons.clone())));
            }
            RiseDocumentBlock::Media { index } => {
                out.push(RiseDocumentEditorRow::of_kind(RiseDocumentEditorRowKind::Media { index: *index }));
            }
            RiseDocumentBlock::Divider => {
                out.push(RiseDocumentEditorRow::of_kind(RiseDocumentEditorRowKind::Divider));
            }
            RiseDocumentBlock::Unsupported { .. } => {
                out.push(RiseDocumentEditorRow::of_kind(RiseDocumentEditorRowKind::Unsupported(block.clone())));
            }
        }
    }
}

fn textual_rows(blocks: &[RiseDocumentBlock], kind: &RiseDocumentEditorRowKind) -> Vec<RiseDocumentEditorRow> {
    let mut nested: Vec<RiseDocumentEditorRow> = Vec::new();
    append(blocks, &mut nested);
    for row in &mut nested {
        if matches!(row.kind, RiseDocumentEditorRowKind::Paragraph) {
            row.kind = kind.clone();
        }
    }
    nested
}

fn first_paragraph_runs(blocks: &[RiseDocumentBlock]) -> Vec<RiseDocumentRun> {
    for block in blocks {
        match block {
            RiseDocumentBlock::Paragraph { runs } | RiseDocumentBlock::Heading { runs, .. } => return runs.clone(),
            _ => {}
        }
    }
    Vec::new()
}

fn same_container(lhs: &RiseDocumentEditorRowKind, rhs: &RiseDocumentEditorRowKind) -> bool {
    matches!(
        (lhs, rhs),
        (RiseDocumentEditorRowKind::Quote, RiseDocumentEditorRowKind::Quote)
            | (RiseDocumentEditorRowKind::PullQuote, RiseDocumentEditorRowKind::PullQuote)
    )
}

/// a task item keeps a stable eight hex identity so collab mode can address one checkbox
/// without a wire migration
fn task_identity(row: &RiseDocumentEditorRow) -> String {
    row.id.uuid_string().replace('-', "").chars().take(8).collect::<String>().to_lowercase()
}

fn is_droppable(block: &RiseDocumentBlock) -> bool {
    !matches!(block, RiseDocumentBlock::Paragraph { .. })
}
