use std::collections::HashMap;
use std::rc::Rc;

use gpui::http_client::http::{Request, Response};
use gpui::{App, Entity, Task};
use rise_core::foundation::foundation_url::Url;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// takes the request and gives back the edited one
pub type SaiFetchRequestInterceptor = Rc<dyn Fn(Request<Vec<u8>>, &mut App) -> Task<anyhow::Result<Request<Vec<u8>>>>>;

/// takes request, response and body and gives back the edited body
pub type SaiFetchResponseInterceptor =
    Rc<dyn Fn(Rc<Request<Vec<u8>>>, Rc<Response<()>>, Vec<u8>, &mut App) -> Task<anyhow::Result<Vec<u8>>>>;

/// json decoder for the http layer, serde names of the type are the keys it reads
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaiFetchJsonDecoder;

impl SaiFetchJsonDecoder {
    pub fn decode<T: DeserializeOwned>(&self, data: &[u8]) -> serde_json::Result<T> {
        serde_json::from_slice(data)
    }
}

/// how json keys get encoded
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SaiFetchKeyEncodingStrategy {
    #[default]
    UseDefaultKeys,
    ConvertToSnakeCase,
}

/// json encoder for the http layer
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaiFetchJsonEncoder {
    pub key_encoding_strategy: SaiFetchKeyEncodingStrategy,
}

impl SaiFetchJsonEncoder {
    pub fn encode<T: Serialize>(&self, value: &T) -> serde_json::Result<Value> {
        let encoded = serde_json::to_value(value)?;
        Ok(match self.key_encoding_strategy {
            SaiFetchKeyEncodingStrategy::UseDefaultKeys => encoded,
            SaiFetchKeyEncodingStrategy::ConvertToSnakeCase => convert_keys_to_snake_case(encoded),
        })
    }
}

pub(crate) fn convert_keys_to_snake_case(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (convert_to_snake_case(&key), convert_keys_to_snake_case(value)))
                .collect::<Map<String, Value>>(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(convert_keys_to_snake_case).collect()),
        other => other,
    }
}

fn convert_to_snake_case(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    let Some(first) = chars.iter().position(|c| *c != '_') else {
        return key.to_owned();
    };
    let last = chars.iter().rposition(|c| *c != '_').unwrap_or(first);
    let core = &chars[first..=last];

    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for (index, c) in core.iter().enumerate() {
        let starts_word = index > 0 && c.is_uppercase() && {
            let previous = core[index - 1];
            let next_is_lower = core.get(index + 1).is_some_and(|n| n.is_lowercase());
            previous.is_lowercase() || previous.is_numeric() || (previous.is_uppercase() && next_is_lower)
        };
        if starts_word && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }

    let mut out: String = chars[..first].iter().collect();
    out.push_str(&words.join("_"));
    out.extend(chars[last + 1..].iter());
    out
}

/// global http config, main thread only
pub struct SaiFetchHttpConfiguration {
    pub base_url: Option<Url>,
    pub default_headers: HashMap<String, String>,
    pub json_decoder: SaiFetchJsonDecoder,
    pub json_encoder: SaiFetchJsonEncoder,
    pub request_interceptors: Vec<SaiFetchRequestInterceptor>,
    pub response_interceptors: Vec<SaiFetchResponseInterceptor>,
}

impl SaiFetchHttpConfiguration {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::new())
    }

    fn new() -> Self {
        Self {
            base_url: None,
            default_headers: HashMap::new(),
            json_decoder: SaiFetchJsonDecoder,
            json_encoder: SaiFetchJsonEncoder {
                key_encoding_strategy: SaiFetchKeyEncodingStrategy::ConvertToSnakeCase,
            },
            request_interceptors: Vec::new(),
            response_interceptors: Vec::new(),
        }
    }
}
