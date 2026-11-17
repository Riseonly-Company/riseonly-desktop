use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Value};

use crate::core::maintenance::maintenance_gate::RiseServerClock;

pub enum AccessTokenUserId {}

impl AccessTokenUserId {
    const CLAIM_ID: &'static str = "id";
    const CLAIM_SUB: &'static str = "sub";
    const CLAIM_EXP: &'static str = "exp";

    pub fn extract(jwt: &str) -> Option<String> {
        let obj = Self::payload_object(jwt)?;
        if let Some(id) = obj.get(Self::CLAIM_ID).and_then(Value::as_str)
            && !id.is_empty()
        {
            return Some(id.to_owned());
        }
        if let Some(sub) = obj.get(Self::CLAIM_SUB).and_then(Value::as_str)
            && !sub.is_empty()
        {
            return Some(sub.to_owned());
        }
        None
    }

    pub fn expiration_date(jwt: &str) -> Option<Date> {
        let exp = Self::payload_number(jwt, Self::CLAIM_EXP)?;
        let seconds = if exp > 1_000_000_000_000.0 { exp / 1000.0 } else { exp };
        Some(Date::from_time_interval_since_1970(seconds))
    }

    pub fn is_expired(jwt: &str, leeway_seconds: TimeInterval) -> bool {
        let trimmed = trimming_whitespaces_and_newlines(jwt);
        if trimmed.is_empty() {
            return true;
        }
        let Some(exp) = Self::expiration_date(trimmed) else {
            return false;
        };
        exp.time_interval_since(&RiseServerClock::now()) < leeway_seconds
    }

    fn payload_number(jwt: &str, key: &str) -> Option<TimeInterval> {
        let obj = Self::payload_object(jwt)?;
        match obj.get(key)? {
            Value::Number(number) => number.as_f64(),
            Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn payload_object(jwt: &str) -> Option<Map<String, Value>> {
        let trimmed = trimming_whitespaces_and_newlines(jwt);
        let mut parts = trimmed.split('.').filter(|part| !part.is_empty());
        parts.next()?;
        let claims = parts.next()?;
        let mut payload = Self::rejoined(&Self::rejoined(claims, '-', "+"), '_', "/");
        let pad = 4 - payload.len() % 4;
        if pad < 4 {
            payload.push_str(&"=".repeat(pad));
        }
        let data = STANDARD.decode(payload.as_bytes()).ok()?;
        match serde_json::from_slice::<Value>(&data).ok()? {
            Value::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// split then join, empty pieces of the split get dropped
    fn rejoined(text: &str, separator: char, joiner: &str) -> String {
        text.split(separator).filter(|piece| !piece.is_empty()).collect::<Vec<_>>().join(joiner)
    }
}
