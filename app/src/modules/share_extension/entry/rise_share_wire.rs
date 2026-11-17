use std::collections::HashMap;
use std::fmt;

use rise_canon::rise_key::notify as notify_key;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, Serializer};

use crate::modules::share_extension::entry::share_extension_strings::ShareStrings;

/// json value type of the share layer
#[derive(Clone, Debug, PartialEq)]
pub enum RiseShareJSONValue {
    Object(HashMap<String, RiseShareJSONValue>),
    Array(Vec<RiseShareJSONValue>),
    String(String),
    Integer(i64),
    Number(f64),
    Bool(bool),
    Null,
}

impl<'de> Deserialize<'de> for RiseShareJSONValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(RiseShareJSONValueVisitor)
    }
}

struct RiseShareJSONValueVisitor;

impl<'de> Visitor<'de> for RiseShareJSONValueVisitor {
    type Value = RiseShareJSONValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Unsupported JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(RiseShareJSONValue::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(RiseShareJSONValue::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        RiseShareJSONValue::deserialize(deserializer)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(RiseShareJSONValue::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(RiseShareJSONValue::Integer(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(match i64::try_from(value) {
            Ok(integer) => RiseShareJSONValue::Integer(integer),
            Err(_) => RiseShareJSONValue::Number(value as f64),
        })
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Ok(match exact_integer(value) {
            Some(integer) => RiseShareJSONValue::Integer(integer),
            None => RiseShareJSONValue::Number(value),
        })
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(RiseShareJSONValue::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(RiseShareJSONValue::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(value) = seq.next_element::<RiseShareJSONValue>()? {
            values.push(value);
        }
        Ok(RiseShareJSONValue::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = HashMap::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((key, value)) = map.next_entry::<String, RiseShareJSONValue>()? {
            values.insert(key, value);
        }
        Ok(RiseShareJSONValue::Object(values))
    }
}

impl Serialize for RiseShareJSONValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Object(value) => value.serialize(serializer),
            Self::Array(value) => value.serialize(serializer),
            Self::String(value) => serializer.serialize_str(value),
            Self::Integer(value) => serializer.serialize_i64(*value),
            Self::Number(value) => serializer.serialize_f64(*value),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Null => serializer.serialize_unit(),
        }
    }
}

/// the integer if the float is exact, none otherwise
fn exact_integer(value: f64) -> Option<i64> {
    const UPPER_EXCLUSIVE: f64 = 9_223_372_036_854_775_808.0;
    if !value.is_finite() || value.fract() != 0.0 || value < -UPPER_EXCLUSIVE || value >= UPPER_EXCLUSIVE {
        return None;
    }
    Some(value as i64)
}

fn swift_double_description(value: f64) -> String {
    const PLAIN_LIMIT: f64 = 1e16;
    if value.is_finite() && value.fract() == 0.0 && value.abs() < PLAIN_LIMIT {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

impl RiseShareJSONValue {
    pub fn object_value(&self) -> Option<&HashMap<String, RiseShareJSONValue>> {
        match self {
            Self::Object(value) => Some(value),
            _ => None,
        }
    }

    pub fn array_value(&self) -> Option<&[RiseShareJSONValue]> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    pub fn string_value(&self) -> Option<String> {
        match self {
            Self::String(value) => Some(value.clone()),
            Self::Integer(value) => Some(value.to_string()),
            Self::Number(value) => Some(swift_double_description(*value)),
            _ => None,
        }
    }

    pub fn integer_value(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            Self::Number(value) => exact_integer(*value),
            Self::String(value) => value.parse::<i64>().ok(),
            _ => None,
        }
    }

    pub fn bool_value(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Integer(value) => Some(*value != 0),
            Self::String(value) => match value.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }
}

pub trait RiseShareJSONObject {
    fn first_value(&self, keys: &[&str]) -> Option<&RiseShareJSONValue>;
    fn first_string(&self, keys: &[&str]) -> Option<String>;
    fn first_integer(&self, keys: &[&str]) -> Option<i64>;
    fn first_bool(&self, keys: &[&str]) -> Option<bool>;
}

impl RiseShareJSONObject for HashMap<String, RiseShareJSONValue> {
    fn first_value(&self, keys: &[&str]) -> Option<&RiseShareJSONValue> {
        keys.iter().find_map(|key| self.get(*key))
    }

    fn first_string(&self, keys: &[&str]) -> Option<String> {
        self.first_value(keys)?.string_value()
    }

    fn first_integer(&self, keys: &[&str]) -> Option<i64> {
        self.first_value(keys)?.integer_value()
    }

    fn first_bool(&self, keys: &[&str]) -> Option<bool> {
        self.first_value(keys)?.bool_value()
    }
}

/// share replies are canon types, the layer decodes what the gateway answers
pub type RiseShareTargetsWireResponse = rise_canon::rise_response::chat::GetShareTargets;
pub type RiseShareTopicsWireResponse = rise_canon::rise_response::chat::GetShareTopics;
pub type RiseSharePayloadWireResponse = rise_canon::rise_response::message::SharePayload;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseShareTransportError {
    InvalidURL,
    InvalidResponse,
    Timeout,
    HttpStatus(i64, String),
    Server { code: Option<String>, message: String },
}

impl RiseShareTransportError {
    const STATUS_UNAUTHORIZED: i64 = 401;
    const STATUS_FORBIDDEN: i64 = 403;
    const STATUS_REQUEST_TIMEOUT: i64 = 408;
    const STATUS_GATEWAY_TIMEOUT: i64 = 504;

    fn normalized(code: Option<&str>, message: &str) -> String {
        format!("{} {}", code.unwrap_or(""), message).to_lowercase()
    }

    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::InvalidURL | Self::InvalidResponse => ShareStrings::invalid_server_response(),
            Self::Timeout => ShareStrings::request_timed_out(),
            Self::HttpStatus(status, _) => match *status {
                Self::STATUS_UNAUTHORIZED => ShareStrings::session_expired(),
                Self::STATUS_FORBIDDEN => ShareStrings::invalid_selection(),
                Self::STATUS_REQUEST_TIMEOUT | Self::STATUS_GATEWAY_TIMEOUT => ShareStrings::request_timed_out(),
                _ => ShareStrings::service_unavailable(),
            },
            Self::Server { code, message } => {
                let normalized = Self::normalized(code.as_deref(), message);
                if normalized.contains("token") || normalized.contains("auth") {
                    ShareStrings::session_expired()
                } else if normalized.contains("timeout") || normalized.contains("deadline") {
                    ShareStrings::request_timed_out()
                } else {
                    ShareStrings::service_unavailable()
                }
            }
        })
    }

    /// error text to show the user
    pub fn localized_description(&self) -> String {
        self.error_description().unwrap_or_default()
    }

    pub fn is_authentication_failure(&self) -> bool {
        match self {
            Self::HttpStatus(status, _) => *status == Self::STATUS_UNAUTHORIZED,
            Self::Server { code, message } => {
                let normalized = Self::normalized(code.as_deref(), message);
                normalized.contains("unauthor")
                    || normalized.contains("token_expired")
                    || normalized.contains(notify_key::INVALID_TOKEN)
                    || normalized.contains("auth_required")
            }
            _ => false,
        }
    }
}

impl fmt::Display for RiseShareTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.localized_description())
    }
}

impl std::error::Error for RiseShareTransportError {}
