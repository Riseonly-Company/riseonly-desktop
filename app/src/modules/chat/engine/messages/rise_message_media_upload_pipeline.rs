use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use gpui::{AsyncApp, Size, size};
use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use parking_lot::Mutex;
use rise_canon::rise_upload::RealtimeStage as CanonRealtimeStage;
use rise_chat::MediaKind as ChatMediaKind;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_chunked_upload::{
    RiseChunkedUploadClient, RiseChunkedUploadProgressHandler, RiseChunkedUploadRequest, RiseChunkedUploadResult,
    RiseChunkedUploadSessionEvent, RiseChunkedUploadSessionEventHandler, RiseChunkedUploadStep,
    RiseChunkedUploadTransferProgress,
};
use rise_engine::rise_upload::rise_upload_progress::{
    RiseUploadCancellationHub, RiseUploadRealtimeProgress, RiseUploadRealtimeProgressHub,
    RiseUploadRealtimeProgressHubHandler,
};
use rise_media::video_export::{POSTER_MAX_SIDE, poster as video_poster, probe as video_probe};
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;
use rise_platform::photo_library::{
    self, ExportOutcome, ExportProgress, ImageContentMode, ImageDeliveryMode, ImageRequestId, PhotoLibraryUnavailable,
};
use tokio::task::{AbortHandle, JoinHandle};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai_outgoing_media_hooks::{
    OutgoingMediaCompressorResult, OutgoingMediaIsCancelled, OutgoingMediaProgress,
};
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapEncoder};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::chat::engine::messages::rise_message_contracts::{
    RiseMessageDraft, RiseMessageUploadEvent, RiseMessageUploadEvents, RiseMessageUploadPipeline,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_upload_retry::{
    RiseMessageUploadRetry, RiseMessageUploadRetryCodec,
};
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;
use crate::modules::chat::shared::utils::voice::voice_message_metadata::VoiceMessageMetadata;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    RiseStoredOutgoingMedia, RiseStoredOutgoingMessage,
};
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_state_codable::EditorStateCodec;
use crate::modules::media_picker::editor::media_reedit_store::MediaReeditStore;
use crate::modules::media_picker::export::media_exporter::MediaExporter;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::media_picker::library::photos_image_request::PhotosImageRequest;
use crate::modules::share_extension::shared::rise_outgoing_media::RiseOutgoingImageEncoder;

const DEFAULT_FILE_CONTEXT: &str = "chat";
const POSTER_CONTENT_TYPE: &str = "image/jpeg";
const IMAGE_CONTENT_TYPE_PREFIX: &str = "image/";
const VIDEO_CONTENT_TYPE_PREFIX: &str = "video/";
const AUDIO_CONTENT_TYPE_PREFIX: &str = "audio/";
const FILE_MEDIA_TYPE: &str = "file";
const FINALIZING_STAGE: &str = "finalizing";
const POSTER_FILE_PREFIX: &str = "ro-upload-poster-";
const PHASSET_FILE_PREFIX: &str = "ro-rise-phasset-";
const IMAGE_FETCH_PROGRESS_SHARE: f64 = 0.7;
const VIDEO_MATERIALIZATION_PROGRESS_WEIGHT: f64 = 0.5;
const IMAGE_MATERIALIZATION_PROGRESS_WEIGHT: f64 = 0.15;
const IMAGE_LOADED_PROGRESS: f64 = 0.72;
const IMAGE_RENDERED_PROGRESS: f64 = 0.9;
const VIDEO_LOADED_PROGRESS: f64 = 0.1;
const MATERIALIZED_JPEG_QUALITY: f64 = 0.95;
const PHOTOS_CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// main thread from engine code
/// started before any send so a missing hop means the app is going away
fn main_hop() -> anyhow::Result<RiseEngineMainHop> {
    RiseEngineMainHop::installed().ok_or_else(|| anyhow::Error::from(CancellationError))
}

fn write_data_atomically(destination: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut staging = destination.as_os_str().to_owned();
    staging.push(".tmp");
    let staging = PathBuf::from(staging);
    if let Err(error) = std::fs::write(&staging, data).and_then(|()| std::fs::rename(&staging, destination)) {
        let _ = std::fs::remove_file(&staging);
        return Err(error);
    }
    Ok(())
}

/// removes the generated file on drop, also when the future is dropped
struct RiseGeneratedFileGuard(Option<PathBuf>);

impl Drop for RiseGeneratedFileGuard {
    fn drop(&mut self) {
        let Some(path) = self.0.take() else {
            return;
        };
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let _ = FileManager.remove_item(&path);
        }));
    }
}

struct RiseAbortOnDrop(AbortHandle);

impl Drop for RiseAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RiseMessageUploadProgressRelayPhase {
    Preparing,
    Uploading,
    Processing,
    Completed,
}

struct RiseMessageUploadProgressRelayState {
    index: i64,
    highest_phase: RiseMessageUploadProgressRelayPhase,
    is_finished: bool,

    preparing_fraction: f64,
    preparing_total_bytes: i64,
    is_preparing_pending: bool,

    uploading_fraction: f64,
    uploading_completed_bytes: i64,
    uploading_total_bytes: i64,
    is_uploading_pending: bool,

    processing_fraction: f64,
    processing_total_bytes: i64,
    is_processing_pending: bool,

    completed_total_bytes: i64,
    is_completed_pending: bool,
}

impl RiseMessageUploadProgressRelayState {
    fn new(index: i64) -> Self {
        Self {
            index,
            highest_phase: RiseMessageUploadProgressRelayPhase::Preparing,
            is_finished: false,
            preparing_fraction: -1.0,
            preparing_total_bytes: 0,
            is_preparing_pending: false,
            uploading_fraction: -1.0,
            uploading_completed_bytes: 0,
            uploading_total_bytes: 0,
            is_uploading_pending: false,
            processing_fraction: -1.0,
            processing_total_bytes: 0,
            is_processing_pending: false,
            completed_total_bytes: 0,
            is_completed_pending: false,
        }
    }

    fn publish(&mut self, event: RiseMessageUploadEvent) -> bool {
        if self.is_finished {
            return false;
        }

        match event {
            RiseMessageUploadEvent::Preparing { index, fraction, total_bytes } => {
                if index != self.index || self.highest_phase > RiseMessageUploadProgressRelayPhase::Preparing {
                    return false;
                }
                self.highest_phase = RiseMessageUploadProgressRelayPhase::Preparing;
                let fraction = fraction.clamp(0.0, 1.0);
                let total_bytes = total_bytes.max(0);
                if !(fraction > self.preparing_fraction || total_bytes > self.preparing_total_bytes) {
                    return false;
                }
                self.preparing_fraction = self.preparing_fraction.max(fraction);
                self.preparing_total_bytes = self.preparing_total_bytes.max(total_bytes);
                self.is_preparing_pending = true;
                true
            }
            RiseMessageUploadEvent::Uploading { index, fraction, completed_bytes, total_bytes } => {
                if index != self.index || self.highest_phase > RiseMessageUploadProgressRelayPhase::Uploading {
                    return false;
                }
                self.highest_phase = RiseMessageUploadProgressRelayPhase::Uploading;
                let fraction = fraction.clamp(0.0, 1.0);
                let total_bytes = total_bytes.max(0);
                let completed_bytes = completed_bytes.max(0).min(total_bytes);
                if !(self.uploading_fraction < 0.0
                    || fraction > self.uploading_fraction
                    || completed_bytes > self.uploading_completed_bytes
                    || total_bytes > self.uploading_total_bytes)
                {
                    return false;
                }
                self.uploading_fraction = self.uploading_fraction.max(fraction);
                self.uploading_completed_bytes = self.uploading_completed_bytes.max(completed_bytes);
                self.uploading_total_bytes = self.uploading_total_bytes.max(total_bytes);
                self.is_uploading_pending = true;
                true
            }
            RiseMessageUploadEvent::Processing { index, fraction, total_bytes } => {
                if index != self.index || self.highest_phase > RiseMessageUploadProgressRelayPhase::Processing {
                    return false;
                }
                self.highest_phase = RiseMessageUploadProgressRelayPhase::Processing;
                let fraction = fraction.clamp(0.0, 1.0);
                let total_bytes = total_bytes.max(0);
                if !(self.processing_fraction < 0.0
                    || fraction > self.processing_fraction
                    || total_bytes > self.processing_total_bytes)
                {
                    return false;
                }
                self.processing_fraction = self.processing_fraction.max(fraction);
                self.processing_total_bytes = self.processing_total_bytes.max(total_bytes);
                self.is_processing_pending = true;
                true
            }
            RiseMessageUploadEvent::Completed { index, total_bytes } => {
                if index != self.index {
                    return false;
                }
                self.highest_phase = RiseMessageUploadProgressRelayPhase::Completed;
                let total_bytes = total_bytes.max(0);
                if self.is_completed_pending && total_bytes <= self.completed_total_bytes {
                    return false;
                }
                self.completed_total_bytes = self.completed_total_bytes.max(total_bytes);
                self.is_completed_pending = true;
                true
            }
        }
    }

    fn take_next(&mut self) -> Option<RiseMessageUploadEvent> {
        if self.is_preparing_pending {
            self.is_preparing_pending = false;
            return Some(RiseMessageUploadEvent::Preparing {
                index: self.index,
                fraction: self.preparing_fraction.max(0.0),
                total_bytes: self.preparing_total_bytes,
            });
        }
        if self.is_uploading_pending {
            self.is_uploading_pending = false;
            return Some(RiseMessageUploadEvent::Uploading {
                index: self.index,
                fraction: self.uploading_fraction.max(0.0),
                completed_bytes: self.uploading_completed_bytes,
                total_bytes: self.uploading_total_bytes,
            });
        }
        if self.is_processing_pending {
            self.is_processing_pending = false;
            return Some(RiseMessageUploadEvent::Processing {
                index: self.index,
                fraction: self.processing_fraction.max(0.0),
                total_bytes: self.processing_total_bytes,
            });
        }
        if self.is_completed_pending {
            self.is_completed_pending = false;
            return Some(RiseMessageUploadEvent::Completed {
                index: self.index,
                total_bytes: self.completed_total_bytes,
            });
        }
        None
    }

    fn finish(&mut self) -> bool {
        if self.is_finished {
            return false;
        }
        self.is_finished = true;
        true
    }
}

struct RiseMessageUploadProgressRelayStorage {
    state: Mutex<RiseMessageUploadProgressRelayState>,
}

impl RiseMessageUploadProgressRelayStorage {
    fn new(index: i64) -> Self {
        Self { state: Mutex::new(RiseMessageUploadProgressRelayState::new(index)) }
    }

    fn publish(&self, event: RiseMessageUploadEvent) -> bool {
        self.state.lock().publish(event)
    }

    fn take_next(&self) -> Option<RiseMessageUploadEvent> {
        self.state.lock().take_next()
    }

    fn finish(&self) -> bool {
        self.state.lock().finish()
    }
}

/// bridges sync photos and compression and transport callbacks into the async progress sink
/// no task per callback
/// one coalesced value per phase, drained in phase order
struct RiseMessageUploadProgressRelay {
    storage: Arc<RiseMessageUploadProgressRelayStorage>,
    continuation: Mutex<Option<mpsc::Sender<()>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl RiseMessageUploadProgressRelay {
    fn new(index: i64, consumer: RiseMessageUploadEvents) -> Self {
        let storage = Arc::new(RiseMessageUploadProgressRelayStorage::new(index));
        let (continuation, mut stream) = mpsc::channel::<()>(1);
        let worker_storage = storage.clone();
        let worker = RiseAccountRuntime::current().spawn(async move {
            while stream.next().await.is_some() {
                while let Some(event) = worker_storage.take_next() {
                    consumer(event).await;
                }
            }
            while let Some(event) = worker_storage.take_next() {
                consumer(event).await;
            }
        });
        Self { storage, continuation: Mutex::new(Some(continuation)), worker: Mutex::new(Some(worker)) }
    }

    fn publish(&self, event: RiseMessageUploadEvent) {
        if !self.storage.publish(event) {
            return;
        }
        if let Some(continuation) = self.continuation.lock().as_mut() {
            let _ = continuation.try_send(());
        }
    }

    fn close(&self) {
        if self.storage.finish() {
            self.continuation.lock().take();
        }
    }

    async fn finish(&self) {
        self.close();
        let worker = self.worker.lock().take();
        if let Some(worker) = worker {
            let _ = worker.await;
        }
    }
}

struct RiseMessageUploadProgressRelayCloseGuard(Arc<RiseMessageUploadProgressRelay>);

impl Drop for RiseMessageUploadProgressRelayCloseGuard {
    fn drop(&mut self) {
        self.0.close();
    }
}

struct RiseRealtimeBindingGuard {
    hub: Arc<RiseUploadRealtimeProgressHub>,
    upload_id: Arc<Mutex<Option<String>>>,
}

impl Drop for RiseRealtimeBindingGuard {
    fn drop(&mut self) {
        let Some(upload_id) = self.upload_id.lock().take() else {
            return;
        };
        let hub = self.hub.clone();
        drop(RiseAccountRuntime::current().spawn(async move {
            hub.unbind(&upload_id).await;
        }));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseMessageMediaUploadPipelineRealtimeStage {
    Uploading,
    Processing,
    Unknown,
}

#[derive(Clone)]
pub struct RiseMessageMediaUploadPipeline {
    local_files: Vec<RiseOutgoingMediaFile>,
    picker_items: Vec<MediaPickerItem>,
    voice_metadata_slots: Vec<VoiceMessageMetadata>,
    account_user_id: String,
    account_epoch: Option<String>,
    upload_realtime_progress: Arc<RiseUploadRealtimeProgressHub>,
}

impl RiseMessageUploadPipeline for RiseMessageMediaUploadPipeline {
    fn prepare(
        &self,
        draft: RiseMessageDraft,
        local_id: String,
        cancellation: Arc<RiseUploadCancellationHub>,
        events: RiseMessageUploadEvents,
    ) -> BoxFuture<'static, anyhow::Result<RiseStoredOutgoingMessage>> {
        let this = self.clone();
        async move {
            let prepared = this.prepare_operation(draft, &local_id, cancellation, events).await;
            if let Err(error) = &prepared {
                tracing::warn!(
                    target: "riseonly::upload",
                    "prepare failed local_id={local_id} files={} error={error:#}",
                    this.local_files.len()
                );
            }
            prepared
        }
        .boxed()
    }

    fn retry(&self, body: &RiseStoredOutgoingMessage, scope: &RiseMessageScope) -> Option<RiseMessageUploadRetry> {
        RiseMessageUploadRetryCodec::retry(
            body,
            scope,
            &self.local_files,
            &self.picker_items,
            &self.voice_metadata_slots,
        )
    }
}

impl RiseMessageMediaUploadPipeline {
    pub fn new(
        local_files: Vec<RiseOutgoingMediaFile>,
        picker_items: Vec<MediaPickerItem>,
        voice_metadata_slots: Vec<VoiceMessageMetadata>,
        account_user_id: String,
        account_epoch: Option<String>,
        upload_realtime_progress: Arc<RiseUploadRealtimeProgressHub>,
    ) -> Self {
        Self {
            local_files,
            picker_items,
            voice_metadata_slots,
            account_user_id,
            account_epoch,
            upload_realtime_progress,
        }
    }

    async fn prepare_operation(
        &self,
        draft: RiseMessageDraft,
        local_id: &str,
        cancellation: Arc<RiseUploadCancellationHub>,
        events: RiseMessageUploadEvents,
    ) -> anyhow::Result<RiseStoredOutgoingMessage> {
        Self::validate_cancellation(&cancellation, local_id, None).await?;
        let account_user_id = self.account_user_id.clone();
        let account_epoch = self.account_epoch.clone();
        main_hop()?
            .run(move |mut cx: AsyncApp| {
                async move {
                    RiseConversationUploadSupport::prepare_authentication(
                        Some(account_user_id),
                        account_epoch,
                        &mut cx,
                    )
                    .await
                }
                .boxed_local()
            })
            .await?;

        let mut uploaded_by_index: HashMap<i64, RiseStoredOutgoingMedia> =
            HashMap::with_capacity(self.local_files.len());

        let trimmed_chat_id = draft.body.chat_id.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default();
        let upload_resource_id = (!trimmed_chat_id.is_empty()).then(|| trimmed_chat_id.to_owned());

        for (index, local_file) in self.local_files.iter().enumerate() {
            if let Some(media_body) = self
                .upload_attachment(index, local_file, &draft, local_id, &cancellation, upload_resource_id.as_deref(), &events)
                .await?
            {
                uploaded_by_index.insert(index as i64, media_body);
            }
        }

        Self::validate_cancellation(&cancellation, local_id, None).await?;
        let cancelled_slot_indices = cancellation.cancelled_slot_indices(local_id).await;
        let uploaded = Self::finalized_media_bodies(
            uploaded_by_index,
            self.local_files.len() as i64,
            &self.picker_items,
            &cancelled_slot_indices,
        )?;

        let mut body = draft.body;
        body.media_items = Some(uploaded);
        Ok(body)
    }

    #[allow(clippy::too_many_arguments)]
    async fn upload_attachment(
        &self,
        index: usize,
        local_file: &RiseOutgoingMediaFile,
        draft: &RiseMessageDraft,
        local_id: &str,
        cancellation: &Arc<RiseUploadCancellationHub>,
        upload_resource_id: Option<&str>,
        events: &RiseMessageUploadEvents,
    ) -> anyhow::Result<Option<RiseStoredOutgoingMedia>> {
        let slot_index = index as i64;
        let progress_relay = Arc::new(RiseMessageUploadProgressRelay::new(slot_index, events.clone()));
        let _close_relay = RiseMessageUploadProgressRelayCloseGuard(progress_relay.clone());
        let attempt = self
            .attempt_attachment(index, local_file, draft, local_id, cancellation, upload_resource_id, &progress_relay)
            .await;
        progress_relay.finish().await;
        match attempt {
            Ok(body) => Ok(Some(body)),
            Err(error) if error.downcast_ref::<CancellationError>().is_some() => {
                if cancellation.is_cancelled(local_id, None).await {
                    return Err(CancellationError.into());
                }
                if cancellation.is_cancelled(local_id, Some(slot_index)).await {
                    return Ok(None);
                }
                Err(CancellationError.into())
            }
            Err(error) => Err(error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn attempt_attachment(
        &self,
        index: usize,
        local_file: &RiseOutgoingMediaFile,
        draft: &RiseMessageDraft,
        local_id: &str,
        cancellation: &Arc<RiseUploadCancellationHub>,
        upload_resource_id: Option<&str>,
        progress_relay: &Arc<RiseMessageUploadProgressRelay>,
    ) -> anyhow::Result<RiseStoredOutgoingMedia> {
        let slot_index = index as i64;
        let account_user_id = self.account_user_id.as_str();
        let account_epoch = self.account_epoch.as_deref();
        let hop = main_hop()?;
        let picker_item = self.picker_items.get(index);
        Self::validate_cancellation(cancellation, local_id, Some(slot_index)).await?;
        RiseConversationUploadSupport::validate(Some(account_user_id), account_epoch)?;
        progress_relay.publish(RiseMessageUploadEvent::Preparing { index: slot_index, fraction: 0.0, total_bytes: 0 });

        let is_context_cancelled: OutgoingMediaIsCancelled = {
            let cancellation = cancellation.clone();
            let owner_id = local_id.to_owned();
            let account_user_id = self.account_user_id.clone();
            let account_epoch = self.account_epoch.clone();
            Arc::new(move || -> BoxFuture<'static, bool> {
                let cancellation = cancellation.clone();
                let owner_id = owner_id.clone();
                let account_user_id = account_user_id.clone();
                let account_epoch = account_epoch.clone();
                async move {
                    if cancellation.is_cancelled(&owner_id, Some(slot_index)).await {
                        return true;
                    }
                    RiseConversationUploadSupport::validate(Some(account_user_id.as_str()), account_epoch.as_deref())
                        .is_err()
                }
                .boxed()
            })
        };
        let has_deferred_image_source =
            picker_item.is_some_and(|item| item.deferred_photo_library_image_asset_identifier().is_some());
        let has_deferred_video_source =
            picker_item.is_some_and(|item| item.deferred_photo_library_video_asset_identifier().is_some());
        let materialization_progress_weight = if has_deferred_video_source {
            VIDEO_MATERIALIZATION_PROGRESS_WEIGHT
        } else if has_deferred_image_source {
            IMAGE_MATERIALIZATION_PROGRESS_WEIGHT
        } else {
            0.0
        };
        let on_materialization_progress: OutgoingMediaProgress = {
            let progress_relay = progress_relay.clone();
            Arc::new(move |fraction: f64| {
                progress_relay.publish(RiseMessageUploadEvent::Preparing {
                    index: slot_index,
                    fraction: fraction * materialization_progress_weight,
                    total_bytes: 0,
                });
            })
        };
        let materialized = if has_deferred_image_source {
            RiseDeferredPhotoImageSource::materialize_if_needed(
                picker_item,
                self.local_files.len() == 1,
                on_materialization_progress,
                is_context_cancelled.clone(),
            )
            .await?
        } else {
            RiseDeferredPhotoVideoSource::materialize_if_needed(
                picker_item,
                on_materialization_progress,
                is_context_cancelled.clone(),
            )
            .await?
        };
        let _materialized_temporary_file =
            RiseGeneratedFileGuard(materialized.as_ref().and_then(|source| source.generated_temporary_url.clone()));
        let _materialized_original_image = RiseGeneratedFileGuard(
            materialized
                .as_ref()
                .and_then(|source| source.reedit_document.as_ref())
                .map(|document| document.original_image_url.clone()),
        );
        Self::validate_cancellation(cancellation, local_id, Some(slot_index)).await?;
        RiseConversationUploadSupport::validate(Some(account_user_id), account_epoch)?;
        if let Some(reedit_document) = materialized.as_ref().and_then(|source| source.reedit_document.clone()) {
            Self::persist_deferred_reedit_document(
                reedit_document,
                local_id.to_owned(),
                self.account_user_id.clone(),
                self.account_epoch.clone(),
            )
            .await;
            Self::validate_cancellation(cancellation, local_id, Some(slot_index)).await?;
            RiseConversationUploadSupport::validate(Some(account_user_id), account_epoch)?;
        }

        let mut source_file = local_file.clone();
        if let Some(materialized) = &materialized {
            source_file.url = materialized.url.clone();
            source_file.byte_count = None;
        }
        let original_bytes = Self::file_byte_count(&source_file);
        let compression_progress_start = materialization_progress_weight;
        let compression_progress_scale = 1.0 - materialization_progress_weight;
        progress_relay.publish(RiseMessageUploadEvent::Preparing {
            index: slot_index,
            fraction: compression_progress_start,
            total_bytes: original_bytes,
        });
        let premium = {
            let account_user_id = self.account_user_id.clone();
            hop.run(move |cx: AsyncApp| {
                async move {
                    Ok::<bool, anyhow::Error>(cx.update(|cx| {
                        let auth = AuthServicesStore::shared(cx).read(cx);
                        if auth.effective_user_id_for_wire().as_deref() != Some(account_user_id.as_str()) {
                            return false;
                        }
                        auth.profile().is_some_and(|profile| profile.id == account_user_id && profile.more.is_premium)
                    }))
                }
                .boxed_local()
            })
            .await?
        };
        let on_compression_progress: OutgoingMediaProgress = {
            let progress_relay = progress_relay.clone();
            Arc::new(move |fraction: f64| {
                progress_relay.publish(RiseMessageUploadEvent::Preparing {
                    index: slot_index,
                    fraction: compression_progress_start + fraction * compression_progress_scale,
                    total_bytes: original_bytes,
                });
            })
        };
        let compressed = OutgoingMediaCompressor::compress_if_needed(
            source_file.clone(),
            picker_item
                .and_then(|item| item.video_payload.as_ref())
                .filter(|video| video.is_processing_deferred)
                .cloned(),
            MediaCompressionConfiguration::chat().for_tier(premium),
            Some(on_compression_progress),
            Some(is_context_cancelled.clone()),
        )
        .await?;
        let source_url = Url::from_file_path(&source_file.url).map(|url| url.standardized());
        let _compressed_temporary_file = RiseGeneratedFileGuard(if Some(compressed.url.standardized()) != source_url {
            compressed.url.to_file_path()
        } else {
            None
        });
        Self::validate_cancellation(cancellation, local_id, Some(slot_index)).await?;
        RiseConversationUploadSupport::validate(Some(account_user_id), account_epoch)?;

        let media_upload_hints =
            RiseOutgoingMediaUploadHints::make(&compressed, &source_file, picker_item, is_context_cancelled.clone())
                .await;
        let _generated_thumbnail =
            RiseGeneratedFileGuard(media_upload_hints.as_ref().and_then(|hints| hints.generated_thumbnail_url.clone()));
        let prepared = RiseConversationUploadSupport::prepared_local_file(&source_file, &compressed);
        let uses_transport_byte_progress_exclusively =
            Self::uses_transport_byte_progress_exclusively(&source_file);
        let additional_validation: RiseChunkedUploadStep = {
            let cancellation = cancellation.clone();
            let owner_id = local_id.to_owned();
            Arc::new(move || -> BoxFuture<'static, anyhow::Result<()>> {
                let cancellation = cancellation.clone();
                let owner_id = owner_id.clone();
                async move { Self::validate_cancellation(&cancellation, &owner_id, Some(slot_index)).await }.boxed()
            })
        };
        let client: RiseChunkedUploadClient = {
            let account_user_id = self.account_user_id.clone();
            let account_epoch = self.account_epoch.clone();
            hop.run(move |cx: AsyncApp| {
                async move {
                    Ok::<RiseChunkedUploadClient, anyhow::Error>(cx.update(|cx| {
                        RiseConversationUploadSupport::make_client(
                            Some(account_user_id),
                            account_epoch,
                            Some(additional_validation),
                            cx,
                        )
                    }))
                }
                .boxed_local()
            })
            .await?
        };
        let cancellation_signal = cancellation.signal(local_id, Some(slot_index)).await;
        let compressed_byte_count = compressed.byte_count;
        progress_relay.publish(RiseMessageUploadEvent::Uploading {
            index: slot_index,
            fraction: 0.0,
            completed_bytes: 0,
            total_bytes: compressed_byte_count,
        });
        let Some(compressed_path) = compressed.url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let mut request = RiseChunkedUploadRequest::new(
            compressed_path,
            prepared.filename.clone(),
            compressed.content_type.clone(),
            source_file.file_context.clone().unwrap_or_else(|| DEFAULT_FILE_CONTEXT.to_owned()),
        );
        request.resource_id = upload_resource_id.map(str::to_owned);
        request.video_width = media_upload_hints.as_ref().and_then(|hints| hints.width);
        request.video_height = media_upload_hints.as_ref().and_then(|hints| hints.height);
        request.video_duration = media_upload_hints.as_ref().and_then(|hints| hints.duration);
        request.video_thumbnail_file_url = media_upload_hints.as_ref().and_then(|hints| hints.thumbnail_url.clone());
        request.awaits_network = true;

        let realtime_binding = RiseRealtimeBindingGuard {
            hub: self.upload_realtime_progress.clone(),
            upload_id: Arc::new(Mutex::new(None)),
        };
        let on_session_event: RiseChunkedUploadSessionEventHandler = {
            let hub = self.upload_realtime_progress.clone();
            let bound_upload_id = realtime_binding.upload_id.clone();
            let progress_relay = progress_relay.clone();
            Arc::new(move |session_event: RiseChunkedUploadSessionEvent| -> BoxFuture<'static, ()> {
                let hub = hub.clone();
                let bound_upload_id = bound_upload_id.clone();
                let progress_relay = progress_relay.clone();
                async move {
                    match session_event {
                        RiseChunkedUploadSessionEvent::Started { upload_id } => {
                            if uses_transport_byte_progress_exclusively {
                                return;
                            }
                            *bound_upload_id.lock() = Some(upload_id.clone());
                            let handler: RiseUploadRealtimeProgressHubHandler = Arc::new(
                                move |progress: RiseUploadRealtimeProgress| -> BoxFuture<'static, ()> {
                                    match Self::realtime_stage(progress.stage()) {
                                        RiseMessageMediaUploadPipelineRealtimeStage::Uploading => {
                                            progress_relay.publish(RiseMessageUploadEvent::Uploading {
                                                index: slot_index,
                                                fraction: progress.fraction(),
                                                completed_bytes: (compressed_byte_count as f64 * progress.fraction())
                                                    as i64,
                                                total_bytes: compressed_byte_count,
                                            });
                                        }
                                        RiseMessageMediaUploadPipelineRealtimeStage::Processing => {
                                            progress_relay.publish(RiseMessageUploadEvent::Processing {
                                                index: slot_index,
                                                fraction: progress.fraction(),
                                                total_bytes: compressed_byte_count,
                                            });
                                        }
                                        RiseMessageMediaUploadPipelineRealtimeStage::Unknown => {}
                                    }
                                    async {}.boxed()
                                },
                            );
                            hub.bind(&upload_id, handler).await;
                        }
                        RiseChunkedUploadSessionEvent::Finished { upload_id } => {
                            if !uses_transport_byte_progress_exclusively {
                                hub.unbind(&upload_id).await;
                                *bound_upload_id.lock() = None;
                            }
                        }
                    }
                }
                .boxed()
            })
        };
        let on_progress: RiseChunkedUploadProgressHandler = {
            let progress_relay = progress_relay.clone();
            Arc::new(move |progress: RiseChunkedUploadTransferProgress| {
                progress_relay.publish(RiseMessageUploadEvent::Uploading {
                    index: slot_index,
                    fraction: progress.fraction,
                    completed_bytes: progress.completed_bytes,
                    total_bytes: progress.total_bytes,
                });
            })
        };
        let result =
            client.upload(request, Some(cancellation_signal), Some(on_session_event), Some(on_progress)).await?;
        Self::validate_cancellation(cancellation, local_id, Some(slot_index)).await?;
        RiseConversationUploadSupport::validate(Some(account_user_id), account_epoch)?;

        RiseConversationUploadSupport::seed_prepared_image_cache_if_possible(
            &compressed,
            &result.file_url,
            self.account_user_id_owned(),
            self.account_epoch.clone(),
        )
        .await;
        RiseConversationUploadSupport::seed_prepared_video_cache_if_possible(
            &compressed,
            &result.file_url,
            self.account_user_id_owned(),
            self.account_epoch.clone(),
        )
        .await;
        if let Some(hints) = media_upload_hints.as_ref().filter(|hints| hints.is_sole_thumbnail_source)
            && let Some(poster_url) = hints.thumbnail_url.as_deref().and_then(Url::from_file_path)
            && let Some(remote_thumbnail_url) = &result.thumbnail_url
        {
            RiseConversationUploadSupport::seed_prepared_image_cache_if_possible_local_file_url(
                &poster_url,
                POSTER_CONTENT_TYPE,
                remote_thumbnail_url,
                self.account_user_id_owned(),
                self.account_epoch.clone(),
            )
            .await;
        }
        Self::validate_cancellation(cancellation, local_id, Some(slot_index)).await?;
        RiseConversationUploadSupport::validate(Some(account_user_id), account_epoch)?;
        if !cancellation.seal(local_id, slot_index).await {
            return Err(CancellationError.into());
        }
        let voice_metadata = self.voice_metadata_slots.get(index);
        let trimmed_chat_id = draft.body.chat_id.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if voice_metadata.is_some()
            && !trimmed_chat_id.is_empty()
            && let Some(source) = compressed.url.to_file_path()
        {
            let message_id = local_id.to_owned();
            let chat_id = trimmed_chat_id.to_owned();
            let stored = hop
                .run(move |cx: AsyncApp| {
                    async move {
                        Ok::<Option<PathBuf>, anyhow::Error>(
                            VoiceMessageDiskCache::store_local(&source, &message_id, &chat_id, &chat_id, &cx).await,
                        )
                    }
                    .boxed_local()
                })
                .await;
            tracing::warn!(target: "riseonly::X757", "voice seed local_id={local_id} stored={:?}", stored.as_ref().map(Option::is_some).ok());
            drop(stored);
            RiseConversationUploadSupport::validate(Some(account_user_id), account_epoch)?;
        }
        let body = Self::media_body(&result, &prepared, &compressed, picker_item, voice_metadata);
        progress_relay.publish(RiseMessageUploadEvent::Completed {
            index: slot_index,
            total_bytes: compressed_byte_count,
        });
        Ok(body)
    }

    fn account_user_id_owned(&self) -> Option<String> {
        Some(self.account_user_id.clone())
    }

    pub fn realtime_stage(raw: Option<&str>) -> RiseMessageMediaUploadPipelineRealtimeStage {
        let Some(stage) = raw.map(|raw| trimming_whitespaces_and_newlines(raw).to_lowercase()) else {
            return RiseMessageMediaUploadPipelineRealtimeStage::Unknown;
        };
        if stage == FINALIZING_STAGE {
            return RiseMessageMediaUploadPipelineRealtimeStage::Processing;
        }
        match CanonRealtimeStage::from_wire(&stage) {
            Some(CanonRealtimeStage::Uploading) => RiseMessageMediaUploadPipelineRealtimeStage::Uploading,
            Some(
                CanonRealtimeStage::Completing
                | CanonRealtimeStage::Processing
                | CanonRealtimeStage::Compressed
                | CanonRealtimeStage::UploadingToStorage,
            ) => RiseMessageMediaUploadPipelineRealtimeStage::Processing,
            Some(CanonRealtimeStage::Compressing | CanonRealtimeStage::Completed | CanonRealtimeStage::Error)
            | None => RiseMessageMediaUploadPipelineRealtimeStage::Unknown,
        }
    }

    pub fn uses_transport_byte_progress_exclusively(local_file: &RiseOutgoingMediaFile) -> bool {
        let media_type =
            local_file.media_type.as_deref().map(|media_type| trimming_whitespaces_and_newlines(media_type).to_lowercase());
        if let Some(media_type) = media_type.as_deref() {
            if media_type == ChatMediaKind::Image.wire() || media_type == ChatMediaKind::Video.wire() {
                return true;
            }
            if media_type == ChatMediaKind::Audio.wire()
                || media_type == ChatMediaKind::Document.wire()
                || media_type == FILE_MEDIA_TYPE
                || media_type == ChatMediaKind::VideoNote.wire()
                || media_type == ChatMediaKind::Voice.wire()
            {
                return false;
            }
        }
        let content_type = trimming_whitespaces_and_newlines(&local_file.content_type).to_lowercase();
        content_type.starts_with(IMAGE_CONTENT_TYPE_PREFIX) || content_type.starts_with(VIDEO_CONTENT_TYPE_PREFIX)
    }

    pub fn finalized_media_bodies(
        uploaded_by_index: HashMap<i64, RiseStoredOutgoingMedia>,
        local_file_count: i64,
        picker_items: &[MediaPickerItem],
        cancelled_slot_indices: &HashSet<i64>,
    ) -> anyhow::Result<Vec<RiseStoredOutgoingMedia>> {
        let mut surviving = uploaded_by_index;
        surviving.retain(|index, _| !cancelled_slot_indices.contains(index));
        if surviving.is_empty() {
            return Err(CancellationError.into());
        }
        if let Some(audio_index) = picker_items.iter().position(|item| item.audio_payload.is_some()) {
            let audio_index = audio_index as i64;
            if !surviving.contains_key(&audio_index) {
                return Err(CancellationError.into());
            }
            return Ok(Self::finalize_audio(surviving, audio_index, (audio_index > 0).then_some(0)));
        }
        let uploaded: Vec<RiseStoredOutgoingMedia> =
            (0..local_file_count.max(0)).filter_map(|index| surviving.remove(&index)).collect();
        if uploaded.is_empty() {
            return Err(CancellationError.into());
        }
        Ok(uploaded)
    }

    pub fn media_body(
        result: &RiseChunkedUploadResult,
        local_file: &RiseOutgoingMediaFile,
        compressed: &OutgoingMediaCompressorResult,
        picker_item: Option<&MediaPickerItem>,
        voice_metadata: Option<&VoiceMessageMetadata>,
    ) -> RiseStoredOutgoingMedia {
        let media_type = match local_file.media_type.as_deref().filter(|preferred| !preferred.is_empty()) {
            Some(preferred) => preferred.to_owned(),
            None if voice_metadata.is_some() => ChatMediaKind::Voice.wire().to_owned(),
            None => Self::guess_media_type(&compressed.content_type),
        };
        let mut body = RiseStoredOutgoingMedia::new(media_type.clone());
        body.file_id = result.file_id.clone().or_else(|| Self::file_id(&result.file_key));
        body.file_url = Some(result.file_url.clone());
        body.thumbnail_url = result.thumbnail_url.clone();
        body.width = result.width.or(compressed.pixel_width);
        body.height = result.height.or(compressed.pixel_height);
        body.file_name = Some(local_file.filename.clone());
        body.file_size = Some(result.file_size.unwrap_or(compressed.byte_count));
        body.mime_type = Some(if result.mime_type.is_empty() {
            compressed.content_type.clone()
        } else {
            result.mime_type.clone()
        });
        body.duration = result.duration;
        body.waveform =
            voice_metadata.and_then(|metadata| RiseStoredOutgoingMedia::waveform_wire_string(&metadata.waveform));
        if let Some(voice_metadata) = voice_metadata
            && body.duration.is_none()
        {
            body.duration = Some(voice_metadata.duration_seconds);
        }
        if media_type == ChatMediaKind::Video.wire()
            && let Some(video) = picker_item.and_then(|item| item.video_payload.as_ref())
        {
            if body.width.is_none() && video.pixel_size.width > 0.0 {
                body.width = Some(video.pixel_size.width.round() as i64);
            }
            if body.height.is_none() && video.pixel_size.height > 0.0 {
                body.height = Some(video.pixel_size.height.round() as i64);
            }
            if body.duration.is_none() && video.duration > 0.0 {
                body.duration = Some(video.duration.round() as i64);
            }
        }
        if let Some(item) = picker_item
            && (item.has_spoiler || item.view_mode.is_some())
            && !item.is_file
            && item.kind != MediaPickerItemKind::File
        {
            body.has_spoiler = Some(true);
        }
        if let Some(item) = picker_item
            && let Some(view_mode) = item.view_mode.as_ref()
            && !item.is_file
            && item.kind != MediaPickerItemKind::File
        {
            body.view_mode = Some(view_mode.wire_mode());
            body.view_seconds = view_mode.seconds();
        }
        if let Some(item) = picker_item
            && let Some(audio) = &item.audio_payload
        {
            body.file_name = item.audio_file_name();
            body.duration = Some(audio.duration.round() as i64);
            if let Some(cover_url) = &audio.cover_url {
                body.thumbnail_url = Some(cover_url.absolute_string().to_owned());
            }
        }
        body
    }

    fn finalize_audio(
        mut uploaded: HashMap<i64, RiseStoredOutgoingMedia>,
        audio_index: i64,
        cover_index: Option<i64>,
    ) -> Vec<RiseStoredOutgoingMedia> {
        let cover_url = cover_index
            .and_then(|cover_index| uploaded.get(&cover_index))
            .and_then(|cover| cover.file_url.clone())
            .filter(|cover_url| !cover_url.is_empty());
        let Some(mut audio) = uploaded.remove(&audio_index) else {
            let mut indices: Vec<i64> = uploaded.keys().copied().collect();
            indices.sort_unstable();
            return indices.into_iter().filter_map(|index| uploaded.remove(&index)).collect();
        };
        if let Some(cover_url) = cover_url {
            audio.thumbnail_url = Some(cover_url);
        }
        vec![audio]
    }

    async fn validate_cancellation(
        cancellation: &RiseUploadCancellationHub,
        owner_id: &str,
        slot_index: Option<i64>,
    ) -> anyhow::Result<()> {
        if cancellation.is_cancelled(owner_id, slot_index).await {
            return Err(CancellationError.into());
        }
        Ok(())
    }

    fn file_id(key: &str) -> Option<String> {
        let stem = Path::new(key).file_stem().and_then(|stem| stem.to_str()).unwrap_or_default();
        (!stem.is_empty()).then(|| stem.to_owned())
    }

    fn guess_media_type(mime_type: &str) -> String {
        let mime_type = mime_type.to_lowercase();
        let kind = if mime_type.starts_with(IMAGE_CONTENT_TYPE_PREFIX) {
            ChatMediaKind::Image
        } else if mime_type.starts_with(VIDEO_CONTENT_TYPE_PREFIX) {
            ChatMediaKind::Video
        } else if mime_type.starts_with(AUDIO_CONTENT_TYPE_PREFIX) {
            ChatMediaKind::Audio
        } else {
            ChatMediaKind::Document
        };
        kind.wire().to_owned()
    }

    fn file_byte_count(file: &RiseOutgoingMediaFile) -> i64 {
        if let Some(byte_count) = file.byte_count.filter(|byte_count| *byte_count > 0) {
            return byte_count;
        }
        FileManager
            .attributes_of_item(&file.url)
            .ok()
            .and_then(|attributes| i64::try_from(attributes.size).ok())
            .unwrap_or(0)
    }

    async fn persist_deferred_reedit_document(
        document: RiseDeferredImageReeditDocument,
        local_id: String,
        account_user_id: String,
        account_epoch: Option<String>,
    ) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let persisted = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        if !SaiAccountCacheScope::is_current_account_epoch(
                            Some(account_user_id.as_str()),
                            account_epoch.as_deref(),
                        ) {
                            return;
                        }
                        MediaReeditStore::shared(cx).update(cx, |store, _| {
                            let Some(edit_id) = store.save_document(
                                &document.original_image_url,
                                &document.editor_state_data,
                                MediaReeditStore::chat_ttl_seconds(),
                            ) else {
                                return;
                            };
                            store.register_pending_temp(&local_id, &edit_id);
                            store.link(&local_id, &edit_id);
                        });
                    });
                    Ok::<(), anyhow::Error>(())
                }
                .boxed_local()
            })
            .await;
        drop(persisted);
    }
}

#[derive(Clone, Debug)]
pub struct RiseDeferredImageReeditDocument {
    pub original_image_url: PathBuf,
    pub editor_state_data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct RiseDeferredMediaSource {
    pub url: PathBuf,
    pub generated_temporary_url: Option<PathBuf>,
    pub reedit_document: Option<RiseDeferredImageReeditDocument>,
}

struct RiseOutgoingMediaUploadHintsVideoMetadata {
    width: i64,
    height: i64,
    duration: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct RiseOutgoingMediaUploadHints {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<i64>,
    pub thumbnail_url: Option<PathBuf>,
    pub generated_thumbnail_url: Option<PathBuf>,
    pub(crate) is_sole_thumbnail_source: bool,
}

impl RiseOutgoingMediaUploadHints {
    const DOCUMENT_THUMBNAIL_MAXIMUM_PIXEL_SIZE: u32 = 320;

    const POSTER_QUALITY_LADDER: [f64; 5] = [0.7, 0.6, 0.5, 0.4, 0.3];
    const POSTER_DOWNSCALE_ATTEMPTS: usize = 3;
    const POSTER_DOWNSCALE_FACTOR: f64 = 0.7;

    fn maximum_thumbnail_bytes() -> i64 {
        RiseChunkedUploadClient::maximum_inline_video_thumbnail_bytes()
    }

    pub async fn make(
        compressed: &OutgoingMediaCompressorResult,
        local_file: &RiseOutgoingMediaFile,
        picker_item: Option<&MediaPickerItem>,
        is_cancelled: OutgoingMediaIsCancelled,
    ) -> Option<Self> {
        if is_cancelled().await {
            return None;
        }

        if !compressed.content_type.to_lowercase().starts_with(VIDEO_CONTENT_TYPE_PREFIX) {
            return Self::make_image_document(compressed, local_file, is_cancelled).await;
        }

        let video = picker_item.and_then(|item| item.video_payload.as_ref());
        let asset = compressed.url.to_file_path();
        let actual_metadata = match &asset {
            Some(asset) => Self::load_metadata(asset).await,
            None => None,
        };
        let existing_thumbnail_url =
            video.and_then(|video| video.poster_url.as_ref()).and_then(Self::usable_thumbnail_url);
        let generated = match &asset {
            Some(asset) => Self::generate_thumbnail(asset, is_cancelled).await,
            None => None,
        };
        let poster_dimensions = generated.as_ref().map(|generated| (generated.1, generated.2));
        let generated_thumbnail_url = generated.map(|generated| generated.0);

        let video_width =
            video.map(|video| video.pixel_size.width.round() as i64).filter(|width| *width > 0);
        let video_height =
            video.map(|video| video.pixel_size.height.round() as i64).filter(|height| *height > 0);
        let duration = video.map(|video| video.duration.round() as i64).filter(|duration| *duration > 0);

        Some(Self {
            width: actual_metadata
                .as_ref()
                .map(|metadata| metadata.width)
                .or(compressed.pixel_width)
                .or(video_width)
                .or(poster_dimensions.map(|dimensions| dimensions.0)),
            height: actual_metadata
                .as_ref()
                .map(|metadata| metadata.height)
                .or(compressed.pixel_height)
                .or(video_height)
                .or(poster_dimensions.map(|dimensions| dimensions.1)),
            duration: actual_metadata.as_ref().and_then(|metadata| metadata.duration).or(duration),
            thumbnail_url: generated_thumbnail_url.clone().or(existing_thumbnail_url),
            generated_thumbnail_url,
            is_sole_thumbnail_source: false,
        })
    }

    async fn make_image_document(
        compressed: &OutgoingMediaCompressorResult,
        local_file: &RiseOutgoingMediaFile,
        is_cancelled: OutgoingMediaIsCancelled,
    ) -> Option<Self> {
        let is_document = local_file.media_type.as_deref().is_some_and(|media_type| {
            trimming_whitespaces_and_newlines(media_type).to_lowercase() == ChatMediaKind::Document.wire()
        });
        if !is_document
            || !(compressed.content_type.to_lowercase().starts_with(IMAGE_CONTENT_TYPE_PREFIX)
                || compressed.pixel_width.is_some())
        {
            return None;
        }
        let source = compressed.url.to_file_path()?;
        let generated = Self::generate_image_thumbnail(source, is_cancelled).await?;

        Some(Self {
            width: None,
            height: None,
            duration: None,
            thumbnail_url: Some(generated.clone()),
            generated_thumbnail_url: Some(generated),
            is_sole_thumbnail_source: true,
        })
    }

    async fn generate_image_thumbnail(source_url: PathBuf, is_cancelled: OutgoingMediaIsCancelled) -> Option<PathBuf> {
        if is_cancelled().await {
            return None;
        }
        let maximum_pixel_size = Self::DOCUMENT_THUMBNAIL_MAXIMUM_PIXEL_SIZE;
        let generated = RiseAccountRuntime::current()
            .spawn_blocking(move || -> Option<PathBuf> {
                let source = RiseOutgoingImageEncoder::source_with_url(&source_url)?;
                let reduced = if source.width().max(source.height()) > maximum_pixel_size {
                    source.thumbnail(maximum_pixel_size, maximum_pixel_size)
                } else {
                    source
                };
                let data = Self::encoded_poster(reduced)?;
                Self::write_poster(&data)
            })
            .await
            .ok()
            .flatten()?;
        if is_cancelled().await {
            let _ = FileManager.remove_item(&generated);
            return None;
        }
        Some(generated)
    }

    fn write_poster(data: &[u8]) -> Option<PathBuf> {
        let output_url =
            temporary_directory().join(format!("{POSTER_FILE_PREFIX}{}.jpg", Uuid::new_v4().uuid_string()));
        match write_data_atomically(&output_url, data) {
            Ok(()) => Some(output_url),
            Err(_) => {
                let _ = FileManager.remove_item(&output_url);
                None
            }
        }
    }

    fn usable_thumbnail_url(url: &Url) -> Option<PathBuf> {
        let path = url.to_file_path()?;
        let path_extension = path.extension()?.to_str()?.to_lowercase();
        if path_extension != "jpg" && path_extension != "jpeg" {
            return None;
        }
        let byte_count = i64::try_from(FileManager.attributes_of_item(&path).ok()?.size).ok()?;
        (byte_count > 0 && byte_count <= Self::maximum_thumbnail_bytes()).then_some(path)
    }

    async fn load_metadata(asset: &Path) -> Option<RiseOutgoingMediaUploadHintsVideoMetadata> {
        let asset = asset.to_path_buf();
        let probe = RiseAccountRuntime::current().spawn_blocking(move || video_probe(&asset)).await.ok()?.ok()?;
        if !probe.has_video {
            return None;
        }
        let transformed = probe.display_size();
        let width = transformed.width.abs().round() as i64;
        let height = transformed.height.abs().round() as i64;
        if width <= 0 || height <= 0 {
            return None;
        }
        let duration_seconds = probe.duration_seconds;
        let duration =
            (duration_seconds.is_finite() && duration_seconds > 0.0).then(|| duration_seconds.round() as i64);
        Some(RiseOutgoingMediaUploadHintsVideoMetadata { width, height, duration })
    }

    async fn generate_thumbnail(
        asset: &Path,
        is_cancelled: OutgoingMediaIsCancelled,
    ) -> Option<(PathBuf, i64, i64)> {
        if is_cancelled().await {
            return None;
        }
        let asset = asset.to_path_buf();
        let frame = RiseAccountRuntime::current()
            .spawn_blocking(move || video_poster(&asset, 0.0, POSTER_MAX_SIDE))
            .await
            .ok()?
            .ok()?;
        if is_cancelled().await {
            return None;
        }
        RiseAccountRuntime::current()
            .spawn_blocking(move || -> Option<(PathBuf, i64, i64)> {
                let (width, height) = (frame.width, frame.height);
                let mut rgba = frame.bgra;
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                let image = DynamicImage::ImageRgba8(RgbaImage::from_raw(width, height, rgba)?);
                let data = Self::encoded_poster(image)?;
                let output_url = Self::write_poster(&data)?;
                Some((output_url, i64::from(width.max(1)), i64::from(height.max(1))))
            })
            .await
            .ok()
            .flatten()
    }

    fn encoded_poster(image: DynamicImage) -> Option<Vec<u8>> {
        let mut candidate = image;
        for attempt in 0..Self::POSTER_DOWNSCALE_ATTEMPTS {
            if attempt > 0 {
                let Some(smaller) = Self::downscaled(&candidate) else {
                    break;
                };
                candidate = smaller;
            }
            for quality in Self::POSTER_QUALITY_LADDER {
                if let Some(data) = RiseOutgoingImageEncoder::white_backed_jpeg_data(&candidate, quality)
                    && !data.is_empty()
                    && i64::try_from(data.len()).is_ok_and(|length| length <= Self::maximum_thumbnail_bytes())
                {
                    return Some(data);
                }
            }
        }
        None
    }

    fn downscaled(image: &DynamicImage) -> Option<DynamicImage> {
        let width = (f64::from(image.width()) * Self::POSTER_DOWNSCALE_FACTOR).round();
        let height = (f64::from(image.height()) * Self::POSTER_DOWNSCALE_FACTOR).round();
        if width < 1.0 || height < 1.0 {
            return None;
        }
        Some(image.resize_exact(width as u32, height as u32, FilterType::CatmullRom))
    }
}

struct RiseDeferredRenderCancellation(Arc<AtomicBool>);

impl Drop for RiseDeferredRenderCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

pub enum RiseDeferredPhotoImageSource {}

impl RiseDeferredPhotoImageSource {
    pub async fn materialize_if_needed(
        picker_item: Option<&MediaPickerItem>,
        preserve_reedit_document: bool,
        on_progress: OutgoingMediaProgress,
        is_cancelled: OutgoingMediaIsCancelled,
    ) -> anyhow::Result<Option<RiseDeferredMediaSource>> {
        let Some(picker_item) = picker_item else {
            return Ok(None);
        };
        let Some(asset_identifier) = picker_item.deferred_photo_library_image_asset_identifier() else {
            return Ok(None);
        };
        let Some(image_payload) = picker_item.image_payload.as_ref() else {
            return Ok(None);
        };
        let Some(edit_state) = image_payload.deferred_edit_state.clone() else {
            return Ok(None);
        };

        Self::check_cancellation(&is_cancelled).await?;

        let request = Arc::new(RisePhotosImageRequest::default());
        let fetch_progress = on_progress.clone();
        let image = request
            .load(
                &asset_identifier,
                Self::request_target_size(),
                move |fraction| fetch_progress(fraction * IMAGE_FETCH_PROGRESS_SHARE),
                is_cancelled.clone(),
            )
            .await;
        let image = image?;
        Self::check_cancellation(&is_cancelled).await?;
        on_progress(IMAGE_LOADED_PROGRESS);

        let editor_state_data =
            image_payload.editor_state_data.clone().or_else(|| EditorStateCodec::encode_image(&edit_state));
        let was_edited = edit_state.was_edited;
        let has_editor_state_data = editor_state_data.is_some();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel_render = RiseDeferredRenderCancellation(cancelled.clone());
        let render_progress = on_progress.clone();
        let rendered = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<(PathBuf, Option<PathBuf>)> {
                let check = || -> anyhow::Result<()> {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(CancellationError.into());
                    }
                    Ok(())
                };
                check()?;
                let rendered: CleverImage = if edit_state.has_meaningful_edits() {
                    EditorImageBake::render_edited_image(&image, &edit_state)
                } else {
                    image.clone()
                };
                check()?;
                render_progress(IMAGE_RENDERED_PROGRESS);

                let data = CleverImageBitmapEncoder::rgba(&rendered)
                    .and_then(|rgba| {
                        RiseOutgoingImageEncoder::white_backed_jpeg_data(
                            &DynamicImage::ImageRgba8(rgba),
                            MATERIALIZED_JPEG_QUALITY,
                        )
                    })
                    .ok_or(RiseDeferredPhotoImageError::EncodingFailed)?;
                let output_url =
                    temporary_directory().join(format!("{PHASSET_FILE_PREFIX}{}.jpg", Uuid::new_v4().uuid_string()));
                let mut original_image_url: Option<PathBuf> = None;
                let written = write_data_atomically(&output_url, &data).map_err(anyhow::Error::from).and_then(|()| {
                    if preserve_reedit_document && was_edited && has_editor_state_data {
                        original_image_url = Some(MediaExporter::write_reedit_original(&image)?);
                    }
                    check()
                });
                if let Err(error) = written {
                    let _ = FileManager.remove_item(&output_url);
                    if let Some(original_image_url) = &original_image_url {
                        let _ = FileManager.remove_item(original_image_url);
                    }
                    return Err(error);
                }
                render_progress(1.0);
                Ok((output_url, original_image_url))
            })
            .await;
        let (output_url, original_image_url) = match rendered {
            Ok(rendered) => rendered?,
            Err(_) => return Err(CancellationError.into()),
        };
        if let Err(error) = Self::check_cancellation(&is_cancelled).await {
            let _ = FileManager.remove_item(&output_url);
            if let Some(original_image_url) = &original_image_url {
                let _ = FileManager.remove_item(original_image_url);
            }
            return Err(error);
        }

        let reedit_document = match (original_image_url, editor_state_data) {
            (Some(original_image_url), Some(editor_state_data)) => {
                Some(RiseDeferredImageReeditDocument { original_image_url, editor_state_data })
            }
            _ => None,
        };
        Ok(Some(RiseDeferredMediaSource {
            url: output_url.clone(),
            generated_temporary_url: Some(output_url),
            reedit_document,
        }))
    }

    fn request_target_size() -> Size<f32> {
        let side = MediaCompressionConfiguration::source_pixel_max_side() as f32;
        size(side, side)
    }

    async fn check_cancellation(is_cancelled: &OutgoingMediaIsCancelled) -> anyhow::Result<()> {
        if is_cancelled().await {
            return Err(CancellationError.into());
        }
        Ok(())
    }
}

pub enum RiseDeferredPhotoVideoSource {}

impl RiseDeferredPhotoVideoSource {
    pub async fn materialize_if_needed(
        picker_item: Option<&MediaPickerItem>,
        on_progress: OutgoingMediaProgress,
        is_cancelled: OutgoingMediaIsCancelled,
    ) -> anyhow::Result<Option<RiseDeferredMediaSource>> {
        let Some(asset_identifier) = picker_item.and_then(MediaPickerItem::deferred_photo_library_video_asset_identifier)
        else {
            return Ok(None);
        };

        Self::check_cancellation(&is_cancelled).await?;

        let request = Arc::new(RisePhotosAVAssetRequest::default());
        let export_id = Uuid::new_v4().uuid_string();
        let directory = temporary_directory().join(format!("{PHASSET_FILE_PREFIX}{export_id}"));
        let created = directory.clone();
        RiseAccountRuntime::current()
            .spawn_blocking(move || std::fs::create_dir_all(&created))
            .await??;
        let export_directory = RiseDeferredExportDirectory(directory);
        let directory = export_directory.0.as_path();
        let export_progress = on_progress.clone();
        let source_url = request
            .load(
                &asset_identifier,
                directory,
                move |progress| {
                    export_progress(match progress {
                        ExportProgress::Fetching(fraction) => fraction * VIDEO_LOADED_PROGRESS,
                        ExportProgress::Encoding(fraction) => {
                            VIDEO_LOADED_PROGRESS + fraction * (1.0 - VIDEO_LOADED_PROGRESS)
                        }
                    });
                },
                is_cancelled.clone(),
            )
            .await?;

        let generated_temporary_url = if source_url.starts_with(directory) {
            let file_name = source_url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let moved = temporary_directory().join(format!("{PHASSET_FILE_PREFIX}{export_id}-{file_name}"));
            let (from, to) = (source_url.clone(), moved.clone());
            RiseAccountRuntime::current().spawn_blocking(move || std::fs::rename(&from, &to)).await??;
            Some(moved)
        } else {
            None
        };
        let source_url = generated_temporary_url.clone().unwrap_or(source_url);
        on_progress(1.0);
        if let Err(error) = Self::check_cancellation(&is_cancelled).await {
            if let Some(generated_temporary_url) = &generated_temporary_url {
                let _ = FileManager.remove_item(generated_temporary_url);
            }
            return Err(error);
        }

        Ok(Some(RiseDeferredMediaSource { url: source_url, generated_temporary_url, reedit_document: None }))
    }

    async fn check_cancellation(is_cancelled: &OutgoingMediaIsCancelled) -> anyhow::Result<()> {
        if is_cancelled().await {
            return Err(CancellationError.into());
        }
        Ok(())
    }
}

trait RisePhotosCancellableRequest: Send + Sync {
    fn cancel(&self);
}

struct RiseDeferredExportDirectory(PathBuf);

impl Drop for RiseDeferredExportDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct RisePhotosCancelOnDrop(Arc<dyn RisePhotosCancellableRequest>);

impl Drop for RisePhotosCancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn spawn_photos_cancellation_monitor(
    request: Weak<dyn RisePhotosCancellableRequest>,
    is_cancelled: OutgoingMediaIsCancelled,
) -> RiseAbortOnDrop {
    let monitor = RiseAccountRuntime::current().spawn(async move {
        loop {
            if is_cancelled().await {
                if let Some(request) = request.upgrade() {
                    request.cancel();
                }
                return;
            }
            tokio::time::sleep(PHOTOS_CANCELLATION_POLL_INTERVAL).await;
        }
    });
    RiseAbortOnDrop(monitor.abort_handle())
}

#[derive(Default)]
struct RisePhotosImageRequestState {
    continuation: Option<oneshot::Sender<anyhow::Result<CleverImage>>>,
    request_id: Option<ImageRequestId>,
    is_finished: bool,
}

#[derive(Default)]
struct RisePhotosImageRequest {
    state: Mutex<RisePhotosImageRequestState>,
}

impl RisePhotosImageRequest {
    async fn load(
        self: &Arc<Self>,
        asset_identifier: &str,
        target_size: Size<f32>,
        on_progress: impl Fn(f64) + Send + Sync + 'static,
        is_cancelled: OutgoingMediaIsCancelled,
    ) -> anyhow::Result<CleverImage> {
        let weak_request: Weak<Self> = Arc::downgrade(self);
        let _cancellation_monitor = spawn_photos_cancellation_monitor(weak_request, is_cancelled);
        let _cancel_on_drop = RisePhotosCancelOnDrop(self.clone());

        let (continuation, receiver) = oneshot::channel();
        let should_start = {
            let mut state = self.state.lock();
            if state.is_finished {
                false
            } else {
                state.continuation = Some(continuation);
                true
            }
        };
        if !should_start {
            return Err(CancellationError.into());
        }

        let weak_self = Arc::downgrade(self);
        let local_identifier = asset_identifier.to_owned();
        let request_id = PhotosImageRequest::request_image_with_progress(
            asset_identifier,
            target_size,
            ImageContentMode::AspectFit,
            ImageDeliveryMode::HighQuality,
            true,
            on_progress,
            move |delivery| {
                if delivery.is_degraded {
                    return;
                }
                let Some(this) = weak_self.upgrade() else {
                    return;
                };
                if delivery.is_cancelled {
                    this.finish(Err(CancellationError.into()));
                } else if let Some(image) = delivery.image {
                    this.finish(Ok(image));
                } else {
                    this.finish(Err(RiseDeferredPhotoImageError::AssetUnavailable(local_identifier.clone()).into()));
                }
            },
        );
        if let Some(request_id) = request_id {
            let should_cancel_request = {
                let mut state = self.state.lock();
                if state.is_finished {
                    true
                } else {
                    state.request_id = Some(request_id);
                    false
                }
            };
            if should_cancel_request {
                photo_library::cancel_image(request_id);
            }
        }
        receiver.await.unwrap_or_else(|_| Err(CancellationError.into()))
    }

    fn finish(&self, result: anyhow::Result<CleverImage>) {
        let continuation = {
            let mut state = self.state.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            state.request_id = None;
            state.continuation.take()
        };
        if let Some(continuation) = continuation {
            let _ = continuation.send(result);
        }
    }
}

impl RisePhotosCancellableRequest for RisePhotosImageRequest {
    fn cancel(&self) {
        let (continuation, request_id) = {
            let mut state = self.state.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            (state.continuation.take(), state.request_id.take())
        };
        if let Some(request_id) = request_id {
            photo_library::cancel_image(request_id);
        }
        if let Some(continuation) = continuation {
            let _ = continuation.send(Err(CancellationError.into()));
        }
    }
}

#[derive(Default)]
struct RisePhotosAVAssetRequestState {
    continuation: Option<oneshot::Sender<anyhow::Result<PathBuf>>>,
    request_id: Option<ImageRequestId>,
    is_finished: bool,
}

#[derive(Default)]
struct RisePhotosAVAssetRequest {
    state: Mutex<RisePhotosAVAssetRequestState>,
}

impl RisePhotosAVAssetRequest {
    async fn load(
        self: &Arc<Self>,
        asset_identifier: &str,
        directory: &Path,
        on_progress: impl Fn(ExportProgress) + Send + Sync + 'static,
        is_cancelled: OutgoingMediaIsCancelled,
    ) -> anyhow::Result<PathBuf> {
        let weak_request: Weak<Self> = Arc::downgrade(self);
        let _cancellation_monitor = spawn_photos_cancellation_monitor(weak_request, is_cancelled);
        let _cancel_on_drop = RisePhotosCancelOnDrop(self.clone());

        let (continuation, receiver) = oneshot::channel();
        let should_start = {
            let mut state = self.state.lock();
            if state.is_finished {
                false
            } else {
                state.continuation = Some(continuation);
                true
            }
        };
        if !should_start {
            return Err(CancellationError.into());
        }

        let weak_self = Arc::downgrade(self);
        let local_identifier = asset_identifier.to_owned();
        let started = photo_library::export_video_with_progress(asset_identifier, directory, on_progress, move |outcome| {
            if let Some(this) = weak_self.upgrade() {
                this.finish_outcome(outcome, local_identifier);
            }
        });
        let request_id = match started {
            Ok(request_id) => Some(request_id),
            Err(error) => {
                self.finish_outcome(ExportOutcome::Failed(error), asset_identifier.to_owned());
                None
            }
        };
        if let Some(request_id) = request_id {
            let should_cancel_request = {
                let mut state = self.state.lock();
                if state.is_finished {
                    true
                } else {
                    state.request_id = Some(request_id);
                    false
                }
            };
            if should_cancel_request {
                photo_library::cancel_export(request_id);
            }
        }
        receiver.await.unwrap_or_else(|_| Err(CancellationError.into()))
    }

    fn finish_outcome(&self, outcome: ExportOutcome, local_identifier: String) {
        match outcome {
            ExportOutcome::Exported(path) => self.finish(Ok(path)),
            ExportOutcome::Cancelled => self.finish(Err(CancellationError.into())),
            ExportOutcome::Failed(PhotoLibraryUnavailable::NotAVideo(_)) => {
                self.finish(Err(RiseDeferredPhotoVideoError::AssetUnavailable(local_identifier).into()));
            }
            ExportOutcome::Failed(error) => self.finish(Err(error.into())),
        }
    }

    fn finish(&self, result: anyhow::Result<PathBuf>) {
        let continuation = {
            let mut state = self.state.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            state.request_id = None;
            state.continuation.take()
        };
        if let Some(continuation) = continuation {
            let _ = continuation.send(result);
        }
    }
}

impl RisePhotosCancellableRequest for RisePhotosAVAssetRequest {
    fn cancel(&self) {
        let (continuation, request_id) = {
            let mut state = self.state.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            (state.continuation.take(), state.request_id.take())
        };
        if let Some(request_id) = request_id {
            photo_library::cancel_export(request_id);
        }
        if let Some(continuation) = continuation {
            let _ = continuation.send(Err(CancellationError.into()));
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RiseDeferredPhotoImageError {
    AssetUnavailable(String),
    EncodingFailed,
}

impl RiseDeferredPhotoImageError {
    fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::AssetUnavailable(_) => "The selected image is no longer available in Photos.".to_owned(),
            Self::EncodingFailed => "The selected image could not be prepared for upload.".to_owned(),
        })
    }
}

impl fmt::Display for RiseDeferredPhotoImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for RiseDeferredPhotoImageError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RiseDeferredPhotoVideoError {
    AssetUnavailable(String),
}

impl RiseDeferredPhotoVideoError {
    fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::AssetUnavailable(_) => "The selected video is no longer available in Photos.".to_owned(),
        })
    }
}

impl fmt::Display for RiseDeferredPhotoVideoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for RiseDeferredPhotoVideoError {}
