use std::rc::Rc;

use gpui::App;
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationMerger, SaiWsPaginationOptions, SaiWsPaginationPageKind,
    SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::{
    SaiWsCachedContext, SaiWsColdDiskLoaded, SaiWsResponseCache,
};

const LEGACY_RELATIVE_ID_PATH: &str = "_sai_legacy_relative_id";
const HYDRATE_SERVICE: &str = "_";
const HYDRATE_METHOD: &str = "_hydrate";
const PATH_SEPARATOR: &str = ".";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiUpdaterWriteScope {
    State,
    Cache,
    DiskCache,
    All,
}

impl SaiUpdaterWriteScope {
    pub const ALL_CASES: [SaiUpdaterWriteScope; 4] = [Self::State, Self::Cache, Self::DiskCache, Self::All];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Cache => "cache",
            Self::DiskCache => "diskCache",
            Self::All => "all",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|scope| scope.raw_value() == raw)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SaiUpdaterError {
    #[error("SaiUpdater requires a non-empty cacheKey.")]
    EmptyCacheKey,
    #[error("SaiUpdater transform must return UTF-8 JSON bytes (object, array, or fragment).")]
    TransformProducedInvalidJSON,
    #[error("SaiUpdater root must be a JSON object.")]
    NotJSONObject,
    #[error("SaiUpdater value at path is not an array: {0}")]
    NotArrayAtPath(String),
    #[error("SaiUpdater JSON path not found: {0}")]
    JsonPathNotFound(String),
}

pub enum SaiUpdater {}

impl SaiUpdater {
    pub fn update(
        cache_key: &str,
        transform: impl FnOnce(&SaiWsCachedContext) -> anyhow::Result<Vec<u8>>,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        Self::update_json_value(
            cache_key,
            |context| {
                let data = transform(context)?;
                serde_json::from_slice::<Value>(&data)
                    .map_err(|_| anyhow::Error::new(SaiUpdaterError::TransformProducedInvalidJSON))
            },
            write_to,
            cx,
        )
    }

    /// rewrites a cached json value, any result is valid json here so nothing gets rejected
    pub fn update_json_value(
        cache_key: &str,
        transform: impl FnOnce(&SaiWsCachedContext) -> anyhow::Result<Value>,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let ck = trimming_whitespaces_and_newlines(cache_key);
        if ck.is_empty() {
            return Err(anyhow::Error::new(SaiUpdaterError::EmptyCacheKey));
        }

        let ctx = SaiWsResponseCache::make_cached_context(ck, SaiWsDiskCachePolicy::PreserveExisting, cx);
        let value = transform(&ctx)?;

        SaiWsResponseCache::apply_sai_updater_payload(value, ck, write_to, false, cx)
    }

    pub fn patch_root(
        cache_key: &str,
        fields: &Map<String, Value>,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        Self::update_json_value(
            cache_key,
            |ctx| {
                let mut root = Self::root(ctx);
                for (k, v) in fields {
                    root.insert(k.clone(), v.clone());
                }
                Ok(Value::Object(root))
            },
            write_to,
            cx,
        )
    }

    pub fn replace_root(
        cache_key: &str,
        root: Map<String, Value>,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        Self::update_json_value(cache_key, |_| Ok(Value::Object(root)), write_to, cx)
    }

    pub fn merge_paginated_page(
        cache_key: &str,
        path_to_array: &str,
        path_to_id: Option<&str>,
        page_payload: &Value,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        Self::update_json_value(
            cache_key,
            |ctx| {
                let options = SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                    relative_id: None,
                    page_kind: Some(SaiWsPaginationPageKind::Continuation),
                    response_relative_id_path: LEGACY_RELATIVE_ID_PATH.to_owned(),
                    has_more_path: None,
                    ..Default::default()
                });
                let result = SaiWsPaginationMerger::merge(
                    ctx.payload.as_ref(),
                    Some(page_payload),
                    &options,
                    path_to_array,
                    path_to_id,
                )?;
                Ok(Value::Object(result.root))
            },
            write_to,
            cx,
        )
    }

    pub fn patch_row(
        cache_key: &str,
        path_to_array: &str,
        path_to_id: &str,
        match_id: &str,
        patch: &Map<String, Value>,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        Self::update_json_value(
            cache_key,
            |ctx| {
                let mut root = Self::root(ctx);
                let Some(rows) = Self::array_of_any(Self::read_array(&root, path_to_array)) else {
                    return Err(anyhow::Error::new(SaiUpdaterError::NotArrayAtPath(
                        path_to_array.to_owned(),
                    )));
                };
                let rows: Vec<Value> = rows
                    .into_iter()
                    .map(|row| {
                        if Self::row_id(&row, path_to_id).as_deref() != Some(match_id) {
                            return row;
                        }
                        let Some(mut dict) = AnyDictionaryBridge::dictionary(Some(&row)) else {
                            return row;
                        };
                        for (k, v) in patch {
                            dict.insert(k.clone(), v.clone());
                        }
                        Value::Object(dict)
                    })
                    .collect();
                Self::write_array(&mut root, path_to_array, rows);
                Ok(Value::Object(root))
            },
            write_to,
            cx,
        )
    }

    pub fn remove_row(
        cache_key: &str,
        path_to_array: &str,
        path_to_id: &str,
        match_id: &str,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        Self::update_json_value(
            cache_key,
            |ctx| {
                let mut root = Self::root(ctx);
                let Some(rows) = Self::array_of_any(Self::read_array(&root, path_to_array)) else {
                    return Err(anyhow::Error::new(SaiUpdaterError::NotArrayAtPath(
                        path_to_array.to_owned(),
                    )));
                };
                let rows: Vec<Value> = rows
                    .into_iter()
                    .filter(|row| Self::row_id(row, path_to_id).as_deref() != Some(match_id))
                    .collect();
                Self::write_array(&mut root, path_to_array, rows);
                Ok(Value::Object(root))
            },
            write_to,
            cx,
        )
    }

    pub fn insert_row_if_missing(
        cache_key: &str,
        path_to_array: &str,
        path_to_id: &str,
        row: Map<String, Value>,
        at_start: bool,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let row = Value::Object(row);
        let id = Self::row_id(&row, path_to_id).unwrap_or_default();
        Self::update_json_value(
            cache_key,
            |ctx| {
                let mut root = Self::root(ctx);
                let mut rows =
                    Self::array_of_any(Self::read_array(&root, path_to_array)).unwrap_or_default();
                if !id.is_empty()
                    && rows
                        .iter()
                        .any(|existing| Self::row_id(existing, path_to_id).as_deref() == Some(id.as_str()))
                {
                    return Ok(Value::Object(root));
                }
                if at_start {
                    rows.insert(0, row);
                } else {
                    rows.push(row);
                }
                Self::write_array(&mut root, path_to_array, rows);
                Ok(Value::Object(root))
            },
            write_to,
            cx,
        )
    }

    pub fn hydrate_slice_from_disk<T: DeserializeOwned + Clone + 'static>(
        slice: &Rc<SaiTypedSlice<T>>,
        cache_key: &str,
        take_path: Option<&str>,
        cx: &mut App,
    ) {
        if slice.data().is_some() {
            return;
        }
        let key = trimming_whitespaces_and_newlines(cache_key);
        if key.is_empty() {
            return;
        }
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: HYDRATE_SERVICE.to_owned(),
            method: HYDRATE_METHOD.to_owned(),
            cache_key: Some(key.to_owned()),
            store_response_in_cache: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            ..SaiWsRequestOptionsInit::default()
        });
        let weak_slice = Rc::downgrade(slice);
        let cold_take_path = take_path.map(str::to_owned);
        let on_cold_disk_loaded: SaiWsColdDiskLoaded = Box::new(move |value: &Value, _cx: &mut App| {
            let Some(slice) = weak_slice.upgrade() else {
                return;
            };
            if slice.data().is_some() || value.is_null() {
                return;
            }
            slice.mark_fulfilled(Some(value), cold_take_path.as_deref());
        });
        let (raw, _, had_disk) = SaiWsResponseCache::peek(&options, Some(on_cold_disk_loaded), cx);
        if !had_disk {
            return;
        }
        let Some(raw) = raw.filter(|raw| !raw.is_null()) else {
            return;
        };
        slice.mark_fulfilled(Some(&raw), take_path);
    }

    fn root(ctx: &SaiWsCachedContext) -> Map<String, Value> {
        AnyDictionaryBridge::dictionary(ctx.payload.as_ref()).unwrap_or_default()
    }

    fn array_of_any(value: Option<&Value>) -> Option<Vec<Value>> {
        match value? {
            Value::Array(array) => Some(array.clone()),
            _ => None,
        }
    }

    /// row id as a string, a bool becomes 1 or 0
    fn row_id(row: &Value, key: &str) -> Option<String> {
        let d = AnyDictionaryBridge::dictionary(Some(row))?;
        match d.get(key)? {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(flag) => Some(if *flag { "1" } else { "0" }.to_owned()),
            _ => None,
        }
    }

    fn path_parts(path: &str) -> Vec<&str> {
        path.split(PATH_SEPARATOR)
            .map(trimming_whitespaces)
            .filter(|part| !part.is_empty())
            .collect()
    }

    fn read_array<'a>(root: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
        let parts = Self::path_parts(path);
        let (first, rest) = parts.split_first()?;
        let mut cur = root.get(*first)?;
        for part in rest {
            cur = cur.as_object()?.get(*part)?;
        }
        Some(cur)
    }

    fn write_array(root: &mut Map<String, Value>, path: &str, value: Vec<Value>) {
        let parts = Self::path_parts(path);
        let Some((first, rest)) = parts.split_first() else {
            return;
        };
        if rest.is_empty() {
            root.insert((*first).to_owned(), Value::Array(value));
            return;
        }
        let mut nested = AnyDictionaryBridge::dictionary(root.get(*first)).unwrap_or_default();
        Self::write_array(&mut nested, &rest.join(PATH_SEPARATOR), value);
        root.insert((*first).to_owned(), Value::Object(nested));
    }
}
