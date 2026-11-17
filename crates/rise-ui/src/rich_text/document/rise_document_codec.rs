use std::collections::{BTreeMap, HashSet};

use rise_canon::rise_key::document as key;
use rise_canon::{RiseDocumentBlockKind, RiseDocumentInlineMark, RiseDocumentInlineRef, RiseDocumentListStyle};
use rise_engine::RiseJSONValue;

use super::rise_document::{
    RiseDocument, RiseDocumentBlock, RiseDocumentButton, RiseDocumentListItem, RiseDocumentReference,
    RiseDocumentRun, RiseDocumentTable, RiseDocumentTableCell, RiseDocumentTableRow,
};

mod k {
    use super::key;

    pub const VERSION: &str = key::V;
    pub const BLOCKS: &str = key::B;
    pub const KIND: &str = key::K;
    pub const SPANS: &str = key::S;
    pub const LEVEL: &str = key::LVL;
    pub const LANGUAGE: &str = key::LANG;
    pub const TEXT: &str = key::TX;
    pub const STYLE: &str = key::ST;
    pub const ITEMS: &str = key::IT;
    pub const ID: &str = key::ID;
    pub const DONE: &str = key::DONE;
    pub const TITLE: &str = key::TTL;
    pub const EXPANDED: &str = key::EXP;
    pub const COLUMNS: &str = key::COLS;
    pub const HEADER: &str = key::HDR;
    pub const WIDTHS: &str = key::W;
    pub const ROWS: &str = key::R;
    pub const CELLS: &str = key::C;
    pub const BUTTONS: &str = key::BTNS;
    pub const LABEL: &str = key::LBL;
    pub const URL: &str = key::URL;
    pub const INDEX: &str = key::IDX;

    pub const RUN_TEXT: &str = key::T;
    pub const RUN_MARKS: &str = key::M;
    pub const RUN_URL: &str = key::U;
    pub const RUN_REF: &str = key::R;

    pub const REF_KIND: &str = key::K;
    pub const REF_EMOJI_ID: &str = key::ID;
    pub const REF_EMOJI_PACK: &str = key::PACK;
    pub const REF_USER_ID: &str = key::UID;
    pub const REF_ROLE_ID: &str = key::RID;
    pub const REF_ROLE_COLOR: &str = key::COLOR;
}

type JsonObject = BTreeMap<String, RiseJSONValue>;

pub enum RiseDocumentCodec {}

impl RiseDocumentCodec {
    /// none for absent, empty or malformed input
    /// a message whose document cant be parsed still has readable content to fall back to
    pub fn decode(raw: Option<&str>) -> Option<RiseDocument> {
        let raw = raw.filter(|raw| !raw.is_empty())?;
        let value = RiseJSONValue::decode(raw.as_bytes()).ok()?;
        Self::decode_value(&value)
    }

    pub fn decode_value(value: &RiseJSONValue) -> Option<RiseDocument> {
        let object = value.object_value()?;
        let version = integer(object.get(k::VERSION)).unwrap_or(RiseDocument::CURRENT_VERSION);
        let Some(RiseJSONValue::Array(raw_blocks)) = object.get(k::BLOCKS) else {
            return None;
        };
        let blocks = raw_blocks.iter().map(decode_block).collect();
        Some(RiseDocument::with_version(version, blocks))
    }

    pub fn encode(document: &RiseDocument) -> RiseJSONValue {
        let mut object = JsonObject::new();
        object.insert(k::VERSION.to_owned(), RiseJSONValue::Integer(document.version));
        object.insert(
            k::BLOCKS.to_owned(),
            RiseJSONValue::Array(document.blocks.iter().map(encode_block).collect()),
        );
        RiseJSONValue::Object(object)
    }

    pub fn encode_to_string(document: &RiseDocument) -> Option<String> {
        let data = Self::encode(document).data(true).ok()?;
        String::from_utf8(data).ok()
    }
}

fn decode_block(value: &RiseJSONValue) -> RiseDocumentBlock {
    let unsupported = || RiseDocumentBlock::Unsupported { raw: value.clone() };
    let Some(object) = value.object_value() else {
        return unsupported();
    };
    let Some(kind) = object
        .get(k::KIND)
        .and_then(RiseJSONValue::string_value)
        .and_then(RiseDocumentBlockKind::from_wire)
    else {
        return unsupported();
    };
    match kind {
        RiseDocumentBlockKind::Paragraph => RiseDocumentBlock::Paragraph { runs: decode_runs(object.get(k::SPANS)) },
        RiseDocumentBlockKind::Heading => {
            let level = integer(object.get(k::LEVEL)).unwrap_or(1);
            RiseDocumentBlock::Heading { level: level.clamp(1, 3), runs: decode_runs(object.get(k::SPANS)) }
        }
        RiseDocumentBlockKind::Quote => RiseDocumentBlock::Quote { blocks: decode_blocks(object.get(k::BLOCKS)) },
        RiseDocumentBlockKind::PullQuote => {
            RiseDocumentBlock::PullQuote { blocks: decode_blocks(object.get(k::BLOCKS)) }
        }
        RiseDocumentBlockKind::Code => RiseDocumentBlock::Code {
            language: string(object, k::LANGUAGE),
            text: string(object, k::TEXT).unwrap_or_default(),
        },
        RiseDocumentBlockKind::Formula => {
            RiseDocumentBlock::Formula { text: string(object, k::TEXT).unwrap_or_default() }
        }
        RiseDocumentBlockKind::List => {
            let style = object
                .get(k::STYLE)
                .and_then(RiseJSONValue::string_value)
                .and_then(RiseDocumentListStyle::from_wire)
                .unwrap_or(RiseDocumentListStyle::Bulleted);
            RiseDocumentBlock::List { style, items: decode_list_items(object.get(k::ITEMS)) }
        }
        RiseDocumentBlockKind::Collapsible => RiseDocumentBlock::Collapsible {
            title: decode_runs(object.get(k::TITLE)),
            expanded: boolean(object.get(k::EXPANDED)).unwrap_or(false),
            blocks: decode_blocks(object.get(k::BLOCKS)),
        },
        RiseDocumentBlockKind::Table => RiseDocumentBlock::Table(decode_table(object)),
        RiseDocumentBlockKind::ButtonRow => {
            RiseDocumentBlock::ButtonRow { buttons: decode_buttons(object.get(k::BUTTONS)) }
        }
        RiseDocumentBlockKind::Media => {
            match integer(object.get(k::INDEX)).and_then(|index| usize::try_from(index).ok()) {
                Some(index) => RiseDocumentBlock::Media { index },
                None => unsupported(),
            }
        }
        RiseDocumentBlockKind::Divider => RiseDocumentBlock::Divider,
    }
}

fn decode_blocks(value: Option<&RiseJSONValue>) -> Vec<RiseDocumentBlock> {
    match value {
        Some(RiseJSONValue::Array(items)) => items.iter().map(decode_block).collect(),
        _ => Vec::new(),
    }
}

fn decode_list_items(value: Option<&RiseJSONValue>) -> Vec<RiseDocumentListItem> {
    let Some(RiseJSONValue::Array(items)) = value else {
        return Vec::new();
    };
    let empty = JsonObject::new();
    items
        .iter()
        .map(|item| {
            let object = item.object_value().unwrap_or(&empty);
            RiseDocumentListItem {
                id: string(object, k::ID),
                is_done: boolean(object.get(k::DONE)),
                blocks: decode_blocks(object.get(k::BLOCKS)),
            }
        })
        .collect()
}

fn decode_table(object: &JsonObject) -> RiseDocumentTable {
    let mut rows: Vec<RiseDocumentTableRow> = Vec::new();
    if let Some(RiseJSONValue::Array(raw_rows)) = object.get(k::ROWS) {
        rows = raw_rows
            .iter()
            .map(|row| {
                let Some(RiseJSONValue::Array(raw_cells)) = row.object_value().and_then(|row| row.get(k::CELLS))
                else {
                    return RiseDocumentTableRow { cells: Vec::new() };
                };
                RiseDocumentTableRow {
                    cells: raw_cells
                        .iter()
                        .map(|cell| RiseDocumentTableCell {
                            blocks: decode_blocks(cell.object_value().and_then(|cell| cell.get(k::BLOCKS))),
                        })
                        .collect(),
                }
            })
            .collect();
    }
    let mut widths: Option<Vec<f64>> = None;
    if let Some(RiseJSONValue::Array(raw_widths)) = object.get(k::WIDTHS) {
        let parsed: Vec<f64> = raw_widths.iter().filter_map(|width| number(Some(width))).collect();
        if parsed.len() == raw_widths.len() && !parsed.is_empty() {
            widths = Some(parsed);
        }
    }
    let backed = rows
        .iter()
        .map(|row| row.cells.len())
        .max()
        .unwrap_or(0)
        .max(widths.as_ref().map_or(0, Vec::len));
    let declared = integer(object.get(k::COLUMNS)).unwrap_or(i64::try_from(backed).unwrap_or(i64::MAX));
    let declared = usize::try_from(declared.max(0)).unwrap_or(usize::MAX);
    RiseDocumentTable {
        column_count: declared.min(backed),
        has_header_row: boolean(object.get(k::HEADER)).unwrap_or(false),
        column_widths: widths,
        rows,
    }
}

fn decode_buttons(value: Option<&RiseJSONValue>) -> Vec<RiseDocumentButton> {
    let Some(RiseJSONValue::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let object = item.object_value()?;
            let label = string(object, k::LABEL)?;
            let url = string(object, k::URL)?;
            Some(RiseDocumentButton { id: string(object, k::ID).unwrap_or_default(), label, url })
        })
        .collect()
}

fn decode_runs(value: Option<&RiseJSONValue>) -> Vec<RiseDocumentRun> {
    let Some(RiseJSONValue::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let object = item.object_value()?;
            let mut marks: HashSet<RiseDocumentInlineMark> = HashSet::new();
            if let Some(RiseJSONValue::Array(raw_marks)) = object.get(k::RUN_MARKS) {
                for mark in raw_marks {
                    if let Some(parsed) = mark.string_value().and_then(RiseDocumentInlineMark::from_wire) {
                        marks.insert(parsed);
                    }
                }
            }
            Some(RiseDocumentRun::with(
                string(object, k::RUN_TEXT).unwrap_or_default(),
                marks,
                string(object, k::RUN_URL),
                decode_reference(object.get(k::RUN_REF)),
            ))
        })
        .collect()
}

fn decode_reference(value: Option<&RiseJSONValue>) -> Option<RiseDocumentReference> {
    let object = value?.object_value()?;
    let kind = object
        .get(k::REF_KIND)
        .and_then(RiseJSONValue::string_value)
        .and_then(RiseDocumentInlineRef::from_wire)?;
    match kind {
        RiseDocumentInlineRef::CustomEmoji => {
            let id = string(object, k::REF_EMOJI_ID).filter(|id| !id.is_empty())?;
            Some(RiseDocumentReference::CustomEmoji { id, pack_id: string(object, k::REF_EMOJI_PACK) })
        }
        RiseDocumentInlineRef::Mention => {
            let user_id = string(object, k::REF_USER_ID).filter(|id| !id.is_empty())?;
            Some(RiseDocumentReference::Mention { user_id })
        }
        RiseDocumentInlineRef::RoleMention => {
            let role_id = string(object, k::REF_ROLE_ID).filter(|id| !id.is_empty())?;
            Some(RiseDocumentReference::RoleMention { role_id, color: string(object, k::REF_ROLE_COLOR) })
        }
    }
}

fn encode_block(block: &RiseDocumentBlock) -> RiseJSONValue {
    if let RiseDocumentBlock::Unsupported { raw } = block {
        return raw.clone();
    }
    let Some(kind) = block.kind() else {
        return RiseJSONValue::Null;
    };
    let mut object = JsonObject::new();
    object.insert(k::KIND.to_owned(), RiseJSONValue::String(kind.wire().to_owned()));
    match block {
        RiseDocumentBlock::Paragraph { runs } => {
            object.insert(k::SPANS.to_owned(), encode_runs(runs));
        }
        RiseDocumentBlock::Heading { level, runs } => {
            object.insert(k::LEVEL.to_owned(), RiseJSONValue::Integer(*level));
            object.insert(k::SPANS.to_owned(), encode_runs(runs));
        }
        RiseDocumentBlock::Quote { blocks } | RiseDocumentBlock::PullQuote { blocks } => {
            object.insert(k::BLOCKS.to_owned(), encode_blocks(blocks));
        }
        RiseDocumentBlock::Code { language, text } => {
            if let Some(language) = language.as_ref().filter(|language| !language.is_empty()) {
                object.insert(k::LANGUAGE.to_owned(), RiseJSONValue::String(language.clone()));
            }
            object.insert(k::TEXT.to_owned(), RiseJSONValue::String(text.clone()));
        }
        RiseDocumentBlock::Formula { text } => {
            object.insert(k::TEXT.to_owned(), RiseJSONValue::String(text.clone()));
        }
        RiseDocumentBlock::List { style, items } => {
            object.insert(k::STYLE.to_owned(), RiseJSONValue::String(style.wire().to_owned()));
            let items = items
                .iter()
                .map(|item| {
                    let mut entry = JsonObject::new();
                    entry.insert(k::BLOCKS.to_owned(), encode_blocks(&item.blocks));
                    if let Some(id) = item.id.as_ref().filter(|id| !id.is_empty()) {
                        entry.insert(k::ID.to_owned(), RiseJSONValue::String(id.clone()));
                    }
                    if let Some(is_done) = item.is_done {
                        entry.insert(k::DONE.to_owned(), RiseJSONValue::Bool(is_done));
                    }
                    RiseJSONValue::Object(entry)
                })
                .collect();
            object.insert(k::ITEMS.to_owned(), RiseJSONValue::Array(items));
        }
        RiseDocumentBlock::Collapsible { title, expanded, blocks } => {
            object.insert(k::TITLE.to_owned(), encode_runs(title));
            object.insert(k::EXPANDED.to_owned(), RiseJSONValue::Bool(*expanded));
            object.insert(k::BLOCKS.to_owned(), encode_blocks(blocks));
        }
        RiseDocumentBlock::Table(table) => {
            object.insert(
                k::COLUMNS.to_owned(),
                RiseJSONValue::Integer(i64::try_from(table.column_count).unwrap_or(i64::MAX)),
            );
            object.insert(k::HEADER.to_owned(), RiseJSONValue::Bool(table.has_header_row));
            if let Some(widths) = &table.column_widths {
                object.insert(
                    k::WIDTHS.to_owned(),
                    RiseJSONValue::Array(widths.iter().map(|width| RiseJSONValue::Number(*width)).collect()),
                );
            }
            let rows = table
                .rows
                .iter()
                .map(|row| {
                    let cells = row
                        .cells
                        .iter()
                        .map(|cell| {
                            let mut entry = JsonObject::new();
                            entry.insert(k::BLOCKS.to_owned(), encode_blocks(&cell.blocks));
                            RiseJSONValue::Object(entry)
                        })
                        .collect();
                    let mut entry = JsonObject::new();
                    entry.insert(k::CELLS.to_owned(), RiseJSONValue::Array(cells));
                    RiseJSONValue::Object(entry)
                })
                .collect();
            object.insert(k::ROWS.to_owned(), RiseJSONValue::Array(rows));
        }
        RiseDocumentBlock::ButtonRow { buttons } => {
            let buttons = buttons
                .iter()
                .map(|button| {
                    let mut entry = JsonObject::new();
                    entry.insert(k::ID.to_owned(), RiseJSONValue::String(button.id.clone()));
                    entry.insert(k::LABEL.to_owned(), RiseJSONValue::String(button.label.clone()));
                    entry.insert(k::URL.to_owned(), RiseJSONValue::String(button.url.clone()));
                    RiseJSONValue::Object(entry)
                })
                .collect();
            object.insert(k::BUTTONS.to_owned(), RiseJSONValue::Array(buttons));
        }
        RiseDocumentBlock::Media { index } => {
            object.insert(
                k::INDEX.to_owned(),
                RiseJSONValue::Integer(i64::try_from(*index).unwrap_or(i64::MAX)),
            );
        }
        RiseDocumentBlock::Divider | RiseDocumentBlock::Unsupported { .. } => {}
    }
    RiseJSONValue::Object(object)
}

fn encode_blocks(blocks: &[RiseDocumentBlock]) -> RiseJSONValue {
    RiseJSONValue::Array(blocks.iter().map(encode_block).collect())
}

fn encode_runs(runs: &[RiseDocumentRun]) -> RiseJSONValue {
    RiseJSONValue::Array(runs.iter().map(encode_run).collect())
}

fn encode_run(run: &RiseDocumentRun) -> RiseJSONValue {
    let mut object = JsonObject::new();
    object.insert(k::RUN_TEXT.to_owned(), RiseJSONValue::String(run.text.clone()));
    if !run.marks.is_empty() {
        let ordered = RiseDocumentInlineMark::ALL
            .iter()
            .filter(|mark| run.marks.contains(*mark))
            .map(|mark| RiseJSONValue::String(mark.wire().to_owned()))
            .collect();
        object.insert(k::RUN_MARKS.to_owned(), RiseJSONValue::Array(ordered));
    }
    if let Some(url) = run.url.as_ref().filter(|url| !url.is_empty()) {
        object.insert(k::RUN_URL.to_owned(), RiseJSONValue::String(url.clone()));
    }
    if let Some(reference) = &run.reference {
        object.insert(k::RUN_REF.to_owned(), encode_reference(reference));
    }
    RiseJSONValue::Object(object)
}

fn encode_reference(reference: &RiseDocumentReference) -> RiseJSONValue {
    let mut object = JsonObject::new();
    object.insert(k::REF_KIND.to_owned(), RiseJSONValue::String(reference.kind().wire().to_owned()));
    match reference {
        RiseDocumentReference::CustomEmoji { id, pack_id } => {
            object.insert(k::REF_EMOJI_ID.to_owned(), RiseJSONValue::String(id.clone()));
            if let Some(pack_id) = pack_id.as_ref().filter(|pack_id| !pack_id.is_empty()) {
                object.insert(k::REF_EMOJI_PACK.to_owned(), RiseJSONValue::String(pack_id.clone()));
            }
        }
        RiseDocumentReference::Mention { user_id } => {
            object.insert(k::REF_USER_ID.to_owned(), RiseJSONValue::String(user_id.clone()));
        }
        RiseDocumentReference::RoleMention { role_id, color } => {
            object.insert(k::REF_ROLE_ID.to_owned(), RiseJSONValue::String(role_id.clone()));
            if let Some(color) = color.as_ref().filter(|color| !color.is_empty()) {
                object.insert(k::REF_ROLE_COLOR.to_owned(), RiseJSONValue::String(color.clone()));
            }
        }
    }
    RiseJSONValue::Object(object)
}

fn string(object: &JsonObject, key: &str) -> Option<String> {
    object.get(key).and_then(RiseJSONValue::string_value).map(str::to_owned)
}

/// int from a value truncated toward zero
/// out of range or NaN or infinite reads as absent instead of trapping
fn integer(value: Option<&RiseJSONValue>) -> Option<i64> {
    match value? {
        RiseJSONValue::Integer(raw) => Some(*raw),
        RiseJSONValue::UnsignedInteger(raw) => i64::try_from(*raw).ok(),
        RiseJSONValue::Number(raw) => {
            let truncated = raw.trunc();
            (truncated.is_finite() && truncated >= i64::MIN as f64 && truncated < -(i64::MIN as f64))
                .then_some(truncated as i64)
        }
        RiseJSONValue::String(raw) => raw.parse::<i64>().ok(),
        _ => None,
    }
}

fn number(value: Option<&RiseJSONValue>) -> Option<f64> {
    match value? {
        RiseJSONValue::Number(raw) => Some(*raw),
        RiseJSONValue::Integer(raw) => Some(*raw as f64),
        RiseJSONValue::UnsignedInteger(raw) => Some(*raw as f64),
        _ => None,
    }
}

fn boolean(value: Option<&RiseJSONValue>) -> Option<bool> {
    match value? {
        RiseJSONValue::Bool(raw) => Some(*raw),
        RiseJSONValue::Integer(raw) => Some(*raw != 0),
        _ => None,
    }
}
