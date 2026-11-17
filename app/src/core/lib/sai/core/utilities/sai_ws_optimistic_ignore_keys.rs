use serde_json::{Map, Value};

const NESTED_ARRAY_MARKER: &str = ".*.";

pub enum SaiWsOptimisticIgnoreKeys {}

impl SaiWsOptimisticIgnoreKeys {
    /// items.*.field keeps the targets field in every row of items
    /// a plain key keeps the targets whole value
    pub fn apply(source: &Map<String, Value>, target: &Map<String, Value>, ignore_keys: &[String]) -> Map<String, Value> {
        if ignore_keys.is_empty() {
            return source.clone();
        }
        let mut result = source.clone();

        for ignore_key in ignore_keys {
            if let Some(start) = ignore_key.find(NESTED_ARRAY_MARKER) {
                let array_key = &ignore_key[..start];
                let nested_key = &ignore_key[start + NESTED_ARRAY_MARKER.len()..];

                let Some(target_array) = Self::any_array(target.get(array_key)) else {
                    continue;
                };
                let merged = match Self::any_array(result.get(array_key)) {
                    Some(source_array) => Value::Array(
                        source_array
                            .iter()
                            .enumerate()
                            .map(|(index, source_item)| {
                                let Some(slot) = source_item.as_object() else {
                                    return source_item.clone();
                                };
                                let mut slot = slot.clone();
                                if let Some(nested) = target_array
                                    .get(index)
                                    .and_then(Value::as_object)
                                    .and_then(|target_slot| target_slot.get(nested_key))
                                {
                                    slot.insert(nested_key.to_owned(), nested.clone());
                                }
                                Value::Object(slot)
                            })
                            .collect(),
                    ),
                    None => Value::Array(target_array.clone()),
                };
                result.insert(array_key.to_owned(), merged);
            } else if let Some(kept) = target.get(ignore_key) {
                result.insert(ignore_key.clone(), kept.clone());
            }
        }

        result
    }

    fn any_array(any: Option<&Value>) -> Option<&Vec<Value>> {
        any?.as_array()
    }
}
