use std::collections::{HashMap, HashSet};

use gpui::{App, Global};
use rise_canon::rise_key::{chat, events};
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;

const PREFIXES: [&str; 2] = ["get_forum_structure-", "getTopics-"];
const SUMMARY_KEYS: [&str; 5] = [
    events::LAST_MESSAGE,
    events::UNREAD_COUNT,
    chat::UNREAD_MENTIONS_COUNT,
    chat::UNREAD_REPLIES_COUNT,
    chat::UNREAD_REACTIONS_COUNT,
];

/// listings whose latest answer was a dropped one, so owners keep asking again
#[derive(Default)]
struct SaiWsDroppedTopicSummaryCarriedOver {
    cache_keys: HashSet<String>,
}

impl Global for SaiWsDroppedTopicSummaryCarriedOver {}

/// chat service answers forum listings while message service is down but drops last messages and badges
/// such a listing keeps the summaries of the in memory copy it replaces
/// titles and order still come from the new answer
pub enum SaiWsDroppedTopicSummaryCarryOver {}

impl SaiWsDroppedTopicSummaryCarryOver {
    pub fn is_carrying_over(cache_key: &str, cx: &App) -> bool {
        cx.try_global::<SaiWsDroppedTopicSummaryCarriedOver>()
            .is_some_and(|carried| carried.cache_keys.contains(cache_key))
    }

    pub fn payload(incoming: Option<Value>, cache_key: Option<&str>, cx: &mut App) -> Option<Value> {
        let Some(cache_key) = cache_key.filter(|cache_key| Self::matches(cache_key)) else {
            return incoming;
        };
        let dropped_root = match &incoming {
            Some(Value::Object(root)) if Self::looks_dropped(root) => Some(root.clone()),
            _ => None,
        };
        let Some(root) = dropped_root else {
            cx.default_global::<SaiWsDroppedTopicSummaryCarriedOver>()
                .cache_keys
                .remove(cache_key);
            return incoming;
        };
        let Some(previous) = SaiWsResponseCache::memory_only(cache_key, cx)
            .and_then(|value| value.as_object().cloned())
            .filter(|previous| !Self::looks_dropped(previous))
        else {
            return incoming;
        };

        let mut previous_topics: HashMap<String, Map<String, Value>> = HashMap::new();
        for topic in Self::topics(&previous) {
            if let Some(Value::String(id)) = topic.get(chat::TOPIC_ID)
                && !id.is_empty()
            {
                previous_topics.insert(id.clone(), topic.clone());
            }
        }

        let mut merged = root.clone();
        if let Some(Value::Array(list)) = root.get(chat::TOPICS) {
            let patched: Vec<Value> = list
                .iter()
                .map(|item| {
                    let Value::Object(topic) = item else {
                        return item.clone();
                    };
                    let Some(Value::String(id)) = topic.get(chat::TOPIC_ID) else {
                        return item.clone();
                    };
                    let Some(old) = previous_topics.get(id) else {
                        return item.clone();
                    };
                    Value::Object(Self::carrying_summary(old, topic))
                })
                .collect();
            merged.insert(chat::TOPICS.to_owned(), Value::Array(patched));
        }
        if let (Some(Value::Object(general)), Some(Value::Object(old))) =
            (root.get(chat::GENERAL_TOPIC), previous.get(chat::GENERAL_TOPIC))
        {
            merged.insert(
                chat::GENERAL_TOPIC.to_owned(),
                Value::Object(Self::carrying_summary(old, general)),
            );
        }
        cx.default_global::<SaiWsDroppedTopicSummaryCarriedOver>()
            .cache_keys
            .insert(cache_key.to_owned());
        Some(Value::Object(merged))
    }

    fn matches(cache_key: &str) -> bool {
        let logical = SaiAccountCacheScope::logical_key(cache_key);
        PREFIXES.iter().any(|prefix| logical.starts_with(prefix))
    }

    fn topics(root: &Map<String, Value>) -> Vec<&Map<String, Value>> {
        let mut all: Vec<&Map<String, Value>> = match root.get(chat::TOPICS) {
            Some(Value::Array(list)) => list.iter().filter_map(Value::as_object).collect(),
            _ => Vec::new(),
        };
        if let Some(Value::Object(general)) = root.get(chat::GENERAL_TOPIC) {
            all.push(general);
        }
        all
    }

    /// outage answer has no preview and no badge anywhere, a real forum always has the creation notice
    fn looks_dropped(root: &Map<String, Value>) -> bool {
        let all = Self::topics(root);
        if all.is_empty() {
            return false;
        }
        !all.iter().any(|topic| {
            Self::has_value(topic.get(chat::LAST_MESSAGE))
                || SUMMARY_KEYS[1..].iter().any(|key| Self::count(topic.get(*key)) > 0)
        })
    }

    fn carrying_summary(old: &Map<String, Value>, topic: &Map<String, Value>) -> Map<String, Value> {
        let mut patched = topic.clone();
        for key in SUMMARY_KEYS {
            if let Some(value) = old.get(key) {
                patched.insert(key.to_owned(), value.clone());
            }
        }
        if Self::count(old.get(chat::LAST_ACTIVITY_AT)) > Self::count(topic.get(chat::LAST_ACTIVITY_AT))
            && let Some(value) = old.get(chat::LAST_ACTIVITY_AT)
        {
            patched.insert(chat::LAST_ACTIVITY_AT.to_owned(), value.clone());
        }
        patched
    }

    fn has_value(value: Option<&Value>) -> bool {
        value.is_some_and(|value| !value.is_null())
    }

    /// count from a json value, a bool is 0 or 1 and a double is cut toward zero
    fn count(value: Option<&Value>) -> i64 {
        match value {
            Some(Value::Number(number)) => number
                .as_i64()
                .or_else(|| number.as_u64().map(|n| i64::try_from(n).unwrap_or(i64::MAX)))
                .or_else(|| number.as_f64().map(|n| n as i64))
                .unwrap_or(0),
            Some(Value::Bool(flag)) => i64::from(*flag),
            _ => 0,
        }
    }
}
