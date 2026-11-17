use std::path::PathBuf;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use gpui::{AsyncApp, Context};
use parking_lot::Mutex;
use rise_canon::RiseUploadContextFormats;
use rise_canon::rise_key::bot as bot_key;
use rise_canon::rise_key::message as message_key;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_chunked_upload::{
    RiseChunkedUploadProgressHandler, RiseChunkedUploadRequest, RiseChunkedUploadTransferProgress,
};
use rise_engine::rise_upload::rise_upload_progress::RiseUploadCancellationSignal;
use rise_platform::file_manager::FileManager;
use serde_json::{Map, Value};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_updater::sai_updater::SaiUpdaterWriteScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::lib::sai::sai_outgoing_media_hooks::{OutgoingMediaIsCancelled, OutgoingMediaProgress};
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::chat::engine::messages::rise_message_media_upload_pipeline::{
    RiseDeferredImageReeditDocument, RiseDeferredPhotoImageSource,
};
use crate::modules::chat::stores::message::message_actions::message_types::MessageEditMediaSnapshot;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    EditMessageGatewayDTO, MessagePayloadCodec,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;

const EDITED_MEDIA_TYPE: &str = "image";
const EDITED_MEDIA_PNG_CONTENT_TYPE: &str = "image/png";
const EDITED_MEDIA_JPEG_CONTENT_TYPE: &str = "image/jpeg";
const EDIT_DATE_MILLISECONDS_THRESHOLD: f64 = 1_000_000_000_000.0;

pub type MessageEditUploadProgress = Arc<dyn Fn(f64) -> BoxFuture<'static, ()> + Send + Sync>;

struct RiseEditUploadProgressRelay {
    continuation: Mutex<Option<watch::Sender<f64>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl RiseEditUploadProgressRelay {
    fn new(
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        consumer: MessageEditUploadProgress,
    ) -> Self {
        let (sender, mut receiver) = watch::channel(0.0_f64);
        let worker = RiseAccountRuntime::current().spawn(async move {
            while receiver.changed().await.is_ok() {
                let fraction = *receiver.borrow_and_update();
                if !SaiAccountCacheScope::is_current_account_epoch(
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                ) {
                    continue;
                }
                consumer(fraction).await;
            }
        });
        Self { continuation: Mutex::new(Some(sender)), worker: Mutex::new(Some(worker)) }
    }

    fn publish(&self, fraction: f64) {
        if let Some(sender) = self.continuation.lock().as_ref() {
            sender.send_replace(fraction.clamp(0.0, 1.0));
        }
    }

    async fn finish(&self) {
        self.continuation.lock().take();
        let worker = self.worker.lock().take();
        if let Some(worker) = worker {
            let _ = worker.await;
        }
    }
}

struct RiseEditGeneratedFileGuard(Option<PathBuf>);

impl Drop for RiseEditGeneratedFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            remove_edit_generated_file(path);
        }
    }
}

fn remove_edit_generated_file(path: PathBuf) {
    drop(RiseAccountRuntime::current().spawn_blocking(move || {
        let _ = FileManager.remove_item(&path);
    }));
}

pub struct RiseEditedMediaUploadResult {
    pub slot: Map<String, Value>,
    pub deferred_reedit_document: Option<RiseDeferredImageReeditDocument>,
}

impl MessageServicesStore {
    pub fn apply_edit_optimistic(
        &mut self,
        message_id: &str,
        new_content: &str,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) {
        let edited_at = RiseClock::now().time_interval_since_1970();
        self.mutate_messages_list(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |messages| {
                if let Some(message) = messages.iter_mut().find(|message| message.matches_message_id(message_id)) {
                    message.apply_edited_content(Some(new_content), edited_at);
                }
            },
            cx,
        );
    }

    pub fn edit_message_success_handler(&mut self, any: Option<&Value>, chat_id: &str, cx: &mut Context<Self>) {
        let Some(dto) = MessagePayloadCodec::decode::<EditMessageGatewayDTO>(any) else {
            return;
        };
        let Some(mid) = dto.message_id else {
            return;
        };
        let Some(edit_date) = dto.edit_date.map(|edit_date| edit_date as f64) else {
            return;
        };

        self.mutate_messages_list(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |messages| {
                let Some(message) = messages.iter_mut().find(|message| message.id == mid) else {
                    return;
                };
                if message.location.as_ref().is_some_and(|location| location.live()) {
                    return;
                }
                message.edited_at = Some(if edit_date > EDIT_DATE_MILLISECONDS_THRESHOLD {
                    edit_date / 1000.0
                } else {
                    edit_date
                });
            },
            cx,
        );
    }

    pub fn edit_message_error_handler(
        &mut self,
        error: &anyhow::Error,
        chat_id: &str,
        message_id: &str,
        cx: &mut Context<Self>,
    ) {
        Sai::log(
            "Message",
            || format!("edit_message({chat_id}/{message_id}) error: {error}"),
            SaiLogKind::Network,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_edit_media_optimistic(
        &mut self,
        message_id: &str,
        caption: &str,
        local_file_url: &Url,
        width: i64,
        height: i64,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) {
        let now_ms = RiseClock::now().time_interval_since_1970() * 1000.0;
        let mut snapshot = None;
        self.direct_patch_message_row(
            chat_id,
            message_id,
            SaiUpdaterWriteScope::Cache,
            true,
            None,
            |row| {
                let prev_media = MessagePayloadCodec::array_of_dictionaries(row.get(bot_key::MEDIA_ITEMS));
                let slot_id = prev_media
                    .first()
                    .and_then(|media| media.get(bot_key::ID))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{message_id}-0"));
                snapshot = Some(MessageEditMediaSnapshot {
                    media_items: prev_media,
                    content: row.get(bot_key::CONTENT).and_then(Value::as_str).map(str::to_owned),
                    edit_date: row.get(bot_key::EDIT_DATE).cloned(),
                    original_content: row.get(bot_key::ORIGINAL_CONTENT).and_then(Value::as_str).map(str::to_owned),
                });
                let file_url = local_file_url.absolute_string();
                let mut slot = Map::from_iter([
                    (bot_key::ID.to_owned(), Value::from(slot_id)),
                    (bot_key::MEDIA_TYPE.to_owned(), Value::from(EDITED_MEDIA_TYPE)),
                    (bot_key::FILE_URL.to_owned(), Value::from(file_url)),
                    (bot_key::THUMBNAIL_URL.to_owned(), Value::from(file_url)),
                    (RiseLocalKey::IS_UPLOADING.to_owned(), Value::from(true)),
                    (RiseLocalKey::UPLOAD_PROGRESS.to_owned(), Value::from(0)),
                    (bot_key::MIME_TYPE.to_owned(), Value::from(EDITED_MEDIA_PNG_CONTENT_TYPE)),
                ]);
                if width > 0 {
                    slot.insert(bot_key::WIDTH.to_owned(), Value::from(width));
                }
                if height > 0 {
                    slot.insert(bot_key::HEIGHT.to_owned(), Value::from(height));
                }
                if !row.contains_key(bot_key::ORIGINAL_CONTENT) {
                    let original =
                        row.get(bot_key::CONTENT).and_then(Value::as_str).map(str::to_owned).unwrap_or_default();
                    row.insert(bot_key::ORIGINAL_CONTENT.to_owned(), Value::from(original));
                }
                row.insert(bot_key::CONTENT.to_owned(), Value::from(caption));
                row.insert(bot_key::EDIT_DATE.to_owned(), Value::from(now_ms));
                row.insert(bot_key::MEDIA_ITEMS.to_owned(), Value::Array(vec![Value::Object(slot)]));
                row.insert(bot_key::HAS_ATTACHMENTS.to_owned(), Value::from(true));
            },
            cx,
        );
        if let Some(snapshot) = snapshot {
            self.edit_media_snapshots.insert(message_id.to_owned(), snapshot);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn upload_edited_media(
        item: &MediaPickerItem,
        chat_id: &str,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cancellation_signal: RiseUploadCancellationSignal,
        is_cancelled: OutgoingMediaIsCancelled,
        on_progress: MessageEditUploadProgress,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<RiseEditedMediaUploadResult> {
        Self::validate_edit_upload_cancellation(&is_cancelled).await?;
        RiseConversationUploadSupport::prepare_authentication(account_user_id.clone(), account_epoch.clone(), cx)
            .await?;
        Self::validate_edit_upload_cancellation(&is_cancelled).await?;
        let progress_relay = Arc::new(RiseEditUploadProgressRelay::new(
            account_user_id.clone(),
            account_epoch.clone(),
            on_progress,
        ));
        let on_materialization_progress: OutgoingMediaProgress = {
            let progress_relay = progress_relay.clone();
            Arc::new(move |progress: f64| progress_relay.publish(progress * 0.075))
        };
        let materialized = match RiseDeferredPhotoImageSource::materialize_if_needed(
            Some(item),
            true,
            on_materialization_progress,
            is_cancelled.clone(),
        )
        .await
        {
            Ok(materialized) => materialized,
            Err(error) => {
                progress_relay.finish().await;
                return Err(error);
            }
        };
        let is_materialized = materialized.is_some();
        let (materialized_url, generated_temporary_url, reedit_document) = match materialized {
            Some(source) => (Some(source.url), source.generated_temporary_url, source.reedit_document),
            None => (None, None, None),
        };
        let _materialized_temporary_file = RiseEditGeneratedFileGuard(generated_temporary_url);
        let original_image_url = reedit_document.as_ref().map(|document| document.original_image_url.clone());

        let outcome: anyhow::Result<RiseEditedMediaUploadResult> = async {
            let local_url = match materialized_url {
                Some(url) => url,
                None => item
                    .url
                    .to_file_path()
                    .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?,
            };
            let content_type =
                if is_materialized { EDITED_MEDIA_JPEG_CONTENT_TYPE } else { EDITED_MEDIA_PNG_CONTENT_TYPE };
            let local =
                RiseOutgoingMediaFile::new(local_url, item.suggested_file_name.clone(), content_type.to_owned());
            let compression_progress_start = if is_materialized { 0.075 } else { 0.0 };
            let compression_progress_scale = if is_materialized { 0.075 } else { 0.15 };
            let on_compression_progress: OutgoingMediaProgress = {
                let progress_relay = progress_relay.clone();
                Arc::new(move |progress: f64| {
                    progress_relay.publish(compression_progress_start + progress * compression_progress_scale);
                })
            };
            let compressed = OutgoingMediaCompressor::compress_if_needed(
                local.clone(),
                None,
                MediaCompressionConfiguration::chat(),
                Some(on_compression_progress),
                Some(is_cancelled.clone()),
            )
            .await?;
            let compressed_path = compressed
                .url
                .to_file_path()
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
            let _compressed_temporary_file =
                RiseEditGeneratedFileGuard((compressed_path != local.url).then(|| compressed_path.clone()));
            RiseConversationUploadSupport::validate(account_user_id.as_deref(), account_epoch.as_deref())?;
            Self::validate_edit_upload_cancellation(&is_cancelled).await?;
            let prepared = RiseConversationUploadSupport::prepared_local_file(&local, &compressed);
            let client = {
                let is_cancelled = is_cancelled.clone();
                let account_user_id = account_user_id.clone();
                let account_epoch = account_epoch.clone();
                cx.update(move |cx| {
                    RiseConversationUploadSupport::make_client(
                        account_user_id,
                        account_epoch,
                        Some(Arc::new(move || {
                            let is_cancelled = is_cancelled.clone();
                            async move { Self::validate_edit_upload_cancellation(&is_cancelled).await }.boxed()
                        })),
                        cx,
                    )
                })
            };
            let mut request = RiseChunkedUploadRequest::new(
                compressed_path,
                prepared.filename.clone(),
                compressed.content_type.clone(),
                RiseUploadContextFormats::CHAT.context.to_owned(),
            );
            request.resource_id = Some(trimming_whitespaces_and_newlines(chat_id).to_owned());
            let on_upload_progress: RiseChunkedUploadProgressHandler = {
                let progress_relay = progress_relay.clone();
                Arc::new(move |progress: RiseChunkedUploadTransferProgress| {
                    progress_relay.publish(0.15 + (progress.fraction.max(0.0) * 0.85).min(0.85));
                })
            };
            let result = client.upload(request, Some(cancellation_signal), None, Some(on_upload_progress)).await?;
            RiseConversationUploadSupport::validate(account_user_id.as_deref(), account_epoch.as_deref())?;
            Self::validate_edit_upload_cancellation(&is_cancelled).await?;
            RiseConversationUploadSupport::seed_prepared_image_cache_if_possible(
                &compressed,
                &result.file_url,
                account_user_id.clone(),
                account_epoch.clone(),
            )
            .await;
            RiseConversationUploadSupport::seed_prepared_video_cache_if_possible(
                &compressed,
                &result.file_url,
                account_user_id.clone(),
                account_epoch.clone(),
            )
            .await;
            Self::validate_edit_upload_cancellation(&is_cancelled).await?;
            let completion = RiseConversationUploadSupport::completion_dictionary(&result, &compressed);
            let slot = cx.update(|cx| {
                let services = Self::shared(cx);
                services.read(cx).extract_uploaded_media_slot(&completion, &prepared, Some(item), None)
            });
            Ok(RiseEditedMediaUploadResult { slot, deferred_reedit_document: reedit_document })
        }
        .await;

        if outcome.is_err()
            && let Some(original_image_url) = original_image_url
        {
            remove_edit_generated_file(original_image_url);
        }
        progress_relay.finish().await;
        outcome
    }

    async fn validate_edit_upload_cancellation(is_cancelled: &OutgoingMediaIsCancelled) -> anyhow::Result<()> {
        if is_cancelled().await {
            return Err(CancellationError.into());
        }
        Ok(())
    }

    pub fn edit_media_success_handler(
        &mut self,
        any: Option<&Value>,
        message_id: &str,
        chat_id: &str,
        uploaded_slot: Map<String, Value>,
        cx: &mut Context<Self>,
    ) {
        let dto = MessagePayloadCodec::decode::<EditMessageGatewayDTO>(any);
        let mut slot = uploaded_slot;
        slot.insert(RiseLocalKey::IS_UPLOADING.to_owned(), Value::from(false));
        slot.insert(RiseLocalKey::UPLOAD_PROGRESS.to_owned(), Value::from(100));
        self.direct_patch_message_row(
            chat_id,
            message_id,
            SaiUpdaterWriteScope::Cache,
            true,
            None,
            |row| {
                if slot.get(bot_key::ID).and_then(Value::as_str).is_none()
                    && let Some(existing_id) = MessagePayloadCodec::array_of_dictionaries(row.get(bot_key::MEDIA_ITEMS))
                        .first()
                        .and_then(|media| media.get(bot_key::ID))
                        .and_then(Value::as_str)
                {
                    slot.insert(bot_key::ID.to_owned(), Value::from(existing_id));
                }
                row.insert(bot_key::MEDIA_ITEMS.to_owned(), Value::Array(vec![Value::Object(slot)]));
                row.insert(bot_key::HAS_ATTACHMENTS.to_owned(), Value::from(true));
                if let Some(edit_date) = dto.and_then(|dto| dto.edit_date) {
                    row.insert(bot_key::EDIT_DATE.to_owned(), Value::from(edit_date));
                }
            },
            cx,
        );
        self.edit_media_snapshots.remove(message_id);
    }

    pub fn edit_media_error_handler(
        &mut self,
        error: &anyhow::Error,
        message_id: &str,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) {
        Sai::log(
            "Message",
            || format!("edit_message_media({chat_id}/{message_id}) revert: {error}"),
            SaiLogKind::Network,
            cx,
        );
        self.revert_edit_media(message_id, chat_id, cx);
    }

    fn revert_edit_media(&mut self, message_id: &str, chat_id: &str, cx: &mut Context<Self>) {
        let Some(snap) = self.edit_media_snapshots.get(message_id).cloned() else {
            return;
        };
        self.direct_patch_message_row(
            chat_id,
            message_id,
            SaiUpdaterWriteScope::Cache,
            true,
            None,
            |row| {
                row.insert(bot_key::HAS_ATTACHMENTS.to_owned(), Value::from(!snap.media_items.is_empty()));
                row.insert(
                    bot_key::MEDIA_ITEMS.to_owned(),
                    Value::Array(snap.media_items.into_iter().map(Value::Object).collect()),
                );
                if let Some(content) = snap.content {
                    row.insert(bot_key::CONTENT.to_owned(), Value::from(content));
                }
                if let Some(edit_date) = snap.edit_date {
                    row.insert(bot_key::EDIT_DATE.to_owned(), edit_date);
                } else {
                    row.remove(message_key::EDIT_DATE);
                }
                if let Some(original) = snap.original_content {
                    row.insert(bot_key::ORIGINAL_CONTENT.to_owned(), Value::from(original));
                } else {
                    row.remove(message_key::ORIGINAL_CONTENT);
                }
            },
            cx,
        );
        self.edit_media_snapshots.remove(message_id);
    }
}
