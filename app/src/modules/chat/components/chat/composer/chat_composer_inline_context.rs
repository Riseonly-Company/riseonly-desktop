use std::cmp::Ordering;
use std::collections::HashSet;

use rise_chat::{MessageEntityField, MessageEntityType};
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::foundation_text::{drop_first_characters, is_whitespace_or_newline, trimming_whitespaces_and_newlines};
use rise_core::foundation::ns_range::NSRange;
use rise_validate::{BOT_COMMAND, TAG, Unit};
use serde_json::{Map, Value};

use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotTagRules;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatComposerUserMention;

const AT_SIGN: u16 = 64;
const HASH_SIGN: u16 = 35;
const SLASH: u16 = 47;

/// what the composer line is typing at the caret
/// a user mention token, a leading inline bot query or nothing
/// ranges are utf16 like the field selection
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ChatComposerInlineContext {
    #[default]
    Inactive,
    Candidates { query: String, token_range: NSRange },
    Bot { bot_tag: String, query: String, query_range: NSRange },
    GroupCommands { query: String, token_range: NSRange },
    Topics { query: String, token_range: NSRange },
}

impl ChatComposerInlineContext {
    pub fn is_presented(&self) -> bool {
        *self != Self::Inactive
    }

    pub fn bot_tag(&self) -> Option<&str> {
        match self {
            Self::Bot { bot_tag, .. } => Some(bot_tag),
            _ => None,
        }
    }

    pub fn query(&self) -> &str {
        match self {
            Self::Inactive => "",
            Self::Candidates { query, .. }
            | Self::Bot { query, .. }
            | Self::GroupCommands { query, .. }
            | Self::Topics { query, .. } => query,
        }
    }

    pub fn command_awaiting_argument(query: &str) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(query);
        if !trimmed.starts_with('/') || !BOT_COMMAND.is_valid(drop_first_characters(trimmed, 1)) {
            return None;
        }
        Some(trimmed.to_owned())
    }

    /// selection is none when nothing is selected
    /// allows group commands and allows topic mentions default to false
    pub fn resolve(
        text: &str,
        selection: Option<NSRange>,
        selected_mentions: &[ChatComposerUserMention],
        allows_group_commands: bool,
        allows_topic_mentions: bool,
    ) -> ChatComposerInlineContext {
        let source: Vec<u16> = text.encode_utf16().collect();
        let Some(selection) = selection else {
            return Self::Inactive;
        };
        if selection.length != 0 || selection.location > source.len() {
            return Self::Inactive;
        }

        let caret = selection.location;
        if allows_group_commands && let Some(context) = Self::group_commands_context(&source, caret) {
            return context;
        }
        if !source.is_empty()
            && source[0] == AT_SIGN
            && let Some(whitespace) = Self::first_whitespace(&source, caret)
        {
            let tag_range = NSRange::new(1, whitespace.saturating_sub(1));
            let tag = substring(&source, tag_range);
            let leading_mention_range = NSRange::new(0, whitespace);
            let is_selected_user_mention = selected_mentions.iter().any(|mention| {
                mention.range == leading_mention_range && compare_case_insensitive(&mention.tag, &tag) == Ordering::Equal
            });
            if !is_selected_user_mention && Self::valid_bot_tag(&tag) {
                let mut query_start = whitespace;
                while query_start < caret && is_whitespace_unit(source[query_start]) {
                    query_start += 1;
                }
                let query_range = NSRange::new(query_start, caret - query_start);
                return Self::Bot {
                    bot_tag: tag.to_lowercase(),
                    query: substring(&source, query_range),
                    query_range,
                };
            }
        }

        let mut token_start = caret;
        while token_start > 0 && !is_whitespace_unit(source[token_start - 1]) {
            token_start -= 1;
        }
        let mut token_end = caret;
        while token_end < source.len() && !is_whitespace_unit(source[token_end]) {
            token_end += 1;
        }
        let token_range = NSRange::new(token_start, token_end - token_start);
        if allows_topic_mentions
            && token_range.length > 0
            && caret > token_range.location
            && source[token_range.location] == HASH_SIGN
        {
            let query_range = NSRange::new(token_range.location + 1, caret - token_range.location - 1);
            let query = substring(&source, query_range);
            if query.contains('#') || query.contains('@') {
                return Self::Inactive;
            }
            return Self::Topics { query, token_range };
        }
        if token_range.length == 0 || caret <= token_range.location || source[token_range.location] != AT_SIGN {
            return Self::Inactive;
        }

        let query_range = NSRange::new(token_range.location + 1, caret.saturating_sub(token_range.location + 1));
        let query = substring(&source, query_range);
        let token = substring(&source, token_range);
        let tag_body = drop_first_characters(&token, 1);
        if tag_body.contains('@') || !Self::fits_tag_rule(tag_body) || !Self::fits_tag_rule(&query) {
            return Self::Inactive;
        }
        Self::Candidates { query, token_range }
    }

    /// returns the edited text and the caret right after the replacement
    pub fn replacing(range: NSRange, text: &str, replacement: &str) -> (String, NSRange) {
        let source: Vec<u16> = text.encode_utf16().collect();
        let Some(end) = range.location.checked_add(range.length).filter(|end| *end <= source.len()) else {
            return (text.to_owned(), NSRange::new(source.len(), 0));
        };
        let inserted: Vec<u16> = replacement.encode_utf16().collect();
        let mut edited = Vec::with_capacity(source.len() - range.length + inserted.len());
        edited.extend_from_slice(&source[..range.location]);
        edited.extend_from_slice(&inserted);
        edited.extend_from_slice(&source[end..]);
        let location = range.location + inserted.len();
        (String::from_utf16_lossy(&edited), NSRange::new(location, 0))
    }

    fn group_commands_context(source: &[u16], caret: usize) -> Option<ChatComposerInlineContext> {
        if source.is_empty() || source[0] != SLASH || caret < 1 {
            return None;
        }
        let mut token_end = 0;
        while token_end < source.len() && !is_whitespace_unit(source[token_end]) {
            token_end += 1;
        }
        if caret > token_end {
            return None;
        }
        let query = substring(source, NSRange::new(1, caret - 1));
        let command_part = query.split('@').next().unwrap_or_default();
        if !(command_part.is_empty()
            || BOT_COMMAND.is_valid(command_part)
            || command_part.chars().all(|character| character.is_alphabetic() || character.is_numeric() || character == '_'))
        {
            return None;
        }
        Some(Self::GroupCommands { query, token_range: NSRange::new(0, token_end) })
    }

    fn first_whitespace(source: &[u16], caret: usize) -> Option<usize> {
        if caret <= 1 {
            return None;
        }
        (1..caret).find(|index| is_whitespace_unit(source[*index]))
    }

    fn valid_bot_tag(value: &str) -> bool {
        BotTagRules::is_valid(&value.to_lowercase())
    }

    fn tag_maximum() -> Option<(usize, Unit)> {
        TAG.maximum()
    }

    fn fits_tag_rule(value: &str) -> bool {
        let Some((maximum, unit)) = Self::tag_maximum() else {
            return true;
        };
        unit.count(value) <= maximum
    }
}

/// keeps user mention ranges and the wire mention entities in step with the composer text
pub enum ChatComposerMentionEntityCodec {}

impl ChatComposerMentionEntityCodec {
    pub fn rebased(mentions: &[ChatComposerUserMention], old_text: &str, new_text: &str) -> Vec<ChatComposerUserMention> {
        if mentions.is_empty() {
            return Vec::new();
        }
        let old_source: Vec<u16> = old_text.encode_utf16().collect();
        let new_source: Vec<u16> = new_text.encode_utf16().collect();
        if old_text == new_text {
            return mentions.iter().filter(|mention| Self::mention_is_valid(mention, &new_source)).cloned().collect();
        }

        let shared_prefix = Self::common_prefix_length(&old_source, &new_source);
        let shared_suffix = Self::common_suffix_length(&old_source, &new_source, shared_prefix);
        let old_changed_end = old_source.len() - shared_suffix;
        let new_changed_end = new_source.len() - shared_suffix;

        mentions
            .iter()
            .filter_map(|mention| {
                let old_range = mention.range;
                if old_range.upper_bound() > old_source.len() || !Self::mention_is_valid(mention, &old_source) {
                    return None;
                }

                let new_range = if old_range.upper_bound() <= shared_prefix {
                    old_range
                } else if old_range.location >= old_changed_end {
                    NSRange::new(old_range.location - old_changed_end + new_changed_end, old_range.length)
                } else {
                    return None;
                };

                let updated = ChatComposerUserMention {
                    user_id: mention.user_id.clone(),
                    tag: mention.tag.clone(),
                    range: new_range,
                };
                Self::mention_is_valid(&updated, &new_source).then_some(updated)
            })
            .collect()
    }

    pub fn rebased_for_trimming(mentions: &[ChatComposerUserMention], text: &str) -> Vec<ChatComposerUserMention> {
        if mentions.is_empty() {
            return Vec::new();
        }
        let source: Vec<u16> = text.encode_utf16().collect();
        let mut lower_bound = 0;
        let mut upper_bound = source.len();
        while lower_bound < upper_bound && is_whitespace_unit(source[lower_bound]) {
            lower_bound += 1;
        }
        while upper_bound > lower_bound && is_whitespace_unit(source[upper_bound - 1]) {
            upper_bound -= 1;
        }
        let trimmed_source = &source[lower_bound..upper_bound];

        mentions
            .iter()
            .filter_map(|mention| {
                if mention.range.location < lower_bound || mention.range.upper_bound() > upper_bound {
                    return None;
                }
                let updated = ChatComposerUserMention {
                    user_id: mention.user_id.clone(),
                    tag: mention.tag.clone(),
                    range: NSRange::new(mention.range.location - lower_bound, mention.range.length),
                };
                Self::mention_is_valid(&updated, trimmed_source).then_some(updated)
            })
            .collect()
    }

    /// replaces every mention entity in the json with the given mentions
    /// a mention drops the role mention entities it overlaps. none when no entity is left
    pub fn encode(text: &str, mentions: &[ChatComposerUserMention], merging_into: Option<&str>) -> Option<String> {
        let type_key = MessageEntityField::EntityType.wire();
        let offset_key = MessageEntityField::Offset.wire();
        let length_key = MessageEntityField::Length.wire();
        let mention_type = MessageEntityType::Mention.wire();
        let role_mention_type = MessageEntityType::RoleMention.wire();

        let mut entities = Self::existing_entities(merging_into);
        entities.retain(|entity| entity.get(type_key).and_then(Value::as_str) != Some(mention_type));

        let source: Vec<u16> = text.encode_utf16().collect();
        let mut inserted: HashSet<String> = HashSet::new();
        for mention in mentions.iter().filter(|mention| !mention.user_id.is_empty() && !mention.tag.is_empty()) {
            let range = mention.range;
            if !Self::mention_is_valid(mention, &source) {
                continue;
            }
            let key = format!("{}::{}::{}", mention.user_id, range.location, range.length);
            if !inserted.insert(key) {
                continue;
            }
            entities.retain(|entity| {
                if entity.get(type_key).and_then(Value::as_str) != Some(role_mention_type) {
                    return true;
                }
                let Some(other) = Self::entity_range(entity) else {
                    return true;
                };
                other.intersection(range).is_none()
            });
            let mut entity = Map::new();
            entity.insert(type_key.to_owned(), Value::from(mention_type));
            entity.insert(MessageEntityField::UserId.wire().to_owned(), Value::from(mention.user_id.as_str()));
            entity.insert(offset_key.to_owned(), Value::from(range.location));
            entity.insert(length_key.to_owned(), Value::from(range.length));
            entities.push(entity);
        }

        if entities.is_empty() {
            return None;
        }
        entities.sort_by_key(|entity| Self::integer(entity.get(offset_key)).unwrap_or(i64::MAX));
        let array = Value::Array(entities.into_iter().map(Value::Object).collect());
        let Some(data) = JSONSerialization::data_with_json_object(&array, JSONWritingOptions::default()) else {
            return merging_into.map(str::to_owned);
        };
        String::from_utf8(data).ok()
    }

    fn existing_entities(value: Option<&str>) -> Vec<Map<String, Value>> {
        let Some(value) = value else {
            return Vec::new();
        };
        let Some(Value::Array(items)) = JSONSerialization::json_object(value.as_bytes()) else {
            return Vec::new();
        };
        items
            .into_iter()
            .map(|item| match item {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default()
    }

    fn boundary_is_valid(source: &[u16], range: NSRange) -> bool {
        let before_is_valid = range.location == 0 || is_boundary(source[range.location - 1]);
        let end = range.location + range.length;
        let after_is_valid = end >= source.len() || is_boundary(source[end]);
        before_is_valid && after_is_valid
    }

    fn mention_is_valid(mention: &ChatComposerUserMention, source: &[u16]) -> bool {
        let range = mention.range;
        if mention.user_id.is_empty()
            || mention.tag.is_empty()
            || range.length == 0
            || range.location.checked_add(range.length).is_none_or(|end| end > source.len())
            || compare_case_insensitive(&substring(source, range), &format!("@{}", mention.tag)) != Ordering::Equal
        {
            return false;
        }
        Self::boundary_is_valid(source, range)
    }

    fn common_prefix_length(lhs: &[u16], rhs: &[u16]) -> usize {
        lhs.iter().zip(rhs).take_while(|(left, right)| left == right).count()
    }

    fn common_suffix_length(lhs: &[u16], rhs: &[u16], prefix: usize) -> usize {
        let upper_bound = lhs.len().min(rhs.len()) - prefix;
        lhs.iter().rev().zip(rhs.iter().rev()).take(upper_bound).take_while(|(left, right)| left == right).count()
    }

    fn entity_range(entity: &Map<String, Value>) -> Option<NSRange> {
        let offset = Self::integer(entity.get(MessageEntityField::Offset.wire()))?;
        let length = Self::integer(entity.get(MessageEntityField::Length.wire()))?;
        if offset < 0 || length <= 0 {
            return None;
        }
        Some(NSRange::new(usize::try_from(offset).ok()?, usize::try_from(length).ok()?))
    }

    /// reads an int from a json value, else from a number or a string
    fn integer(value: Option<&Value>) -> Option<i64> {
        match value? {
            Value::Number(number) => number.as_i64().or_else(|| number.as_f64().map(|value| value as i64)),
            Value::Bool(flag) => Some(i64::from(*flag)),
            Value::String(text) => text.parse::<i64>().ok(),
            _ => None,
        }
    }
}

/// substring over utf16 units, a range cutting a surrogate pair keeps the lone half as U+FFFD
fn substring(source: &[u16], range: NSRange) -> String {
    let end = range.upper_bound().min(source.len());
    let start = range.location.min(end);
    String::from_utf16_lossy(&source[start..end])
}

/// whitespace check for one utf16 unit, a surrogate half is no scalar
fn is_whitespace_unit(value: u16) -> bool {
    char::from_u32(u32::from(value)).is_some_and(is_whitespace_or_newline)
}

fn is_boundary(value: u16) -> bool {
    let Some(scalar) = char::from_u32(u32::from(value)) else {
        return true;
    };
    !(scalar.is_alphanumeric() || scalar == '_')
}
