use std::any::Any;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use gpui::{App, Global};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::Value;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_debugger::sai_debug_storage_panel_views::SaiDebugBodyFormat;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_recorder::SaiHttpTraceRecorder;
use crate::core::lib::sai::modules::sai_updater::sai_updater::SaiUpdaterWriteScope;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::{SaiWsDiskCache, SaiWsDiskCacheError};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationStateStore;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsPendingCacheBinding, SaiWsRequestOptions,
    SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;

const MAXIMUM_OBSERVED_PROJECTIONS: usize = 1_024;
const DEFERRED_DISK_PERSISTENCE_PREFIX: &str = "getMessages-";

#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsCachedContext {
    pub cache_key: String,
    pub path_to_array: Option<String>,
    pub path_to_id: Option<String>,
    pub payload: Option<Value>,
}

/// called on the main thread when a cold disk read lands before the wire filled memory
pub type SaiWsColdDiskLoaded = Box<dyn FnOnce(&Value, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaiWsCacheSelectionSource {
    Memory,
    LegacyDisk,
}

struct SaiWsWeakSlice {
    slice: Weak<dyn SaiSliceState>,
    take_path: Option<String>,
}

struct SaiWsObservedProjectionBox {
    cache_key: String,
    revision: i64,
    typed: Box<dyn Any>,
}

struct SaiWsTypedObservedProjectionBox<V> {
    value: V,
}

#[derive(Clone, Default)]
struct SaiWsOptimisticPaths {
    path_to_array: Option<String>,
    path_to_id: Option<String>,
}

#[derive(Default)]
struct SaiWsResponseCacheState {
    memory: HashMap<String, Value>,
    memory_optimistic_paths: HashMap<String, SaiWsOptimisticPaths>,
    slice_by_cache_key: HashMap<String, SaiWsWeakSlice>,
    pending_pagination_disk_payloads: HashMap<String, Value>,
    pagination_disk_writes_in_flight: HashSet<String>,
    pagination_disk_dirty_keys: HashSet<String>,
    pagination_disk_generation_by_key: HashMap<String, u64>,
    next_pagination_disk_generation: u64,
    observed_projections: HashMap<String, SaiWsObservedProjectionBox>,
    strict_memory_only_cache_keys: HashSet<String>,
    legacy_disk_hydrated_memory_keys: HashSet<String>,
    cold_disk_warmups: HashSet<String>,
}

impl Global for SaiWsResponseCacheState {}

impl SaiWsResponseCacheState {
    fn note_legacy_hydrated(&mut self, cache_key: &str, source_was_legacy_disk: bool) {
        if !cfg!(any(debug_assertions, feature = "sai_debug_tools")) {
            return;
        }
        if source_was_legacy_disk {
            self.legacy_disk_hydrated_memory_keys.insert(cache_key.to_owned());
        } else {
            self.legacy_disk_hydrated_memory_keys.remove(cache_key);
        }
    }

    fn pagination_disk_generation(&mut self, cache_key: &str) -> u64 {
        if let Some(generation) = self.pagination_disk_generation_by_key.get(cache_key) {
            return *generation;
        }
        self.next_pagination_disk_generation = self.next_pagination_disk_generation.wrapping_add(1);
        if self.next_pagination_disk_generation == 0 {
            self.next_pagination_disk_generation = 1;
        }
        let generation = self.next_pagination_disk_generation;
        self.pagination_disk_generation_by_key.insert(cache_key.to_owned(), generation);
        generation
    }

    fn invalidate_pagination_disk_persistence(&mut self, cache_key: &str) {
        self.pagination_disk_generation_by_key.remove(cache_key);
        self.pending_pagination_disk_payloads.remove(cache_key);
        self.pagination_disk_writes_in_flight.remove(cache_key);
        self.pagination_disk_dirty_keys.remove(cache_key);
    }

    fn invalidate_all_pagination_disk_persistence(&mut self) {
        self.pagination_disk_generation_by_key.clear();
        self.pending_pagination_disk_payloads.clear();
        self.pagination_disk_writes_in_flight.clear();
        self.pagination_disk_dirty_keys.clear();
    }
}

pub enum SaiWsResponseCache {}

impl SaiWsResponseCache {
    fn state(cx: &mut App) -> &mut SaiWsResponseCacheState {
        cx.default_global::<SaiWsResponseCacheState>()
    }

    fn bump_revision(cache_key: &str, cx: &mut App) {
        SaiCacheRevisionTracker::shared(cx).update(cx, |tracker, cx| tracker.bump(cache_key, cx));
    }

    fn revision(cache_key: &str, cx: &mut App) -> i64 {
        SaiCacheRevisionTracker::observed_revision(cache_key, cx)
    }

    fn trimmed(cache_key: &str) -> String {
        trimming_whitespaces_and_newlines(cache_key).to_owned()
    }

    fn lookup_options(
        cache_key: &str,
        disk_cache_policy: SaiWsDiskCachePolicy,
        preferred_cache_layer: SaiWsCachePreferredLayer,
    ) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: "_".to_owned(),
            method: "_".to_owned(),
            cache_key: Some(cache_key.to_owned()),
            disk_cache_policy: Some(disk_cache_policy),
            preferred_cache_layer,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn prepare_for_request(options: &SaiWsRequestOptions, cx: &mut App) {
        let Some(cache_key) = options.cache_key.as_deref().filter(|key| !key.is_empty()) else {
            return;
        };
        Self::apply_disk_cache_policy(options.disk_cache_policy, cache_key, cx);
    }

    pub fn resolved_value(options: &SaiWsRequestOptions, cx: &mut App) -> Option<Value> {
        let key = options.cache_key.as_deref().filter(|key| !key.is_empty())?;
        if options.disk_cache_policy == SaiWsDiskCachePolicy::MemoryOnly {
            Self::apply_disk_cache_policy(SaiWsDiskCachePolicy::MemoryOnly, key, cx);
        }
        let state = Self::state(cx);
        if state.strict_memory_only_cache_keys.contains(key) {
            return state.memory.get(key).cloned();
        }
        match options.preferred_cache_layer {
            SaiWsCachePreferredLayer::MemoryFirst => {
                if let Some(memory) = state.memory.get(key) {
                    return Some(memory.clone());
                }
                let disk = SaiWsDiskCache::load(key)?;
                Self::hydrate_from_legacy_disk(key, &disk, cx);
                Some(disk)
            }
            SaiWsCachePreferredLayer::DiskFirst => {
                if state.pagination_disk_dirty_keys.contains(key)
                    && let Some(memory) = state.memory.get(key)
                {
                    return Some(memory.clone());
                }
                if let Some(disk) = SaiWsDiskCache::load(key) {
                    Self::hydrate_from_legacy_disk(key, &disk, cx);
                    return Some(disk);
                }
                Self::state(cx).memory.get(key).cloned()
            }
        }
    }

    fn hydrate_from_legacy_disk(cache_key: &str, disk: &Value, cx: &mut App) {
        let state = Self::state(cx);
        state.memory.insert(cache_key.to_owned(), disk.clone());
        state.note_legacy_hydrated(cache_key, true);
        Self::hydrate_optimistic_paths_from_disk_if_needed(cache_key, cx);
    }

    pub fn memory_only(cache_key: &str, cx: &mut App) -> Option<Value> {
        Self::state(cx).memory.get(cache_key).cloned()
    }

    pub fn memory_only_read(cache_key: &str, cx: &App) -> Option<Value> {
        cx.try_global::<SaiWsResponseCacheState>()?.memory.get(cache_key).cloned()
    }

    /// memory only read for getters that hold just app, the view wakes on bumps of this key only
    pub fn observed_memory_only_read(cache_key: &str, cx: &App) -> Option<Value> {
        let ck = trimming_whitespaces_and_newlines(cache_key);
        if ck.is_empty() {
            return None;
        }
        SaiCacheRevisionTracker::observed_revision_in(ck, cx);
        cx.try_global::<SaiWsResponseCacheState>()?.memory.get(ck).cloned()
    }

    /// memory lookup that tracks observation, never falls back to the legacy disk cache
    pub fn observed_memory_only(cache_key: &str, cx: &mut App) -> Option<Value> {
        let ck = trimming_whitespaces_and_newlines(cache_key);
        if ck.is_empty() {
            return None;
        }
        Self::revision(ck, cx);
        Self::state(cx).memory.get(ck).cloned()
    }

    pub fn disk_only(cache_key: &str, _cx: &mut App) -> Option<Value> {
        SaiWsDiskCache::load(cache_key)
    }

    pub fn peek(
        options: &SaiWsRequestOptions,
        on_cold_disk_loaded: Option<SaiWsColdDiskLoaded>,
        cx: &mut App,
    ) -> (Option<Value>, bool, bool) {
        let (value, had_memory, had_disk, _) = Self::peek_with_source(options, false, on_cold_disk_loaded, cx);
        (value, had_memory, had_disk)
    }

    fn warm_cold_disk(cache_key: &str, on_loaded: Option<SaiWsColdDiskLoaded>, cx: &mut App) {
        if !Self::state(cx).cold_disk_warmups.insert(cache_key.to_owned()) {
            return;
        }
        let key = cache_key.to_owned();
        SaiWsDiskCache::warm_asynchronously(
            cache_key.to_owned(),
            move |value, cx| {
                let state = Self::state(cx);
                state.cold_disk_warmups.remove(&key);
                let Some(value) = value else {
                    return;
                };
                if state.memory.contains_key(&key) || state.strict_memory_only_cache_keys.contains(&key) {
                    return;
                }
                Self::hydrate_memory(&key, &value, true, cx);
                Self::hydrate_optimistic_paths_from_disk_if_needed(&key, cx);
                if let Some(on_loaded) = on_loaded {
                    on_loaded(&value, cx);
                }
            },
            cx,
        );
    }

    pub fn peek_with_source(
        options: &SaiWsRequestOptions,
        reads_cold_disk: bool,
        on_cold_disk_loaded: Option<SaiWsColdDiskLoaded>,
        cx: &mut App,
    ) -> (Option<Value>, bool, bool, Option<SaiWsCacheSelectionSource>) {
        let Some(ck) = options.cache_key.as_deref().filter(|key| !key.is_empty()) else {
            return (None, false, false, None);
        };
        if options.disk_cache_policy == SaiWsDiskCachePolicy::MemoryOnly {
            Self::apply_disk_cache_policy(SaiWsDiskCachePolicy::MemoryOnly, ck, cx);
        }
        let state = Self::state(cx);
        let mem = state.memory.get(ck).cloned();
        if state.strict_memory_only_cache_keys.contains(ck) {
            let present = mem.is_some();
            return (mem, present, false, present.then_some(SaiWsCacheSelectionSource::Memory));
        }
        if options.preferred_cache_layer == SaiWsCachePreferredLayer::MemoryFirst && mem.is_some() {
            return (mem, true, false, Some(SaiWsCacheSelectionSource::Memory));
        }
        if options.preferred_cache_layer == SaiWsCachePreferredLayer::DiskFirst
            && state.pagination_disk_dirty_keys.contains(ck)
            && mem.is_some()
        {
            return (
                mem,
                true,
                SaiWsDiskCache::contains(ck),
                Some(SaiWsCacheSelectionSource::Memory),
            );
        }
        let disk = if reads_cold_disk {
            SaiWsDiskCache::load(ck)
        } else {
            let (known, value) = SaiWsDiskCache::load_if_known(ck);
            if !known {
                Self::warm_cold_disk(ck, on_cold_disk_loaded, cx);
            }
            value
        };
        let mem_present = mem.is_some();
        let disk_present = disk.is_some();
        match options.preferred_cache_layer {
            SaiWsCachePreferredLayer::MemoryFirst => match (mem, disk) {
                (Some(m), _) => (Some(m), true, disk_present, Some(SaiWsCacheSelectionSource::Memory)),
                (None, Some(d)) => (Some(d), false, true, Some(SaiWsCacheSelectionSource::LegacyDisk)),
                (None, None) => (None, false, false, None),
            },
            SaiWsCachePreferredLayer::DiskFirst => match (disk, mem) {
                (Some(d), _) => {
                    (Some(d), mem_present, true, Some(SaiWsCacheSelectionSource::LegacyDisk))
                }
                (None, Some(m)) => (Some(m), true, false, Some(SaiWsCacheSelectionSource::Memory)),
                (None, None) => (None, false, false, None),
            },
        }
    }

    /// stores into memory, an equal payload counts as the same object so no revision bump
    pub fn hydrate_memory(
        cache_key: &str,
        value: impl Borrow<Value>,
        source_was_legacy_disk: bool,
        cx: &mut App,
    ) {
        if cache_key.is_empty() {
            return;
        }
        let value: &Value = value.borrow();
        let state = Self::state(cx);
        let is_same_object = state.memory.get(cache_key) == Some(value);
        if !is_same_object {
            state.memory.insert(cache_key.to_owned(), value.clone());
        }
        state.note_legacy_hydrated(cache_key, source_was_legacy_disk);
        if !is_same_object {
            Self::bump_revision(cache_key, cx);
        }
    }

    pub fn note_active_slice(
        slice: Option<impl Borrow<Rc<dyn SaiSliceState>>>,
        cache_key: Option<&str>,
        take_path: Option<&str>,
        cx: &mut App,
    ) {
        let (Some(ck), Some(slice)) = (cache_key.filter(|key| !key.is_empty()), slice) else {
            return;
        };
        let slice: &Rc<dyn SaiSliceState> = slice.borrow();
        Self::state(cx).slice_by_cache_key.insert(
            ck.to_owned(),
            SaiWsWeakSlice {
                slice: Rc::downgrade(slice),
                take_path: take_path.map(str::to_owned),
            },
        );
    }

    pub fn current_payload(
        cache_key: &str,
        preferred_layer: SaiWsCachePreferredLayer,
        disk_cache_policy: SaiWsDiskCachePolicy,
        cx: &mut App,
    ) -> Option<Value> {
        let ck = trimming_whitespaces_and_newlines(cache_key);
        if ck.is_empty() {
            return None;
        }
        let options = Self::lookup_options(ck, disk_cache_policy, preferred_layer);
        Self::resolved_value(&options, cx)
    }

    pub fn observed_payload(
        cache_key: &str,
        preferred_layer: SaiWsCachePreferredLayer,
        disk_cache_policy: SaiWsDiskCachePolicy,
        cx: &mut App,
    ) -> Option<Value> {
        let ck = trimming_whitespaces_and_newlines(cache_key);
        if ck.is_empty() {
            return None;
        }
        Self::revision(ck, cx);
        Self::current_payload(ck, preferred_layer, disk_cache_policy, cx)
    }

    pub fn observed_projection<V: Clone + 'static>(
        cache_key: &str,
        projection_key: &str,
        preferred_layer: SaiWsCachePreferredLayer,
        disk_cache_policy: SaiWsDiskCachePolicy,
        transform: impl FnOnce(Option<&Value>) -> V,
        cx: &mut App,
    ) -> V {
        let ck = Self::trimmed(cache_key);
        let pk = trimming_whitespaces_and_newlines(projection_key);
        if disk_cache_policy == SaiWsDiskCachePolicy::MemoryOnly {
            Self::apply_disk_cache_policy(SaiWsDiskCachePolicy::MemoryOnly, &ck, cx);
        }
        let revision = Self::revision(&ck, cx);
        let slot = format!("{ck}\u{1F}{pk}");
        if let Some(cached) = Self::state(cx).observed_projections.get(&slot)
            && cached.revision == revision
            && let Some(typed) = cached.typed.downcast_ref::<SaiWsTypedObservedProjectionBox<V>>()
        {
            return typed.value.clone();
        }

        let payload = Self::current_payload(&ck, preferred_layer, disk_cache_policy, cx);
        let value = transform(payload.as_ref());
        let state = Self::state(cx);
        if state.observed_projections.len() >= MAXIMUM_OBSERVED_PROJECTIONS
            && !state.observed_projections.contains_key(&slot)
            && let Some(oldest_slot) = state.observed_projections.keys().next().cloned()
        {
            state.observed_projections.remove(&oldest_slot);
        }
        state.observed_projections.insert(
            slot,
            SaiWsObservedProjectionBox {
                cache_key: ck,
                revision,
                typed: Box::new(SaiWsTypedObservedProjectionBox { value: value.clone() }),
            },
        );
        value
    }

    pub fn install_optimistic_root(
        root: Value,
        cache_key: &str,
        store_in_memory: bool,
        persist_disk: bool,
        disk_cache_policy: SaiWsDiskCachePolicy,
        sync_disk_with_optimistic: bool,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let ck = Self::trimmed(cache_key);
        if ck.is_empty() {
            return Ok(());
        }
        if disk_cache_policy == SaiWsDiskCachePolicy::MemoryOnly {
            Self::apply_disk_cache_policy(SaiWsDiskCachePolicy::MemoryOnly, &ck, cx);
        }
        if store_in_memory {
            let state = Self::state(cx);
            state.memory.insert(ck.clone(), root.clone());
            state.note_legacy_hydrated(&ck, false);
            Self::bump_revision(&ck, cx);
        }
        let write_disk = Self::should_persist_payload_to_disk(
            &ck,
            persist_disk,
            sync_disk_with_optimistic && store_in_memory,
            cx,
        );
        if write_disk {
            Self::persist_to_disk(&ck, root, cx)?;
        }
        Ok(())
    }

    pub fn make_cached_context(
        cache_key: &str,
        disk_cache_policy: SaiWsDiskCachePolicy,
        cx: &mut App,
    ) -> SaiWsCachedContext {
        let ck = Self::trimmed(cache_key);
        if ck.is_empty() {
            return SaiWsCachedContext {
                cache_key: ck,
                path_to_array: None,
                path_to_id: None,
                payload: None,
            };
        }
        if disk_cache_policy == SaiWsDiskCachePolicy::MemoryOnly {
            Self::apply_disk_cache_policy(SaiWsDiskCachePolicy::MemoryOnly, &ck, cx);
        }
        Self::hydrate_optimistic_paths_from_disk_if_needed(&ck, cx);
        let paths = Self::state(cx).memory_optimistic_paths.get(&ck).cloned().unwrap_or_default();
        let options =
            Self::lookup_options(&ck, disk_cache_policy, SaiWsCachePreferredLayer::MemoryFirst);
        let payload = Self::resolved_value(&options, cx);
        SaiWsCachedContext {
            cache_key: ck,
            path_to_array: paths.path_to_array,
            path_to_id: paths.path_to_id,
            payload,
        }
    }

    pub fn apply_sai_updater_payload(
        value: Value,
        cache_key: &str,
        scope: SaiUpdaterWriteScope,
        source_was_legacy_disk: bool,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let ck = Self::trimmed(cache_key);
        if ck.is_empty() {
            return Ok(());
        }

        match scope {
            SaiUpdaterWriteScope::State => Self::apply_to_slice(&value, &ck, cx),
            SaiUpdaterWriteScope::Cache => {
                let state = Self::state(cx);
                state.memory.insert(ck.clone(), value);
                state.note_legacy_hydrated(&ck, source_was_legacy_disk);
            }
            SaiUpdaterWriteScope::DiskCache => {
                if Self::state(cx).strict_memory_only_cache_keys.contains(&ck) {
                    SaiWsDiskCache::remove(&ck);
                } else {
                    Self::persist_to_disk(&ck, value, cx)?;
                    Self::drop_memory_if_present(&ck, cx);
                }
            }
            SaiUpdaterWriteScope::All => {
                let state = Self::state(cx);
                state.memory.insert(ck.clone(), value.clone());
                state.note_legacy_hydrated(&ck, source_was_legacy_disk);
                let strict = state.strict_memory_only_cache_keys.contains(&ck);
                if !strict {
                    Self::persist_to_disk(&ck, value.clone(), cx)?;
                }
                Self::apply_to_slice(&value, &ck, cx);
            }
        }
        Self::bump_revision(&ck, cx);
        Ok(())
    }

    fn apply_to_slice(value: &Value, cache_key: &str, cx: &mut App) {
        let target = Self::state(cx)
            .slice_by_cache_key
            .get(cache_key)
            .and_then(|entry| entry.slice.upgrade().map(|slice| (slice, entry.take_path.clone())));
        let Some((slice, take_path)) = target else {
            return;
        };
        let raw = if value.is_null() { None } else { Some(value) };
        slice.mark_fulfilled(raw, take_path.as_deref());
    }

    pub fn store_success_payload_options(
        value: Option<Value>,
        options: &SaiWsRequestOptions,
        cx: &mut App,
    ) {
        Self::prepare_for_request(options, cx);
        Self::store_success_payload_binding(value, &SaiWsPendingCacheBinding::new(options), cx);
    }

    pub fn store_success_payload_binding(
        value: Option<Value>,
        binding: &SaiWsPendingCacheBinding,
        cx: &mut App,
    ) {
        let Some(key) = binding.cache_key.as_deref().filter(|key| !key.is_empty()) else {
            return;
        };
        if binding.disk_cache_policy == SaiWsDiskCachePolicy::MemoryOnly {
            Self::apply_disk_cache_policy(SaiWsDiskCachePolicy::MemoryOnly, key, cx);
        }
        let Some(value) = value.filter(|value| !value.is_null()) else {
            return;
        };
        if binding.store_response_in_cache {
            let state = Self::state(cx);
            let unchanged = !state.legacy_disk_hydrated_memory_keys.contains(key)
                && state.memory.get(key) == Some(&value);
            if !unchanged {
                state.memory.insert(key.to_owned(), value.clone());
                state.note_legacy_hydrated(key, false);
                Self::bump_revision(key, cx);
            }
        }
        let should_persist_success = binding.persist_cache_to_disk()
            || (binding.store_response_in_cache
                && Self::should_persist_payload_to_disk(key, false, false, cx));
        if should_persist_success {
            if binding.pagination.is_some() {
                Self::schedule_pagination_disk_persistence(key, value, cx);
            } else if let Err(error) = Self::persist_to_disk(key, value, cx) {
                Self::log_disk_failure("disk persistence", key, &error, cx);
            }
        }
        if binding.path_to_array.is_some() || binding.path_to_id.is_some() {
            Self::state(cx).memory_optimistic_paths.insert(
                key.to_owned(),
                SaiWsOptimisticPaths {
                    path_to_array: binding.path_to_array.clone(),
                    path_to_id: binding.path_to_id.clone(),
                },
            );
            if should_persist_success {
                SaiWsDiskCache::save_optimistic_meta_deferred(
                    key.to_owned(),
                    binding.path_to_array.clone(),
                    binding.path_to_id.clone(),
                    cx,
                );
            }
        }
    }

    fn log_disk_failure(what: &'static str, cache_key: &str, error: &impl std::fmt::Display, cx: &mut App) {
        let message = format!("{what} failed for {cache_key}: {error}");
        Sai::log("SaiWs", move || message, SaiLogKind::Cache, cx);
    }

    fn should_persist_payload_to_disk(
        cache_key: &str,
        explicit_persist: bool,
        sync_disk_with_optimistic: bool,
        cx: &mut App,
    ) -> bool {
        if cache_key.is_empty() {
            return false;
        }
        if Self::state(cx).strict_memory_only_cache_keys.contains(cache_key) {
            return false;
        }
        if explicit_persist || sync_disk_with_optimistic {
            return true;
        }
        if Self::uses_deferred_disk_persistence(cache_key) {
            return false;
        }
        SaiWsDiskCache::contains(cache_key)
    }

    fn uses_deferred_disk_persistence(cache_key: &str) -> bool {
        SaiAccountCacheScope::logical_key(cache_key).starts_with(DEFERRED_DISK_PERSISTENCE_PREFIX)
    }

    fn persist_to_disk(cache_key: &str, value: Value, cx: &mut App) -> anyhow::Result<()> {
        if Self::state(cx).strict_memory_only_cache_keys.contains(cache_key) {
            return Ok(());
        }
        SaiWsDiskCache::save_deferred(cache_key.to_owned(), value, cx);
        Ok(())
    }

    fn schedule_pagination_disk_persistence(cache_key: &str, value: Value, cx: &mut App) {
        let state = Self::state(cx);
        if state.strict_memory_only_cache_keys.contains(cache_key) {
            return;
        }
        let generation = state.pagination_disk_generation(cache_key);
        state.pagination_disk_dirty_keys.insert(cache_key.to_owned());
        state.pending_pagination_disk_payloads.insert(cache_key.to_owned(), value);
        if !state.pagination_disk_writes_in_flight.insert(cache_key.to_owned()) {
            return;
        }
        Self::flush_next_pagination_disk_payload(cache_key.to_owned(), generation, cx);
    }

    fn flush_next_pagination_disk_payload(cache_key: String, generation: u64, cx: &mut App) {
        let state = Self::state(cx);
        if state.pagination_disk_generation_by_key.get(&cache_key) != Some(&generation) {
            return;
        }
        let Some(value) = state.pending_pagination_disk_payloads.remove(&cache_key) else {
            state.pagination_disk_writes_in_flight.remove(&cache_key);
            return;
        };
        let completion_key = cache_key.clone();
        SaiWsDiskCache::save_asynchronously(
            cache_key,
            value,
            move |result: Result<(), SaiWsDiskCacheError>, cx: &mut App| {
                let cache_key = completion_key;
                if Self::state(cx).pagination_disk_generation_by_key.get(&cache_key)
                    != Some(&generation)
                {
                    return;
                }
                let succeeded = result.is_ok();
                if let Err(error) = &result {
                    Self::log_disk_failure("pagination disk persistence", &cache_key, error, cx);
                }
                let state = Self::state(cx);
                if state.pending_pagination_disk_payloads.contains_key(&cache_key) {
                    Self::flush_next_pagination_disk_payload(cache_key, generation, cx);
                } else {
                    state.pagination_disk_writes_in_flight.remove(&cache_key);
                    if succeeded {
                        state.pagination_disk_dirty_keys.remove(&cache_key);
                    }
                }
            },
            cx,
        );
    }

    pub fn remove(cache_key: &str, layer: SaiCacheCleanLayer, cx: &mut App) {
        let ck = Self::trimmed(cache_key);
        if ck.is_empty() {
            return;
        }
        let state = Self::state(cx);
        state.observed_projections.retain(|_, projection| projection.cache_key != ck);
        match layer {
            SaiCacheCleanLayer::Everywhere => {
                state.strict_memory_only_cache_keys.remove(&ck);
                state.invalidate_pagination_disk_persistence(&ck);
                state.memory.remove(&ck);
                state.legacy_disk_hydrated_memory_keys.remove(&ck);
                state.memory_optimistic_paths.remove(&ck);
                state.slice_by_cache_key.remove(&ck);
                SaiWsDiskCache::remove(&ck);
                Self::bump_revision(&ck, cx);
                SaiWsPaginationStateStore::remove(&ck, cx);
            }
            SaiCacheCleanLayer::Memory => {
                state.pagination_disk_dirty_keys.remove(&ck);
                state.memory.remove(&ck);
                state.legacy_disk_hydrated_memory_keys.remove(&ck);
                state.memory_optimistic_paths.remove(&ck);
                state.slice_by_cache_key.remove(&ck);
                Self::bump_revision(&ck, cx);
                SaiWsPaginationStateStore::remove(&ck, cx);
            }
            SaiCacheCleanLayer::Disk => {
                state.invalidate_pagination_disk_persistence(&ck);
                SaiWsDiskCache::remove(&ck);
            }
        }
    }

    pub fn debug_all_listed_cache_keys(cx: &mut App) -> Vec<String> {
        let mut keys: HashSet<String> = SaiWsDiskCache::all_indexed_cache_keys().into_iter().collect();
        keys.extend(Self::state(cx).memory.keys().cloned());
        let mut sorted: Vec<String> = keys.into_iter().collect();
        sorted.sort();
        sorted
    }

    pub fn debug_layers_present(cache_key: &str, cx: &mut App) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if Self::state(cx).memory.contains_key(cache_key) {
            parts.push("memory");
        }
        if SaiWsDiskCache::debug_payload_file_bytes(cache_key).is_some() {
            parts.push("disk");
        }
        if parts.is_empty() {
            "—".to_owned()
        } else {
            parts.join(" + ")
        }
    }

    pub fn debug_presentation_source(cache_key: &str, cx: &mut App) -> String {
        let state = Self::state(cx);
        if !state.memory.contains_key(cache_key) {
            return "miss".to_owned();
        }
        if state.legacy_disk_hydrated_memory_keys.contains(cache_key) {
            "legacy-disk-hydrated".to_owned()
        } else {
            "memory-hit".to_owned()
        }
    }

    pub fn debug_resolved_value(cache_key: &str, cx: &mut App) -> Option<Value> {
        if cache_key.is_empty() {
            return None;
        }
        if let Some(memory) = Self::state(cx).memory.get(cache_key) {
            return Some(memory.clone());
        }
        SaiWsDiskCache::load(cache_key)
    }

    pub fn debug_size_label(cache_key: &str, cx: &mut App) -> String {
        if cache_key.is_empty() {
            return "—".to_owned();
        }
        if let Some(memory) = Self::state(cx).memory.get(cache_key) {
            return match serde_json::to_string(memory) {
                Ok(text) => {
                    let char_count = i64::try_from(text.chars().count()).unwrap_or(i64::MAX);
                    format!("memory · {}", SaiDebugBodyFormat::size_label_char_count(char_count))
                }
                Err(_) => "memory · (object)".to_owned(),
            };
        }
        if let Some(bytes) = SaiWsDiskCache::debug_payload_file_bytes(cache_key) {
            let byte_count = i64::try_from(bytes).unwrap_or(i64::MAX);
            return format!("disk · {}", SaiDebugBodyFormat::size_label_byte_count(byte_count));
        }
        "—".to_owned()
    }

    pub fn debug_body_text(cache_key: &str, cx: &mut App) -> String {
        let value = Self::debug_resolved_value(cache_key, cx);
        SaiHttpTraceRecorder::pretty_json_string_from_any(value.as_ref())
    }

    pub fn clear_all(cx: &mut App) {
        let state = Self::state(cx);
        state.invalidate_all_pagination_disk_persistence();
        state.memory.clear();
        state.legacy_disk_hydrated_memory_keys.clear();
        state.memory_optimistic_paths.clear();
        state.slice_by_cache_key.clear();
        state.observed_projections.clear();
        state.strict_memory_only_cache_keys.clear();
        SaiWsDiskCache::remove_all_payload_files_and_index();
        SaiCacheRevisionTracker::shared(cx).update(cx, |tracker, cx| tracker.reset(cx));
        SaiWsPaginationStateStore::reset_all(cx);
    }

    pub fn prepare_disk_clear(cx: &mut App) {
        Self::state(cx).invalidate_all_pagination_disk_persistence();
    }

    pub fn prepare_disk_clear_cache_keys(cache_keys: &[String], cx: &mut App) {
        let state = Self::state(cx);
        for key in cache_keys {
            let trimmed = trimming_whitespaces_and_newlines(key);
            if trimmed.is_empty() {
                continue;
            }
            state.invalidate_pagination_disk_persistence(trimmed);
        }
    }

    pub fn clear_memory(cx: &mut App) {
        let state = Self::state(cx);
        state.pagination_disk_dirty_keys.clear();
        state.memory.clear();
        state.legacy_disk_hydrated_memory_keys.clear();
        state.memory_optimistic_paths.clear();
        state.slice_by_cache_key.clear();
        state.observed_projections.clear();
        SaiCacheRevisionTracker::shared(cx).update(cx, |tracker, cx| tracker.reset(cx));
        SaiWsPaginationStateStore::reset_all(cx);
    }

    pub fn remove_persisted_account(user_id: &str, cx: &mut App) {
        let prefix = SaiAccountCacheScope::prefix(user_id);
        let state = Self::state(cx);
        state.strict_memory_only_cache_keys.retain(|key| !key.starts_with(&prefix));
        let pagination_keys: Vec<String> = state
            .pagination_disk_generation_by_key
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .cloned()
            .collect();
        for key in pagination_keys {
            state.invalidate_pagination_disk_persistence(&key);
        }
        SaiWsDiskCache::remove_all(&prefix);
    }

    pub fn clear_all_for_debug(cx: &mut App) {
        Self::clear_all(cx);
    }

    pub fn drop_memory_if_present(cache_key: &str, cx: &mut App) {
        if cache_key.is_empty() {
            return;
        }
        let state = Self::state(cx);
        state.memory.remove(cache_key);
        state.legacy_disk_hydrated_memory_keys.remove(cache_key);
    }

    fn hydrate_optimistic_paths_from_disk_if_needed(cache_key: &str, cx: &mut App) {
        let state = Self::state(cx);
        if state.memory_optimistic_paths.contains_key(cache_key)
            || state.strict_memory_only_cache_keys.contains(cache_key)
        {
            return;
        }
        let Some((path_to_array, path_to_id)) = SaiWsDiskCache::load_optimistic_meta(cache_key)
        else {
            return;
        };
        state
            .memory_optimistic_paths
            .insert(cache_key.to_owned(), SaiWsOptimisticPaths { path_to_array, path_to_id });
    }

    fn apply_disk_cache_policy(policy: SaiWsDiskCachePolicy, cache_key: &str, cx: &mut App) {
        match policy {
            SaiWsDiskCachePolicy::PreserveExisting => {}
            SaiWsDiskCachePolicy::Persistent => {
                if Self::state(cx).strict_memory_only_cache_keys.remove(cache_key) {
                    Self::bump_revision(cache_key, cx);
                }
            }
            SaiWsDiskCachePolicy::MemoryOnly => {
                let state = Self::state(cx);
                if !state.strict_memory_only_cache_keys.insert(cache_key.to_owned()) {
                    return;
                }
                state.invalidate_pagination_disk_persistence(cache_key);
                SaiWsDiskCache::remove(cache_key);
                Self::bump_revision(cache_key, cx);
            }
        }
    }
}
