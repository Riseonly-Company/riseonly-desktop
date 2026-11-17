use std::fmt;
use std::rc::Rc;

use gpui::App;
use rise_canon::rise_wire_canon::key::{METHOD, SERVICE};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_coalescing::{
    SaiWsCoalescingOptions, SaiWsCoalescingStrategy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticUpdate;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationOptions;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiWsScreenRefresh;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaiWsSendError {
    NotConnected,
    Maintenance,
    GuideSandbox,
    EncodingFailed(String),
    SendFailed(String),
}

impl SaiWsSendError {
    pub fn error_description(&self) -> String {
        match self {
            Self::NotConnected => "WebSocket is not connected.".to_owned(),
            Self::Maintenance => tr("maintenance_action_blocked"),
            Self::GuideSandbox => tr("guide_sandbox_action_unavailable"),
            Self::EncodingFailed(reason) => format!("Failed to encode WebSocket message: {reason}"),
            Self::SendFailed(reason) => format!("Failed to send on WebSocket: {reason}"),
        }
    }
}

impl fmt::Display for SaiWsSendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description())
    }
}

impl std::error::Error for SaiWsSendError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiWsCachePreferredLayer {
    MemoryFirst,
    DiskFirst,
}

impl SaiWsCachePreferredLayer {
    pub const ALL_CASES: [Self; 2] = [Self::MemoryFirst, Self::DiskFirst];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::MemoryFirst => "memoryFirst",
            Self::DiskFirst => "diskFirst",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|layer| layer.raw_value() == raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiWsDiskCachePolicy {
    PreserveExisting,
    Persistent,
    MemoryOnly,
}

impl SaiWsDiskCachePolicy {
    pub const ALL_CASES: [Self; 3] = [Self::PreserveExisting, Self::Persistent, Self::MemoryOnly];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::PreserveExisting => "preserveExisting",
            Self::Persistent => "persistent",
            Self::MemoryOnly => "memoryOnly",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|policy| policy.raw_value() == raw)
    }
}

#[derive(Clone)]
pub struct SaiWsScreenRefreshCallbacks {
    pub on_success: SaiWsOnSuccess,
    pub on_error: SaiWsOnError,
}

pub type SaiWsMakeScreenRefreshResendCallbacks = Rc<dyn Fn(&mut App) -> SaiWsScreenRefreshCallbacks>;

#[derive(Clone)]
pub struct SaiWsRequestOptions {
    pub service: String,
    pub method: String,

    pub cache_key: Option<String>,
    pub reload_if_cached: bool,
    pub allow_concurrent_while_pending: bool,
    pub reflect_pending_in_ui: bool,
    pub send_immediately: bool,
    pub allows_deferred_send: bool,
    pub store_response_in_cache: bool,
    pub disk_cache_policy: SaiWsDiskCachePolicy,
    pub reload_when_only_disk_cached: bool,
    pub preferred_cache_layer: SaiWsCachePreferredLayer,
    pub invoke_success_for_cached_response: bool,
    pub path_to_array: Option<String>,
    pub path_to_id: Option<String>,
    pub pagination: Option<SaiWsPaginationOptions>,
    pub take_path: Option<String>,
    pub sai_slice: Option<Rc<dyn SaiSliceState>>,
    pub track_until_acknowledged: bool,
    pub offline_ack_timeout_seconds: Option<f64>,
    pub optimistic_update: Option<SaiWsOptimisticUpdate>,
    pub screen_refresh: Option<SaiWsScreenRefresh>,
    pub make_screen_refresh_resend_callbacks: Option<SaiWsMakeScreenRefreshResendCallbacks>,
    pub force_bypass_cache_read: bool,
    pub is_screen_refresh_wire: bool,
    pub coalescing: Option<SaiWsCoalescingOptions>,
    pub deep_compare: bool,
    pub deep_compare_body_key: Option<String>,
    pub exclude_from_debugger: bool,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
}

/// arguments for request options, with defaults
#[derive(Clone)]
pub struct SaiWsRequestOptionsInit {
    pub service: String,
    pub method: String,
    pub cache_key: Option<String>,
    pub reload_if_cached: bool,
    pub allow_concurrent_while_pending: bool,
    pub reflect_pending_in_ui: bool,
    pub send_immediately: bool,
    pub allows_deferred_send: bool,
    pub store_response_in_cache: bool,
    pub persist_cache_to_disk: bool,
    pub disk_cache_policy: Option<SaiWsDiskCachePolicy>,
    pub reload_when_only_disk_cached: bool,
    pub preferred_cache_layer: SaiWsCachePreferredLayer,
    pub invoke_success_for_cached_response: bool,
    pub path_to_array: Option<String>,
    pub path_to_id: Option<String>,
    pub pagination: Option<SaiWsPaginationOptions>,
    pub take_path: Option<String>,
    pub optimistic_update: Option<SaiWsOptimisticUpdate>,
    pub screen_refresh: Option<SaiWsScreenRefresh>,
    pub make_screen_refresh_resend_callbacks: Option<SaiWsMakeScreenRefreshResendCallbacks>,
    pub sai_slice: Option<Rc<dyn SaiSliceState>>,
    pub track_until_acknowledged: bool,
    pub offline_ack_timeout_seconds: Option<f64>,
    pub coalescing: Option<SaiWsCoalescingOptions>,
    pub deep_compare: bool,
    pub deep_compare_body_key: Option<String>,
    pub exclude_from_debugger: bool,
    pub force_bypass_cache_read: bool,
    pub is_screen_refresh_wire: bool,
}

impl Default for SaiWsRequestOptionsInit {
    fn default() -> Self {
        Self {
            service: String::new(),
            method: String::new(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            allows_deferred_send: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            disk_cache_policy: None,
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            invoke_success_for_cached_response: true,
            path_to_array: None,
            path_to_id: None,
            pagination: None,
            take_path: None,
            optimistic_update: None,
            screen_refresh: None,
            make_screen_refresh_resend_callbacks: None,
            sai_slice: None,
            track_until_acknowledged: false,
            offline_ack_timeout_seconds: None,
            coalescing: None,
            deep_compare: false,
            deep_compare_body_key: None,
            exclude_from_debugger: false,
            force_bypass_cache_read: false,
            is_screen_refresh_wire: false,
        }
    }
}

impl SaiWsRequestOptions {
    pub fn new(init: SaiWsRequestOptionsInit) -> Self {
        let disk_cache_policy = init.disk_cache_policy.unwrap_or(if init.persist_cache_to_disk {
            SaiWsDiskCachePolicy::Persistent
        } else {
            SaiWsDiskCachePolicy::PreserveExisting
        });
        Self {
            service: init.service,
            method: init.method,
            cache_key: init.cache_key.as_deref().map(SaiAccountCacheScope::scoped_key),
            reload_if_cached: init.reload_if_cached,
            allow_concurrent_while_pending: init.allow_concurrent_while_pending,
            reflect_pending_in_ui: init.reflect_pending_in_ui,
            send_immediately: init.send_immediately,
            allows_deferred_send: init.allows_deferred_send,
            store_response_in_cache: init.store_response_in_cache,
            disk_cache_policy,
            reload_when_only_disk_cached: init.reload_when_only_disk_cached,
            preferred_cache_layer: init.preferred_cache_layer,
            invoke_success_for_cached_response: init.invoke_success_for_cached_response,
            path_to_array: init.path_to_array,
            path_to_id: init.path_to_id,
            pagination: init.pagination,
            take_path: init.take_path,
            sai_slice: init.sai_slice,
            track_until_acknowledged: init.track_until_acknowledged,
            offline_ack_timeout_seconds: init.offline_ack_timeout_seconds,
            optimistic_update: init.optimistic_update,
            screen_refresh: init.screen_refresh,
            make_screen_refresh_resend_callbacks: init.make_screen_refresh_resend_callbacks,
            force_bypass_cache_read: init.force_bypass_cache_read,
            is_screen_refresh_wire: init.is_screen_refresh_wire,
            coalescing: init.coalescing,
            deep_compare: init.deep_compare,
            deep_compare_body_key: init.deep_compare_body_key,
            exclude_from_debugger: init.exclude_from_debugger,
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
        }
    }

    pub fn persist_cache_to_disk(&self) -> bool {
        self.disk_cache_policy == SaiWsDiskCachePolicy::Persistent
    }

    pub fn set_persist_cache_to_disk(&mut self, new_value: bool) {
        self.disk_cache_policy = if new_value {
            SaiWsDiskCachePolicy::Persistent
        } else {
            SaiWsDiskCachePolicy::PreserveExisting
        };
    }

    pub fn typed_slice(&self) -> Option<Rc<dyn SaiSliceState>> {
        self.sai_slice.clone()
    }

    pub fn set_typed_slice(&mut self, new_value: Option<Rc<dyn SaiSliceState>>) {
        self.sai_slice = new_value;
    }

    pub fn deep_compare_scope_key(&self) -> Option<String> {
        if !self.deep_compare {
            return None;
        }
        if let Some(key) = self
            .deep_compare_body_key
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|key| !key.is_empty())
        {
            return Some(key.to_owned());
        }
        if let Some(cache_key) = self
            .cache_key
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|key| !key.is_empty())
        {
            return Some(cache_key.to_owned());
        }
        Some(format!("{}.{}", self.service, self.method))
    }
}

#[derive(Clone, PartialEq)]
pub struct SaiWsPendingCacheBinding {
    pub cache_key: Option<String>,
    pub store_response_in_cache: bool,
    pub disk_cache_policy: SaiWsDiskCachePolicy,
    pub path_to_array: Option<String>,
    pub path_to_id: Option<String>,
    pub pagination: Option<SaiWsPaginationOptions>,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
}

impl SaiWsPendingCacheBinding {
    pub fn new(options: &SaiWsRequestOptions) -> Self {
        fn trimmed(value: Option<&str>) -> Option<String> {
            value
                .map(trimming_whitespaces_and_newlines)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        }
        Self {
            cache_key: options.cache_key.clone(),
            store_response_in_cache: options.store_response_in_cache,
            disk_cache_policy: options.disk_cache_policy,
            account_user_id: options.account_user_id.clone(),
            account_epoch: options.account_epoch.clone(),
            pagination: options.pagination.clone(),
            path_to_array: trimmed(options.path_to_array.as_deref()),
            path_to_id: trimmed(options.path_to_id.as_deref()),
        }
    }

    pub fn persist_cache_to_disk(&self) -> bool {
        self.disk_cache_policy == SaiWsDiskCachePolicy::Persistent
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaiWsRequestOptionsParseError {
    MissingService,
    MissingMethod,
    TypeMismatch { key: String, expected: String },
}

impl SaiWsRequestOptionsParseError {
    pub fn error_description(&self) -> String {
        match self {
            Self::MissingService => {
                "options dictionary must include string key \"service\".".to_owned()
            }
            Self::MissingMethod => "options dictionary must include string key \"method\".".to_owned(),
            Self::TypeMismatch { key, expected } => {
                format!("options[\"{key}\"] has wrong type (expected {expected}).")
            }
        }
    }
}

impl fmt::Display for SaiWsRequestOptionsParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description())
    }
}

impl std::error::Error for SaiWsRequestOptionsParseError {}

impl SaiWsRequestOptions {
    pub fn from_options_dictionary(
        dict: &Map<String, Value>,
    ) -> Result<Self, SaiWsRequestOptionsParseError> {
        let Some(service) = dict.get(SERVICE).and_then(Value::as_str) else {
            return Err(SaiWsRequestOptionsParseError::MissingService);
        };
        let Some(method) = dict.get(METHOD).and_then(Value::as_str) else {
            return Err(SaiWsRequestOptionsParseError::MissingMethod);
        };

        let persist_cache_to_disk = Self::optional_bool(dict, "persistCacheToDisk", false)?;
        let disk_cache_policy = Self::optional_disk_cache_policy(
            dict,
            "diskCachePolicy",
            if persist_cache_to_disk {
                SaiWsDiskCachePolicy::Persistent
            } else {
                SaiWsDiskCachePolicy::PreserveExisting
            },
        )?;
        Ok(Self {
            service: service.to_owned(),
            method: method.to_owned(),
            cache_key: Self::optional_string(dict, "cacheKey")?
                .as_deref()
                .map(SaiAccountCacheScope::scoped_key),
            reload_if_cached: Self::optional_bool(dict, "reloadIfCached", true)?,
            allow_concurrent_while_pending: Self::optional_bool(
                dict,
                "allowConcurrentWhilePending",
                false,
            )?,
            reflect_pending_in_ui: Self::optional_bool(dict, "reflectPendingInUI", true)?,
            send_immediately: Self::optional_bool(dict, "sendImmediately", true)?,
            allows_deferred_send: Self::optional_bool(dict, "allowsDeferredSend", true)?,
            store_response_in_cache: Self::optional_bool(dict, "storeResponseInCache", true)?,
            disk_cache_policy,
            reload_when_only_disk_cached: Self::optional_bool(
                dict,
                "reloadWhenOnlyDiskCached",
                false,
            )?,
            preferred_cache_layer: Self::optional_preferred_layer(
                dict,
                "preferredCacheLayer",
                SaiWsCachePreferredLayer::MemoryFirst,
            )?,
            invoke_success_for_cached_response: Self::optional_bool(
                dict,
                "invokeSuccessForCachedResponse",
                true,
            )?,
            path_to_array: Self::optional_string(dict, "pathToArray")?,
            path_to_id: Self::optional_string(dict, "pathToId")?,
            pagination: None,
            take_path: Self::optional_string(dict, "takePath")?,
            optimistic_update: None,
            screen_refresh: None,
            make_screen_refresh_resend_callbacks: None,
            sai_slice: None,
            force_bypass_cache_read: false,
            is_screen_refresh_wire: false,
            track_until_acknowledged: Self::optional_bool(dict, "trackUntilAcknowledged", false)?,
            offline_ack_timeout_seconds: Self::optional_seconds(dict, "offlineAckTimeoutSeconds")?,
            coalescing: Self::optional_coalescing(dict)?,
            deep_compare: Self::optional_bool(dict, "deepCompare", false)?,
            deep_compare_body_key: Self::optional_string(dict, "deepCompareBodyKey")?,
            exclude_from_debugger: Self::optional_bool(dict, "excludeFromDebugger", false)?,
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
        })
    }

    fn type_mismatch(key: &str, expected: &str) -> SaiWsRequestOptionsParseError {
        SaiWsRequestOptionsParseError::TypeMismatch {
            key: key.to_owned(),
            expected: expected.to_owned(),
        }
    }

    fn optional_coalescing(
        dict: &Map<String, Value>,
    ) -> Result<Option<SaiWsCoalescingOptions>, SaiWsRequestOptionsParseError> {
        let Some(key) = Self::optional_string(dict, "coalescingKey")?.filter(|key| !key.is_empty())
        else {
            return Ok(None);
        };
        let raw = Self::optional_string(dict, "coalescingStrategy")?
            .unwrap_or_else(|| SaiWsCoalescingStrategy::ToggleParity.raw_value().to_owned());
        let Some(strategy) = SaiWsCoalescingStrategy::from_raw(&raw) else {
            return Err(Self::type_mismatch(
                "coalescingStrategy",
                SaiWsCoalescingStrategy::ToggleParity.raw_value(),
            ));
        };
        Ok(Some(SaiWsCoalescingOptions::with_strategy(key, strategy)))
    }

    fn optional_string(
        dict: &Map<String, Value>,
        key: &str,
    ) -> Result<Option<String>, SaiWsRequestOptionsParseError> {
        match dict.get(key) {
            None => Ok(None),
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => Err(Self::type_mismatch(key, "String")),
        }
    }

    fn optional_bool(
        dict: &Map<String, Value>,
        key: &str,
        default: bool,
    ) -> Result<bool, SaiWsRequestOptionsParseError> {
        match dict.get(key) {
            None => Ok(default),
            Some(Value::Bool(value)) => Ok(*value),
            Some(Value::Number(number)) => Ok(number.as_f64().is_some_and(|value| value != 0.0)),
            Some(_) => Err(Self::type_mismatch(key, "Bool")),
        }
    }

    fn optional_seconds(
        dict: &Map<String, Value>,
        key: &str,
    ) -> Result<Option<f64>, SaiWsRequestOptionsParseError> {
        match dict.get(key) {
            None => Ok(None),
            Some(Value::Number(number)) => number
                .as_f64()
                .map(Some)
                .ok_or_else(|| Self::type_mismatch(key, "Number (seconds)")),
            Some(_) => Err(Self::type_mismatch(key, "Number (seconds)")),
        }
    }

    fn optional_preferred_layer(
        dict: &Map<String, Value>,
        key: &str,
        default: SaiWsCachePreferredLayer,
    ) -> Result<SaiWsCachePreferredLayer, SaiWsRequestOptionsParseError> {
        match dict.get(key) {
            None => Ok(default),
            Some(Value::String(raw)) => SaiWsCachePreferredLayer::from_raw(raw)
                .ok_or_else(|| Self::type_mismatch(key, "memoryFirst or diskFirst")),
            Some(_) => Err(Self::type_mismatch(key, "String")),
        }
    }

    fn optional_disk_cache_policy(
        dict: &Map<String, Value>,
        key: &str,
        default: SaiWsDiskCachePolicy,
    ) -> Result<SaiWsDiskCachePolicy, SaiWsRequestOptionsParseError> {
        let Some(value) = dict.get(key) else {
            return Ok(default);
        };
        value
            .as_str()
            .and_then(SaiWsDiskCachePolicy::from_raw)
            .ok_or_else(|| Self::type_mismatch(key, "preserveExisting, persistent, or memoryOnly"))
    }
}
