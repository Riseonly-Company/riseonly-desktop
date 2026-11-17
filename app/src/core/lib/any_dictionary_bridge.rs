use serde_json::{Map, Value};

pub enum AnyDictionaryBridge {}

impl AnyDictionaryBridge {
    /// an empty object reads as no dictionary at all
    pub fn dictionary(value: Option<&Value>) -> Option<Map<String, Value>> {
        let Value::Object(object) = value? else {
            return None;
        };
        if object.is_empty() {
            return None;
        }
        Some(object.clone())
    }

    pub fn array_of_dictionaries(value: Option<&Value>) -> Vec<Map<String, Value>> {
        let Some(Value::Array(elements)) = value else {
            return Vec::new();
        };
        elements
            .iter()
            .filter_map(|element| Self::dictionary(Some(element)))
            .collect()
    }
}
