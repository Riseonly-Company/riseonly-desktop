use std::sync::LazyLock;

use rise_canon::RiseValidation;
use rise_canon::rise_blob::message_poll::{Answer, Poll};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Value};

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_rules::RiseValidationCanonRules;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessagePollInfo, MessagePollOption,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessagePollDraft {
    pub question: String,
    pub options: Vec<String>,
    pub is_anonymous: bool,
    pub allows_multiple: bool,
    pub quiz_correct_index: Option<usize>,
}

impl Default for MessagePollDraft {
    fn default() -> Self {
        Self {
            question: String::new(),
            options: Vec::new(),
            is_anonymous: true,
            allows_multiple: false,
            quiz_correct_index: None,
        }
    }
}

static MESSAGE_POLL_MAX_OPTIONS: LazyLock<usize> =
    LazyLock::new(|| RiseValidation::collection_limit("poll_options").maximum);

pub enum MessagePollCodec {}

impl MessagePollCodec {
    pub const MIN_OPTIONS: usize = 2;

    pub fn question_rule() -> RiseFieldRule {
        RiseFieldRule::client_rule("message_poll_question")
    }

    pub fn option_rule() -> RiseFieldRule {
        RiseFieldRule::client_rule("message_poll_option")
    }

    pub fn max_options() -> usize {
        *MESSAGE_POLL_MAX_OPTIONS
    }

    pub fn option_id(index: usize) -> String {
        index.to_string()
    }

    pub fn encode(draft: &MessagePollDraft) -> Option<String> {
        let question = trimming_whitespaces_and_newlines(&draft.question);
        let options: Vec<&str> = draft
            .options
            .iter()
            .map(|option| trimming_whitespaces_and_newlines(option))
            .filter(|option| !option.is_empty())
            .collect();
        if question.is_empty() || options.len() < Self::MIN_OPTIONS || options.len() > Self::max_options() {
            return None;
        }
        let quiz = draft
            .quiz_correct_index
            .and_then(|index| (index < options.len()).then(|| Self::option_id(index)));
        let poll = Poll {
            question: Some(question.to_owned()),
            options: Some(
                options
                    .iter()
                    .enumerate()
                    .map(|(offset, text)| Answer {
                        id: Some(Self::option_id(offset)),
                        text: Some((*text).to_owned()),
                        voter_count: None,
                    })
                    .collect(),
            ),
            is_anonymous: Some(draft.is_anonymous),
            allows_multiple_answers: Some(if quiz.is_none() { draft.allows_multiple } else { false }),
            correct_option_id: quiz,
            ..Poll::default()
        };
        let value = serde_json::to_value(&poll).ok()?;
        serde_json::to_string(&Self::sorted_keys(value)).ok()
    }

    pub fn decode(content: &str) -> Option<MessagePollInfo> {
        let poll = serde_json::from_str::<Poll>(content).ok()?;
        let question = trimming_whitespaces_and_newlines(poll.question.as_deref()?).to_owned();
        if question.is_empty() {
            return None;
        }
        let options: Vec<MessagePollOption> = poll
            .options
            .unwrap_or_default()
            .into_iter()
            .filter_map(|option| {
                let id = option.id?;
                let text = option.text.filter(|text| !text.is_empty())?;
                Some(MessagePollOption { id, text, voter_count: option.voter_count.unwrap_or(0) })
            })
            .collect();
        if options.len() < Self::MIN_OPTIONS {
            return None;
        }
        Some(MessagePollInfo {
            question,
            options,
            total_voters: poll.total_voter_count.unwrap_or(0),
            is_anonymous: poll.is_anonymous.unwrap_or(true),
            allows_multiple: poll.allows_multiple_answers.unwrap_or(false),
            quiz_correct_id: poll.correct_option_id,
            closed: poll.is_closed.unwrap_or(false),
            chosen_option_ids: poll.chosen_option_ids.unwrap_or_default(),
        })
    }

    fn sorted_keys(value: Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut entries: Vec<(String, Value)> =
                    object.into_iter().map(|(key, value)| (key, Self::sorted_keys(value))).collect();
                entries.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));
                Value::Object(entries.into_iter().collect::<Map<String, Value>>())
            }
            Value::Array(values) => Value::Array(values.into_iter().map(Self::sorted_keys).collect()),
            other => other,
        }
    }
}

impl GetMessageMessage {
    pub fn adopt_poll_payload(&mut self) {
        if self.content_type != MessageContentType::Poll || self.poll.is_some() {
            return;
        }
        let Some(decoded) = MessagePollCodec::decode(&self.text) else {
            return;
        };
        self.poll = Some(decoded);
        self.text = String::new();
    }
}
