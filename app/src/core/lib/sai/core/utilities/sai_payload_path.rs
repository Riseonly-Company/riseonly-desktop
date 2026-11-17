use std::any::type_name;

use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::convert_keys_to_snake_case;

const PATH_SEPARATOR: char = '.';

pub enum SaiPayloadPath {}

impl SaiPayloadPath {
    /// a key the payload doesnt hold gives the payload itself and not nothing
    /// callers read envelopes and bare rows through the same path
    pub fn extract<'a>(value: Option<&'a Value>, path: Option<&str>) -> Option<&'a Value> {
        let value = value.filter(|value| !value.is_null())?;
        let Some(path) = path else {
            return Some(value);
        };
        let trimmed = trimming_whitespaces_and_newlines(path);
        if trimmed.is_empty() {
            return Some(value);
        }

        let keys: Vec<&str> = trimmed
            .split(PATH_SEPARATOR)
            .map(trimming_whitespaces)
            .filter(|key| !key.is_empty())
            .collect();
        if keys.is_empty() {
            return Some(value);
        }

        let mut current = value;
        for key in keys {
            let Some(dict) = current.as_object() else {
                return Some(value);
            };
            let Some(next) = dict.get(key) else {
                return Some(value);
            };
            current = next;
        }
        if current.is_null() { None } else { Some(current) }
    }
}

/// json codec, encode converts to snake case because hand written serialize impls emit camel keys
/// cached rows and lookups use snake, decode needs nothing as both spellings are read
pub enum SaiJSONCodec {}

impl SaiJSONCodec {
    pub fn encode<T: Serialize>(value: &T) -> Option<Map<String, Value>> {
        match convert_keys_to_snake_case(serde_json::to_value(value).ok()?) {
            Value::Object(dict) => Some(dict),
            _ => None,
        }
    }

    pub fn decode<T: DeserializeOwned>(any: Option<&Value>) -> Option<T> {
        let dict = Self::any_to_dict(any)?;
        match serde_json::from_value::<T>(Value::Object(dict.clone())) {
            Ok(decoded) => Some(decoded),
            Err(error) => {
                if cfg!(debug_assertions) {
                    tracing::debug!(target: "riseonly.sai", "SaiJSONCodec decode failed for {}: {error}", type_name::<T>());
                }
                None
            }
        }
    }

    fn any_to_dict(any: Option<&Value>) -> Option<&Map<String, Value>> {
        any?.as_object()
    }
}
