use std::sync::LazyLock;

use rise_canon::{ChatMessageEntityField, RiseLimits};
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::foundation_text::{characters, is_whitespace_or_newline};
use rise_validate::{MESSAGE_CONTENT, MESSAGE_WIRE_JSON, TextSchema, Unit};
use serde_json::{Map, Number, Value};

use crate::core::lib::canon_rules::{RiseLimitsCanonRules, ValidationSchemaCanonRules};

/// long text goes out as several messages, cut at a line break or space
/// never inside an entity
pub enum ChatComposerTextSplitter {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatComposerTextSplitterPart {
    pub text: String,
    pub entities: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatComposerTextSplitterBudget {
    pub characters: usize,
    pub character_unit: Unit,
    pub entity_count: usize,
    pub entity_bytes: usize,
    pub parts: usize,
}

static MESSAGE_BUDGET: LazyLock<ChatComposerTextSplitterBudget> = LazyLock::new(|| {
    let content = ChatComposerTextSplitterBudget::max_check(&MESSAGE_CONTENT);
    let wire = ChatComposerTextSplitterBudget::max_check(&MESSAGE_WIRE_JSON);
    ChatComposerTextSplitterBudget {
        characters: content.0,
        character_unit: content.1,
        entity_count: ChatComposerTextSplitterBudget::canon_count(RiseLimits::service_value("message_service_max_entities_count")),
        entity_bytes: wire.0,
        parts: ChatComposerTextSplitterBudget::canon_count(RiseLimits::client_value("chat_composer_split_max_parts")),
    }
});

impl ChatComposerTextSplitterBudget {
    pub fn message() -> ChatComposerTextSplitterBudget {
        *MESSAGE_BUDGET
    }

    fn max_check(schema: &TextSchema) -> (usize, Unit) {
        schema.maximum().unwrap_or((usize::MAX, Unit::Chars))
    }

    fn canon_count(value: i64) -> usize {
        usize::try_from(value).unwrap_or(0)
    }
}

struct ChatComposerTextSplitterSpan {
    start: usize,
    end: usize,
    bytes: usize,
    object: Map<String, Value>,
}

impl ChatComposerTextSplitter {
    /// cheap enough for every composer change, counts first and parses json only when there are
    /// entities
    pub fn exceeds_one_message(text: &str, entities: Option<&str>, budget: &ChatComposerTextSplitterBudget) -> bool {
        if text.len() > budget.characters && budget.character_unit.count(text) > budget.characters {
            return true;
        }
        let Some(entities) = entities.filter(|entities| !entities.is_empty()) else {
            return false;
        };
        if entities.len() > budget.entity_bytes {
            return true;
        }
        let Some(Value::Array(array)) = JSONSerialization::json_object(entities.as_bytes()) else {
            return false;
        };
        array.len() > budget.entity_count
    }

    /// none when the text needs more messages than the budget allows
    pub fn split(text: &str, entities: Option<&str>, budget: &ChatComposerTextSplitterBudget) -> Option<Vec<ChatComposerTextSplitterPart>> {
        if !Self::exceeds_one_message(text, entities, budget) {
            return Some(if text.is_empty() {
                Vec::new()
            } else {
                vec![ChatComposerTextSplitterPart { text: text.to_owned(), entities: entities.map(str::to_owned) }]
            });
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        let count = units.len();
        let parsed = Self::parse_entities(entities.unwrap_or(""));
        let mut order: Vec<usize> = (0..parsed.len()).collect();
        order.sort_by_key(|&index| parsed[index].end);
        let spans: Vec<&ChatComposerTextSplitterSpan> = order.iter().map(|&index| &parsed[index]).collect();
        let blocked = Self::blocked_cuts(&spans, count, budget);
        let composed_starts = Self::composed_character_starts(text);

        let mut parts: Vec<ChatComposerTextSplitterPart> = Vec::new();
        let mut start = Self::skip_whitespace(&units, 0);
        let mut alive = 0;
        while start < count {
            if parts.len() == budget.parts {
                return None;
            }
            while alive < spans.len() && spans[alive].end <= start {
                alive += 1;
            }

            let mut characters = 0;
            let mut entity_count = 0;
            let mut entity_bytes = 2;
            let mut pointer = alive;
            let mut hard = start;
            let mut position = start;
            let mut overflow = false;
            while position < count {
                if budget.character_unit != Unit::Chars || !Self::is_trail_surrogate(units[position]) {
                    if characters == budget.characters {
                        overflow = true;
                        break;
                    }
                    characters += 1;
                }
                position += 1;
                while pointer < spans.len() && spans[pointer].end <= position {
                    entity_count += 1;
                    entity_bytes += spans[pointer].bytes + if entity_count > 1 { 1 } else { 0 };
                    pointer += 1;
                }
                if entity_count > budget.entity_count || entity_bytes > budget.entity_bytes {
                    overflow = true;
                    break;
                }
                hard = position;
            }

            let cut = if !overflow {
                count
            } else {
                Self::cut_position(&units, &composed_starts, start, hard.max(start + 1), &blocked)
            };
            let mut end = cut;
            while end > start && Self::is_whitespace(units[end - 1]) {
                end -= 1;
            }
            if end > start {
                parts.push(ChatComposerTextSplitterPart {
                    text: String::from_utf16_lossy(&units[start..end]),
                    entities: Self::encode_entities(&parsed, start, end),
                });
            }
            start = Self::skip_whitespace(&units, cut);
        }
        if parts.len() > budget.parts { None } else { Some(parts) }
    }

    fn parse_entities(json: &str) -> Vec<ChatComposerTextSplitterSpan> {
        if json.is_empty() {
            return Vec::new();
        }
        let Some(Value::Array(array)) = JSONSerialization::json_object(json.as_bytes()) else {
            return Vec::new();
        };
        if !array.iter().all(Value::is_object) {
            return Vec::new();
        }
        array
            .into_iter()
            .filter_map(|element| {
                let Value::Object(object) = element else {
                    return None;
                };
                let offset = Self::int_value(object.get(ChatMessageEntityField::Offset.raw_value()))?;
                let length = Self::int_value(object.get(ChatMessageEntityField::Length.raw_value()))?;
                if offset < 0 || length <= 0 {
                    return None;
                }
                let bytes = JSONSerialization::data_with_json_object(
                    &Value::Object(object.clone()),
                    JSONWritingOptions::default(),
                )
                .map(|data| data.len())
                .unwrap_or(0);
                let start = usize::try_from(offset).ok()?;
                let end = usize::try_from(offset + length).ok()?;
                Some(ChatComposerTextSplitterSpan { start, end, bytes, object })
            })
            .collect()
    }

    /// reads an int from json, a bool counts too and a fraction truncates
    fn int_value(value: Option<&Value>) -> Option<i64> {
        match value? {
            Value::Number(number) => number.as_i64().or_else(|| number.as_f64().map(|value| value as i64)),
            Value::Bool(flag) => Some(i64::from(*flag)),
            _ => None,
        }
    }

    fn encode_entities(spans: &[ChatComposerTextSplitterSpan], start: usize, end: usize) -> Option<String> {
        let mut out: Vec<Value> = Vec::new();
        for span in spans.iter().filter(|span| span.end > start && span.start < end) {
            let clipped_start = span.start.max(start);
            let clipped_end = span.end.min(end);
            if clipped_end <= clipped_start {
                continue;
            }
            let mut object = span.object.clone();
            object.insert(
                ChatMessageEntityField::Offset.raw_value().to_owned(),
                Value::Number(Number::from(clipped_start - start)),
            );
            object.insert(
                ChatMessageEntityField::Length.raw_value().to_owned(),
                Value::Number(Number::from(clipped_end - clipped_start)),
            );
            out.push(Value::Object(object));
        }
        if out.is_empty() {
            return None;
        }
        let data = JSONSerialization::data_with_json_object(&Value::Array(out), JSONWritingOptions::default())?;
        String::from_utf8(data).ok()
    }

    fn blocked_cuts(spans: &[&ChatComposerTextSplitterSpan], count: usize, budget: &ChatComposerTextSplitterBudget) -> Vec<bool> {
        let mut blocked = vec![false; count + 1];
        for span in spans.iter().filter(|span| span.end - span.start < budget.characters) {
            let lower = (span.start + 1).min(count);
            let upper = span.end.min(count);
            if lower >= upper {
                continue;
            }
            for position in lower..upper {
                blocked[position] = true;
            }
        }
        blocked
    }

    fn composed_character_starts(text: &str) -> Vec<usize> {
        let mut starts = Vec::new();
        let mut offset = 0;
        for character in characters(text) {
            starts.push(offset);
            offset += character.encode_utf16().count();
        }
        starts
    }

    fn cut_position(units: &[u16], composed_starts: &[usize], start: usize, hard: usize, blocked: &[bool]) -> usize {
        let floor = start + (hard - start) / 2;
        if let Some(line) = Self::last_cut(units, hard, floor, blocked, |unit| unit == 0x0A) {
            return line;
        }
        if let Some(space) = Self::last_cut(units, hard, floor, blocked, Self::is_whitespace) {
            return space;
        }
        let mut cut = hard;
        while cut > start + 1 && blocked[cut] {
            cut -= 1;
        }
        if cut < units.len() {
            let composed = match composed_starts.binary_search(&cut) {
                Ok(index) => composed_starts[index],
                Err(index) => composed_starts[index.saturating_sub(1)],
            };
            if composed > start {
                cut = composed;
            }
        }
        cut
    }

    /// cuts right after the last matching unit so the separator ends the earlier part and gets trimmed
    fn last_cut(units: &[u16], hard: usize, floor: usize, blocked: &[bool], matches: impl Fn(u16) -> bool) -> Option<usize> {
        let mut cut = hard;
        while cut > floor {
            if matches(units[cut - 1]) && !blocked[cut] {
                return Some(cut);
            }
            cut -= 1;
        }
        None
    }

    fn skip_whitespace(units: &[u16], from: usize) -> usize {
        let mut position = from;
        while position < units.len() && Self::is_whitespace(units[position]) {
            position += 1;
        }
        position
    }

    /// same whitespace set the send path trims with so a part never shifts entity offsets on send
    fn is_whitespace(unit: u16) -> bool {
        if Self::is_lead_surrogate(unit) || Self::is_trail_surrogate(unit) {
            return false;
        }
        char::from_u32(u32::from(unit)).is_some_and(is_whitespace_or_newline)
    }

    fn is_lead_surrogate(unit: u16) -> bool {
        (0xD800..=0xDBFF).contains(&unit)
    }

    fn is_trail_surrogate(unit: u16) -> bool {
        (0xDC00..=0xDFFF).contains(&unit)
    }
}
