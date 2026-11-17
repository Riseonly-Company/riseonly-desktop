use std::fmt;

use rise_canon::rise_wire_canon::key;
use rise_core::foundation::foundation_date::TimeInterval;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;

const DEFAULT_HTTP_METHOD: &str = "POST";
const OPTION_CACHE_KEY: &str = "cacheKey";
const OPTION_RELOAD_IF_CACHED: &str = "reloadIfCached";
const OPTION_FETCH_IF_HAVE_DATA: &str = "fetchIfHaveData";
const OPTION_ALLOW_CONCURRENT_WHILE_PENDING: &str = "allowConcurrentWhilePending";
const OPTION_FETCH_IF_PENDING: &str = "fetchIfPending";
const OPTION_TIMEOUT: &str = "timeout";

#[derive(Clone, Debug, PartialEq)]
pub struct SaiFetchOptions {
    pub cache_key: Option<String>,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
    pub reload_if_cached: bool,
    pub allow_concurrent_while_pending: bool,
    pub http_method: String,
    pub timeout_seconds: Option<TimeInterval>,
}

impl Default for SaiFetchOptions {
    fn default() -> Self {
        Self::new(None, true, false, DEFAULT_HTTP_METHOD, None)
    }
}

impl SaiFetchOptions {
    pub fn new(
        cache_key: Option<&str>,
        reload_if_cached: bool,
        allow_concurrent_while_pending: bool,
        http_method: &str,
        timeout_seconds: Option<TimeInterval>,
    ) -> Self {
        Self {
            cache_key: cache_key.map(SaiAccountCacheScope::scoped_key),
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            reload_if_cached,
            allow_concurrent_while_pending,
            http_method: http_method.to_owned(),
            timeout_seconds,
        }
    }

    /// options built from a cache key string
    pub fn with_cache_key(cache_key: &str, reload_if_cached: bool, allow_concurrent_while_pending: bool) -> Self {
        Self::new(
            Some(cache_key),
            reload_if_cached,
            allow_concurrent_while_pending,
            DEFAULT_HTTP_METHOD,
            None,
        )
    }

    /// options built from a cache object
    pub fn with_cache_key_cache(cache: &SaiCache, reload_if_cached: bool, allow_concurrent_while_pending: bool) -> Self {
        Self::with_cache_key(&cache.key, reload_if_cached, allow_concurrent_while_pending)
    }

    /// builds options from a json style map
    pub fn from_options_dictionary(dict: &Map<String, Value>) -> Result<Self, SaiFetchOptionsParseError> {
        let id_or_cache = non_empty_string(dict, OPTION_CACHE_KEY).or_else(|| non_empty_string(dict, key::ID));
        let http_method = non_empty_string(dict, key::METHOD).unwrap_or(DEFAULT_HTTP_METHOD);
        Ok(Self {
            cache_key: id_or_cache.map(SaiAccountCacheScope::scoped_key),
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            reload_if_cached: Self::optional_bool(dict, OPTION_RELOAD_IF_CACHED, OPTION_FETCH_IF_HAVE_DATA, true)?,
            allow_concurrent_while_pending: Self::optional_bool(
                dict,
                OPTION_ALLOW_CONCURRENT_WHILE_PENDING,
                OPTION_FETCH_IF_PENDING,
                false,
            )?,
            http_method: http_method.to_owned(),
            timeout_seconds: Self::optional_seconds(dict, OPTION_TIMEOUT)?,
        })
    }

    fn optional_bool(
        dict: &Map<String, Value>,
        key: &str,
        alt_key: &str,
        def: bool,
    ) -> Result<bool, SaiFetchOptionsParseError> {
        if dict.contains_key(key) {
            return Self::optional_bool_single(dict, key, def);
        }
        if dict.contains_key(alt_key) {
            return Self::optional_bool_single(dict, alt_key, def);
        }
        Ok(def)
    }

    fn optional_bool_single(dict: &Map<String, Value>, key: &str, def: bool) -> Result<bool, SaiFetchOptionsParseError> {
        let Some(v) = dict.get(key) else {
            return Ok(def);
        };
        match v {
            Value::Bool(b) => Ok(*b),
            Value::Number(n) => Ok(n.as_f64().is_some_and(|n| n != 0.0)),
            _ => Err(SaiFetchOptionsParseError::TypeMismatch {
                key: key.to_owned(),
                expected: "Bool".to_owned(),
            }),
        }
    }

    fn optional_seconds(dict: &Map<String, Value>, key: &str) -> Result<Option<TimeInterval>, SaiFetchOptionsParseError> {
        let Some(v) = dict.get(key) else {
            return Ok(None);
        };
        let seconds = match v {
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => v.as_f64(),
        };
        match seconds {
            Some(seconds) => Ok(Some(seconds)),
            None => Err(SaiFetchOptionsParseError::TypeMismatch {
                key: key.to_owned(),
                expected: "Number (seconds)".to_owned(),
            }),
        }
    }
}

fn non_empty_string<'a>(dict: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    dict.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaiFetchOptionsParseError {
    TypeMismatch { key: String, expected: String },
}

impl SaiFetchOptionsParseError {
    pub fn error_description(&self) -> Option<String> {
        match self {
            Self::TypeMismatch { key, expected } => {
                Some(format!("SaiFetch options[\"{key}\"] has wrong type (expected {expected})."))
            }
        }
    }
}

impl fmt::Display for SaiFetchOptionsParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for SaiFetchOptionsParseError {}
