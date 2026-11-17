use rise_canon::rise_key::{bot, chat, events};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;

pub enum SaiWsDiskPayloadSanitizer {}

const DATE_KEYS: [&str; 4] = [
    events::UPDATED_AT,
    bot::EDIT_DATE,
    events::EDITED_AT,
    events::LAST_ACTIVITY,
];

const DATE_STRIP_PREFIXES: [&str; 2] = ["getChats-", "getMessages-"];

const LIVE_STATE_KEYS: [&str; 2] = [chat::VOICE_STATE, chat::HAS_LIVE_VOICE];

const LIVE_STRIP_PREFIXES: [&str; 3] = ["get_forum_structure-", "getTopics-", "getChats-"];

impl SaiWsDiskPayloadSanitizer {
    pub fn prepare_for_disk(cache_key: &str, value: Value) -> Value {
        let mut keys = Self::live_state_keys(cache_key);
        if Self::matches(cache_key, &DATE_STRIP_PREFIXES) {
            for key in DATE_KEYS {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        Self::stripping(&keys, value)
    }

    /// same rule on the way back in
    /// old payloads or ones that skipped prepare_for_disk may still carry occupancy
    /// dates are not stripped here
    pub fn prepare_from_disk(cache_key: &str, value: Value) -> Value {
        Self::stripping(&Self::live_state_keys(cache_key), value)
    }

    fn live_state_keys(cache_key: &str) -> Vec<&'static str> {
        if Self::matches(cache_key, &LIVE_STRIP_PREFIXES) {
            LIVE_STATE_KEYS.to_vec()
        } else {
            Vec::new()
        }
    }

    fn matches(cache_key: &str, prefixes: &[&str]) -> bool {
        let logical = SaiAccountCacheScope::logical_key(trimming_whitespaces_and_newlines(cache_key));
        if logical.is_empty() {
            return false;
        }
        prefixes.iter().any(|prefix| logical.starts_with(prefix))
    }

    fn stripping(keys: &[&str], value: Value) -> Value {
        if keys.is_empty() {
            return value;
        }
        Self::stripped(keys, &value).unwrap_or(value)
    }

    /// none means nothing was removed below this node so the caller keeps its value
    /// runs on cold launch read so untouched subtrees are not rebuilt
    fn stripped(keys: &[&str], value: &Value) -> Option<Value> {
        match value {
            Value::Object(dict) => Self::stripped_dictionary(keys, dict).map(Value::Object),
            Value::Array(array) => Self::stripped_array(keys, array).map(Value::Array),
            _ => None,
        }
    }

    fn stripped_dictionary(keys: &[&str], dict: &Map<String, Value>) -> Option<Map<String, Value>> {
        let mut out: Option<Map<String, Value>> = None;
        for (key, value) in dict {
            if keys.contains(&key.as_str()) {
                out.get_or_insert_with(|| dict.clone()).remove(key);
                continue;
            }
            let Some(replacement) = Self::stripped(keys, value) else {
                continue;
            };
            out.get_or_insert_with(|| dict.clone()).insert(key.clone(), replacement);
        }
        out
    }

    fn stripped_array(keys: &[&str], array: &[Value]) -> Option<Vec<Value>> {
        let mut out: Option<Vec<Value>> = None;
        for (index, value) in array.iter().enumerate() {
            let Some(replacement) = Self::stripped(keys, value) else {
                continue;
            };
            out.get_or_insert_with(|| array.to_vec())[index] = replacement;
        }
        out
    }
}
