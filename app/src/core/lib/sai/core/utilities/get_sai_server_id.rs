use rise_canon::RiseIdentityCanon;
use rise_canon::rise_key::{bot, message};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Number, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;

const PATH_SEPARATOR: char = '.';

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GetSaiServerIdOptions {
    pub path_to_id: String,
    pub path_to_server_id: String,
}

impl Default for GetSaiServerIdOptions {
    fn default() -> Self {
        Self {
            path_to_id: message::ID.to_owned(),
            path_to_server_id: Self::server_id_key().to_owned(),
        }
    }
}

impl GetSaiServerIdOptions {
    fn server_id_key() -> &'static str {
        RiseIdentityCanon::OPTIMISTIC_SERVER_IDENTIFIER_KEYS[0]
    }
}

pub enum GetSaiServerId {}

impl GetSaiServerId {
    pub fn is_temp_id(id: Option<&str>) -> bool {
        id.is_some_and(|id| id.starts_with(RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX))
    }

    pub fn from(obj: Option<&Value>, options: &GetSaiServerIdOptions) -> String {
        match AnyDictionaryBridge::dictionary(obj) {
            Some(dict) => Self::from_dictionary(&dict, options),
            None => Self::string_value(obj).unwrap_or_default(),
        }
    }

    pub fn from_dictionary(dict: &Map<String, Value>, options: &GetSaiServerIdOptions) -> String {
        let id = Self::string_value(Self::value(&options.path_to_id, dict).as_ref()).unwrap_or_default();
        let server_id =
            Self::string_value(Self::value(&options.path_to_server_id, dict).as_ref()).unwrap_or_default();
        if Self::is_temp_id(Some(&id)) && !server_id.is_empty() {
            return server_id;
        }
        if !id.is_empty() {
            return id;
        }
        if !server_id.is_empty() {
            return server_id;
        }
        String::new()
    }

    pub fn apply_temp_id_preserving_server_id(row: &Map<String, Value>, temp_id: Option<&str>) -> Map<String, Value> {
        let Some(temp_id) = temp_id.filter(|temp_id| Self::is_temp_id(Some(temp_id))) else {
            return row.clone();
        };
        let mut out = row.clone();
        let wire_id = Self::string_value(out.get(message::ID)).or_else(|| Self::string_value(out.get(bot::MESSAGE_ID)));
        if let Some(wire_id) = wire_id
            && !wire_id.is_empty()
            && !Self::is_temp_id(Some(&wire_id))
        {
            out.insert(GetSaiServerIdOptions::server_id_key().to_owned(), Value::String(wire_id));
        }
        out.insert(message::ID.to_owned(), Value::String(temp_id.to_owned()));
        out
    }

    fn value(path: &str, dict: &Map<String, Value>) -> Option<Value> {
        let mut current = Some(Value::Object(dict.clone()));
        for key in path.split(PATH_SEPARATOR).filter(|key| !key.is_empty()) {
            let d = AnyDictionaryBridge::dictionary(current.as_ref())?;
            current = d.get(key).cloned();
        }
        current
    }

    fn string_value(any: Option<&Value>) -> Option<String> {
        match any? {
            Value::String(s) => {
                let t = trimming_whitespaces_and_newlines(s);
                if t.is_empty() { None } else { Some(t.to_owned()) }
            }
            Value::Number(n) => Some(Self::ns_number_string_value(n)),
            Value::Bool(b) => Some(String::from(if *b { "1" } else { "0" })),
            Value::Null | Value::Array(_) | Value::Object(_) => None,
        }
    }

    fn ns_number_string_value(n: &Number) -> String {
        if n.is_f64()
            && let Some(f) = n.as_f64()
            && f.is_finite()
            && f.fract() == 0.0
            && f.abs() < 1e15
        {
            return format!("{f:.0}");
        }
        n.to_string()
    }
}
