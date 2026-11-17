use std::collections::HashMap;
use std::fmt;

use gpui::{App, AppContext, Context, Entity, Global};
use rise_canon::rise_key::bot as rise_key_bot;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Number, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsPendingCacheBinding;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiWsPaginationEdge {
    Leading,
    Trailing,
}

impl SaiWsPaginationEdge {
    pub const ALL_CASES: [Self; 2] = [Self::Leading, Self::Trailing];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Leading => "leading",
            Self::Trailing => "trailing",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SaiWsPaginationMergeDirection {
    #[default]
    Append,
    Prepend,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SaiWsPaginationDuplicatePolicy {
    #[default]
    KeepExisting,
    UseIncoming,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaiWsPaginationCursorFallback {
    FirstItemId,
    LastItemId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaiWsPaginationPageKind {
    Initial,
    Continuation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaiWsPaginationFailureKind {
    Transport,
    ProtocolViolation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiWsPaginationOptions {
    pub page_kind: SaiWsPaginationPageKind,
    pub edge: SaiWsPaginationEdge,
    pub requested_relative_id: Option<String>,
    pub response_relative_id_path: String,
    pub response_relative_id_fallback_path: Option<String>,
    pub response_relative_id_fallback: Option<SaiWsPaginationCursorFallback>,
    pub has_more_path: Option<String>,
    pub merge_direction: SaiWsPaginationMergeDirection,
    pub duplicate_policy: SaiWsPaginationDuplicatePolicy,
}

/// arguments for the relative id pagination option, with defaults
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiWsPaginationRelativeIdArgs {
    pub edge: SaiWsPaginationEdge,
    pub relative_id: Option<String>,
    pub page_kind: Option<SaiWsPaginationPageKind>,
    pub response_relative_id_path: String,
    pub response_relative_id_fallback_path: Option<String>,
    pub response_relative_id_fallback: Option<SaiWsPaginationCursorFallback>,
    pub has_more_path: Option<String>,
    pub merge_direction: SaiWsPaginationMergeDirection,
    pub duplicate_policy: SaiWsPaginationDuplicatePolicy,
}

impl Default for SaiWsPaginationRelativeIdArgs {
    fn default() -> Self {
        Self {
            edge: SaiWsPaginationEdge::Trailing,
            relative_id: None,
            page_kind: None,
            response_relative_id_path: rise_key_bot::RELATIVE_ID.to_owned(),
            response_relative_id_fallback_path: None,
            response_relative_id_fallback: None,
            has_more_path: Some(rise_key_bot::HAS_MORE.to_owned()),
            merge_direction: SaiWsPaginationMergeDirection::Append,
            duplicate_policy: SaiWsPaginationDuplicatePolicy::KeepExisting,
        }
    }
}

impl From<Option<&str>> for SaiWsPaginationRelativeIdArgs {
    fn from(relative_id: Option<&str>) -> Self {
        Self {
            relative_id: relative_id.map(str::to_owned),
            ..Self::default()
        }
    }
}

impl SaiWsPaginationOptions {
    pub fn is_initial_page(&self) -> bool {
        self.page_kind == SaiWsPaginationPageKind::Initial
    }

    pub fn relative_id(args: impl Into<SaiWsPaginationRelativeIdArgs>) -> Self {
        let args = args.into();
        let relative_id = Self::normalized(args.relative_id.as_deref());
        let page_kind = args.page_kind.unwrap_or(if relative_id.is_none() {
            SaiWsPaginationPageKind::Initial
        } else {
            SaiWsPaginationPageKind::Continuation
        });
        Self {
            page_kind,
            edge: args.edge,
            requested_relative_id: relative_id,
            response_relative_id_path: args.response_relative_id_path,
            response_relative_id_fallback_path: Self::normalized(
                args.response_relative_id_fallback_path.as_deref(),
            ),
            response_relative_id_fallback: args.response_relative_id_fallback,
            has_more_path: Self::normalized(args.has_more_path.as_deref()),
            merge_direction: args.merge_direction,
            duplicate_policy: args.duplicate_policy,
        }
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        (!value.is_empty()).then(|| value.to_owned())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaiWsPaginationEdgeState {
    pub is_loading: bool,
    pub error: Option<String>,
    pub failure_kind: Option<SaiWsPaginationFailureKind>,
}

impl SaiWsPaginationEdgeState {
    pub fn can_request_automatically(&self) -> bool {
        !self.is_loading && self.error.is_none()
    }
}

/// pagination state as an entity, every change notifies observers
#[derive(Default)]
pub struct SaiWsPaginationState {
    is_initial_page_loading: bool,
    leading: SaiWsPaginationEdgeState,
    trailing: SaiWsPaginationEdgeState,
    head_generation: u64,
}

impl SaiWsPaginationState {
    pub fn is_initial_page_loading(&self) -> bool {
        self.is_initial_page_loading
    }

    pub fn leading(&self) -> &SaiWsPaginationEdgeState {
        &self.leading
    }

    pub fn trailing(&self) -> &SaiWsPaginationEdgeState {
        &self.trailing
    }

    pub fn head_generation(&self) -> u64 {
        self.head_generation
    }

    pub fn state(&self, edge: SaiWsPaginationEdge) -> &SaiWsPaginationEdgeState {
        match edge {
            SaiWsPaginationEdge::Leading => &self.leading,
            SaiWsPaginationEdge::Trailing => &self.trailing,
        }
    }

    pub fn can_request_automatically(&self, edge: SaiWsPaginationEdge) -> bool {
        !self.is_initial_page_loading && self.state(edge).can_request_automatically()
    }

    pub fn clear_error(&mut self, edge: SaiWsPaginationEdge, cx: &mut Context<Self>) {
        let state = self.edge_mut(edge);
        state.error = None;
        state.failure_kind = None;
        cx.notify();
    }

    fn begin(&mut self, pagination: &SaiWsPaginationOptions, cx: &mut Context<Self>) {
        if pagination.is_initial_page() {
            self.is_initial_page_loading = true;
        } else {
            let state = self.edge_mut(pagination.edge);
            state.is_loading = true;
            state.error = None;
            state.failure_kind = None;
        }
        cx.notify();
    }

    fn finish(
        &mut self,
        pagination: &SaiWsPaginationOptions,
        result: &SaiWsPaginationMergeResult,
        cx: &mut Context<Self>,
    ) {
        let failure_kind = result
            .protocol_error
            .as_ref()
            .map(|_| SaiWsPaginationFailureKind::ProtocolViolation);
        if pagination.is_initial_page() {
            self.is_initial_page_loading = false;
            self.head_generation = self.head_generation.wrapping_add(1);
        }
        let initial = pagination.is_initial_page();
        let state = self.edge_mut(pagination.edge);
        if !initial {
            state.is_loading = false;
        }
        state.error = result.protocol_error.clone();
        state.failure_kind = failure_kind;
        cx.notify();
    }

    fn fail(
        &mut self,
        pagination: &SaiWsPaginationOptions,
        error: &anyhow::Error,
        failure_kind: SaiWsPaginationFailureKind,
        cx: &mut Context<Self>,
    ) {
        if pagination.is_initial_page() {
            self.is_initial_page_loading = false;
        } else {
            let state = self.edge_mut(pagination.edge);
            state.is_loading = false;
            state.error = Some(CanonErrors::refusal_text(error));
            state.failure_kind = Some(failure_kind);
        }
        cx.notify();
    }

    fn cancel(&mut self, pagination: &SaiWsPaginationOptions, cx: &mut Context<Self>) {
        if pagination.is_initial_page() {
            self.is_initial_page_loading = false;
        } else {
            self.edge_mut(pagination.edge).is_loading = false;
        }
        cx.notify();
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.is_initial_page_loading = false;
        self.leading = SaiWsPaginationEdgeState::default();
        self.trailing = SaiWsPaginationEdgeState::default();
        cx.notify();
    }

    fn edge_mut(&mut self, edge: SaiWsPaginationEdge) -> &mut SaiWsPaginationEdgeState {
        match edge {
            SaiWsPaginationEdge::Leading => &mut self.leading,
            SaiWsPaginationEdge::Trailing => &mut self.trailing,
        }
    }
}

/// pagination states keyed by the account scoped cache key
#[derive(Default)]
struct SaiWsPaginationStates {
    states: HashMap<String, Entity<SaiWsPaginationState>>,
}

impl Global for SaiWsPaginationStates {}

pub enum SaiWsPaginationStateStore {}

impl SaiWsPaginationStateStore {
    pub fn state(cache_key: &str, cx: &mut App) -> Entity<SaiWsPaginationState> {
        let key = SaiAccountCacheScope::scoped_key(cache_key);
        if let Some(state) = cx
            .try_global::<SaiWsPaginationStates>()
            .and_then(|store| store.states.get(&key))
        {
            return state.clone();
        }
        let state = cx.new(|_| SaiWsPaginationState::default());
        cx.default_global::<SaiWsPaginationStates>()
            .states
            .insert(key, state.clone());
        state
    }

    pub fn begin(binding: Option<&SaiWsPendingCacheBinding>, cx: &mut App) {
        let Some((cache_key, pagination)) = Self::paginated(binding) else { return };
        Self::state(cache_key, cx).update(cx, |state, cx| state.begin(pagination, cx));
    }

    pub fn finish(
        binding: Option<&SaiWsPendingCacheBinding>,
        result: &SaiWsPaginationMergeResult,
        cx: &mut App,
    ) {
        let Some((cache_key, pagination)) = Self::paginated(binding) else { return };
        Self::state(cache_key, cx).update(cx, |state, cx| state.finish(pagination, result, cx));
    }

    pub fn fail(binding: Option<&SaiWsPendingCacheBinding>, error: &anyhow::Error, cx: &mut App) {
        let Some((cache_key, pagination)) = Self::paginated(binding) else { return };
        let failure_kind = if error.downcast_ref::<SaiWsPaginationError>().is_some() {
            SaiWsPaginationFailureKind::ProtocolViolation
        } else {
            SaiWsPaginationFailureKind::Transport
        };
        Self::state(cache_key, cx)
            .update(cx, |state, cx| state.fail(pagination, error, failure_kind, cx));
    }

    pub fn cancel(binding: Option<&SaiWsPendingCacheBinding>, cx: &mut App) {
        let Some((cache_key, pagination)) = Self::paginated(binding) else { return };
        Self::state(cache_key, cx).update(cx, |state, cx| state.cancel(pagination, cx));
    }

    pub fn remove(cache_key: &str, cx: &mut App) {
        let key = SaiAccountCacheScope::scoped_key(cache_key);
        let removed = cx
            .default_global::<SaiWsPaginationStates>()
            .states
            .remove(&key);
        if let Some(state) = removed {
            state.update(cx, |state, cx| state.reset(cx));
        }
    }

    pub fn reset_all(cx: &mut App) {
        let states = std::mem::take(&mut cx.default_global::<SaiWsPaginationStates>().states);
        for state in states.into_values() {
            state.update(cx, |state, cx| state.reset(cx));
        }
    }

    fn paginated(
        binding: Option<&SaiWsPendingCacheBinding>,
    ) -> Option<(&str, &SaiWsPaginationOptions)> {
        let binding = binding?;
        Some((binding.cache_key.as_deref()?, binding.pagination.as_ref()?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaiWsPaginationError {
    CacheKeyRequired,
    CacheStorageRequired,
    InvalidItemsPath(String),
    InvalidRelativeIdPath(String),
    InvalidHasMorePath(String),
    ResponseIsNotObject,
    ItemsAreNotArray(String),
    ConcurrentRequestsNotSupported,
}

impl SaiWsPaginationError {
    pub fn error_description(&self) -> String {
        match self {
            Self::CacheKeyRequired => "SaiWs pagination requires a cacheKey.".to_owned(),
            Self::CacheStorageRequired => {
                "SaiWs pagination requires storeResponseInCache=true.".to_owned()
            }
            Self::InvalidItemsPath(path) => format!("SaiWs pagination itemsPath is invalid: {path}"),
            Self::InvalidRelativeIdPath(path) => {
                format!("SaiWs pagination relative-id path is invalid: {path}")
            }
            Self::InvalidHasMorePath(path) => {
                format!("SaiWs pagination has-more path is invalid: {path}")
            }
            Self::ResponseIsNotObject => {
                "SaiWs pagination response must be a JSON object.".to_owned()
            }
            Self::ItemsAreNotArray(path) => {
                format!("SaiWs pagination response value is not an array at: {path}")
            }
            Self::ConcurrentRequestsNotSupported => {
                "SaiWs pagination allows one in-flight page per cacheKey.".to_owned()
            }
        }
    }
}

impl fmt::Display for SaiWsPaginationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description())
    }
}

impl std::error::Error for SaiWsPaginationError {}

#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsPaginationMergeResult {
    pub root: Map<String, Value>,
    pub has_more: bool,
    pub relative_id: Option<String>,
    pub received_count: i64,
    pub inserted_count: i64,
    pub protocol_error: Option<String>,
}

const PROTOCOL_ERROR_MISSING_RELATIVE_ID: &str =
    "Pagination returned has_more=true without relative_id.";
const PROTOCOL_ERROR_RELATIVE_ID_DID_NOT_ADVANCE: &str = "Pagination relative_id did not advance.";

pub enum SaiWsPaginationMerger {}

impl SaiWsPaginationMerger {
    pub fn merge(
        existing_payload: Option<&Value>,
        page_payload: Option<&Value>,
        options: &SaiWsPaginationOptions,
        items_path: &str,
        item_id_path: Option<&str>,
    ) -> Result<SaiWsPaginationMergeResult, SaiWsPaginationError> {
        let items_path_parts = Self::path_parts(items_path);
        if items_path_parts.is_empty() {
            return Err(SaiWsPaginationError::InvalidItemsPath(items_path.to_owned()));
        }
        let mut page_root = AnyDictionaryBridge::dictionary(page_payload)
            .ok_or(SaiWsPaginationError::ResponseIsNotObject)?;
        let page_items = Self::array(Self::value_in_map(&page_root, &items_path_parts))
            .cloned()
            .ok_or_else(|| SaiWsPaginationError::ItemsAreNotArray(items_path.to_owned()))?;

        let relative_id_path_parts = Self::path_parts(&options.response_relative_id_path);
        if relative_id_path_parts.is_empty() {
            return Err(SaiWsPaginationError::InvalidRelativeIdPath(
                options.response_relative_id_path.clone(),
            ));
        }
        let fallback_relative_id_path_parts = match &options.response_relative_id_fallback_path {
            Some(fallback_path) => {
                let parts = Self::path_parts(fallback_path);
                if parts.is_empty() {
                    return Err(SaiWsPaginationError::InvalidRelativeIdPath(fallback_path.clone()));
                }
                Some(parts)
            }
            None => None,
        };
        let primary_relative_id_value =
            Self::value_in_map(&page_root, &relative_id_path_parts).cloned();
        let primary_relative_id = Self::scalar_string(primary_relative_id_value.as_ref());
        let alternate_relative_id_value = if primary_relative_id.is_none() {
            fallback_relative_id_path_parts
                .as_ref()
                .and_then(|parts| Self::value_in_map(&page_root, parts))
                .cloned()
        } else {
            None
        };
        let alternate_relative_id = Self::scalar_string(alternate_relative_id_value.as_ref());
        let used_alternate_response_path =
            primary_relative_id.is_none() && alternate_relative_id.is_some();
        let response_relative_id_value = if primary_relative_id.is_none() {
            alternate_relative_id_value
        } else {
            primary_relative_id_value
        };
        let response_relative_id = primary_relative_id.or(alternate_relative_id);
        let has_more_path_parts = match &options.has_more_path {
            Some(has_more_path) => {
                let parts = Self::path_parts(has_more_path);
                if parts.is_empty() {
                    return Err(SaiWsPaginationError::InvalidHasMorePath(has_more_path.clone()));
                }
                Some(parts)
            }
            None => None,
        };
        let explicit_has_more = has_more_path_parts
            .as_ref()
            .and_then(|parts| Self::bool_value(Self::value_in_map(&page_root, parts)));
        let fallback_relative_id_value =
            if response_relative_id.is_none() && explicit_has_more != Some(false) {
                Self::relative_id_fallback_value(
                    &page_items,
                    item_id_path,
                    options.response_relative_id_fallback,
                )
            } else {
                None
            };
        let relative_id_value = if explicit_has_more == Some(false) {
            None
        } else if response_relative_id.is_none() {
            fallback_relative_id_value.clone()
        } else {
            response_relative_id_value
        };
        let relative_id = Self::scalar_string(relative_id_value.as_ref());
        if response_relative_id.is_none()
            && let Some(fallback_relative_id_value) = fallback_relative_id_value
        {
            Self::write(fallback_relative_id_value, &mut page_root, &relative_id_path_parts);
        }
        let has_more = explicit_has_more.unwrap_or(relative_id.is_some());
        if let Some(has_more_path_parts) = &has_more_path_parts {
            Self::write(Value::Bool(has_more), &mut page_root, has_more_path_parts);
        }
        let protocol_error = Self::protocol_error(
            has_more,
            relative_id.as_deref(),
            options.requested_relative_id.as_deref(),
        );
        let received_count = Self::count(page_items.len());

        if options.is_initial_page() {
            let mut empty = Vec::new();
            let unique_items = Self::merge_incoming_items(
                &page_items,
                &mut empty,
                item_id_path,
                options.duplicate_policy,
            );
            let inserted_count = Self::count(unique_items.len());
            let mut root = page_root;
            Self::drop_stale_cursor(
                &mut root,
                relative_id.is_none(),
                used_alternate_response_path,
                &relative_id_path_parts,
                fallback_relative_id_path_parts.as_deref(),
            );
            Self::write(Value::Array(unique_items), &mut root, &items_path_parts);
            return Ok(SaiWsPaginationMergeResult {
                root,
                has_more,
                relative_id,
                received_count,
                inserted_count,
                protocol_error,
            });
        }

        let existing_root = AnyDictionaryBridge::dictionary(existing_payload).unwrap_or_default();
        let mut existing_items = Self::normalized_existing_items(
            Self::array(Self::value_in_map(&existing_root, &items_path_parts))
                .cloned()
                .unwrap_or_default(),
            item_id_path,
            options.duplicate_policy,
        );
        let unique_page_items = Self::merge_incoming_items(
            &page_items,
            &mut existing_items,
            item_id_path,
            options.duplicate_policy,
        );
        let inserted_count = Self::count(unique_page_items.len());
        let merged_items = match options.merge_direction {
            SaiWsPaginationMergeDirection::Append => {
                existing_items.extend(unique_page_items);
                existing_items
            }
            SaiWsPaginationMergeDirection::Prepend => {
                let mut merged = unique_page_items;
                merged.extend(existing_items);
                merged
            }
        };

        let mut root = Self::deep_merge(existing_root, page_root);
        Self::drop_stale_cursor(
            &mut root,
            relative_id.is_none(),
            used_alternate_response_path,
            &relative_id_path_parts,
            fallback_relative_id_path_parts.as_deref(),
        );
        Self::write(Value::Array(merged_items), &mut root, &items_path_parts);
        Ok(SaiWsPaginationMergeResult {
            root,
            has_more,
            relative_id,
            received_count,
            inserted_count,
            protocol_error,
        })
    }

    /// shared tail of both branches
    /// no next cursor clears both cursor paths and a
    /// cursor from the fallback path clears the primary one
    fn drop_stale_cursor(
        root: &mut Map<String, Value>,
        has_no_relative_id: bool,
        used_alternate_response_path: bool,
        relative_id_path_parts: &[String],
        fallback_relative_id_path_parts: Option<&[String]>,
    ) {
        if has_no_relative_id {
            Self::remove_value(root, relative_id_path_parts);
            if let Some(fallback_relative_id_path_parts) = fallback_relative_id_path_parts {
                Self::remove_value(root, fallback_relative_id_path_parts);
            }
        } else if used_alternate_response_path {
            Self::remove_value(root, relative_id_path_parts);
        }
    }

    fn protocol_error(
        has_more: bool,
        relative_id: Option<&str>,
        requested_relative_id: Option<&str>,
    ) -> Option<String> {
        if !has_more {
            return None;
        }
        let Some(relative_id) = relative_id else {
            return Some(PROTOCOL_ERROR_MISSING_RELATIVE_ID.to_owned());
        };
        if requested_relative_id == Some(relative_id) {
            return Some(PROTOCOL_ERROR_RELATIVE_ID_DID_NOT_ADVANCE.to_owned());
        }
        None
    }

    fn merge_incoming_items(
        incoming: &[Value],
        existing: &mut [Value],
        id_path: Option<&str>,
        duplicate_policy: SaiWsPaginationDuplicatePolicy,
    ) -> Vec<Value> {
        let Some(id_path) = id_path else { return incoming.to_vec() };
        let id_parts = Self::path_parts(id_path);
        if id_parts.is_empty() {
            return incoming.to_vec();
        }

        let mut existing_index_by_id: HashMap<String, usize> =
            HashMap::with_capacity(existing.len());
        for (index, item) in existing.iter().enumerate() {
            if let Some(id) = Self::scalar_string(Self::value_in(item, &id_parts)) {
                existing_index_by_id.insert(id, index);
            }
        }
        let mut output: Vec<Value> = Vec::with_capacity(incoming.len());
        let mut output_index_by_id: HashMap<String, usize> = HashMap::with_capacity(incoming.len());
        for item in incoming {
            let Some(id) = Self::scalar_string(Self::value_in(item, &id_parts)) else {
                output.push(item.clone());
                continue;
            };
            if let Some(&existing_index) = existing_index_by_id.get(&id) {
                if duplicate_policy == SaiWsPaginationDuplicatePolicy::UseIncoming {
                    existing[existing_index] = item.clone();
                }
            } else if let Some(&output_index) = output_index_by_id.get(&id) {
                if duplicate_policy == SaiWsPaginationDuplicatePolicy::UseIncoming {
                    output[output_index] = item.clone();
                }
            } else {
                output_index_by_id.insert(id, output.len());
                output.push(item.clone());
            }
        }
        output
    }

    fn normalized_existing_items(
        items: Vec<Value>,
        id_path: Option<&str>,
        duplicate_policy: SaiWsPaginationDuplicatePolicy,
    ) -> Vec<Value> {
        let Some(id_path) = id_path else { return items };
        let id_parts = Self::path_parts(id_path);
        if id_parts.is_empty() {
            return items;
        }

        let mut output: Vec<Value> = Vec::with_capacity(items.len());
        let mut index_by_id: HashMap<String, usize> = HashMap::with_capacity(items.len());
        for item in items {
            let Some(id) = Self::scalar_string(Self::value_in(&item, &id_parts)) else {
                output.push(item);
                continue;
            };
            if let Some(&index) = index_by_id.get(&id) {
                if duplicate_policy == SaiWsPaginationDuplicatePolicy::UseIncoming {
                    output[index] = item;
                }
            } else {
                index_by_id.insert(id, output.len());
                output.push(item);
            }
        }
        output
    }

    fn relative_id_fallback_value(
        page_items: &[Value],
        item_id_path: Option<&str>,
        fallback: Option<SaiWsPaginationCursorFallback>,
    ) -> Option<Value> {
        let fallback = fallback?;
        let id_parts = Self::path_parts(item_id_path?);
        if id_parts.is_empty() {
            return None;
        }
        let item = match fallback {
            SaiWsPaginationCursorFallback::FirstItemId => page_items.first(),
            SaiWsPaginationCursorFallback::LastItemId => page_items.last(),
        }?;
        Self::value_in(item, &id_parts).cloned()
    }

    fn deep_merge(
        existing: Map<String, Value>,
        incoming: Map<String, Value>,
    ) -> Map<String, Value> {
        let mut output = existing;
        for (key, incoming_value) in incoming {
            let merged = match (output.remove(&key), incoming_value) {
                (Some(Value::Object(existing_value)), Value::Object(incoming_value))
                    if !existing_value.is_empty() && !incoming_value.is_empty() =>
                {
                    Value::Object(Self::deep_merge(existing_value, incoming_value))
                }
                (_, incoming_value) => incoming_value,
            };
            output.insert(key, merged);
        }
        output
    }

    /// value at a path inside an object root, path is never empty here
    fn value_in_map<'a>(root: &'a Map<String, Value>, path: &[String]) -> Option<&'a Value> {
        let (first, rest) = path.split_first()?;
        Self::value_in(root.get(first)?, rest)
    }

    fn value_in<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
        let mut current = root;
        for component in path {
            current = Self::dictionary(current)?.get(component)?;
        }
        Some(current)
    }

    /// object as a map without copying, an empty object counts as none
    fn dictionary(value: &Value) -> Option<&Map<String, Value>> {
        value.as_object().filter(|object| !object.is_empty())
    }

    fn write(value: Value, root: &mut Map<String, Value>, path: &[String]) {
        let Some((first, rest)) = path.split_first() else { return };
        if rest.is_empty() {
            root.insert(first.clone(), value);
            return;
        }
        let mut nested = match root.remove(first) {
            Some(Value::Object(nested)) => nested,
            _ => Map::new(),
        };
        Self::write(value, &mut nested, rest);
        root.insert(first.clone(), Value::Object(nested));
    }

    fn remove_value(root: &mut Map<String, Value>, path: &[String]) {
        let Some((first, rest)) = path.split_first() else { return };
        if rest.is_empty() {
            root.remove(first);
            return;
        }
        if let Some(Value::Object(nested)) = root.get_mut(first)
            && !nested.is_empty()
        {
            Self::remove_value(nested, rest);
        }
    }

    fn path_parts(path: &str) -> Vec<String> {
        path.split('.')
            .map(trimming_whitespaces_and_newlines)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn array(value: Option<&Value>) -> Option<&Vec<Value>> {
        value?.as_array()
    }

    fn scalar_string(value: Option<&Value>) -> Option<String> {
        let string = match value? {
            Value::String(value) => value.clone(),
            Value::Number(value) => Self::ns_number_string_value(value),
            Value::Bool(value) => String::from(if *value { "1" } else { "0" }),
            Value::Null | Value::Array(_) | Value::Object(_) => return None,
        };
        let normalized = trimming_whitespaces_and_newlines(&string);
        (!normalized.is_empty()).then(|| normalized.to_owned())
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

    fn bool_value(value: Option<&Value>) -> Option<bool> {
        match value? {
            Value::Bool(value) => Some(*value),
            Value::Number(value) => Some(value.as_f64().is_some_and(|value| value != 0.0)),
            Value::String(value) => match value.to_lowercase().as_str() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                _ => None,
            },
            Value::Null | Value::Array(_) | Value::Object(_) => None,
        }
    }

    fn count(len: usize) -> i64 {
        i64::try_from(len).unwrap_or(i64::MAX)
    }
}
