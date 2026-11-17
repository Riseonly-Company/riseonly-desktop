use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_i18n::tr;

use crate::modules::chat::stores::message::message_actions::message_types::{MessageSystemInfo, MessageSystemKind};

const GROUP_CREATED_PHRASE: &str = "Group was created";
const CHANNEL_CREATED_PHRASE: &str = "Channel was created";
const GROUP_NAMED_PREFIX: &str = "Group '";
const CHANNEL_NAMED_PREFIX: &str = "Channel '";
const NAME_QUOTE: char = '\'';
const MENTION_PREFIX: &str = "@";
const MODERATION_REMOVED_VERB: &str = "removed";
const MODERATION_BANNED_VERB: &str = "banned";
const MODERATION_WORD_COUNT: usize = 3;

pub enum MessageSystemContentParser {}

impl MessageSystemContentParser {
    pub fn localize(content: &str) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(content);
        if trimmed.is_empty() {
            return None;
        }

        if let Some(key) = Self::quoted_name_frame(trimmed) {
            if key == GROUP_CREATED_PHRASE {
                return Some(tr("group_created_message"));
            }
            if key == CHANNEL_CREATED_PHRASE {
                return Some(tr("channel_created_message"));
            }
        }

        if trimmed.contains(GROUP_CREATED_PHRASE) || trimmed.starts_with(GROUP_NAMED_PREFIX) {
            return Some(tr("group_created_message"));
        }
        if trimmed.contains(CHANNEL_CREATED_PHRASE) || trimmed.starts_with(CHANNEL_NAMED_PREFIX) {
            return Some(tr("channel_created_message"));
        }
        None
    }

    /// chat service writes a kick or ban as admin removed member or banned member
    /// and sends no target of its own so the member tag is read from the text
    pub fn moderation_target(content: &str) -> Option<String> {
        let words: Vec<&str> = trimming_whitespaces_and_newlines(content)
            .split(' ')
            .filter(|word| !word.is_empty())
            .collect();
        if words.len() != MODERATION_WORD_COUNT {
            return None;
        }
        let actor = words[0];
        let verb = words[1];
        let target = words[2];
        if !actor.starts_with(MENTION_PREFIX)
            || !(verb == MODERATION_REMOVED_VERB || verb == MODERATION_BANNED_VERB)
            || !target.starts_with(MENTION_PREFIX)
            || character_count(target) <= 1
        {
            return None;
        }
        Some(target.to_owned())
    }

    pub fn infer(content: &str) -> Option<MessageSystemInfo> {
        let trimmed = trimming_whitespaces_and_newlines(content);
        if trimmed.is_empty() {
            return None;
        }

        if let Some(key) = Self::quoted_name_frame(trimmed) {
            if key == GROUP_CREATED_PHRASE {
                return Some(Self::info(MessageSystemKind::ChatCreated));
            }
            if key == CHANNEL_CREATED_PHRASE {
                return Some(Self::info(MessageSystemKind::ChannelCreated));
            }
        }

        if trimmed.contains(GROUP_CREATED_PHRASE) || trimmed.starts_with(GROUP_NAMED_PREFIX) {
            return Some(Self::info(MessageSystemKind::ChatCreated));
        }
        if trimmed.contains(CHANNEL_CREATED_PHRASE) || trimmed.starts_with(CHANNEL_NAMED_PREFIX) {
            return Some(Self::info(MessageSystemKind::ChannelCreated));
        }
        None
    }

    /// text around a quoted name, Group 'x' was created reads as Group was created
    fn quoted_name_frame(trimmed: &str) -> Option<String> {
        let parts: Vec<&str> = trimmed.split(NAME_QUOTE).collect();
        if parts.len() < 2 {
            return None;
        }
        let first = trimming_whitespaces(parts[0]);
        let last = trimming_whitespaces(parts[parts.len() - 1]);
        Some(format!("{first} {last}"))
    }

    fn info(kind: MessageSystemKind) -> MessageSystemInfo {
        MessageSystemInfo { kind, ..MessageSystemInfo::default() }
    }
}
