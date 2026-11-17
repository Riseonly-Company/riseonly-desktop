use std::collections::HashMap;
use std::sync::LazyLock;

use parking_lot::Mutex;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Number, Value};

pub enum SaiWsJSONObjectDeepCompare {}

impl SaiWsJSONObjectDeepCompare {
    pub fn bodies_equal(a: &Value, b: &Value) -> bool {
        let (Some(na), Some(nb)) = (Self::normalize(a), Self::normalize(b)) else {
            return false;
        };
        Self::is_equal(na, nb)
    }

    /// only an object or array is allowed at the top, anything else gives none
    /// key order doesnt matter for equality here
    fn normalize(value: &Value) -> Option<&Value> {
        (value.is_object() || value.is_array()).then_some(value)
    }

    /// json equality, numbers compare by value so 1 equals 1.0 and true equals 1
    fn is_equal(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Null, Value::Null) => true,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Self::is_equal(a, b))
            }
            (Value::Object(a), Value::Object(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(key, value)| b.get(key).is_some_and(|other| Self::is_equal(value, other)))
            }
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => Self::numbers_equal(a, b),
            (Value::Bool(flag), Value::Number(number)) | (Value::Number(number), Value::Bool(flag)) => {
                Self::numbers_equal(number, &Number::from(u8::from(*flag)))
            }
            _ => false,
        }
    }

    fn numbers_equal(a: &Number, b: &Number) -> bool {
        if let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) {
            return a == b;
        }
        if let (Some(a), Some(b)) = (a.as_u64(), b.as_u64()) {
            return a == b;
        }
        match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
}

static LAST_BY_SCOPE: LazyLock<Mutex<HashMap<String, Value>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub enum SaiWsLastOutboundBodyRegistry {}

impl SaiWsLastOutboundBodyRegistry {
    pub fn last_body(for_scope: &str) -> Option<Value> {
        let k = Self::normalize(for_scope);
        LAST_BY_SCOPE.lock().get(k).cloned()
    }

    pub fn remember_body(body: &Value, scope: &str) {
        let k = Self::normalize(scope);
        let Some(snap) = Self::snapshot(body) else {
            return;
        };
        LAST_BY_SCOPE.lock().insert(k.to_owned(), snap);
    }

    pub fn clear_scope(scope: &str) {
        LAST_BY_SCOPE.lock().remove(Self::normalize(scope));
    }

    pub fn clear_all() {
        LAST_BY_SCOPE.lock().clear();
    }

    fn normalize(scope: &str) -> &str {
        trimming_whitespaces_and_newlines(scope)
    }

    fn snapshot(value: &Value) -> Option<Value> {
        (value.is_object() || value.is_array()).then(|| value.clone())
    }
}
