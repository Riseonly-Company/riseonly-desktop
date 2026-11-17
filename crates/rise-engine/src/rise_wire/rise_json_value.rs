use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::ser::{self, SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// any json value the wire carries
#[derive(Clone, Debug)]
pub enum RiseJSONValue {
    Null,
    Bool(bool),
    Integer(i64),
    UnsignedInteger(u64),
    Number(f64),
    String(String),
    Array(Vec<RiseJSONValue>),
    Object(BTreeMap<String, RiseJSONValue>),
}

/// a number json cant carry, nan or infinity
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
#[error("JSON numbers must be finite")]
pub struct RiseJSONValueNotFinite;

impl RiseJSONValue {
    pub fn subscript_key(&self, key: &str) -> Option<&RiseJSONValue> {
        let RiseJSONValue::Object(value) = self else { return None };
        value.get(key)
    }

    pub fn subscript_index(&self, index: i64) -> Option<&RiseJSONValue> {
        let RiseJSONValue::Array(value) = self else { return None };
        let index = usize::try_from(index).ok()?;
        value.get(index)
    }

    pub fn object_value(&self) -> Option<&BTreeMap<String, RiseJSONValue>> {
        let RiseJSONValue::Object(value) = self else { return None };
        Some(value)
    }

    pub fn string_value(&self) -> Option<&str> {
        let RiseJSONValue::String(value) = self else { return None };
        Some(value)
    }

    /// encodes to bytes, optionally with sorted keys
    pub fn data(&self, sorted_keys: bool) -> Result<Vec<u8>, serde_json::Error> {
        let _ = sorted_keys;
        serde_json::to_vec(self)
    }

    pub fn decode(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }

    pub fn encode<Value: Serialize + ?Sized>(value: &Value) -> Result<Self, serde_json::Error> {
        serde_json::to_value(value).map(Self::from)
    }

    /// reads a typed value out of this tree
    pub fn decode_as<Value: DeserializeOwned>(&self) -> Result<Value, serde_json::Error> {
        serde_json::from_value(serde_json::Value::from(self.clone()))
    }

    /// number rule, i64 first then u64 then a finite f64
    /// 5.0 decodes as an integer and 5.5 as a double
    fn from_double(value: f64) -> Result<Self, RiseJSONValueNotFinite> {
        if !value.is_finite() {
            return Err(RiseJSONValueNotFinite);
        }
        if value.fract() == 0.0 {
            if value >= i64::MIN as f64 && value < i64::MAX as f64 {
                return Ok(RiseJSONValue::Integer(value as i64));
            }
            if value >= 0.0 && value < u64::MAX as f64 {
                return Ok(RiseJSONValue::UnsignedInteger(value as u64));
            }
        }
        Ok(RiseJSONValue::Number(value))
    }

    fn from_unsigned(value: u64) -> Self {
        match i64::try_from(value) {
            Ok(value) => RiseJSONValue::Integer(value),
            Err(_) => RiseJSONValue::UnsignedInteger(value),
        }
    }
}

impl PartialEq for RiseJSONValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (RiseJSONValue::Null, RiseJSONValue::Null) => true,
            (RiseJSONValue::Bool(lhs), RiseJSONValue::Bool(rhs)) => lhs == rhs,
            (RiseJSONValue::Integer(lhs), RiseJSONValue::Integer(rhs)) => lhs == rhs,
            (RiseJSONValue::UnsignedInteger(lhs), RiseJSONValue::UnsignedInteger(rhs)) => {
                lhs == rhs
            }
            (RiseJSONValue::Number(lhs), RiseJSONValue::Number(rhs)) => lhs == rhs,
            (RiseJSONValue::String(lhs), RiseJSONValue::String(rhs)) => lhs == rhs,
            (RiseJSONValue::Array(lhs), RiseJSONValue::Array(rhs)) => lhs == rhs,
            (RiseJSONValue::Object(lhs), RiseJSONValue::Object(rhs)) => lhs == rhs,
            _ => false,
        }
    }
}

impl Eq for RiseJSONValue {}

impl Hash for RiseJSONValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            RiseJSONValue::Null => {}
            RiseJSONValue::Bool(value) => value.hash(state),
            RiseJSONValue::Integer(value) => value.hash(state),
            RiseJSONValue::UnsignedInteger(value) => value.hash(state),
            RiseJSONValue::Number(value) => {
                let normalized = if *value == 0.0 { 0.0_f64 } else { *value };
                normalized.to_bits().hash(state);
            }
            RiseJSONValue::String(value) => value.hash(state),
            RiseJSONValue::Array(value) => value.hash(state),
            RiseJSONValue::Object(value) => value.hash(state),
        }
    }
}

impl Serialize for RiseJSONValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            RiseJSONValue::Null => serializer.serialize_unit(),
            RiseJSONValue::Bool(value) => serializer.serialize_bool(*value),
            RiseJSONValue::Integer(value) => serializer.serialize_i64(*value),
            RiseJSONValue::UnsignedInteger(value) => serializer.serialize_u64(*value),
            RiseJSONValue::Number(value) => {
                if !value.is_finite() {
                    return Err(ser::Error::custom(RiseJSONValueNotFinite));
                }
                serializer.serialize_f64(*value)
            }
            RiseJSONValue::String(value) => serializer.serialize_str(value),
            RiseJSONValue::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(value)?;
                }
                sequence.end()
            }
            RiseJSONValue::Object(values) => {
                let mut map = serializer.serialize_map(Some(values.len()))?;
                for (key, value) in values {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for RiseJSONValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(RiseJSONValueVisitor)
    }
}

struct RiseJSONValueVisitor;

impl<'de> Visitor<'de> for RiseJSONValueVisitor {
    type Value = RiseJSONValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<RiseJSONValue, E> {
        Ok(RiseJSONValue::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<RiseJSONValue, E> {
        Ok(RiseJSONValue::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<RiseJSONValue, D::Error> {
        RiseJSONValue::deserialize(deserializer)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<RiseJSONValue, E> {
        Ok(RiseJSONValue::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<RiseJSONValue, E> {
        Ok(RiseJSONValue::Integer(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<RiseJSONValue, E> {
        Ok(RiseJSONValue::from_unsigned(value))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<RiseJSONValue, E> {
        RiseJSONValue::from_double(value).map_err(E::custom)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<RiseJSONValue, E> {
        Ok(RiseJSONValue::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<RiseJSONValue, E> {
        Ok(RiseJSONValue::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<RiseJSONValue, A::Error> {
        let mut values = Vec::with_capacity(access.size_hint().unwrap_or(0));
        while let Some(value) = access.next_element::<RiseJSONValue>()? {
            values.push(value);
        }
        Ok(RiseJSONValue::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<RiseJSONValue, A::Error> {
        let mut values = BTreeMap::new();
        while let Some((key, value)) = access.next_entry::<String, RiseJSONValue>()? {
            values.insert(key, value);
        }
        Ok(RiseJSONValue::Object(values))
    }
}

impl From<serde_json::Value> for RiseJSONValue {
    fn from(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => RiseJSONValue::Null,
            serde_json::Value::Bool(value) => RiseJSONValue::Bool(value),
            serde_json::Value::Number(number) => {
                if let Some(value) = number.as_i64() {
                    RiseJSONValue::Integer(value)
                } else if let Some(value) = number.as_u64() {
                    RiseJSONValue::UnsignedInteger(value)
                } else {
                    let value = number.as_f64().unwrap_or(0.0);
                    RiseJSONValue::from_double(value).unwrap_or(RiseJSONValue::Null)
                }
            }
            serde_json::Value::String(value) => RiseJSONValue::String(value),
            serde_json::Value::Array(values) => {
                RiseJSONValue::Array(values.into_iter().map(RiseJSONValue::from).collect())
            }
            serde_json::Value::Object(values) => RiseJSONValue::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, RiseJSONValue::from(value)))
                    .collect(),
            ),
        }
    }
}

impl From<RiseJSONValue> for serde_json::Value {
    fn from(value: RiseJSONValue) -> Self {
        match value {
            RiseJSONValue::Null => serde_json::Value::Null,
            RiseJSONValue::Bool(value) => serde_json::Value::Bool(value),
            RiseJSONValue::Integer(value) => serde_json::Value::from(value),
            RiseJSONValue::UnsignedInteger(value) => serde_json::Value::from(value),
            RiseJSONValue::Number(value) => serde_json::Number::from_f64(value)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
            RiseJSONValue::String(value) => serde_json::Value::String(value),
            RiseJSONValue::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(serde_json::Value::from).collect())
            }
            RiseJSONValue::Object(values) => serde_json::Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, serde_json::Value::from(value)))
                    .collect(),
            ),
        }
    }
}
