use std::any::Any;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_errors::ErrorCode as RiseErrorsCode;
use rise_canon::rise_key::bot as rise_key_bot;
use rise_canon::rise_wire_canon::{STATUS_FAILURE_VALUES, key as wire_key};
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_json_serialization::JSONSerialization;
use rise_i18n::tr;
use serde_json::{Map, Number, Value};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_failure_relay::RiseServerRefusalRelay;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiPayloadPath;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::core::websocket::sai_ws_response_debt_coordinator::SaiWsResponseDebtCoordinator;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_store::SaiHttpTraceStore;
#[cfg(debug_assertions)]
use crate::core::lib::sai::modules::sai_ws::rise_reply_shadow::RiseReplyShadow;
use crate::core::lib::sai::modules::sai_ws::sai_ws_dropped_topic_summary_carry_over::SaiWsDroppedTopicSummaryCarryOver;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_coordinator::SaiWsOptimisticCoordinator;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationMerger, SaiWsPaginationStateStore,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsPendingCacheBinding, SaiWsRequestOptions,
    SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::lib::sai::sai_auth_hooks::{sai_auth_handle_auth_error, sai_auth_hooks, sai_auth_note_authorized_reply};
use crate::core::lib::sai::sai_messages_cache_hooks::sai_messages_cache_notify_did_change;

const LOG_NAME: &str = "WS";
const SAI_DEBUG_TOOLS: bool = cfg!(any(debug_assertions, feature = "sai_debug_tools"));
const UNKNOWN_SERVER_ERROR_DESCRIPTION: &str = "WebSocket response error";
const MESSAGES_CACHE_KEY_PREFIX: &str = "getMessages-";
const DETAIL_INTEGER_LIMIT: f64 = 9e15;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaiWsResponseError {
    pub message: String,
    pub code: Option<String>,
    pub is_transport_drop: bool,
    pub details: HashMap<String, String>,
}

impl SaiWsResponseError {
    pub fn error_description(&self) -> String {
        CanonErrors::text(self.code.as_deref(), &self.details).unwrap_or_else(|| self.message.clone())
    }

    /// technical detail that is never shown on screen
    pub fn failure_reason(&self) -> Option<String> {
        Some(self.message.clone())
    }
}

impl fmt::Display for SaiWsResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description())
    }
}

impl std::error::Error for SaiWsResponseError {}

/// success handler of a pending request, every handler gets its own boxed copy of the payload
pub type SaiWsPendingOnSuccess = Box<dyn Fn(Option<Box<dyn Any>>, &mut App)>;

/// error handler of a pending request
pub type SaiWsPendingOnError = Box<dyn Fn(&anyhow::Error, &mut App)>;

struct Entry {
    on_success_handlers: Vec<SaiWsPendingOnSuccess>,
    on_error_handlers: Vec<SaiWsPendingOnError>,
    cache_binding: Option<SaiWsPendingCacheBinding>,
    take_path: Option<String>,
    sai_slice: Option<Rc<dyn SaiSliceState>>,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    wire: Option<(String, String)>,
}

impl Entry {
    fn is_current_account(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(
            self.account_user_id.as_deref(),
            self.account_epoch.as_deref(),
        )
    }

    fn fail_handlers(&self, error: &anyhow::Error, cx: &mut App) {
        for handler in &self.on_error_handlers {
            handler(error, cx);
        }
    }

    fn mark_rejected_if_needed(&self, error: &anyhow::Error) {
        if SaiWsPendingRegistry::should_reject_slice(self.cache_binding.as_ref())
            && let Some(slice) = &self.sai_slice
        {
            slice.mark_rejected(error);
        }
    }

    /// hands the slice callback payload or the raw path value to every success handler
    fn succeed_handlers(&self, raw: Option<&Value>, cx: &mut App) {
        let take_path = self.take_path.as_deref();
        for handler in &self.on_success_handlers {
            let payload = self
                .sai_slice
                .as_ref()
                .and_then(|slice| slice.callback_payload(raw, take_path))
                .or_else(|| {
                    SaiPayloadPath::extract(raw, take_path)
                        .map(|value| Box::new(value.clone()) as Box<dyn Any>)
                });
            handler(payload, cx);
        }
    }
}

/// registry of pending requests as an entity
/// only the in flight map is observed so only its changes notify
#[derive(Default)]
pub struct SaiWsPendingRegistry {
    pending: HashMap<String, Entry>,
    in_flight_by_cache_key: HashMap<String, u64>,
}

impl SaiWsPendingRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::default())
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.pending.clear();
        self.in_flight_by_cache_key.clear();
        cx.notify();
        SaiWsPaginationStateStore::reset_all(cx);
    }

    pub fn test_pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn test_in_flight_count(&self) -> usize {
        self.in_flight_by_cache_key.len()
    }

    pub fn has_pending(&self, request_id_millis: u64) -> bool {
        self.pending.contains_key(&Self::wire_id_key(request_id_millis))
    }

    pub fn test_register_pending(
        &mut self,
        request_id_millis: u64,
        cache_key: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let binding = cache_key.map(|cache_key| {
            SaiWsPendingCacheBinding::new(&SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
                service: "_".to_owned(),
                method: "_".to_owned(),
                cache_key: Some(cache_key),
                ..Default::default()
            }))
        });
        let account_user_id = binding.as_ref().and_then(|binding| binding.account_user_id.clone());
        let account_epoch = binding.as_ref().and_then(|binding| binding.account_epoch.clone());
        self.register(
            request_id_millis,
            binding,
            None,
            None,
            account_user_id,
            account_epoch,
            None,
            Some(Box::new(|_: Option<Box<dyn Any>>, _: &mut App| {})),
            Some(Box::new(|_: &anyhow::Error, _: &mut App| {})),
            cx,
        );
    }

    pub fn wire_id_key(id_millis: u64) -> String {
        id_millis.to_string()
    }

    pub fn request_id_string(value: Option<&Value>) -> Option<String> {
        match value? {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(Self::ns_number_string_value(number)),
            Value::Bool(flag) => Some(String::from(if *flag { "1" } else { "0" })),
            Value::Null => None,
            other @ (Value::Array(_) | Value::Object(_)) => Some(other.to_string()),
        }
    }

    pub fn in_flight_request_id(&self, cache_key: &str) -> Option<u64> {
        self.in_flight_by_cache_key.get(cache_key).copied()
    }

    pub fn has_in_flight_request(&self, cache_key: Option<&str>) -> bool {
        match cache_key {
            Some(cache_key) if !cache_key.is_empty() => {
                self.in_flight_by_cache_key.contains_key(cache_key)
            }
            _ => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &mut self,
        request_id_millis: u64,
        cache_binding: Option<SaiWsPendingCacheBinding>,
        take_path: Option<String>,
        sai_slice: Option<Rc<dyn SaiSliceState>>,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        wire: Option<(String, String)>,
        on_success: Option<SaiWsPendingOnSuccess>,
        on_error: Option<SaiWsPendingOnError>,
        cx: &mut Context<Self>,
    ) {
        let key = Self::wire_id_key(request_id_millis);
        if let Some(cache_key) = cache_binding.as_ref().and_then(|binding| binding.cache_key.as_ref())
            && !cache_key.is_empty()
        {
            self.in_flight_by_cache_key.insert(cache_key.clone(), request_id_millis);
            cx.notify();
        }
        SaiWsPaginationStateStore::begin(cache_binding.as_ref(), cx);
        self.pending.insert(
            key,
            Entry {
                on_success_handlers: on_success.into_iter().collect(),
                on_error_handlers: on_error.into_iter().collect(),
                cache_binding,
                take_path,
                sai_slice,
                account_user_id,
                account_epoch,
                wire,
            },
        );
    }

    pub fn attach_handlers(
        &mut self,
        request_id_millis: u64,
        on_success: Option<SaiWsPendingOnSuccess>,
        on_error: Option<SaiWsPendingOnError>,
    ) -> bool {
        let Some(entry) = self.pending.get_mut(&Self::wire_id_key(request_id_millis)) else {
            return false;
        };
        entry.on_success_handlers.extend(on_success);
        entry.on_error_handlers.extend(on_error);
        true
    }

    pub fn remove_pending(&mut self, request_id_millis: u64, cx: &mut Context<Self>) {
        let key = Self::wire_id_key(request_id_millis);
        if let Some(entry) = self.pending.remove(&key) {
            self.clear_in_flight_if_needed(entry.cache_binding.as_ref(), &key, cx);
        }
    }

    pub fn fail_pending(
        &mut self,
        request_id_millis: u64,
        error: &anyhow::Error,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = Self::wire_id_key(request_id_millis);
        let Some(entry) = self.pending.remove(&key) else { return false };
        self.clear_in_flight_if_needed(entry.cache_binding.as_ref(), &key, cx);

        if !entry.is_current_account() {
            Self::ignore_stale_account_response(key, cx);
            return true;
        }

        if SAI_DEBUG_TOOLS {
            let message = error.to_string();
            SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
                store.fail_ws_trace_if_linked(&key, &message, cx);
            });
        }

        SaiWsOptimisticCoordinator::handle_failure(
            request_id_millis,
            &**error,
            entry.sai_slice.clone(),
            entry.cache_binding.as_ref(),
            cx,
        );
        SaiWsPaginationStateStore::fail(entry.cache_binding.as_ref(), error, cx);
        entry.mark_rejected_if_needed(error);
        entry.fail_handlers(error, cx);
        true
    }

    pub fn reset_after_socket_drop(&mut self, cx: &mut Context<Self>) {
        let debt = SaiWsResponseDebtCoordinator::shared(cx);
        let mut preserve: HashSet<u64> = debt.read(cx).tracked_wire_ids();
        preserve.extend(SaiWsOptimisticCoordinator::pre_wire_optimistic_wire_ids(cx));

        let (dropped, kept): (Vec<(String, Entry)>, Vec<(String, Entry)>) =
            std::mem::take(&mut self.pending).into_iter().partition(|(key, _)| {
                key.parse::<u64>().ok().is_none_or(|id| !preserve.contains(&id))
            });

        for (_, entry) in &dropped {
            SaiWsPaginationStateStore::cancel(entry.cache_binding.as_ref(), cx);
        }

        if preserve.is_empty() {
            self.in_flight_by_cache_key.clear();
        } else {
            self.pending = kept.into_iter().collect();
            self.in_flight_by_cache_key.retain(|_, id| preserve.contains(id));
        }
        cx.notify();

        SaiWsOptimisticCoordinator::reset_after_socket_drop(&preserve, cx);

        let error = anyhow::Error::new(SaiWsResponseError {
            message: tr("error_connection_lost"),
            is_transport_drop: true,
            ..SaiWsResponseError::default()
        });
        for (key, entry) in dropped {
            Self::fail_dropped_entry(&key, &entry, &error, cx);
        }
    }

    fn fail_dropped_entry(key: &str, entry: &Entry, error: &anyhow::Error, cx: &mut App) {
        if !entry.is_current_account() {
            Self::acknowledge_debt(key, cx);
            return;
        }
        if SAI_DEBUG_TOOLS {
            let message = error
                .downcast_ref::<SaiWsResponseError>()
                .map_or_else(|| error.to_string(), |error| error.message.clone());
            SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
                store.fail_ws_trace_if_linked(key, &message, cx);
            });
        }
        entry.mark_rejected_if_needed(error);
        entry.fail_handlers(error, cx);
    }

    pub fn handle_incoming_json_object(
        &mut self,
        json: &Map<String, Value>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(key) = Self::request_id_string(json.get(rise_key_bot::REQUEST_ID)) else {
            return false;
        };
        let Some(entry) = self.pending.remove(&key) else { return false };
        self.clear_in_flight_if_needed(entry.cache_binding.as_ref(), &key, cx);

        if !entry.is_current_account() {
            Self::ignore_stale_account_response(key, cx);
            return true;
        }

        if SAI_DEBUG_TOOLS {
            let server_error = Self::is_server_error_frame(json);
            SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
                store.complete_ws_trace_if_linked(&key, json, server_error, cx);
            });
        }

        if Self::is_server_error_frame(json) {
            Self::fail_with_server_error(&key, &entry, json, cx);
            Self::acknowledge_debt(&key, cx);
            return true;
        }
        sai_auth_note_authorized_reply(cx);

        let payload = json.get(wire_key::DATA).or_else(|| json.get(wire_key::RESULT));
        let value = payload.filter(|payload| !payload.is_null()).cloned();
        #[cfg(debug_assertions)]
        if let Some((service, method)) = &entry.wire {
            RiseReplyShadow::check(service, method, value.as_ref(), cx);
        }
        match entry.cache_binding.as_ref() {
            Some(binding) => {
                if !Self::fulfill_with_cache(&key, &entry, binding, value, cx) {
                    Self::acknowledge_debt(&key, cx);
                    return true;
                }
            }
            None => {
                if let Some(slice) = &entry.sai_slice {
                    slice.mark_fulfilled(value.as_ref(), entry.take_path.as_deref());
                }
                entry.succeed_handlers(value.as_ref(), cx);
            }
        }
        Self::acknowledge_debt(&key, cx);
        true
    }

    fn fail_with_server_error(
        key: &str,
        entry: &Entry,
        json: &Map<String, Value>,
        cx: &mut App,
    ) {
        let message = Self::server_error_description(json);
        let code = Self::server_error_code(json);
        let response_error = SaiWsResponseError {
            message,
            code,
            is_transport_drop: false,
            details: Self::server_error_details(json),
        };
        if let Some(hooks) = sai_auth_hooks()
            && (hooks.is_auth_error)(
                response_error.code.as_deref(),
                Some(response_error.message.as_str()),
            )
        {
            sai_auth_handle_auth_error(cx);
        }
        if response_error.code.as_deref() == Some(RiseErrorsCode::AuthAccountBlocked.wire())
            && let Some(hooks) = sai_auth_hooks()
        {
            (hooks.handle_account_blocked)(cx);
        }
        RiseServerRefusalRelay::publish(response_error.canon_failure(), cx);
        if let Ok(wire_id) = key.parse::<u64>() {
            SaiWsOptimisticCoordinator::handle_failure(
                wire_id,
                &response_error,
                entry.sai_slice.clone(),
                entry.cache_binding.as_ref(),
                cx,
            );
        }
        let error = anyhow::Error::new(response_error);
        SaiWsPaginationStateStore::fail(entry.cache_binding.as_ref(), &error, cx);
        entry.mark_rejected_if_needed(&error);
        entry.fail_handlers(&error, cx);
    }

    fn fulfill_with_cache(
        key: &str,
        entry: &Entry,
        binding: &SaiWsPendingCacheBinding,
        value: Option<Value>,
        cx: &mut App,
    ) -> bool {
        let (used_optimistic_insert_merge, stored) = match key.parse::<u64>() {
            Ok(wire_id) => {
                let used = SaiWsOptimisticCoordinator::has_insert_session(wire_id, cx);
                let merged = SaiWsOptimisticCoordinator::merged_payload_for_cache_if_needed(
                    wire_id,
                    value.clone(),
                    binding,
                    cx,
                );
                (used, merged.or_else(|| value.clone()))
            }
            Err(_) => (false, value.clone()),
        };
        let cache_payload = match &binding.pagination {
            Some(pagination) => {
                let existing = binding.cache_key.as_deref().and_then(|cache_key| {
                    SaiWsResponseCache::current_payload(
                        cache_key,
                        SaiWsCachePreferredLayer::MemoryFirst,
                        binding.disk_cache_policy,
                        cx,
                    )
                });
                let merged = SaiWsPaginationMerger::merge(
                    existing.as_ref(),
                    stored.as_ref(),
                    pagination,
                    binding.path_to_array.as_deref().unwrap_or_default(),
                    binding.path_to_id.as_deref(),
                );
                match merged {
                    Ok(result) => {
                        SaiWsPaginationStateStore::finish(Some(binding), &result, cx);
                        Some(Value::Object(result.root))
                    }
                    Err(merge_error) => {
                        let error = anyhow::Error::new(merge_error);
                        SaiWsPaginationStateStore::fail(Some(binding), &error, cx);
                        entry.mark_rejected_if_needed(&error);
                        entry.fail_handlers(&error, cx);
                        return false;
                    }
                }
            }
            None => SaiWsDroppedTopicSummaryCarryOver::payload(
                stored.clone(),
                binding.cache_key.as_deref(),
                cx,
            ),
        };
        SaiWsResponseCache::store_success_payload_binding(cache_payload, binding, cx);
        let resolved = match binding.cache_key.as_deref().filter(|cache_key| !cache_key.is_empty()) {
            Some(cache_key) => SaiWsResponseCache::memory_only(cache_key, cx).or_else(|| {
                SaiWsResponseCache::current_payload(
                    cache_key,
                    SaiWsCachePreferredLayer::MemoryFirst,
                    binding.disk_cache_policy,
                    cx,
                )
            }),
            None => stored,
        };
        let fulfilled = resolved.filter(|resolved| !resolved.is_null());
        if let Some(slice) = &entry.sai_slice {
            slice.mark_fulfilled(fulfilled.as_ref(), entry.take_path.as_deref());
        }
        if used_optimistic_insert_merge
            && let Some(cache_key) = binding.cache_key.as_deref()
            && SaiAccountCacheScope::logical_key(cache_key).starts_with(MESSAGES_CACHE_KEY_PREFIX)
        {
            sai_messages_cache_notify_did_change(cache_key, cx);
        }
        entry.succeed_handlers(value.as_ref().or(fulfilled.as_ref()), cx);
        true
    }

    fn ignore_stale_account_response(key: String, cx: &mut App) {
        Self::acknowledge_debt(&key, cx);
        Sai::log(
            LOG_NAME,
            move || format!("ignored stale account response request_id={key}"),
            SaiLogKind::Cache,
            cx,
        );
    }

    fn acknowledge_debt(key: &str, cx: &mut App) {
        SaiWsResponseDebtCoordinator::shared(cx)
            .update(cx, |debt, cx| debt.on_wire_acknowledged(key, cx));
    }

    fn clear_in_flight_if_needed(
        &mut self,
        binding: Option<&SaiWsPendingCacheBinding>,
        wire_id_key: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(cache_key) = binding.and_then(|binding| binding.cache_key.as_deref()) else {
            return;
        };
        if cache_key.is_empty() {
            return;
        }
        let Ok(request_id) = wire_id_key.parse::<u64>() else { return };
        if self.in_flight_by_cache_key.get(cache_key) == Some(&request_id) {
            self.in_flight_by_cache_key.remove(cache_key);
            cx.notify();
        }
    }

    fn should_reject_slice(binding: Option<&SaiWsPendingCacheBinding>) -> bool {
        binding
            .and_then(|binding| binding.pagination.as_ref())
            .is_none_or(|pagination| pagination.is_initial_page())
    }

    fn is_server_error_frame(json: &Map<String, Value>) -> bool {
        if let Some(status) = json.get(wire_key::STATUS).and_then(Value::as_str)
            && STATUS_FAILURE_VALUES
                .iter()
                .any(|failure| compare_case_insensitive(failure, status) == Ordering::Equal)
        {
            return true;
        }
        Self::carries_remote_failure(json.get(wire_key::ERROR))
    }

    fn carries_remote_failure(value: Option<&Value>) -> bool {
        match value {
            None | Some(Value::Null) => false,
            Some(Value::String(text)) => !text.is_empty(),
            Some(Value::Object(object)) => !object.is_empty(),
            Some(_) => true,
        }
    }

    fn server_error_description(json: &Map<String, Value>) -> String {
        match json.get(wire_key::ERROR) {
            Some(Value::String(text)) if !text.is_empty() => text.clone(),
            Some(Value::Object(error)) => Self::non_empty_string(error, wire_key::MESSAGE)
                .or_else(|| Self::non_empty_string(error, wire_key::CODE))
                .unwrap_or_else(|| Value::Object(error.clone()).to_string()),
            _ => UNKNOWN_SERVER_ERROR_DESCRIPTION.to_owned(),
        }
    }

    fn server_error_code(json: &Map<String, Value>) -> Option<String> {
        json.get(wire_key::ERROR)
            .and_then(Value::as_object)
            .and_then(|error| Self::non_empty_string(error, wire_key::CODE))
            .or_else(|| Self::non_empty_string(json, wire_key::CODE))
            .or_else(|| Self::non_empty_string(json, rise_key_bot::ERROR_CODE))
    }

    fn non_empty_string(container: &Map<String, Value>, key: &str) -> Option<String> {
        container
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    }

    fn server_error_details(json: &Map<String, Value>) -> HashMap<String, String> {
        let mut details = HashMap::new();
        let nested = json.get(wire_key::ERROR).and_then(Value::as_object);
        for container in [Some(json), nested].into_iter().flatten() {
            for (key, value) in Self::details_object(container.get(wire_key::DETAILS)) {
                if let Some(text) = Self::detail_value(&value) {
                    details.insert(key, text);
                }
            }
        }
        details
    }

    fn details_object(value: Option<&Value>) -> Map<String, Value> {
        match value {
            Some(Value::Object(object)) => object.clone(),
            Some(Value::String(text)) => match JSONSerialization::json_object(text.as_bytes()) {
                Some(Value::Object(object)) => object,
                _ => Map::new(),
            },
            _ => Map::new(),
        }
    }

    fn detail_value(value: &Value) -> Option<String> {
        match value {
            Value::String(text) => (!text.is_empty()).then(|| text.clone()),
            Value::Number(number) => {
                if let Some(double) = number.as_f64()
                    && double == double.round()
                    && double.abs() < DETAIL_INTEGER_LIMIT
                {
                    return Some(format!("{double:.0}"));
                }
                Some(Self::ns_number_string_value(number))
            }
            Value::Bool(flag) => Some(String::from(if *flag { "1" } else { "0" })),
            Value::Null | Value::Array(_) | Value::Object(_) => None,
        }
    }

    fn ns_number_string_value(number: &Number) -> String {
        if number.is_f64()
            && let Some(value) = number.as_f64()
            && value.is_finite()
            && value.fract() == 0.0
            && value.abs() < 1e15
        {
            return format!("{value:.0}");
        }
        number.to_string()
    }
}
