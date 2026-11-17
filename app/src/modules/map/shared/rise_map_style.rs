use std::collections::HashMap;
use std::sync::LazyLock;

use parking_lot::Mutex;
use rise_theme::RiseThemeAppearance;
use serde_json::Value;

/// base map style json, one bundled file per appearance over openfreemap vector tiles
/// plain style spec, opens in maputnik as is and ships unchanged to android and desktop
/// labels are authored against name:en and rewritten to app language
pub enum RiseMapStyle {}

static CACHE: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

impl RiseMapStyle {
    const TILE_LANGUAGES: [&'static str; 37] = [
        "ar", "bn", "cs", "da", "de", "el", "en", "es", "fa", "fi", "fr", "he", "hi", "hu", "id", "it", "ja", "ka",
        "kk", "ko", "nl", "no", "pl", "pt", "ro", "ru", "sr", "sv", "ta", "th", "tr", "uk", "ur", "vi", "zh",
        "zh-Hans", "zh-Hant",
    ];

    const AUTHORED_LABEL_KEY: &'static str = "name:en";

    const BLANK_STYLE: &'static str = r##"{"version":8,"sources":{},"layers":[{"id":"background","type":"background","paint":{"background-color":"#F4F2EE"}}]}"##;

    pub fn json(appearance: RiseThemeAppearance, language_code: &str) -> String {
        let label_key = format!("name:{}", Self::tile_language(language_code));
        let cache_key = format!("{}|{label_key}", appearance.raw_value());
        if let Some(cached) = CACHE.lock().get(&cache_key) {
            return cached.clone();
        }

        let relabeled = serde_json::from_str::<Value>(Self::resource_name(appearance))
            .and_then(|style| serde_json::to_string(&Self::relabel(style, &label_key)));
        let json = match relabeled {
            Ok(json) => json,
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "bundled map style unreadable");
                return Self::BLANK_STYLE.to_owned();
            }
        };
        CACHE.lock().insert(cache_key, json.clone());
        json
    }

    fn resource_name(appearance: RiseThemeAppearance) -> &'static str {
        match appearance {
            RiseThemeAppearance::Light => include_str!("../styles/rise-map-light.json"),
            RiseThemeAppearance::Dark => include_str!("../styles/rise-map-dark.json"),
        }
    }

    fn tile_language(code: &str) -> String {
        if Self::TILE_LANGUAGES.iter().any(|language| *language == code) {
            return code.to_owned();
        }
        if let Some(base) = code.split('-').find(|piece| !piece.is_empty())
            && Self::TILE_LANGUAGES.iter().any(|language| *language == base)
        {
            return base.to_owned();
        }
        "latin".to_owned()
    }

    fn relabel(node: Value, label_key: &str) -> Value {
        match node {
            Value::Array(array) => {
                if array.len() == 2
                    && array[0].as_str() == Some("get")
                    && array[1].as_str() == Some(Self::AUTHORED_LABEL_KEY)
                {
                    return Value::Array(vec![Value::from("get"), Value::from(label_key)]);
                }
                Value::Array(array.into_iter().map(|child| Self::relabel(child, label_key)).collect())
            }
            Value::Object(object) => Value::Object(
                object
                    .into_iter()
                    .map(|(key, child)| (key, Self::relabel(child, label_key)))
                    .collect(),
            ),
            other => other,
        }
    }
}
