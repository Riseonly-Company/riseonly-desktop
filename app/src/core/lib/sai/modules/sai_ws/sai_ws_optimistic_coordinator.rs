use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::rc::Rc;

use gpui::{App, Global};
use rise_canon::RiseIdentityCanon;
use rise_canon::rise_key::{bot, events};
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use serde_json::{Map, Value};

use super::sai_ws_optimistic_update::{
    SaiWsCreatePatch, SaiWsExtractRealData, SaiWsMatchTempWithReal, SaiWsOnOptimisticFailure,
    SaiWsOptimisticAddStrategy, SaiWsOptimisticError, SaiWsOptimisticSuccessContext,
    SaiWsOptimisticUpdate,
};
use super::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsPendingCacheBinding, SaiWsRequestOptions,
};
use super::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::get_sai_server_id::{GetSaiServerId, GetSaiServerIdOptions};
use crate::core::lib::sai::core::utilities::sai_ws_optimistic_ignore_keys::SaiWsOptimisticIgnoreKeys;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredOutboundCoordinator;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::sai_image_disk_cache_hooks::sai_clever_image_disk_prefetch_schedule;

const OPTIMISTIC_TEMPORARY_PREFIX: &str = RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiWsPreparedOptimisticSend {
    pub wire_request_id: u64,
    pub outcome: SaiWsOptimisticApplyOutcome,
}

/// result of applying an optimistic change
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaiWsOptimisticApplyOutcome {
    pub did_register_session: bool,
    pub insert_temp_id: Option<String>,
}

/// error from applying before send, optimistic error or
/// cache write failure or a feature closure error in one type
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct SaiWsOptimisticApplyError(#[from] anyhow::Error);

enum SaiWsJsonObjectBridge {}

impl SaiWsJsonObjectBridge {
    fn dictionary(any: Option<&Value>) -> Option<Map<String, Value>> {
        AnyDictionaryBridge::dictionary(any)
    }

    fn any_array(any: Option<&Value>) -> Option<Vec<Value>> {
        any?.as_array().cloned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Insert,
    Delete,
    Update,
    Patch,
}

#[derive(Clone)]
struct Session {
    cache_key: String,
    path_to_array: String,
    row_id_key: String,
    temp_id: String,
    temp_id_key: String,
    temp_flag_key: String,
    mode: Mode,
    store_in_memory: bool,
    persist_disk: bool,
    disk_cache_policy: SaiWsDiskCachePolicy,
    sync_disk_cache: bool,
    preferred_layer: SaiWsCachePreferredLayer,
    extract_real_data: Option<SaiWsExtractRealData>,
    extract_ignore_keys: Vec<String>,
    prefetch_clever_image_disk_keys: Vec<String>,
    match_temp_with_real: Option<SaiWsMatchTempWithReal>,
    on_optimistic_failure: Option<SaiWsOnOptimisticFailure>,
    patch_rollback: Option<Map<String, Value>>,
    patch_match_id: Option<String>,
}

impl Session {
    fn new(
        mode: Mode,
        cache_key: &str,
        path_to_array: &str,
        row_id_key: &str,
        temp_id: &str,
        opt: &SaiWsOptimisticUpdate,
        options: &SaiWsRequestOptions,
    ) -> Self {
        Self {
            cache_key: cache_key.to_owned(),
            path_to_array: path_to_array.to_owned(),
            row_id_key: row_id_key.to_owned(),
            temp_id: temp_id.to_owned(),
            temp_id_key: opt.temp_id_key.clone(),
            temp_flag_key: opt.temp_flag_key.clone(),
            mode,
            store_in_memory: options.store_response_in_cache,
            persist_disk: options.persist_cache_to_disk(),
            disk_cache_policy: options.disk_cache_policy,
            sync_disk_cache: opt.sync_disk_cache,
            preferred_layer: options.preferred_cache_layer,
            extract_real_data: opt.extract_real_data.clone(),
            extract_ignore_keys: opt.extract_ignore_keys.clone(),
            prefetch_clever_image_disk_keys: opt
                .files
                .as_ref()
                .map(|files| files.prefetch_clever_image_disk_keys.clone())
                .unwrap_or_default(),
            match_temp_with_real: opt.match_temp_with_real.clone(),
            on_optimistic_failure: opt.on_optimistic_failure.clone(),
            patch_rollback: None,
            patch_match_id: None,
        }
    }
}

#[derive(Default)]
struct SaiWsOptimisticCoordinatorState {
    sessions: HashMap<u64, Session>,
    pre_wire_optimistic_ids: HashSet<u64>,
}

impl Global for SaiWsOptimisticCoordinatorState {}

/// how a cache write lands, request layers or the ones a session remembered
#[derive(Clone, Copy)]
struct CacheWrite {
    store_in_memory: bool,
    persist_disk: bool,
    disk_cache_policy: SaiWsDiskCachePolicy,
    preferred_layer: SaiWsCachePreferredLayer,
    sync_disk_with_optimistic: bool,
}

impl CacheWrite {
    fn from_options(options: &SaiWsRequestOptions) -> Self {
        Self {
            store_in_memory: options.store_response_in_cache,
            persist_disk: options.persist_cache_to_disk(),
            disk_cache_policy: options.disk_cache_policy,
            preferred_layer: options.preferred_cache_layer,
            sync_disk_with_optimistic: options
                .optimistic_update
                .as_ref()
                .is_none_or(|opt| opt.sync_disk_cache),
        }
    }

    fn from_session(session: &Session) -> Self {
        Self {
            store_in_memory: session.store_in_memory,
            persist_disk: session.persist_disk,
            disk_cache_policy: session.disk_cache_policy,
            preferred_layer: session.preferred_layer,
            sync_disk_with_optimistic: session.sync_disk_cache,
        }
    }
}

type RowsMap<'a> = dyn FnMut(Vec<Value>, &mut App) -> anyhow::Result<Vec<Value>> + 'a;

/// coordinator for optimistic sends and rollbacks
pub enum SaiWsOptimisticCoordinator {}

impl SaiWsOptimisticCoordinator {
    fn state(cx: &mut App) -> &mut SaiWsOptimisticCoordinatorState {
        cx.default_global::<SaiWsOptimisticCoordinatorState>()
    }

    pub fn reset_after_socket_drop(preserving: &HashSet<u64>, cx: &mut App) {
        let mut preserving = preserving.clone();
        let prepared = SaiWsDeferredOutboundCoordinator::shared(cx)
            .read(cx)
            .prepared_optimistic_wire_ids();
        preserving.extend(prepared);
        let state = Self::state(cx);
        preserving.extend(state.pre_wire_optimistic_ids.iter().copied());
        if preserving.is_empty() {
            state.sessions.clear();
            return;
        }
        state.sessions.retain(|id, _| preserving.contains(id));
    }

    pub fn reset(cx: &mut App) {
        let state = Self::state(cx);
        state.sessions.clear();
        state.pre_wire_optimistic_ids.clear();
    }

    pub fn begin_pre_wire_optimistic(wire_request_id: u64, cx: &mut App) {
        Self::state(cx).pre_wire_optimistic_ids.insert(wire_request_id);
    }

    pub fn finish_pre_wire_optimistic(wire_request_id: u64, cx: &mut App) {
        Self::state(cx).pre_wire_optimistic_ids.remove(&wire_request_id);
    }

    pub fn pre_wire_optimistic_wire_ids(cx: &App) -> HashSet<u64> {
        cx.try_global::<SaiWsOptimisticCoordinatorState>()
            .map(|state| state.pre_wire_optimistic_ids.clone())
            .unwrap_or_default()
    }

    pub fn has_insert_session(wire_request_id: u64, cx: &App) -> bool {
        cx.try_global::<SaiWsOptimisticCoordinatorState>()
            .and_then(|state| state.sessions.get(&wire_request_id))
            .is_some_and(|session| session.mode == Mode::Insert)
    }

    pub fn should_bypass_cache_only_short_circuit(options: &SaiWsRequestOptions, _cx: &App) -> bool {
        let Some(opt) = &options.optimistic_update else { return false };
        if !opt.enabled {
            return false;
        }
        options.reload_if_cached && opt.requires_network_bypassing_cache_only_hit()
    }

    fn is_files_only_optimistic_upload(opt: &SaiWsOptimisticUpdate) -> bool {
        let Some(files) = &opt.files else { return false };
        if !files.enabled || files.local_files.is_empty() {
            return false;
        }
        opt.create_temp_data.is_none() && !opt.patch_in_place && !opt.delete_mode && !opt.update_mode
    }

    pub fn apply_before_send_if_needed(
        wire_request_id: u64,
        outbound_body: &Value,
        options: &SaiWsRequestOptions,
        cx: &mut App,
    ) -> Result<SaiWsOptimisticApplyOutcome, SaiWsOptimisticApplyError> {
        Self::apply_before_send(wire_request_id, outbound_body, options, cx).map_err(SaiWsOptimisticApplyError)
    }

    fn apply_before_send(
        wire_request_id: u64,
        outbound_body: &Value,
        options: &SaiWsRequestOptions,
        cx: &mut App,
    ) -> anyhow::Result<SaiWsOptimisticApplyOutcome> {
        let Some(opt) = options.optimistic_update.as_ref().filter(|opt| opt.enabled) else {
            return Ok(SaiWsOptimisticApplyOutcome::default());
        };

        if Self::is_files_only_optimistic_upload(opt) {
            return Ok(SaiWsOptimisticApplyOutcome::default());
        }

        let ck = Self::resolved_cache_key(options, opt)
            .map(|key| trimming_whitespaces_and_newlines(&key).to_owned())
            .filter(|key| !key.is_empty())
            .ok_or(SaiWsOptimisticError::CacheKeyRequired)?;

        if opt.patch_root_in_place {
            let (Some(patch), Some(rollback_builder)) = (&opt.create_patch, &opt.create_rollback_snapshot)
            else {
                return Ok(SaiWsOptimisticApplyOutcome::default());
            };
            if opt.patch_paths.is_empty() {
                return Ok(SaiWsOptimisticApplyOutcome::default());
            }

            let roll = Self::apply_root_patch_mutation(&ck, outbound_body, patch, rollback_builder, options, cx)?;

            let mut session = Session::new(Mode::Patch, &ck, "", &opt.match_id_key, "", opt, options);
            session.patch_rollback = Some(roll);
            Self::state(cx).sessions.insert(wire_request_id, session);
            return Ok(SaiWsOptimisticApplyOutcome { did_register_session: true, insert_temp_id: None });
        }

        let path = options
            .path_to_array
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|path| !path.is_empty())
            .ok_or(SaiWsOptimisticError::PathToArrayRequired)?
            .to_owned();

        let row_key = options
            .path_to_id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|key| !key.is_empty())
            .unwrap_or(opt.match_id_key.as_str())
            .to_owned();

        if opt.patch_in_place {
            let mid = opt.patch_match_id.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("");
            let (Some(patch), Some(rollback_builder)) = (&opt.create_patch, &opt.create_rollback_snapshot)
            else {
                return Ok(SaiWsOptimisticApplyOutcome::default());
            };
            if mid.is_empty() || opt.patch_paths.is_empty() {
                return Ok(SaiWsOptimisticApplyOutcome::default());
            }

            let roll = Self::apply_patch_mutation(
                &ck,
                &path,
                &row_key,
                mid,
                outbound_body,
                patch,
                rollback_builder,
                options,
                cx,
            )?;

            let mut session = Session::new(Mode::Patch, &ck, &path, &row_key, "", opt, options);
            session.patch_rollback = Some(roll);
            session.patch_match_id = Some(mid.to_owned());
            Self::state(cx).sessions.insert(wire_request_id, session);
            return Ok(SaiWsOptimisticApplyOutcome { did_register_session: true, insert_temp_id: None });
        }

        if opt.delete_mode
            && let Some(del) = opt
                .delete_id
                .as_deref()
                .map(trimming_whitespaces_and_newlines)
                .filter(|id| !id.is_empty())
        {
            Self::mutate_array(&ck, &path, CacheWrite::from_options(options), cx, &mut |rows, _cx| {
                Ok(rows.into_iter().filter(|row| !Self::row_matches_id(row, del, &row_key)).collect())
            })?;
            let mut session = Session::new(Mode::Delete, &ck, &path, &row_key, "", opt, options);
            session.patch_match_id = Some(del.to_owned());
            Self::state(cx).sessions.insert(wire_request_id, session);
            return Ok(SaiWsOptimisticApplyOutcome { did_register_session: true, insert_temp_id: None });
        }

        if opt.update_mode
            && let Some(uid) = opt
                .update_id
                .as_deref()
                .map(trimming_whitespaces_and_newlines)
                .filter(|id| !id.is_empty())
            && let Some(updater) = &opt.update_temp_data
        {
            Self::mutate_array(&ck, &path, CacheWrite::from_options(options), cx, &mut |rows, cx| {
                rows.into_iter()
                    .map(|row| {
                        if Self::row_matches_id(&row, uid, &row_key) {
                            updater(outbound_body, &row, cx)
                        } else {
                            Ok(row)
                        }
                    })
                    .collect()
            })?;
            let mut session = Session::new(Mode::Update, &ck, &path, &row_key, "", opt, options);
            session.patch_match_id = Some(uid.to_owned());
            Self::state(cx).sessions.insert(wire_request_id, session);
            return Ok(SaiWsOptimisticApplyOutcome { did_register_session: true, insert_temp_id: None });
        }

        let Some(builder) = &opt.create_temp_data else {
            return Ok(SaiWsOptimisticApplyOutcome::default());
        };

        let temp_id = format!(
            "{OPTIMISTIC_TEMPORARY_PREFIX}{wire_request_id}_{}",
            rand::random_range(0..=999_999_u32)
        );
        let built = builder(outbound_body, cx)?;
        let mut row = Self::normalize_to_json_object(&built)?;
        row.insert(opt.temp_id_key.clone(), Value::String(temp_id.clone()));
        row.insert(opt.temp_flag_key.clone(), Value::Bool(true));
        let row = Value::Object(row);

        Self::mutate_array(&ck, &path, CacheWrite::from_options(options), cx, &mut |rows, _cx| {
            Ok(Self::insert_row(rows, &row, opt.add_strategy, opt.insert_after_last_temp, &opt.temp_flag_key))
        })?;
        if let Some(on_added) = &opt.on_added_temp_data {
            on_added(&row, cx);
        }

        let session = Session::new(Mode::Insert, &ck, &path, &row_key, &temp_id, opt, options);
        Self::state(cx).sessions.insert(wire_request_id, session);
        Ok(SaiWsOptimisticApplyOutcome { did_register_session: true, insert_temp_id: Some(temp_id) })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn merge_uploaded_file_into_cached_temp_row(
        cache_key: &str,
        path_to_array: &str,
        temp_id: &str,
        temp_id_key: &str,
        path_in_temp_data: &str,
        index: usize,
        server_slot: &Map<String, Value>,
        options: &SaiWsRequestOptions,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let dot = trimming_whitespaces_and_newlines(path_in_temp_data);
        if dot.is_empty() {
            return Ok(());
        }

        Self::mutate_array(cache_key, path_to_array, CacheWrite::from_options(options), cx, &mut |rows, _cx| {
            rows.into_iter()
                .map(|row| -> anyhow::Result<Value> {
                    let Some(mut d) = SaiWsJsonObjectBridge::dictionary(Some(&row)) else { return Ok(row) };
                    if d.get(temp_id_key).and_then(Value::as_str) != Some(temp_id) {
                        return Ok(row);
                    }
                    Self::merge_server_slot(&mut d, dot, index, server_slot)?;
                    Ok(Value::Object(d))
                })
                .collect()
        })
    }

    pub fn merged_payload_for_cache_if_needed(
        wire_request_id: u64,
        server_payload: Option<Value>,
        binding: &SaiWsPendingCacheBinding,
        cx: &mut App,
    ) -> Option<Value> {
        let Some(session) = Self::state(cx).sessions.remove(&wire_request_id) else { return server_payload };
        if session.mode != Mode::Insert {
            return server_payload;
        }
        if let Some(payload) = &server_payload
            && !session.prefetch_clever_image_disk_keys.is_empty()
        {
            sai_clever_image_disk_prefetch_schedule(payload, &session.prefetch_clever_image_disk_keys, cx);
        }
        if binding.cache_key.as_deref() != Some(session.cache_key.as_str()) {
            return server_payload;
        }
        let Some(pre) = SaiWsResponseCache::current_payload(
            &session.cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            binding.disk_cache_policy,
            cx,
        ) else {
            return server_payload;
        };
        match Self::replace_temp_with_real(&pre, &session, server_payload.as_ref(), cx) {
            Ok(merged) => Some(merged),
            Err(_) => server_payload,
        }
    }

    /// rollback on failure, cache only and the slice is unused because the caller rejects it
    pub fn handle_failure(
        wire_request_id: u64,
        error: &dyn Error,
        _slice: Option<Rc<dyn SaiSliceState>>,
        _binding: Option<&SaiWsPendingCacheBinding>,
        cx: &mut App,
    ) {
        let Some(session) = Self::state(cx).sessions.remove(&wire_request_id) else { return };
        let retain = session.on_optimistic_failure.as_ref().is_some_and(|on_failure| on_failure(None, error, cx));
        if retain {
            Self::state(cx).sessions.insert(wire_request_id, session);
            return;
        }
        let ck = trimming_whitespaces_and_newlines(&session.cache_key).to_owned();
        if ck.is_empty() {
            return;
        }
        let write = CacheWrite::from_session(&session);

        match session.mode {
            Mode::Insert => {
                let _ = Self::mutate_array(&ck, &session.path_to_array, write, cx, &mut |rows, _cx| {
                    Ok(rows
                        .into_iter()
                        .filter(|row| !Self::is_temp_row(row, &session.temp_id, &session.temp_id_key))
                        .collect())
                });
            }
            Mode::Patch => {
                let Some(roll) = &session.patch_rollback else { return };
                if let Some(mid) = &session.patch_match_id
                    && !session.path_to_array.is_empty()
                {
                    let _ = Self::mutate_array(&ck, &session.path_to_array, write, cx, &mut |rows, _cx| {
                        Ok(rows
                            .into_iter()
                            .map(|row| {
                                let Some(mut d) = SaiWsJsonObjectBridge::dictionary(Some(&row)) else { return row };
                                if !Self::row_matches_id(&row, mid, &session.row_id_key) {
                                    return row;
                                }
                                for (k, v) in roll {
                                    d.insert(k.clone(), v.clone());
                                }
                                Value::Object(d)
                            })
                            .collect())
                    });
                } else {
                    let _ = SaiUpdater::patch_root(&ck, roll, SaiUpdaterWriteScope::All, cx);
                }
            }
            Mode::Delete | Mode::Update => {}
        }
    }

    fn resolved_cache_key(options: &SaiWsRequestOptions, opt: &SaiWsOptimisticUpdate) -> Option<String> {
        let target = opt.target_cache_key.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("");
        if !target.is_empty() {
            return Some(target.to_owned());
        }
        options.cache_key.clone()
    }

    fn apply_root_patch_mutation(
        cache_key: &str,
        outbound_body: &Value,
        create_patch: &SaiWsCreatePatch,
        create_rollback_snapshot: &SaiWsCreatePatch,
        options: &SaiWsRequestOptions,
        cx: &mut App,
    ) -> anyhow::Result<Map<String, Value>> {
        let root = SaiWsResponseCache::current_payload(
            cache_key,
            options.preferred_cache_layer,
            options.disk_cache_policy,
            cx,
        )
        .ok_or(SaiWsOptimisticError::ApplyMutationFailed)?;
        let mut dict = SaiWsJsonObjectBridge::dictionary(Some(&root)).ok_or(SaiWsOptimisticError::NotJsonObject)?;
        let dict_value = Value::Object(dict.clone());
        let rollback = create_rollback_snapshot(outbound_body, &dict_value, cx)?;
        let patch = create_patch(outbound_body, &dict_value, cx)?;
        for (k, v) in patch {
            dict.insert(k, v);
        }
        let write = CacheWrite::from_options(options);
        SaiWsResponseCache::install_optimistic_root(
            Value::Object(dict),
            cache_key,
            write.store_in_memory,
            write.persist_disk,
            write.disk_cache_policy,
            write.sync_disk_with_optimistic,
            cx,
        )?;
        Ok(rollback)
    }

    fn merge_server_slot(
        row: &mut Map<String, Value>,
        path_in_temp_data: &str,
        index: usize,
        server_slot: &Map<String, Value>,
    ) -> Result<(), SaiWsOptimisticError> {
        let segments: Vec<String> = path_in_temp_data
            .split('.')
            .map(|segment| trimming_whitespaces(segment).to_owned())
            .filter(|segment| !segment.is_empty())
            .collect();
        if segments.is_empty() {
            return Err(SaiWsOptimisticError::FileUploadPathInTempDataInvalid(path_in_temp_data.to_owned()));
        }

        fn dive(
            cur: &mut Map<String, Value>,
            segments: &[String],
            seg_index: usize,
            index: usize,
            server_slot: &Map<String, Value>,
        ) -> Result<(), SaiWsOptimisticError> {
            let key = &segments[seg_index];
            if seg_index == segments.len() - 1 {
                let raw = SaiWsJsonObjectBridge::any_array(cur.get(key))
                    .ok_or_else(|| SaiWsOptimisticError::FileUploadPathInTempDataInvalid(key.clone()))?;
                let mut arr: Vec<Map<String, Value>> = raw
                    .iter()
                    .filter_map(|item| SaiWsJsonObjectBridge::dictionary(Some(item)))
                    .collect();
                if arr.is_empty() {
                    return Err(SaiWsOptimisticError::FileUploadPathInTempDataInvalid(key.clone()));
                }
                let count = arr.len();
                let Some(cell) = arr.get_mut(index) else {
                    return Err(SaiWsOptimisticError::FileUploadSlotIndexOutOfRange { index, count });
                };
                for (k, v) in server_slot {
                    cell.insert(k.clone(), v.clone());
                }
                cur.insert(key.clone(), Value::Array(arr.into_iter().map(Value::Object).collect()));
                return Ok(());
            }
            let mut nested = SaiWsJsonObjectBridge::dictionary(cur.get(key))
                .ok_or_else(|| SaiWsOptimisticError::FileUploadPathInTempDataInvalid(key.clone()))?;
            dive(&mut nested, segments, seg_index + 1, index, server_slot)?;
            cur.insert(key.clone(), Value::Object(nested));
            Ok(())
        }

        dive(row, &segments, 0, index, server_slot)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_patch_mutation(
        cache_key: &str,
        path_to_array: &str,
        row_id_key: &str,
        match_id: &str,
        outbound_body: &Value,
        create_patch: &SaiWsCreatePatch,
        create_rollback_snapshot: &SaiWsCreatePatch,
        options: &SaiWsRequestOptions,
        cx: &mut App,
    ) -> anyhow::Result<Map<String, Value>> {
        let mut captured_rollback: Map<String, Value> = Map::new();
        Self::mutate_array(cache_key, path_to_array, CacheWrite::from_options(options), cx, &mut |rows, cx| {
            rows.into_iter()
                .map(|row| -> anyhow::Result<Value> {
                    if !Self::row_matches_id(&row, match_id, row_id_key) {
                        return Ok(row);
                    }
                    let Some(mut dict) = SaiWsJsonObjectBridge::dictionary(Some(&row)) else { return Ok(row) };
                    if captured_rollback.is_empty() {
                        captured_rollback = create_rollback_snapshot(outbound_body, &row, cx)?;
                    }
                    let patch = create_patch(outbound_body, &row, cx)?;
                    for (k, v) in patch {
                        dict.insert(k, v);
                    }
                    Ok(Value::Object(dict))
                })
                .collect()
        })?;
        Ok(captured_rollback)
    }

    fn replace_temp_with_real(
        root: &Value,
        session: &Session,
        server_payload: Option<&Value>,
        cx: &mut App,
    ) -> anyhow::Result<Value> {
        let ctx = SaiWsOptimisticSuccessContext {
            temp_id: session.temp_id.clone(),
            temp_id_key: session.temp_id_key.clone(),
            temp_flag_key: session.temp_flag_key.clone(),
        };
        let real_obj = match (&session.extract_real_data, server_payload) {
            (Some(extract), Some(sp)) => Self::normalize_to_json_object(&extract(sp, &ctx, cx)?)?,
            (None, Some(sp)) => Self::normalize_to_json_object(sp)?,
            (_, None) => {
                return Self::transform_array(root, &session.path_to_array, cx, &mut |rows, _cx| {
                    Ok(rows
                        .into_iter()
                        .filter(|row| !Self::is_temp_row(row, &session.temp_id, &session.temp_id_key))
                        .collect())
                });
            }
        };

        Self::transform_array(root, &session.path_to_array, cx, &mut |rows, cx| {
            Ok(rows
                .into_iter()
                .map(|row| {
                    if !Self::is_temp_row(&row, &session.temp_id, &session.temp_id_key) {
                        return row;
                    }
                    if let (Some(matches), Some(sp)) = (&session.match_temp_with_real, server_payload)
                        && !matches(&row, sp, cx)
                    {
                        return row;
                    }
                    if let Some(old) = SaiWsJsonObjectBridge::dictionary(Some(&row)) {
                        return Value::Object(Self::merge_real_over_temp(
                            &old,
                            &real_obj,
                            &session.extract_ignore_keys,
                            &session.temp_flag_key,
                        ));
                    }
                    let mut r = real_obj.clone();
                    r.insert(session.temp_flag_key.clone(), Value::Bool(false));
                    Value::Object(r)
                })
                .collect())
        })
    }

    fn merge_real_over_temp(
        old: &Map<String, Value>,
        real: &Map<String, Value>,
        ignore_keys: &[String],
        temp_flag_key: &str,
    ) -> Map<String, Value> {
        let preserve_client_id = GetSaiServerId::is_temp_id(Self::string_value(old.get(bot::ID)).as_deref());
        let mut patch = SaiWsOptimisticIgnoreKeys::apply(real, old, ignore_keys);

        let old_media = SaiWsJsonObjectBridge::any_array(old.get(bot::MEDIA_ITEMS)).unwrap_or_default();
        if !old_media.is_empty() {
            patch.insert(bot::MEDIA_ITEMS.to_owned(), Value::Array(old_media));
        }

        let mut out = old.clone();
        for (k, v) in patch {
            if k == bot::ID && preserve_client_id {
                continue;
            }
            out.insert(k, v);
        }
        if preserve_client_id {
            let wire_id = GetSaiServerId::from_dictionary(real, &GetSaiServerIdOptions::default());
            if !wire_id.is_empty() && !GetSaiServerId::is_temp_id(Some(&wire_id)) {
                out.insert(RiseIdentityCanon::OPTIMISTIC_SERVER_IDENTIFIER_KEYS[0].to_owned(), Value::String(wire_id));
            }
        }
        if let Some(ts) = Self::normalized_timestamp(real).or_else(|| Self::normalized_timestamp(old)) {
            let ts = Value::from(ts);
            out.insert(bot::CREATED_AT.to_owned(), ts.clone());
            out.insert(bot::TIMESTAMP.to_owned(), ts);
        }
        out.insert(temp_flag_key.to_owned(), Value::Bool(false));
        out
    }

    fn normalized_timestamp(row: &Map<String, Value>) -> Option<f64> {
        for key in [events::CREATED_AT, bot::TIMESTAMP] {
            if let Some(d) = row.get(key).and_then(Value::as_f64)
                && d > 0.0
            {
                return Some(if d > 1_000_000_000_000.0 { d / 1000.0 } else { d });
            }
        }
        None
    }

    fn string_value(any: Option<&Value>) -> Option<String> {
        match any? {
            Value::String(s) => {
                let t = trimming_whitespaces_and_newlines(s);
                (!t.is_empty()).then(|| t.to_owned())
            }
            other => Self::ns_number_string_value(other),
        }
    }

    fn ns_number_string_value(value: &Value) -> Option<String> {
        match value {
            Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    return Some(n.to_string());
                }
                let f = n.as_f64()?;
                Some(if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", f as i64) } else { format!("{f}") })
            }
            Value::Bool(b) => Some(if *b { "1" } else { "0" }.to_owned()),
            _ => None,
        }
    }

    fn is_temp_row(row: &Value, temp_id: &str, temp_id_key: &str) -> bool {
        SaiWsJsonObjectBridge::dictionary(Some(row))
            .is_some_and(|d| d.get(temp_id_key).and_then(Value::as_str) == Some(temp_id))
    }

    fn insert_row(
        rows: Vec<Value>,
        row: &Value,
        strategy: SaiWsOptimisticAddStrategy,
        insert_after_last_temp: bool,
        temp_flag_key: &str,
    ) -> Vec<Value> {
        let mut rows = rows;
        if insert_after_last_temp {
            let last = rows.iter().rposition(|r| {
                SaiWsJsonObjectBridge::dictionary(Some(r))
                    .is_some_and(|d| d.get(temp_flag_key).and_then(Value::as_bool) == Some(true))
            });
            if let Some(last) = last {
                rows.insert(last + 1, row.clone());
                return rows;
            }
        }
        match strategy {
            SaiWsOptimisticAddStrategy::Start => rows.insert(0, row.clone()),
            SaiWsOptimisticAddStrategy::End => rows.push(row.clone()),
        }
        rows
    }

    fn row_matches_id(row: &Value, id: &str, id_key: &str) -> bool {
        let Some(d) = SaiWsJsonObjectBridge::dictionary(Some(row)) else { return false };
        match d.get(id_key) {
            Some(Value::String(s)) => s == id,
            Some(other) => Self::ns_number_string_value(other).is_some_and(|s| s == id),
            None => false,
        }
    }

    fn normalize_to_json_object(v: &Value) -> Result<Map<String, Value>, SaiWsOptimisticError> {
        SaiWsJsonObjectBridge::dictionary(Some(v)).ok_or(SaiWsOptimisticError::CreateTempDataFailed)
    }

    fn mutate_array(
        cache_key: &str,
        path_to_array: &str,
        write: CacheWrite,
        cx: &mut App,
        map: &mut RowsMap<'_>,
    ) -> anyhow::Result<()> {
        let root = SaiWsResponseCache::current_payload(cache_key, write.preferred_layer, write.disk_cache_policy, cx)
            .ok_or(SaiWsOptimisticError::ApplyMutationFailed)?;
        let next = Self::transform_array(&root, path_to_array, cx, map)?;
        SaiWsResponseCache::install_optimistic_root(
            next,
            cache_key,
            write.store_in_memory,
            write.persist_disk,
            write.disk_cache_policy,
            write.sync_disk_with_optimistic,
            cx,
        )?;
        Ok(())
    }

    fn transform_array(
        root: &Value,
        path_to_array: &str,
        cx: &mut App,
        map: &mut RowsMap<'_>,
    ) -> anyhow::Result<Value> {
        let parts: Vec<String> = path_to_array
            .split('.')
            .map(|part| trimming_whitespaces(part).to_owned())
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            return Err(SaiWsOptimisticError::PathToArrayRequired.into());
        }
        Self::transform_array_parts(root, &parts, cx, map)
    }

    fn transform_array_parts(
        root: &Value,
        path_parts: &[String],
        cx: &mut App,
        map: &mut RowsMap<'_>,
    ) -> anyhow::Result<Value> {
        let mut dict = SaiWsJsonObjectBridge::dictionary(Some(root)).ok_or(SaiWsOptimisticError::NotJsonObject)?;
        let head = &path_parts[0];
        if path_parts.len() == 1 {
            let arr = SaiWsJsonObjectBridge::any_array(dict.get(head))
                .ok_or_else(|| SaiWsOptimisticError::NotArrayAtPath(head.clone()))?;
            dict.insert(head.clone(), Value::Array(map(arr, cx)?));
            return Ok(Value::Object(dict));
        }
        let child = dict.get(head).ok_or_else(|| SaiWsOptimisticError::JsonPathNotFound(head.clone()))?;
        let next = Self::transform_array_parts(child, &path_parts[1..], cx, map)?;
        dict.insert(head.clone(), next);
        Ok(Value::Object(dict))
    }
}
