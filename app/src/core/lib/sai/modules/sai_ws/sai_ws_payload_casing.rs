use serde_json::{Map, Value};

/// first key that holds a string wins, other value kinds are skipped
pub fn ws_string(payload: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
}
