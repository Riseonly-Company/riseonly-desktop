#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use rise_canon::{rise_key, rise_wire_canon};
use rise_core::foundation::foundation_regex::{
    NUMBER_IN_PATH, UUID_IN_PATH, replacing_occurrences_regular_expression,
};
use rise_core::foundation::foundation_url::Url;
use serde_json::{Map, Value};

use super::sai_api_doc_types::{SaiApiFieldInfo, SaiApiFieldMap};
use super::sai_http_trace_entry::{SaiHttpTraceEntry, SaiTraceTransport};

pub enum SaiApiDocAnalyzer {}

impl SaiApiDocAnalyzer {
    const WS_ROUTING_KEYS: [&'static str; 4] = [
        rise_wire_canon::key::SERVICE,
        rise_wire_canon::key::METHOD,
        rise_wire_canon::key::FRAME_TYPE,
        rise_wire_canon::key::REQUEST_ID,
    ];

    pub const DEFAULT_MAX_DEPTH: i64 = 4;

    const MAX_FIELDS: usize = 80;

    pub fn endpoint_key(entry: &SaiHttpTraceEntry) -> Option<String> {
        match entry.transport {
            SaiTraceTransport::Websocket => {
                let (service, method) = Self::ws_service_and_method(&entry.request_body_wire)?;
                Some(format!("ws:{service}:{method}"))
            }
            SaiTraceTransport::Http => {
                let normalised = Self::normalise_path(&entry.url);
                let m = entry.method.to_uppercase();
                Some(format!("http:{m}:{normalised}"))
            }
        }
    }

    pub fn ws_service_and_method(wire_body: &str) -> Option<(String, String)> {
        let parsed = Self::parse_ws_body(wire_body)?;
        let service = parsed.get(rise_wire_canon::key::SERVICE)?.as_str()?;
        let method = parsed.get(rise_wire_canon::key::METHOD)?.as_str()?;
        if service.is_empty() || method.is_empty() {
            return None;
        }
        Some((service.to_owned(), method.to_owned()))
    }

    pub fn infer_schema(json_string: &str, stripping_ws_routing: bool, max_depth: i64) -> SaiApiFieldMap {
        let Some(mut dict) = Self::parse_ws_body(json_string) else {
            return SaiApiFieldMap::new();
        };
        if stripping_ws_routing {
            for key in Self::WS_ROUTING_KEYS {
                dict.remove(key);
            }
        }
        let mut result = SaiApiFieldMap::new();
        Self::collect_fields(&dict, "", 0, max_depth, &mut result);
        result
    }

    pub fn merge_schema(base: &SaiApiFieldMap, incoming: &SaiApiFieldMap) -> SaiApiFieldMap {
        let mut result = base.clone();
        for (key, info) in incoming {
            if let Some(existing) = result.get_mut(key) {
                existing.types.extend(info.types.iter().cloned());
                existing.types.sort();
                existing.types.dedup();
                existing.is_optional = existing.is_optional || info.is_optional;
            } else {
                result.insert(
                    key.clone(),
                    SaiApiFieldInfo { types: info.types.clone(), is_optional: true },
                );
            }
        }
        for key in base.keys() {
            if !incoming.contains_key(key)
                && let Some(existing) = result.get_mut(key)
            {
                existing.is_optional = true;
            }
        }
        result
    }

    pub fn variant_fingerprint(status_code: i64, response_body_json: &str) -> String {
        let Some(dict) = Self::parse_ws_body(response_body_json) else {
            return format!("{status_code}|empty|?|N");
        };
        let mut keys: Vec<&str> = dict.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let top_keys = keys.join(",");
        let success_flag = match dict.get(rise_key::auth::SUCCESS).and_then(Value::as_bool) {
            Some(true) => "T",
            Some(false) => "F",
            None => "?",
        };
        let error_flag = if dict.contains_key(rise_wire_canon::key::ERROR) { "E" } else { "N" };
        format!("{status_code}|{top_keys}|{success_flag}|{error_flag}")
    }

    fn parse_ws_body(wire: &str) -> Option<Map<String, Value>> {
        match serde_json::from_str::<Value>(wire).ok()? {
            Value::Object(dict) => Some(dict),
            _ => None,
        }
    }

    fn normalise_path(url_string: &str) -> String {
        let Some(url) = Url::from_string(url_string) else {
            return url_string.to_owned();
        };
        let mut result = url.path();
        if let Some(uuid) = UUID_IN_PATH.as_ref() {
            result = replacing_occurrences_regular_expression(&result, uuid, ":id");
        }
        if let Some(number) = NUMBER_IN_PATH.as_ref() {
            result = replacing_occurrences_regular_expression(&result, number, "/:id");
        }
        if result.is_empty() { "/".to_owned() } else { result }
    }

    fn collect_fields(
        dict: &Map<String, Value>,
        prefix: &str,
        depth: i64,
        max_depth: i64,
        result: &mut SaiApiFieldMap,
    ) {
        if depth >= max_depth || result.len() >= Self::MAX_FIELDS {
            return;
        }
        for (key, value) in dict {
            if result.len() >= Self::MAX_FIELDS {
                return;
            }
            let full_key = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
            Self::merge_field(&full_key, Self::json_type_name(value), result);

            match value {
                Value::Object(nested) => {
                    Self::collect_fields(nested, &full_key, depth + 1, max_depth, result);
                }
                Value::Array(items) => {
                    let arr_key = format!("{full_key}[]");
                    Self::merge_field(&arr_key, "array-item", result);
                    if let Some(Value::Object(first_item)) = items.first() {
                        Self::collect_fields(first_item, &arr_key, depth + 1, max_depth, result);
                    }
                }
                _ => {}
            }
        }
    }

    fn merge_field(key: &str, type_name: &str, result: &mut SaiApiFieldMap) {
        if let Some(existing) = result.get_mut(key) {
            if !existing.types.iter().any(|known| known == type_name) {
                existing.types.push(type_name.to_owned());
                existing.types.sort();
            }
        } else {
            result.insert(
                key.to_owned(),
                SaiApiFieldInfo { types: vec![type_name.to_owned()], is_optional: false },
            );
        }
    }

    fn json_type_name(value: &Value) -> &'static str {
        match value {
            Value::String(_) => "string",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Null => "null",
        }
    }
}
