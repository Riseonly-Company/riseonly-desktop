use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{BoxFuture, LocalBoxFuture};
use gpui::{App, AsyncApp};
use rise_canon::RiseUploadFormat;
use rise_canon::rise_key::bot;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::file_manager::FileManager;
use serde_json::{Map, Value};

use super::sai_ws_chunked_file_upload::{
    SaiWsChunkedFileUpload, SaiWsUploadOnByteProgress, SaiWsUploadOnProgress,
};
use super::sai_ws_optimistic_coordinator::SaiWsOptimisticCoordinator;
use super::sai_ws_optimistic_update::{SaiWsOptimisticError, SaiWsOptimisticFiles, SaiWsOptimisticLocalFile};
use super::sai_ws_outbound_cancellation::{SaiWsOutboundCancellation, SaiWsOutboundCancelledError};
use super::sai_ws_request_options::SaiWsRequestOptions;
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai_auth_hooks::{SaiAuthNotSignedIn, sai_auth_hooks};
use crate::core::lib::sai::sai_image_disk_cache_hooks::{
    sai_clever_image_disk_prefetch_schedule, sai_image_disk_cache_hooks,
};
use crate::core::lib::sai::sai_outgoing_media_hooks::{
    OutgoingMediaCompressorResult, OutgoingMediaIsCancelled, OutgoingMediaProgress,
    sai_outgoing_media_compress_if_needed,
};
use crate::core::lib::sai::sai_remote_media_url_hooks::sai_remote_media_url_optional_normalized;

const IMAGE_CONTENT_TYPE_PREFIX: &str = "image/";
const SECURE_REMOTE_URL_PREFIX: &str = "https://";
const FALLBACK_MEDIA_STEM: &str = "media";
const FALLBACK_FILENAME: &str = "file";

/// account a send was issued for, carried into the async upload
#[derive(Clone)]
struct SaiWsUploadAccount {
    user_id: Option<String>,
    epoch: Option<String>,
}

impl SaiWsUploadAccount {
    fn of(options: &SaiWsRequestOptions) -> Self {
        Self { user_id: options.account_user_id.clone(), epoch: options.account_epoch.clone() }
    }

    fn is_current(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(self.user_id.as_deref(), self.epoch.as_deref())
    }

    fn ensure_current(&self) -> anyhow::Result<()> {
        if self.is_current() { Ok(()) } else { Err(SaiWsOutboundCancelledError.into()) }
    }
}

/// runs file uploads for optimistic sends
pub enum SaiWsOptimisticFileUploadRunner {}

impl SaiWsOptimisticFileUploadRunner {
    pub fn has_files_to_upload(options: &SaiWsRequestOptions) -> bool {
        let Some(files) = options.optimistic_update.as_ref().and_then(|opt| opt.files.as_ref()) else {
            return false;
        };
        files.enabled && !files.local_files.is_empty()
    }

    pub fn prepare_authentication_if_needed(
        options: &SaiWsRequestOptions,
        cx: &mut App,
    ) -> LocalBoxFuture<'static, anyhow::Result<()>> {
        if !Self::has_files_to_upload(options) {
            return Box::pin(async { Ok(()) });
        }
        let account = SaiWsUploadAccount::of(options);
        if let Err(error) = account.ensure_current() {
            return Box::pin(async move { Err(error) });
        }
        let refresh: LocalBoxFuture<'static, anyhow::Result<()>> = match sai_auth_hooks() {
            Some(hooks) => (hooks.ensure_valid_access_token)(cx),
            None => Box::pin(async { Err(anyhow::Error::new(SaiAuthNotSignedIn)) }),
        };
        Box::pin(async move {
            refresh.await?;
            account.ensure_current()
        })
    }

    pub fn merge_uploaded_files_if_needed(
        data_value: Value,
        insert_temp_id: Option<String>,
        options: SaiWsRequestOptions,
        cx: &mut App,
    ) -> LocalBoxFuture<'static, anyhow::Result<Value>> {
        let cx = cx.to_async();
        Box::pin(async move { Self::merge_uploaded_files(data_value, insert_temp_id, options, cx).await })
    }

    async fn merge_uploaded_files(
        data_value: Value,
        insert_temp_id: Option<String>,
        options: SaiWsRequestOptions,
        cx: AsyncApp,
    ) -> anyhow::Result<Value> {
        let Some(opt) = options.optimistic_update.clone().filter(|opt| opt.enabled) else {
            return Ok(data_value);
        };
        let Some(files) = opt.files.clone().filter(|files| files.enabled && !files.local_files.is_empty())
        else {
            return Ok(data_value);
        };

        let mut body =
            AnyDictionaryBridge::dictionary(Some(&data_value)).ok_or(SaiWsOptimisticError::FileUploadNeedsDictionaryBody)?;

        let ck = options.cache_key.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("").to_owned();
        let path_arr = options.path_to_array.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("").to_owned();
        let path_slots = trimming_whitespaces_and_newlines(&files.path_in_temp_data).to_owned();
        let can_patch_cache =
            !ck.is_empty() && !path_arr.is_empty() && insert_temp_id.is_some() && !path_slots.is_empty();
        let account = SaiWsUploadAccount::of(&options);

        let mut uploaded: Vec<Value> = Vec::with_capacity(files.local_files.len());

        cx.update(|cx| Self::prepare_authentication_if_needed(&options, cx)).await?;

        for (index, lf) in files.local_files.iter().enumerate() {
            let mut generated_temporary_url: Option<PathBuf> = None;
            let attempt: anyhow::Result<()> = async {
                Self::ensure_upload_still_current(&account, insert_temp_id.as_deref(), index)?;
                let premium = cx.update(|cx| sai_auth_hooks().is_some_and(|hooks| (hooks.is_premium)(cx)));
                let compressed = Self::compress(lf, &files, premium, index, &account, &cx).await?;
                if compressed.url.standardized() != lf.url.standardized() {
                    generated_temporary_url = compressed.url.to_file_path();
                }
                Self::ensure_upload_still_current(&account, insert_temp_id.as_deref(), index)?;
                if account.is_current()
                    && let Some(on_ready) = &files.on_compressed_ready
                {
                    cx.update(|cx| on_ready(index, &compressed, cx));
                }
                Self::ensure_upload_still_current(&account, insert_temp_id.as_deref(), index)?;
                let prepared_local_file = Self::prepared_local_file(lf, &compressed);
                let upload_context = lf.file_context_override.clone().unwrap_or_else(|| files.file_context.clone());

                let on_progress = Self::fraction_forwarder(&files, index, &account);
                let on_byte_progress = Self::byte_forwarder(&files, index, &account);
                let resource_id = Self::resolved_resource_id(&upload_context, files.resource_id.as_deref(), &body);
                let upload_path = compressed
                    .url
                    .to_file_path()
                    .ok_or_else(|| anyhow::anyhow!("compressed upload is not a local file"))?;
                let upload = cx.update(|cx| {
                    SaiWsChunkedFileUpload::upload(
                        upload_path,
                        prepared_local_file.filename.clone(),
                        compressed.content_type.clone(),
                        upload_context.clone(),
                        resource_id.clone(),
                        account.user_id.clone(),
                        account.epoch.clone(),
                        on_progress,
                        on_byte_progress,
                        cx,
                    )
                });
                let done = upload.await?;
                let done = Self::completion_metadata_with_prepared_fallbacks(done, &compressed);
                Self::ensure_upload_still_current(&account, insert_temp_id.as_deref(), index)?;
                let seed = cx.update(|cx| {
                    Self::seed_prepared_image_cache_if_possible(
                        &compressed,
                        &done,
                        account.user_id.clone(),
                        account.epoch.clone(),
                        cx,
                    )
                });
                seed.await;
                Self::ensure_upload_still_current(&account, insert_temp_id.as_deref(), index)?;

                let slot_any = match &files.extract_uploaded_files {
                    Some(extract) => cx.update(|cx| extract(&done, index, &prepared_local_file, cx))?,
                    None => Value::Object(done.clone()),
                };
                let slot_obj = Self::normalize_slot_to_dictionary(&slot_any)?;

                if can_patch_cache && let Some(tid) = insert_temp_id.as_deref() {
                    cx.update(|cx| {
                        SaiWsOptimisticCoordinator::merge_uploaded_file_into_cached_temp_row(
                            &ck,
                            &path_arr,
                            tid,
                            &opt.temp_id_key,
                            &path_slots,
                            index,
                            &slot_obj,
                            &options,
                            cx,
                        )
                    })?;
                }

                uploaded.push(slot_any);
                account.ensure_current()?;
                cx.update(|cx| {
                    if !files.prefetch_clever_image_disk_keys.is_empty() {
                        let keys = &files.prefetch_clever_image_disk_keys;
                        sai_clever_image_disk_prefetch_schedule(&Value::Object(done.clone()), keys, cx);
                        sai_clever_image_disk_prefetch_schedule(&Value::Object(slot_obj.clone()), keys, cx);
                    }
                    if let Some(on_success) = &files.on_file_success {
                        on_success(index, &done, cx);
                    }
                });
                anyhow::Ok(())
            }
            .await;

            let Err(error) = attempt else { continue };
            let slot_cancelled = error.is::<SaiWsOutboundSlotCancelledError>();
            let request_cancelled = error.is::<SaiWsOutboundCancelledError>();
            if (slot_cancelled || request_cancelled)
                && let Some(path) = &generated_temporary_url
            {
                let _ = FileManager.remove_item(path);
            }
            if slot_cancelled {
                continue;
            }
            if request_cancelled || !account.is_current() {
                return Err(SaiWsOutboundCancelledError.into());
            }
            if let Some(on_error) = &files.on_file_error {
                cx.update(|cx| on_error(index, &error, cx));
            }
            return Err(error);
        }

        if insert_temp_id.is_some() && uploaded.is_empty() && !files.local_files.is_empty() {
            return Err(SaiWsOutboundCancelledError.into());
        }

        account.ensure_current()?;
        let finalized = match &files.finalize_uploaded_files {
            Some(finalize) => cx.update(|cx| finalize(uploaded, cx))?,
            None => uploaded,
        };
        account.ensure_current()?;
        let outbound_finalized: Vec<Value> = finalized
            .into_iter()
            .map(|value| match AnyDictionaryBridge::dictionary(Some(&value)) {
                Some(slot) => Value::Object(Self::sanitized_slot_for_outbound(&slot)),
                None => value,
            })
            .collect();

        if files.merge_uploaded_slots_into_body_root {
            for item in &outbound_finalized {
                let Some(dict) = AnyDictionaryBridge::dictionary(Some(item)) else { continue };
                for (key, value) in dict {
                    body.insert(key, value);
                }
            }
        } else if !files.files_param_key.is_empty() {
            body.insert(files.files_param_key.clone(), Value::Array(outbound_finalized));
        }
        Ok(Value::Object(body))
    }

    async fn compress(
        lf: &SaiWsOptimisticLocalFile,
        files: &SaiWsOptimisticFiles,
        premium: bool,
        index: usize,
        account: &SaiWsUploadAccount,
        cx: &AsyncApp,
    ) -> anyhow::Result<OutgoingMediaCompressorResult> {
        let (progress_tx, mut progress_rx) = mpsc::unbounded::<f64>();
        let on_progress: OutgoingMediaProgress = Arc::new(move |p: f64| {
            let _ = progress_tx.unbounded_send(p);
        });
        let forward = files.on_compression_progress.clone();
        let progress_account = account.clone();
        let _pump = cx.spawn(async move |cx| {
            while let Some(p) = progress_rx.next().await {
                if progress_account.is_current()
                    && let Some(forward) = &forward
                {
                    cx.update(|cx| forward(index, p, cx));
                }
            }
        });
        let is_cancelled: OutgoingMediaIsCancelled = Arc::new(|| -> BoxFuture<'static, bool> { Box::pin(async { false }) });
        sai_outgoing_media_compress_if_needed(
            lf.clone(),
            files.compression.for_tier(premium),
            Some(on_progress),
            is_cancelled,
        )
        .await
    }

    fn fraction_forwarder(
        files: &SaiWsOptimisticFiles,
        index: usize,
        account: &SaiWsUploadAccount,
    ) -> Option<SaiWsUploadOnProgress> {
        let forward = files.on_progress.clone()?;
        let account = account.clone();
        Some(Rc::new(move |p: f64, cx: &mut App| {
            if account.is_current() {
                forward(index, p, cx);
            }
        }))
    }

    fn byte_forwarder(
        files: &SaiWsOptimisticFiles,
        index: usize,
        account: &SaiWsUploadAccount,
    ) -> Option<SaiWsUploadOnByteProgress> {
        let forward = files.on_byte_progress.clone()?;
        let account = account.clone();
        Some(Rc::new(move |completed_bytes: i64, total_bytes: i64, cx: &mut App| {
            if account.is_current() {
                forward(index, completed_bytes, total_bytes, cx);
            }
        }))
    }

    fn resolved_resource_id(
        context: &str,
        explicit_resource_id: Option<&str>,
        body: &Map<String, Value>,
    ) -> Option<String> {
        if let Some(explicit) = Self::normalized_identifier(explicit_resource_id) {
            return Some(explicit);
        }
        if trimming_whitespaces_and_newlines(context).to_lowercase() != rise_canon::RiseUploadContextFormats::CHAT.context {
            return None;
        }
        Self::normalized_identifier(body.get(bot::CHAT_ID).and_then(Value::as_str))
    }

    fn normalized_identifier(value: Option<&str>) -> Option<String> {
        let normalized = trimming_whitespaces_and_newlines(value?);
        (!normalized.is_empty()).then(|| normalized.to_owned())
    }

    /// copy of the slot for sending, strips local file urls
    /// the ui slot keeps them so playback reuses the prepared file
    pub fn sanitized_slot_for_outbound(slot: &Map<String, Value>) -> Map<String, Value> {
        let mut outbound = slot.clone();
        outbound.remove(RiseLocalKey::LOCAL_PREVIEW_URL);
        outbound.remove(&rise_canon::RiseWireKey::camel(RiseLocalKey::LOCAL_PREVIEW_URL));
        outbound
    }

    fn ensure_upload_still_current(
        account: &SaiWsUploadAccount,
        insert_temp_id: Option<&str>,
        file_index: usize,
    ) -> anyhow::Result<()> {
        account.ensure_current()?;
        let Some(insert_temp_id) = insert_temp_id else { return Ok(()) };
        if SaiWsOutboundCancellation::is_request_cancelled(insert_temp_id) {
            return Err(SaiWsOutboundCancelledError.into());
        }
        if SaiWsOutboundCancellation::is_slot_cancelled(insert_temp_id, file_index) {
            return Err(SaiWsOutboundSlotCancelledError.into());
        }
        Ok(())
    }

    fn normalize_slot_to_dictionary(value: &Value) -> Result<Map<String, Value>, SaiWsOptimisticError> {
        AnyDictionaryBridge::dictionary(Some(value)).ok_or(SaiWsOptimisticError::FileUploadInvalidSlotPayload)
    }

    pub fn prepared_local_file(
        original: &SaiWsOptimisticLocalFile,
        compressed: &OutgoingMediaCompressorResult,
    ) -> SaiWsOptimisticLocalFile {
        let mut prepared = original.clone();
        let was_transformed = compressed.url.standardized() != original.url.standardized();
        let is_document = original
            .preferred_media_type
            .as_deref()
            .is_some_and(|media_type| trimming_whitespaces_and_newlines(media_type).to_lowercase() == bot::DOCUMENT);
        if was_transformed && !is_document {
            prepared.filename = Self::upload_filename(&original.filename, &compressed.content_type);
        }
        prepared.url = compressed.url.clone();
        prepared.content_type = compressed.content_type.clone();
        prepared
    }

    pub fn seed_prepared_image_cache_if_possible(
        compressed: &OutgoingMediaCompressorResult,
        completion: &Map<String, Value>,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut App,
    ) -> LocalBoxFuture<'static, ()> {
        let raw_remote_url = Self::normalized_string(completion.get(bot::FILE_URL))
            .or_else(|| Self::normalized_string(completion.get(bot::URL)));
        let Some(remote_url) = raw_remote_url
            .filter(|_| compressed.content_type.starts_with(IMAGE_CONTENT_TYPE_PREFIX))
            .and_then(|raw| sai_remote_media_url_optional_normalized(Some(&raw)))
            .filter(|url| url.to_lowercase().starts_with(SECURE_REMOTE_URL_PREFIX))
        else {
            return Box::pin(async {});
        };
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return Box::pin(async {});
        }
        let Some(hooks) = sai_image_disk_cache_hooks() else { return Box::pin(async {}) };
        let Some(source_path) = compressed.url.to_file_path() else { return Box::pin(async {}) };
        let invalidation_token = (hooks.invalidation_token)(account_user_id.as_deref());
        let task = cx.background_executor().spawn(async move {
            let is_current = || {
                SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
            };
            if !is_current() {
                return;
            }
            let Some(data) = Self::read_file(&source_path).filter(|data| !data.is_empty()) else { return };
            if !is_current() {
                return;
            }
            (hooks.store)(data, &remote_url, account_user_id.as_deref(), Some(invalidation_token));
        });
        Box::pin(task)
    }

    fn read_file(path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    pub fn completion_metadata_with_prepared_fallbacks(
        completion: Map<String, Value>,
        compressed: &OutgoingMediaCompressorResult,
    ) -> Map<String, Value> {
        let mut enriched = completion;
        if Self::positive_integer(enriched.get(bot::WIDTH)).is_none()
            && let Some(width) = compressed.pixel_width.filter(|width| *width > 0)
        {
            enriched.insert(bot::WIDTH.to_owned(), Value::from(width));
        }
        if Self::positive_integer(enriched.get(bot::HEIGHT)).is_none()
            && let Some(height) = compressed.pixel_height.filter(|height| *height > 0)
        {
            enriched.insert(bot::HEIGHT.to_owned(), Value::from(height));
        }
        if Self::positive_integer(enriched.get(bot::FILE_SIZE)).is_none() && compressed.byte_count > 0 {
            enriched.insert(bot::FILE_SIZE.to_owned(), Value::from(compressed.byte_count));
        }
        if Self::normalized_string(enriched.get(bot::MIME_TYPE)).is_none() {
            enriched.insert(bot::MIME_TYPE.to_owned(), Value::String(compressed.content_type.clone()));
        }
        enriched
    }

    fn upload_filename(original: &str, content_type: &str) -> String {
        let path = trimming_whitespaces_and_newlines(original);
        let stem = Path::new(path).with_extension("").to_string_lossy().into_owned();
        let safe_stem = if stem.is_empty() { FALLBACK_MEDIA_STEM.to_owned() } else { stem };
        let content_type = content_type.to_lowercase();
        if content_type == RiseUploadFormat::IMAGE_JPEG.mime {
            return format!("{safe_stem}.{}", RiseUploadFormat::IMAGE_JPEG.extension);
        }
        if content_type == RiseUploadFormat::VIDEO_MP4.mime {
            return format!("{safe_stem}.{}", RiseUploadFormat::VIDEO_MP4.extension);
        }
        if path.is_empty() { FALLBACK_FILENAME.to_owned() } else { path.to_owned() }
    }

    /// positive int from a json value, fractions are cut and bools read as 1 and 0
    fn positive_integer(value: Option<&Value>) -> Option<i64> {
        let result = match value? {
            Value::Number(number) => number.as_i64().or_else(|| number.as_f64().map(|f| f as i64))?,
            Value::Bool(flag) => i64::from(*flag),
            Value::String(string) => string.parse::<i64>().ok()?,
            _ => return None,
        };
        (result > 0).then_some(result)
    }

    fn normalized_string(value: Option<&Value>) -> Option<String> {
        let normalized = trimming_whitespaces_and_newlines(value?.as_str()?);
        (!normalized.is_empty()).then(|| normalized.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("outbound file slot cancelled")]
struct SaiWsOutboundSlotCancelledError;
