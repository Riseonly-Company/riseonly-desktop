use std::collections::HashMap;

use rise_canon::rise_key;
use rise_canon::rise_request::event::track_events::EventsElement;
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

pub type EngagementSurface = rise_events::AnalyticsSurface;

#[derive(Clone, Debug, PartialEq)]
pub enum EngagementValue {
    Int(i64),
    Double(f64),
    String(String),
    Bool(bool),
}

impl EngagementValue {
    pub fn wire(&self) -> Value {
        match self {
            Self::Int(v) => Value::from(*v),
            Self::Double(v) => Value::from(*v),
            Self::String(v) => Value::from(v.as_str()),
            Self::Bool(v) => Value::from(*v),
        }
    }
}

impl Serialize for EngagementValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Int(v) => serializer.serialize_i64(*v),
            Self::Double(v) => serializer.serialize_f64(*v),
            Self::String(v) => serializer.serialize_str(v),
            Self::Bool(v) => serializer.serialize_bool(*v),
        }
    }
}

impl<'de> Deserialize<'de> for EngagementValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Bool(v) => Ok(Self::Bool(v)),
            Value::Number(number) => {
                if let Some(v) = number.as_i64() {
                    Ok(Self::Int(v))
                } else if let Some(v) = number.as_f64() {
                    Ok(Self::Double(v))
                } else {
                    Err(D::Error::custom("unsupported EngagementValue"))
                }
            }
            Value::String(v) => Ok(Self::String(v)),
            Value::Null | Value::Array(_) | Value::Object(_) => {
                Err(D::Error::custom("unsupported EngagementValue"))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngagementEvent {
    pub id: String,
    pub event_type: String,
    pub timestamp_ms: i64,
    pub payload: HashMap<String, EngagementValue>,
}

impl EngagementEvent {
    pub fn wire_event(&self) -> EventsElement {
        let finite = self
            .payload
            .values()
            .all(|value| !matches!(value, EngagementValue::Double(v) if !v.is_finite()));
        let json = if finite {
            let p: Map<String, Value> =
                self.payload.iter().map(|(k, v)| (k.clone(), v.wire())).collect();
            JSONSerialization::data_with_json_object(
                &Value::Object(p),
                JSONWritingOptions {
                    sorted_keys: true,
                    ..JSONWritingOptions::default()
                },
            )
            .and_then(|data| String::from_utf8(data).ok())
        } else {
            None
        };
        EventsElement {
            client_event_id: Some(self.id.clone()),
            event_type: Some(self.event_type.clone()),
            payload_json: json,
            timestamp_ms: Some(self.timestamp_ms),
        }
    }

    pub fn wire_dict(&self) -> Map<String, Value> {
        let p: Map<String, Value> =
            self.payload.iter().map(|(k, v)| (k.clone(), v.wire())).collect();
        let mut dict = Map::new();
        dict.insert(rise_key::events::EVENT_TYPE.to_owned(), Value::from(self.event_type.as_str()));
        dict.insert(rise_key::events::TIMESTAMP_MS.to_owned(), Value::from(self.timestamp_ms));
        dict.insert(rise_key::events::PAYLOAD.to_owned(), Value::Object(p));
        dict.insert(rise_key::events::CLIENT_EVENT_ID.to_owned(), Value::from(self.id.as_str()));
        dict
    }
}
