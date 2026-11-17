use std::collections::HashSet;

use rise_canon::{RiseDocumentBlockKind, RiseDocumentInlineMark, RiseDocumentInlineRef, RiseDocumentListStyle};
use rise_engine::RiseJSONValue;

/// a formatted message document, an ordered list of typed blocks
/// source of truth for what the author wrote
/// the message content carries a deterministic plain text projection of it
#[derive(Clone, Debug, PartialEq)]
pub struct RiseDocument {
    pub version: i64,
    pub blocks: Vec<RiseDocumentBlock>,
}

impl RiseDocument {
    pub const CURRENT_VERSION: i64 = 1;

    pub const EMPTY: RiseDocument = RiseDocument { version: Self::CURRENT_VERSION, blocks: Vec::new() };

    /// new document from blocks, current version
    pub fn new(blocks: Vec<RiseDocumentBlock>) -> Self {
        Self { version: Self::CURRENT_VERSION, blocks }
    }

    pub fn with_version(version: i64, blocks: Vec<RiseDocumentBlock>) -> Self {
        Self { version, blocks }
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(RiseDocumentBlock::is_empty)
    }

    pub fn has_unsupported_blocks(&self) -> bool {
        self.blocks.iter().any(RiseDocumentBlock::contains_unsupported)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RiseDocumentBlock {
    Paragraph { runs: Vec<RiseDocumentRun> },
    Heading { level: i64, runs: Vec<RiseDocumentRun> },
    Quote { blocks: Vec<RiseDocumentBlock> },
    PullQuote { blocks: Vec<RiseDocumentBlock> },
    Code { language: Option<String>, text: String },
    Formula { text: String },
    List { style: RiseDocumentListStyle, items: Vec<RiseDocumentListItem> },
    Collapsible { title: Vec<RiseDocumentRun>, expanded: bool, blocks: Vec<RiseDocumentBlock> },
    Table(RiseDocumentTable),
    ButtonRow { buttons: Vec<RiseDocumentButton> },
    Media { index: usize },
    Divider,
    Unsupported { raw: RiseJSONValue },
}

impl RiseDocumentBlock {
    pub fn kind(&self) -> Option<RiseDocumentBlockKind> {
        match self {
            Self::Paragraph { .. } => Some(RiseDocumentBlockKind::Paragraph),
            Self::Heading { .. } => Some(RiseDocumentBlockKind::Heading),
            Self::Quote { .. } => Some(RiseDocumentBlockKind::Quote),
            Self::PullQuote { .. } => Some(RiseDocumentBlockKind::PullQuote),
            Self::Code { .. } => Some(RiseDocumentBlockKind::Code),
            Self::Formula { .. } => Some(RiseDocumentBlockKind::Formula),
            Self::List { .. } => Some(RiseDocumentBlockKind::List),
            Self::Collapsible { .. } => Some(RiseDocumentBlockKind::Collapsible),
            Self::Table(_) => Some(RiseDocumentBlockKind::Table),
            Self::ButtonRow { .. } => Some(RiseDocumentBlockKind::ButtonRow),
            Self::Media { .. } => Some(RiseDocumentBlockKind::Media),
            Self::Divider => Some(RiseDocumentBlockKind::Divider),
            Self::Unsupported { .. } => None,
        }
    }

    pub fn children(&self) -> Vec<&RiseDocumentBlock> {
        match self {
            Self::Quote { blocks } | Self::PullQuote { blocks } => blocks.iter().collect(),
            Self::Collapsible { blocks, .. } => blocks.iter().collect(),
            Self::List { items, .. } => items.iter().flat_map(|item| item.blocks.iter()).collect(),
            Self::Table(table) => table
                .rows
                .iter()
                .flat_map(|row| row.cells.iter().flat_map(|cell| cell.blocks.iter()))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::Paragraph { runs } | Self::Heading { runs, .. } => {
                runs.iter().all(|run| run.text.is_empty() && run.reference.is_none())
            }
            Self::Code { text, .. } | Self::Formula { text } => text.is_empty(),
            Self::Quote { blocks } | Self::PullQuote { blocks } => blocks.iter().all(Self::is_empty),
            Self::Collapsible { title, blocks, .. } => {
                title.iter().all(|run| run.text.is_empty()) && blocks.iter().all(Self::is_empty)
            }
            Self::List { items, .. } => items.iter().all(|item| item.blocks.iter().all(Self::is_empty)),
            Self::ButtonRow { buttons } => buttons.is_empty(),
            Self::Table(_) | Self::Media { .. } | Self::Divider | Self::Unsupported { .. } => false,
        }
    }

    pub fn contains_unsupported(&self) -> bool {
        if matches!(self, Self::Unsupported { .. }) {
            return true;
        }
        self.children().into_iter().any(Self::contains_unsupported)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseDocumentListItem {
    pub id: Option<String>,
    pub is_done: Option<bool>,
    pub blocks: Vec<RiseDocumentBlock>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseDocumentTable {
    pub column_count: usize,
    pub has_header_row: bool,
    pub column_widths: Option<Vec<f64>>,
    pub rows: Vec<RiseDocumentTableRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseDocumentTableRow {
    pub cells: Vec<RiseDocumentTableCell>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseDocumentTableCell {
    pub blocks: Vec<RiseDocumentBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseDocumentButton {
    pub id: String,
    pub label: String,
    pub url: String,
}

/// one stretch of text with a set of marks
/// neighbour runs with equal marks merge on every mutation so serialization and the
/// projection stay byte comparable across clients
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseDocumentRun {
    pub text: String,
    pub marks: HashSet<RiseDocumentInlineMark>,
    pub url: Option<String>,
    pub reference: Option<RiseDocumentReference>,
}

impl RiseDocumentRun {
    /// new run with no marks, no url and no reference
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), marks: HashSet::new(), url: None, reference: None }
    }

    /// new run with text, marks url and reference
    pub fn with(
        text: impl Into<String>,
        marks: HashSet<RiseDocumentInlineMark>,
        url: Option<String>,
        reference: Option<RiseDocumentReference>,
    ) -> Self {
        Self { text: text.into(), marks, url, reference }
    }

    pub fn is_plain(&self) -> bool {
        self.marks.is_empty() && self.url.is_none() && self.reference.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RiseDocumentReference {
    CustomEmoji { id: String, pack_id: Option<String> },
    Mention { user_id: String },
    RoleMention { role_id: String, color: Option<String> },
}

impl RiseDocumentReference {
    pub fn kind(&self) -> RiseDocumentInlineRef {
        match self {
            Self::CustomEmoji { .. } => RiseDocumentInlineRef::CustomEmoji,
            Self::Mention { .. } => RiseDocumentInlineRef::Mention,
            Self::RoleMention { .. } => RiseDocumentInlineRef::RoleMention,
        }
    }
}
