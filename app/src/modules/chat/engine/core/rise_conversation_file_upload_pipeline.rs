use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp};
use rise_canon::rise_key::bot;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_chunked_upload::{
    RiseChunkedUploadProgressHandler, RiseChunkedUploadRequest, RiseChunkedUploadTransferProgress,
};
use rise_platform::file_manager::FileManager;
use serde_json::{Map, Value};
use tokio::task::AbortHandle;

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::{SaiWsOptimisticError, SaiWsOptimisticLocalFile};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsRequestOptions;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskPrefetch;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;

const CHAT_UPLOAD_CONTEXT: &str = "chat";

pub enum RiseConversationFileUploadPipeline {}

struct GeneratedTemporaryFile(Option<Url>);

impl Drop for GeneratedTemporaryFile {
    fn drop(&mut self) {
        let Some(path) = self.0.take().and_then(|url| url.to_file_path()) else {
            return;
        };
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let _ = FileManager.remove_item(&path);
        }));
    }
}

struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl RiseConversationFileUploadPipeline {
    pub fn has_files(options: &SaiWsRequestOptions) -> bool {
        let Some(files) = options.optimistic_update.as_ref().and_then(|update| update.files.as_ref()) else {
            return false;
        };
        files.enabled && !files.local_files.is_empty()
    }

    pub async fn prepare(
        data_value: Value,
        options: &SaiWsRequestOptions,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Value> {
        let Some(optimistic) = options.optimistic_update.as_ref().filter(|optimistic| optimistic.enabled) else {
            return Ok(data_value);
        };
        let Some(files) = optimistic.files.as_ref().filter(|files| files.enabled && !files.local_files.is_empty())
        else {
            return Ok(data_value);
        };
        let Some(mut body) = AnyDictionaryBridge::dictionary(Some(&data_value)) else {
            return Err(SaiWsOptimisticError::FileUploadNeedsDictionaryBody.into());
        };

        let account_user_id = options.account_user_id.clone();
        let account_epoch = options.account_epoch.clone();
        RiseConversationUploadSupport::prepare_authentication(account_user_id.clone(), account_epoch.clone(), cx)
            .await?;
        let client = cx.update(|cx| {
            RiseConversationUploadSupport::make_client(account_user_id.clone(), account_epoch.clone(), None, cx)
        });
        let mut uploaded: Vec<Value> = Vec::with_capacity(files.local_files.len());

        for (index, local_file) in files.local_files.iter().enumerate() {
            let mut generated_temporary_file = GeneratedTemporaryFile(None);
            let attempt = async {
                let Some(source_path) = local_file.url.to_file_path() else {
                    return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
                };
                let mut rise_local_file = RiseOutgoingMediaFile::new(
                    source_path,
                    local_file.filename.clone(),
                    local_file.content_type.clone(),
                );
                rise_local_file.file_context = local_file.file_context_override.clone();
                rise_local_file.media_type = local_file.preferred_media_type.clone();
                rise_local_file.byte_count = local_file.byte_count;
                RiseConversationUploadSupport::validate(account_user_id.as_deref(), account_epoch.as_deref())?;

                let premium = cx.update(|cx| {
                    AuthServicesStore::shared(cx)
                        .read(cx)
                        .profile()
                        .is_some_and(|profile| profile.more.is_premium)
                });
                let compression_progress = {
                    let on_compression_progress = files.on_compression_progress.clone();
                    Self::main_thread_relay(
                        cx,
                        account_user_id.clone(),
                        account_epoch.clone(),
                        move |fraction: f64, cx: &mut App| {
                            if let Some(on_compression_progress) = &on_compression_progress {
                                on_compression_progress(index, fraction, cx);
                            }
                        },
                    )
                };
                let on_compression_progress: Arc<dyn Fn(f64) + Send + Sync> = Arc::new(move |fraction: f64| {
                    let _ = compression_progress.unbounded_send(fraction);
                });
                let compressed = OutgoingMediaCompressor::compress_if_needed(
                    rise_local_file.clone(),
                    None,
                    files.compression.for_tier(premium),
                    Some(on_compression_progress),
                    None,
                )
                .await?;
                if compressed.url.standardized() != local_file.url.standardized() {
                    generated_temporary_file.0 = Some(compressed.url.clone());
                }
                RiseConversationUploadSupport::validate(account_user_id.as_deref(), account_epoch.as_deref())?;
                if let Some(on_compressed_ready) = &files.on_compressed_ready {
                    cx.update(|cx| on_compressed_ready(index, &compressed, cx));
                }

                let prepared = RiseConversationUploadSupport::prepared_local_file(&rise_local_file, &compressed);
                let upload_context =
                    local_file.file_context_override.clone().unwrap_or_else(|| files.file_context.clone());
                let Some(compressed_path) = compressed.url.to_file_path() else {
                    return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
                };
                let mut request = RiseChunkedUploadRequest::new(
                    compressed_path,
                    prepared.filename.clone(),
                    compressed.content_type.clone(),
                    upload_context.clone(),
                );
                request.resource_id =
                    Self::resolved_resource_id(&upload_context, files.resource_id.as_deref(), &body);
                let transfer_progress = {
                    let on_progress = files.on_progress.clone();
                    let on_byte_progress = files.on_byte_progress.clone();
                    Self::main_thread_relay(
                        cx,
                        account_user_id.clone(),
                        account_epoch.clone(),
                        move |progress: RiseChunkedUploadTransferProgress, cx: &mut App| {
                            if let Some(on_progress) = &on_progress {
                                on_progress(index, progress.fraction, cx);
                            }
                            if let Some(on_byte_progress) = &on_byte_progress {
                                on_byte_progress(index, progress.completed_bytes, progress.total_bytes, cx);
                            }
                        },
                    )
                };
                let on_transfer_progress: RiseChunkedUploadProgressHandler =
                    Arc::new(move |progress: RiseChunkedUploadTransferProgress| {
                        let _ = transfer_progress.unbounded_send(progress);
                    });
                let result = client.upload(request, None, None, Some(on_transfer_progress)).await?;
                RiseConversationUploadSupport::validate(account_user_id.as_deref(), account_epoch.as_deref())?;
                let completion = RiseConversationUploadSupport::completion_dictionary(&result, &compressed);
                let seed = {
                    let compressed = compressed.clone();
                    let remote_url = result.file_url.clone();
                    let account_user_id = account_user_id.clone();
                    let account_epoch = account_epoch.clone();
                    RiseAccountRuntime::current().spawn(async move {
                        RiseConversationUploadSupport::seed_prepared_image_cache_if_possible(
                            &compressed,
                            &remote_url,
                            account_user_id.clone(),
                            account_epoch.clone(),
                        )
                        .await;
                        RiseConversationUploadSupport::seed_prepared_video_cache_if_possible(
                            &compressed,
                            &remote_url,
                            account_user_id,
                            account_epoch,
                        )
                        .await;
                    })
                };
                let seed_abort = AbortOnDrop(seed.abort_handle());
                let _ = seed.await;
                drop(seed_abort);

                let slot = match &files.extract_uploaded_files {
                    Some(extract) => {
                        let prepared_file = SaiWsOptimisticLocalFile {
                            url: Url::from_file_path(&prepared.url).unwrap_or_else(|| compressed.url.clone()),
                            filename: prepared.filename.clone(),
                            content_type: prepared.content_type.clone(),
                            file_context_override: prepared.file_context.clone(),
                            preferred_media_type: prepared.media_type.clone(),
                            byte_count: prepared.byte_count,
                        };
                        cx.update(|cx| extract(&completion, index, &prepared_file, cx))?
                    }
                    None => Value::Object(completion.clone()),
                };
                if AnyDictionaryBridge::dictionary(Some(&slot)).is_none() {
                    return Err(SaiWsOptimisticError::FileUploadInvalidSlotPayload.into());
                }
                cx.update(|cx| {
                    if !files.prefetch_clever_image_disk_keys.is_empty() {
                        CleverImageDiskPrefetch::schedule_values_from(
                            &Value::Object(completion.clone()),
                            &files.prefetch_clever_image_disk_keys,
                            cx,
                        );
                        CleverImageDiskPrefetch::schedule_values_from(
                            &slot,
                            &files.prefetch_clever_image_disk_keys,
                            cx,
                        );
                    }
                    if let Some(on_file_success) = &files.on_file_success {
                        on_file_success(index, &completion, cx);
                    }
                });
                uploaded.push(slot);
                Ok::<(), anyhow::Error>(())
            }
            .await;
            drop(generated_temporary_file);

            if let Err(error) = attempt {
                let is_current = SaiAccountCacheScope::is_current_account_epoch(
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                );
                if !is_current || error.is::<CancellationError>() {
                    return Err(CancellationError.into());
                }
                if let Some(on_file_error) = &files.on_file_error {
                    cx.update(|cx| on_file_error(index, &error, cx));
                }
                return Err(error);
            }
        }

        RiseConversationUploadSupport::validate(account_user_id.as_deref(), account_epoch.as_deref())?;
        let finalized = match &files.finalize_uploaded_files {
            Some(finalize) => cx.update(|cx| finalize(uploaded, cx))?,
            None => uploaded,
        };
        let outbound: Vec<Value> = finalized
            .into_iter()
            .map(|value| match AnyDictionaryBridge::dictionary(Some(&value)) {
                Some(slot) => Value::Object(RiseConversationUploadSupport::sanitized_slot_for_outbound(&slot)),
                None => value,
            })
            .collect();
        if files.merge_uploaded_slots_into_body_root {
            for value in &outbound {
                let Some(slot) = AnyDictionaryBridge::dictionary(Some(value)) else {
                    continue;
                };
                for (key, field) in slot {
                    body.insert(key, field);
                }
            }
        } else if !files.files_param_key.is_empty() {
            body.insert(files.files_param_key.clone(), Value::Array(outbound));
        }
        Ok(Value::Object(body))
    }

    /// relays each progress report to the main thread, compressor and uploader report from other threads
    /// loop ends with the sender
    fn main_thread_relay<T: Send + 'static>(
        cx: &AsyncApp,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        deliver: impl Fn(T, &mut App) + 'static,
    ) -> mpsc::UnboundedSender<T> {
        let (sender, mut receiver) = mpsc::unbounded::<T>();
        cx.spawn(async move |cx| {
            while let Some(value) = receiver.next().await {
                if SaiAccountCacheScope::is_current_account_epoch(
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                ) {
                    cx.update(|cx| deliver(value, cx));
                }
            }
        })
        .detach();
        sender
    }

    fn resolved_resource_id(
        context: &str,
        explicit_resource_id: Option<&str>,
        body: &Map<String, Value>,
    ) -> Option<String> {
        if let Some(explicit) = Self::normalized_identifier(explicit_resource_id) {
            return Some(explicit);
        }
        if trimming_whitespaces_and_newlines(context).to_lowercase() != CHAT_UPLOAD_CONTEXT {
            return None;
        }
        Self::normalized_identifier(body.get(bot::CHAT_ID).and_then(Value::as_str))
    }

    fn normalized_identifier(value: Option<&str>) -> Option<String> {
        let normalized = trimming_whitespaces_and_newlines(value?);
        if normalized.is_empty() { None } else { Some(normalized.to_owned()) }
    }
}
