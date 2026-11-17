use std::collections::HashMap;

use gpui::http_client::http::{Request, Response};
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::foundation_text::{
    character_count, character_prefix, trimming_whitespaces_and_newlines,
};
use serde_json::{Map, Value};

const PRETTY_LIMIT_CHARACTERS: usize = 64_000;

pub enum SaiHttpTraceRecorder {}

impl SaiHttpTraceRecorder {
    pub fn request_headers(request: &Request<Vec<u8>>) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        for (k, v) in request.headers() {
            let Ok(v) = v.to_str() else {
                continue;
            };
            if k.as_str().to_lowercase() == "authorization" && v.to_lowercase().starts_with("bearer ") {
                out.insert(k.as_str().to_owned(), "Bearer ***".to_owned());
            } else {
                out.insert(k.as_str().to_owned(), v.to_owned());
            }
        }
        out
    }

    pub fn response_headers(http: &Response<()>) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        for (k, v) in http.headers() {
            let Ok(vs) = v.to_str() else {
                continue;
            };
            out.entry(k.as_str().to_owned())
                .and_modify(|joined| {
                    joined.push_str(", ");
                    joined.push_str(vs);
                })
                .or_insert_with(|| vs.to_owned());
        }
        out
    }

    pub fn wire_json_string(from_http_body: Option<&[u8]>) -> String {
        let data = match from_http_body {
            Some(data) if !data.is_empty() => data,
            _ => return "{}".to_owned(),
        };
        let pretty = JSONSerialization::json_object(data).and_then(|obj| Self::pretty_text(&obj));
        match pretty {
            Some(s) => Self::limited(s),
            None => match std::str::from_utf8(data) {
                Ok(s) => s.to_owned(),
                Err(_) => format!("<binary {} bytes>", data.len()),
            },
        }
    }

    pub fn pretty_json_string_from_json_object(obj: &Map<String, Value>) -> String {
        let value = Value::Object(obj.clone());
        match Self::pretty_text(&value) {
            Some(s) => Self::limited(s),
            None => value.to_string(),
        }
    }

    pub fn pretty_json_string_from_any(value: Option<&Value>) -> String {
        let Some(value) = value else {
            return "(null)".to_owned();
        };
        match value {
            Value::Null => "(null)".to_owned(),
            Value::Object(d) => Self::pretty_json_string_from_json_object(d),
            _ => {
                if JSONSerialization::is_valid_json_object(value) {
                    if let Some(s) = Self::pretty_text(value) {
                        return Self::limited(s);
                    }
                }
                Self::describing(value)
            }
        }
    }

    pub fn pretty_json_string_from_raw_utf8(raw: &str) -> String {
        let t = trimming_whitespaces_and_newlines(raw);
        match JSONSerialization::json_object(t.as_bytes()).and_then(|obj| Self::pretty_text(&obj)) {
            Some(s) => Self::limited(s),
            None => raw.to_owned(),
        }
    }

    pub fn truncate_utf8(data: &[u8], max_chars: usize) -> String {
        let s = match std::str::from_utf8(data) {
            Ok(s) if !s.is_empty() => s,
            _ => return format!("<binary {} bytes>", data.len()),
        };
        let t = trimming_whitespaces_and_newlines(s);
        if character_count(t) <= max_chars {
            return t.to_owned();
        }
        format!("{}…", character_prefix(t, max_chars))
    }

    fn pretty_text(value: &Value) -> Option<String> {
        let options = JSONWritingOptions {
            sorted_keys: true,
            pretty_printed: true,
            ..JSONWritingOptions::default()
        };
        let data = JSONSerialization::data_with_json_object(value, options)?;
        String::from_utf8(data).ok()
    }

    fn limited(s: String) -> String {
        if character_count(&s) > PRETTY_LIMIT_CHARACTERS {
            format!("{}…", character_prefix(&s, PRETTY_LIMIT_CHARACTERS))
        } else {
            s
        }
    }

    fn describing(value: &Value) -> String {
        match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
}
