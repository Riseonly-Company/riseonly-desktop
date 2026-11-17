use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::rise_key::{bot, message, upload};
use rise_canon::rise_request::message::SharePayload;
use rise_canon::rise_request::message::share_payload::{DestinationsElement, MediaItemsElement};
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_chunked_upload::RiseChunkedUploadTransferProgress;
use rise_i18n::tr;
use rise_platform::file_manager::{CocoaErrorCode, FileManager};
use rise_platform::haptics::{NotificationKind, notification};
use serde_json::{Map, Value};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::sai_outgoing_media_hooks::OutgoingMediaProgress;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::export::outgoing_media_compressor::{
    OutgoingMediaCompressor, OutgoingMediaCompressorResult,
};
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::share_extension::send_handoff::share_send_handoff_actions::ShareSendHandoffActionsStore;
use crate::modules::share_extension::send_handoff::share_send_handoff_models::{
    RiseStoredSharePayloadReply, ShareSendAcknowledgement, ShareSendHandoffError, ShareSendHandoffPhase,
};
use crate::modules::share_extension::services::rise_share_app_bridge::RiseShareAppBridge;
use crate::modules::share_extension::shared::rise_share_container::{RiseShareContainer, RiseShareContainerError};
use crate::modules::share_extension::shared::rise_share_models::{
    RiseShareSendAttachment, RiseShareSendJSONValue, RiseShareSendSession,
};

const SUCCEEDED_HOLD: Duration = Duration::from_millis(650);
const SEND_ATTEMPTS: usize = 3;
const FIRST_RETRY_DELAY: Duration = Duration::from_millis(400);
const LATER_RETRY_DELAY: Duration = Duration::from_millis(1_000);
const MAXIMUM_FAILURE_DETAIL_CHARACTERS: usize = 300;
const MAXIMUM_FILENAME_CHARACTERS: usize = 180;
const UPLOAD_FILE_CONTEXT: &str = "chat";

pub struct ShareSendHandoffServicesStore {
    phase: ShareSendHandoffPhase,
    pending_session_ids: Vec<String>,
    operation_task: Option<Task<()>>,
    operation_generation: Option<Uuid>,
    dismissed_session_ids: HashSet<String>,
    commit_started_session_ids: HashSet<String>,
}

impl ShareSendHandoffServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            phase: ShareSendHandoffPhase::Idle,
            pending_session_ids: Vec::new(),
            operation_task: None,
            operation_generation: None,
            dismissed_session_ids: HashSet::new(),
            commit_started_session_ids: HashSet::new(),
        }
    }

    pub fn phase(&self) -> &ShareSendHandoffPhase {
        &self.phase
    }

    pub fn pending_session_ids(&self) -> &[String] {
        &self.pending_session_ids
    }

    pub fn has_pending_handoff(&self) -> bool {
        self.phase.active_session_id().is_some() || !self.pending_session_ids.is_empty()
    }

    pub fn can_cancel_current(&self) -> bool {
        let Some(session_id) = self.phase.active_session_id() else { return false };
        if self.commit_started_session_ids.contains(session_id) {
            return false;
        }
        match &self.phase {
            ShareSendHandoffPhase::Preparing { .. } | ShareSendHandoffPhase::Uploading { .. } => true,
            ShareSendHandoffPhase::Failed { commit_started, .. } => !commit_started,
            _ => false,
        }
    }

    pub fn bootstrap(&mut self, cx: &mut Context<Self>) {
        RiseShareContainer::remove_expired_files();
        let Some(account_id) = Self::current_account_id(cx) else { return };
        let discovered: Vec<String> = RiseShareContainer::pending_send_session_ids(&account_id)
            .into_iter()
            .filter(|id| !self.dismissed_session_ids.contains(id))
            .collect();
        let active_session_id = self.phase.active_session_id().map(str::to_owned);
        for session_id in discovered {
            if Some(&session_id) != active_session_id.as_ref() && !self.pending_session_ids.contains(&session_id) {
                self.pending_session_ids.push(session_id);
            }
        }
        cx.notify();
        self.resume_if_possible(cx);
    }

    pub fn enqueue(&mut self, session_id: String, explicit_url: bool, cx: &mut Context<Self>) {
        if !RiseShareContainer::is_valid_session_identifier(&session_id) {
            Self::notify_invalid_session(cx);
            return;
        }
        self.dismissed_session_ids.remove(&session_id);
        if self.phase.active_session_id() == Some(session_id.as_str()) || self.pending_session_ids.contains(&session_id) {
            return;
        }
        self.pending_session_ids.push(session_id.clone());
        cx.notify();

        let Some(account_id) = Self::current_account_id(cx) else {
            Notifier::send(
                tr("share_send_handoff_sign_in_title"),
                NotifierSendOptions {
                    content: Some(tr("share_send_handoff_sign_in_message")),
                    r#type: NotifierType::Info,
                    position: NotifierPosition::Top,
                    icon: NotifierIcon::System("person.crop.circle.badge.checkmark".to_owned()),
                    ..Default::default()
                },
                cx,
            );
            return;
        };
        if explicit_url {
            let _ = RiseShareContainer::set_send_session_auto_resume(&session_id, &account_id, true);
        }
        self.resume_if_possible(cx);
    }

    pub fn resume_if_possible(&mut self, cx: &mut Context<Self>) {
        if self.phase.active_session_id().is_some()
            || self.operation_task.is_some()
            || Self::current_account_id(cx).is_none()
        {
            return;
        }
        while !self.pending_session_ids.is_empty() {
            let session_id = self.pending_session_ids.remove(0);
            cx.notify();
            if self.dismissed_session_ids.contains(&session_id) {
                continue;
            }
            self.start(session_id, cx);
            return;
        }
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        let session_id = match &self.phase {
            ShareSendHandoffPhase::Failed { session_id, .. } | ShareSendHandoffPhase::Partial { session_id, .. } => {
                session_id.clone()
            }
            _ => return,
        };
        self.start(session_id, cx);
    }

    pub fn finish_acknowledged_partial(&mut self, cx: &mut Context<Self>) {
        let ShareSendHandoffPhase::Partial { session_id, .. } = &self.phase else { return };
        let session_id = session_id.clone();
        let Some(account_id) = Self::current_account_id(cx) else { return };
        RiseShareContainer::remove_acknowledged_send_session(&session_id, &account_id);
        self.commit_started_session_ids.remove(&session_id);
        self.operation_generation = None;
        self.operation_task = None;
        self.phase = ShareSendHandoffPhase::Idle;
        cx.notify();
        self.resume_if_possible(cx);
    }

    pub fn dismiss_committed_until_next_launch(&mut self, cx: &mut Context<Self>) {
        let ShareSendHandoffPhase::Failed { session_id, commit_started: true, .. } = &self.phase else { return };
        self.dismissed_session_ids.insert(session_id.clone());
        self.operation_generation = None;
        self.operation_task = None;
        self.phase = ShareSendHandoffPhase::Idle;
        cx.notify();
        self.resume_if_possible(cx);
    }

    pub fn cancel_and_discard(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.phase.active_session_id().map(str::to_owned) else { return };
        match &self.phase {
            ShareSendHandoffPhase::Preparing { .. } | ShareSendHandoffPhase::Uploading { .. } => {}
            ShareSendHandoffPhase::Failed { commit_started: false, .. } => {}
            _ => return,
        }
        self.dismissed_session_ids.insert(session_id.clone());
        self.operation_generation = None;
        self.operation_task = None;
        self.phase = ShareSendHandoffPhase::Idle;
        cx.notify();
        if let Some(account_id) = Self::current_account_id(cx) {
            RiseShareContainer::discard_app_claimed_cancelled_send_session(&session_id, &account_id);
        }
        let _ = notification(NotificationKind::Warning);
        self.resume_if_possible(cx);
    }

    pub fn reset_for_account_change(&mut self, cx: &mut Context<Self>) {
        self.operation_generation = None;
        self.operation_task = None;
        self.pending_session_ids.clear();
        self.dismissed_session_ids.clear();
        self.commit_started_session_ids.clear();
        self.phase = ShareSendHandoffPhase::Idle;
        cx.notify();
    }

    /// lets go of the operation handle without cancelling, the operation still has work left
    fn release_operation_task(&mut self) {
        if let Some(task) = self.operation_task.take() {
            task.detach();
        }
    }

    fn start(&mut self, session_id: String, cx: &mut Context<Self>) {
        if self.operation_task.is_some() && self.phase.active_session_id() != Some(session_id.as_str()) {
            if !self.pending_session_ids.contains(&session_id) {
                self.pending_session_ids.push(session_id);
                cx.notify();
            }
            return;
        }
        self.operation_task = None;
        let Some(account_id) = Self::current_account_id(cx) else { return };
        match RiseShareContainer::claim_send_session_for_app(&session_id, &account_id) {
            Ok(claimed) => {
                if claimed.commit_started {
                    self.commit_started_session_ids.insert(session_id.clone());
                }
            }
            Err(error) => {
                if Self::is_missing_session(&error) {
                    self.operation_task = None;
                    self.operation_generation = None;
                    self.commit_started_session_ids.remove(&session_id);
                    if self.phase.active_session_id() == Some(session_id.as_str()) {
                        self.phase = ShareSendHandoffPhase::Idle;
                    }
                    cx.notify();
                    self.resume_if_possible(cx);
                    return;
                }
                self.phase = ShareSendHandoffPhase::Failed {
                    session_id,
                    message: Self::failure_text(&anyhow::Error::new(error)),
                    commit_started: false,
                };
                cx.notify();
                return;
            }
        }
        let generation = Uuid::new_v4();
        self.operation_generation = Some(generation);
        self.phase = ShareSendHandoffPhase::Preparing { session_id: session_id.clone() };
        cx.notify();
        self.operation_task = Some(cx.spawn(async move |this, cx| {
            let Err(error) = Self::run(this.clone(), session_id.clone(), generation, cx).await else {
                return;
            };
            if error.is::<CancellationError>() {
                return;
            }
            let _ = this.update(cx, |store, cx| {
                if store.operation_generation != Some(generation) {
                    return;
                }
                store.release_operation_task();
                store.phase = ShareSendHandoffPhase::Failed {
                    message: Self::failure_text(&error),
                    commit_started: store.commit_started_session_ids.contains(&session_id),
                    session_id,
                };
                cx.notify();
            });
        }));
    }

    async fn run(
        this: WeakEntity<Self>,
        session_id: String,
        generation: Uuid,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        let Some(account_id) = cx.update(Self::current_account_id) else {
            return Err(ShareSendHandoffError::AccountChanged.into());
        };
        let epoch = SaiAccountCacheScope::active_epoch();
        if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), epoch.as_deref()) {
            return Err(ShareSendHandoffError::AccountChanged.into());
        }

        let (mut session, directory) = {
            let session_id = session_id.clone();
            let account_id = account_id.clone();
            RiseAccountRuntime::current()
                .spawn_blocking(move || RiseShareContainer::load_send_session(&session_id, &account_id))
                .await
                .map_err(|_| CancellationError)??
        };
        if session.commit_started {
            Self::update_store(&this, cx, |store, _| {
                store.commit_started_session_ids.insert(session_id.clone());
            })?;
        }
        if !session.claimed_by_app {
            return Err(ShareSendHandoffError::InvalidResponse.into());
        }
        Self::ensure_current_in(&this, cx, generation, &account_id, epoch.as_deref())?;

        let upload_chat_id = session
            .destinations
            .first()
            .map(|destination| trimming_whitespaces_and_newlines(&destination.chat_id).to_owned())
            .unwrap_or_default();
        if upload_chat_id.is_empty() {
            return Err(ShareSendHandoffError::InvalidResponse.into());
        }

        let file_attachments: Vec<RiseShareSendAttachment> = session
            .attachments
            .iter()
            .filter(|attachment| attachment.relative_path.is_some())
            .cloned()
            .collect();
        let total_bytes: i64 = file_attachments
            .iter()
            .map(|attachment| attachment.byte_count.unwrap_or(0).max(0))
            .sum();
        let mut completed_bytes: i64 = file_attachments
            .iter()
            .filter(|attachment| attachment.uploaded_media.is_some())
            .map(|attachment| attachment.byte_count.unwrap_or(0).max(0))
            .sum();

        let item_count = file_attachments.len() as i64;
        for (index, attachment) in file_attachments.iter().enumerate() {
            if attachment.uploaded_media.is_some() {
                continue;
            }
            Self::ensure_current_in(&this, cx, generation, &account_id, epoch.as_deref())?;
            let source_url = RiseShareContainer::resolved_send_attachment_url(attachment, &directory)?;
            let fallback_name = source_url
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let filename = Self::safe_filename(attachment.original_file_name.as_deref(), &fallback_name);
            let progress = ShareSendFileProgress {
                session_id: session_id.clone(),
                generation,
                filename: filename.clone(),
                item: index as i64 + 1,
                total: item_count,
                attachment_bytes: attachment.byte_count.unwrap_or(0).max(0),
                done_bytes: completed_bytes,
                total_bytes,
            };
            Self::update_store(&this, cx, |store, cx| {
                store.phase = ShareSendHandoffPhase::Uploading {
                    session_id: session_id.clone(),
                    filename: filename.clone(),
                    item: progress.item,
                    total: progress.total,
                    fraction: if total_bytes > 0 {
                        progress.done_bytes as f64 / total_bytes as f64
                    } else {
                        0.0
                    },
                    sent_bytes: 0,
                    file_bytes: 0,
                };
                cx.notify();
            })?;

            let (progress_sender, mut progress_receiver) = mpsc::unbounded::<ShareSendFileTransfer>();
            let _progress_pump: Task<()> = cx.spawn({
                let this = this.clone();
                let progress = progress.clone();
                async move |cx| {
                    while let Some(transfer) = progress_receiver.next().await {
                        let _ = this.update(cx, |store, cx| store.report_file_progress(&progress, transfer, cx));
                    }
                }
            });

            let mut original = RiseOutgoingMediaFile::new(
                source_url.clone(),
                filename.clone(),
                attachment.mime_type.clone(),
            );
            original.file_context = Some(UPLOAD_FILE_CONTEXT.to_owned());
            original.media_type = Some(if attachment.kind == "file" {
                "document".to_owned()
            } else {
                attachment.kind.clone()
            });
            let preparation_share = if original.media_type.as_deref() == Some("document") { 0.0 } else { 0.5 };
            let preparation_sender = progress_sender.clone();
            let on_preparation: OutgoingMediaProgress = Arc::new(move |preparation: f64| {
                let _ = preparation_sender.unbounded_send(ShareSendFileTransfer {
                    file_fraction: preparation * preparation_share,
                    sent_bytes: 0,
                    file_bytes: 0,
                });
            });
            let compressed = OutgoingMediaCompressor::compress_if_needed(
                original.clone(),
                None,
                MediaCompressionConfiguration::chat(),
                Some(on_preparation),
                None,
            )
            .await?;
            let _generated_temporary_url = ShareSendGeneratedFile(
                compressed
                    .url
                    .to_file_path()
                    .filter(|path| Self::standardized(path) != Self::standardized(&source_url)),
            );
            let uploaded = Self::upload_and_persist(
                &this,
                cx,
                UploadStep {
                    original: &original,
                    compressed: &compressed,
                    attachment,
                    upload_chat_id: &upload_chat_id,
                    session_id: &session_id,
                    account_id: &account_id,
                    epoch: epoch.clone(),
                    generation,
                    preparation_share,
                    progress_sender,
                },
            )
            .await;
            session = uploaded?;
            completed_bytes += progress.attachment_bytes;
            Self::ensure_current_in(&this, cx, generation, &account_id, epoch.as_deref())?;
        }

        let media_items = session
            .attachments
            .iter()
            .filter(|attachment| attachment.relative_path.is_some())
            .map(Self::validated_media_item)
            .collect::<anyhow::Result<Vec<MediaItemsElement>>>()?;
        if media_items.len() != file_attachments.len() {
            return Err(ShareSendHandoffError::InvalidResponse.into());
        }

        Self::update_store(&this, cx, |store, cx| {
            store.phase = ShareSendHandoffPhase::Sending { session_id: session_id.clone() };
            cx.notify();
        })?;
        let body = Self::make_share_payload_body(&session, media_items);
        let response = Self::send_with_retry(&this, cx, body, &session, generation, &account_id, epoch.clone()).await?;
        Self::ensure_current_in(&this, cx, generation, &account_id, epoch.as_deref())?;
        let acknowledgement = Self::validate_acknowledgement(&response, &session)?;

        if let ShareSendAcknowledgement::Partial { message, delivered, failed } = acknowledgement {
            for result in response.results.iter().filter(|result| result.success) {
                cx.update(|cx| RiseShareAppBridge::donate_outgoing_interaction(&result.chat_id, cx));
            }
            Self::update_store(&this, cx, |store, cx| {
                store.release_operation_task();
                store.operation_generation = None;
                store.phase = ShareSendHandoffPhase::Partial {
                    session_id: session_id.clone(),
                    message,
                    delivered,
                    failed,
                };
                cx.notify();
            })?;
            let _ = notification(NotificationKind::Warning);
            return Ok(());
        }

        RiseShareContainer::remove_acknowledged_send_session(&session_id, &account_id);
        for destination in &session.destinations {
            cx.update(|cx| RiseShareAppBridge::donate_outgoing_interaction(&destination.chat_id, cx));
        }
        let destination_count = session.destinations.len() as i64;
        Self::update_store(&this, cx, |store, cx| {
            store.release_operation_task();
            store.operation_generation = None;
            store.commit_started_session_ids.remove(&session_id);
            store.phase = ShareSendHandoffPhase::Succeeded {
                session_id: session_id.clone(),
                destination_count,
            };
            cx.notify();
        })?;
        let _ = notification(NotificationKind::Success);
        cx.background_executor().timer(SUCCEEDED_HOLD).await;
        Self::update_store(&this, cx, |store, cx| {
            if store.phase.active_session_id() != Some(session_id.as_str()) {
                return;
            }
            store.phase = ShareSendHandoffPhase::Idle;
            cx.notify();
            store.resume_if_possible(cx);
        })?;
        Ok(())
    }

    async fn upload_and_persist(
        this: &WeakEntity<Self>,
        cx: &mut AsyncApp,
        step: UploadStep<'_>,
    ) -> anyhow::Result<RiseShareSendSession> {
        let prepared = RiseConversationUploadSupport::prepared_local_file(step.original, step.compressed);
        Self::ensure_current_in(this, cx, step.generation, step.account_id, step.epoch.as_deref())?;
        let upload_sender = step.progress_sender;
        let preparation_share = step.preparation_share;
        let upload = cx.update(|cx| {
            ShareSendHandoffActionsStore::shared(cx).update(cx, |actions, cx| {
                actions.upload_media(
                    step.original.clone(),
                    step.compressed.clone(),
                    prepared.filename.clone(),
                    step.upload_chat_id.to_owned(),
                    step.account_id.to_owned(),
                    step.epoch.clone(),
                    Box::new(move |transfer: RiseChunkedUploadTransferProgress| {
                        let _ = upload_sender.unbounded_send(ShareSendFileTransfer {
                            file_fraction: preparation_share + transfer.fraction * (1.0 - preparation_share),
                            sent_bytes: transfer.completed_bytes,
                            file_bytes: transfer.total_bytes,
                        });
                    }),
                    cx,
                )
            })
        });
        let completed = upload.await?;
        let media = Self::make_media_item(&completed, step.attachment, &prepared.filename, step.compressed)?;
        let session_id = step.session_id.to_owned();
        let account_id = step.account_id.to_owned();
        let attachment_id = step.attachment.id.clone();
        let session = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                RiseShareContainer::persist_send_uploaded_media(&session_id, &account_id, &attachment_id, media)
            })
            .await
            .map_err(|_| CancellationError)??;
        Ok(session)
    }

    fn report_file_progress(
        &mut self,
        progress: &ShareSendFileProgress,
        transfer: ShareSendFileTransfer,
        cx: &mut Context<Self>,
    ) {
        let ShareSendFileTransfer { file_fraction, sent_bytes, file_bytes } = transfer;
        if self.operation_generation != Some(progress.generation)
            || self.phase.active_session_id() != Some(progress.session_id.as_str())
        {
            return;
        }
        let done_file_bytes = progress.attachment_bytes as f64 * file_fraction.clamp(0.0, 1.0);
        let fraction = if progress.total_bytes > 0 {
            (progress.done_bytes as f64 + done_file_bytes) / progress.total_bytes as f64
        } else {
            file_fraction
        };
        if let ShareSendHandoffPhase::Uploading { item, fraction: shown, .. } = &self.phase {
            if *item == progress.item && *shown > fraction {
                return;
            }
        }
        self.phase = ShareSendHandoffPhase::Uploading {
            session_id: progress.session_id.clone(),
            filename: progress.filename.clone(),
            item: progress.item,
            total: progress.total,
            fraction,
            sent_bytes,
            file_bytes,
        };
        cx.notify();
    }

    async fn send_with_retry(
        this: &WeakEntity<Self>,
        cx: &mut AsyncApp,
        body: SharePayload,
        session: &RiseShareSendSession,
        generation: Uuid,
        account_id: &str,
        epoch: Option<String>,
    ) -> anyhow::Result<RiseStoredSharePayloadReply> {
        let mut last_error: Option<anyhow::Error> = None;
        for attempt in 0..SEND_ATTEMPTS {
            Self::ensure_current_in(this, cx, generation, account_id, epoch.as_deref())?;
            let commit_session_id = session.id.clone();
            let commit_account_id = account_id.to_owned();
            let commit_store = this.clone();
            let on_commit_started = Box::new(move |cx: &mut App| -> anyhow::Result<()> {
                let Some(store) = commit_store.upgrade() else {
                    return Err(CancellationError.into());
                };
                RiseShareContainer::mark_send_session_commit_started(&commit_session_id, &commit_account_id)?;
                store.update(cx, |store, _| {
                    store.commit_started_session_ids.insert(commit_session_id);
                });
                Ok(())
            });
            let request = body.clone();
            let call = cx.update(|cx| {
                ShareSendHandoffActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.share_payload_action(request, true, on_commit_started, cx)
                })
            });
            match call.await {
                Ok(response) => {
                    Self::ensure_current_in(this, cx, generation, account_id, epoch.as_deref())?;
                    let failure_results: Vec<_> = response.results.iter().filter(|result| !result.success).collect();
                    if failure_results.is_empty() {
                        return Ok(response);
                    }
                    if failure_results.iter().any(|result| !result.retryable) || attempt == SEND_ATTEMPTS - 1 {
                        return Ok(response);
                    }
                }
                Err(error) if error.is::<CancellationError>() => return Err(error),
                Err(error) => {
                    if attempt == SEND_ATTEMPTS - 1 {
                        return Err(error);
                    }
                    last_error = Some(error);
                }
            }
            let delay = if attempt == 0 { FIRST_RETRY_DELAY } else { LATER_RETRY_DELAY };
            cx.background_executor().timer(delay).await;
        }
        Err(last_error.unwrap_or_else(|| ShareSendHandoffError::InvalidResponse.into()))
    }

    fn validate_acknowledgement(
        response: &RiseStoredSharePayloadReply,
        session: &RiseShareSendSession,
    ) -> anyhow::Result<ShareSendAcknowledgement> {
        if response.idempotency_key.to_lowercase() != session.idempotency_key.to_lowercase() {
            return Err(ShareSendHandoffError::InvalidResponse.into());
        }
        let expected: HashSet<String> = session
            .destinations
            .iter()
            .map(|destination| Self::result_key(&destination.chat_id, destination.topic_id.as_deref()))
            .collect();
        let mut received = HashMap::new();
        for result in &response.results {
            received
                .entry(Self::result_key(&result.chat_id, result.topic_id.as_deref()))
                .or_insert(result);
        }
        let received_keys: HashSet<String> = received.keys().cloned().collect();
        if received.len() != response.results.len() || received_keys != expected {
            return Err(ShareSendHandoffError::InvalidResponse.into());
        }
        let failed_destinations: Vec<String> = session
            .destinations
            .iter()
            .filter_map(|destination| {
                let key = Self::result_key(&destination.chat_id, destination.topic_id.as_deref());
                let result = received.get(&key);
                if result.is_some_and(|result| result.success) {
                    return None;
                }
                let detail = result
                    .and_then(|result| {
                        result
                            .refusal()
                            .and_then(|failure| CanonErrors::text_for(&failure))
                            .or_else(|| result.error.clone())
                    })
                    .map(|detail| trimming_whitespaces_and_newlines(&detail).to_owned())
                    .unwrap_or_default();
                Some(if detail.is_empty() {
                    destination.title.clone()
                } else {
                    format!(
                        "{}: {}",
                        destination.title,
                        character_prefix(&detail, MAXIMUM_FAILURE_DETAIL_CHARACTERS)
                    )
                })
            })
            .collect();
        let delivered = response.results.iter().filter(|result| result.success).count() as i64;
        if !failed_destinations.is_empty() {
            let failed = failed_destinations.len() as i64;
            let error = ShareSendHandoffError::PartialDelivery {
                delivered,
                failed: failed_destinations,
            };
            return Ok(ShareSendAcknowledgement::Partial {
                message: error.to_string(),
                delivered,
                failed,
            });
        }
        if !response.all_succeeded {
            return Err(ShareSendHandoffError::InvalidResponse.into());
        }
        Ok(ShareSendAcknowledgement::Complete)
    }

    fn make_share_payload_body(session: &RiseShareSendSession, media_items: Vec<MediaItemsElement>) -> SharePayload {
        let destinations = session
            .destinations
            .iter()
            .map(|destination| DestinationsElement {
                chat_id: Some(destination.chat_id.clone()),
                topic_id: destination.topic_id.clone().filter(|topic_id| !topic_id.is_empty()),
            })
            .collect();
        let comment = trimming_whitespaces_and_newlines(&session.comment).to_owned();
        let content_type = media_items
            .first()
            .and_then(|item| item.media_type.clone())
            .unwrap_or_else(|| "text".to_owned());
        SharePayload {
            comment: (!comment.is_empty()).then_some(comment),
            content: Some(session.text_content.clone()),
            content_type: Some(content_type),
            destinations: Some(destinations),
            media_items: Some(media_items),
            ..SharePayload::new(session.idempotency_key.clone())
        }
    }

    fn make_media_item(
        upload_response: &Map<String, Value>,
        attachment: &RiseShareSendAttachment,
        filename: &str,
        uploaded: &OutgoingMediaCompressorResult,
    ) -> anyhow::Result<BTreeMap<String, RiseShareSendJSONValue>> {
        let Some(file_url) = Self::first_string(upload_response, &[message::FILE_URL, bot::URL]) else {
            return Err(ShareSendHandoffError::InvalidResponse.into());
        };
        if file_url.is_empty() || uploaded.byte_count <= 0 || uploaded.content_type.is_empty() {
            return Err(ShareSendHandoffError::InvalidResponse.into());
        }
        let mut media = BTreeMap::from([
            (
                bot::MEDIA_TYPE.to_owned(),
                RiseShareSendJSONValue::String(Self::media_type(&attachment.kind).to_owned()),
            ),
            (bot::FILE_URL.to_owned(), RiseShareSendJSONValue::String(file_url)),
            (bot::FILE_NAME.to_owned(), RiseShareSendJSONValue::String(filename.to_owned())),
            (bot::FILE_SIZE.to_owned(), RiseShareSendJSONValue::Integer(uploaded.byte_count)),
            (bot::MIME_TYPE.to_owned(), RiseShareSendJSONValue::String(uploaded.content_type.clone())),
        ]);
        for key in [
            message::FILE_ID,
            message::THUMBNAIL_URL,
            bot::WIDTH,
            bot::HEIGHT,
            bot::DURATION,
            bot::BITRATE,
            bot::FPS,
            bot::CODEC,
            bot::WAVEFORM,
        ] {
            if let Some(encoded) = upload_response.get(key).and_then(Self::persisted_json_value) {
                media.insert(key.to_owned(), encoded);
            }
        }
        if !media.contains_key(bot::FILE_ID) {
            if let Some(file_key) = Self::first_string(upload_response, &[upload::FILE_KEY]) {
                let stem = Self::deleting_path_extension(&Self::last_path_component(&file_key));
                if !stem.is_empty() {
                    media.insert(bot::FILE_ID.to_owned(), RiseShareSendJSONValue::String(stem));
                }
            }
        }
        Ok(media)
    }

    fn validated_media_item(attachment: &RiseShareSendAttachment) -> anyhow::Result<MediaItemsElement> {
        let invalid = || anyhow::Error::new(ShareSendHandoffError::InvalidResponse);
        let Some(media) = &attachment.uploaded_media else { return Err(invalid()) };
        let valid = matches!(
            media.get(bot::MEDIA_TYPE),
            Some(RiseShareSendJSONValue::String(media_type)) if media_type == Self::media_type(&attachment.kind)
        ) && matches!(
            media.get(bot::FILE_URL),
            Some(RiseShareSendJSONValue::String(file_url)) if !file_url.is_empty()
        ) && matches!(
            media.get(bot::FILE_SIZE),
            Some(RiseShareSendJSONValue::Integer(file_size)) if *file_size > 0
        ) && matches!(
            media.get(bot::MIME_TYPE),
            Some(RiseShareSendJSONValue::String(mime)) if !mime.is_empty()
        );
        if !valid {
            return Err(invalid());
        }
        let object = Value::Object(
            media
                .iter()
                .map(|(key, value)| (key.clone(), Self::any_json_value(value)))
                .collect(),
        );
        SaiJSONCodec::decode::<MediaItemsElement>(Some(&object)).ok_or_else(invalid)
    }

    fn ensure_current(
        &self,
        generation: Uuid,
        account_id: &str,
        epoch: Option<&str>,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        if self.operation_generation != Some(generation)
            || Self::current_account_id(cx).as_deref() != Some(account_id)
            || !SaiAccountCacheScope::is_current_account_epoch(Some(account_id), epoch)
        {
            return Err(ShareSendHandoffError::AccountChanged.into());
        }
        Ok(())
    }

    /// a released store counts as a cancelled task
    fn ensure_current_in(
        this: &WeakEntity<Self>,
        cx: &mut AsyncApp,
        generation: Uuid,
        account_id: &str,
        epoch: Option<&str>,
    ) -> anyhow::Result<()> {
        Self::update_store(this, cx, |store, cx| store.ensure_current(generation, account_id, epoch, cx))?
    }

    fn update_store<R>(
        this: &WeakEntity<Self>,
        cx: &mut AsyncApp,
        update: impl FnOnce(&mut Self, &mut Context<Self>) -> R,
    ) -> anyhow::Result<R> {
        this.update(cx, update).map_err(|_| CancellationError.into())
    }

    fn current_account_id(cx: &mut App) -> Option<String> {
        let auth = AuthServicesStore::shared(cx);
        let auth = auth.read(cx);
        if !auth.is_authenticated() {
            return None;
        }
        let id = auth.effective_user_id_for_wire()?;
        let id = trimming_whitespaces_and_newlines(&id).to_owned();
        (!id.is_empty()).then_some(id)
    }

    fn failure_text(error: &anyhow::Error) -> String {
        if let Some(text) = error
            .downcast_ref::<ShareSendHandoffError>()
            .and_then(ShareSendHandoffError::error_description)
        {
            return text;
        }
        if let Some(RiseShareContainerError::AccountMismatch) = error.downcast_ref::<RiseShareContainerError>() {
            return ShareSendHandoffError::AccountChanged
                .error_description()
                .unwrap_or_else(|| CanonErrors::refusal_text(error));
        }
        CanonErrors::refusal_text(error)
    }

    fn is_missing_session(error: &RiseShareContainerError) -> bool {
        match error {
            RiseShareContainerError::File(cocoa) => {
                matches!(cocoa.code, CocoaErrorCode::FileReadNoSuchFile | CocoaErrorCode::FileNoSuchFile)
            }
            RiseShareContainerError::Io(io) => io.kind() == std::io::ErrorKind::NotFound,
            _ => false,
        }
    }

    fn result_key(chat_id: &str, topic_id: Option<&str>) -> String {
        format!("{chat_id}#{}", topic_id.map(trimming_whitespaces_and_newlines).unwrap_or(""))
    }

    fn safe_filename(preferred: Option<&str>, fallback: &str) -> String {
        let source = preferred.filter(|preferred| !preferred.is_empty()).unwrap_or(fallback);
        let leaf = Self::last_path_component(source);
        let value = trimming_whitespaces_and_newlines(&leaf);
        if value.is_empty() || value == "." || value == ".." {
            "Shared file".to_owned()
        } else {
            character_prefix(value, MAXIMUM_FILENAME_CHARACTERS).to_owned()
        }
    }

    fn media_type(kind: &str) -> &'static str {
        match kind {
            "image" => "image",
            "video" => "video",
            "audio" => "audio",
            _ => "document",
        }
    }

    fn first_string(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
        for key in keys {
            match object.get(*key) {
                Some(Value::String(value)) if !value.is_empty() => return Some(value.clone()),
                Some(Value::Number(value)) => return Some(value.to_string()),
                _ => {}
            }
        }
        None
    }

    fn persisted_json_value(value: &Value) -> Option<RiseShareSendJSONValue> {
        Some(match value {
            Value::Null => RiseShareSendJSONValue::Null,
            Value::String(value) => RiseShareSendJSONValue::String(value.clone()),
            Value::Bool(value) => RiseShareSendJSONValue::Bool(*value),
            Value::Number(number) => {
                let double = number.as_f64()?;
                if double.round() == double {
                    RiseShareSendJSONValue::Integer(number.as_i64().unwrap_or(double as i64))
                } else {
                    RiseShareSendJSONValue::Double(double)
                }
            }
            Value::Array(values) => {
                RiseShareSendJSONValue::Array(values.iter().filter_map(Self::persisted_json_value).collect())
            }
            Value::Object(object) => RiseShareSendJSONValue::Object(
                object
                    .iter()
                    .filter_map(|(key, value)| Self::persisted_json_value(value).map(|value| (key.clone(), value)))
                    .collect(),
            ),
        })
    }

    fn any_json_value(value: &RiseShareSendJSONValue) -> Value {
        match value {
            RiseShareSendJSONValue::String(value) => Value::String(value.clone()),
            RiseShareSendJSONValue::Integer(value) => Value::from(*value),
            RiseShareSendJSONValue::Double(value) => serde_json::Number::from_f64(*value).map_or(Value::Null, Value::Number),
            RiseShareSendJSONValue::Bool(value) => Value::Bool(*value),
            RiseShareSendJSONValue::Array(values) => Value::Array(values.iter().map(Self::any_json_value).collect()),
            RiseShareSendJSONValue::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), Self::any_json_value(value)))
                    .collect(),
            ),
            RiseShareSendJSONValue::Null => Value::Null,
        }
    }

    fn notify_invalid_session(cx: &mut App) {
        Notifier::send(
            tr("share_send_handoff_invalid_title"),
            NotifierSendOptions {
                content: Some(tr("share_send_handoff_invalid")),
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..Default::default()
            },
            cx,
        );
    }

    fn standardized(path: &Path) -> Option<Url> {
        Url::from_file_path(path).map(|url| url.standardized())
    }

    /// part after the last slash
    fn last_path_component(path: &str) -> String {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            return if path.is_empty() { String::new() } else { "/".to_owned() };
        }
        trimmed.rsplit('/').next().unwrap_or(trimmed).to_owned()
    }

    fn deleting_path_extension(component: &str) -> String {
        match component.rfind('.') {
            Some(dot) if dot > 0 => component[..dot].to_owned(),
            _ => component.to_owned(),
        }
    }
}

/// what the progress closure captures for one attachment
#[derive(Clone)]
struct ShareSendFileProgress {
    session_id: String,
    generation: Uuid,
    filename: String,
    item: i64,
    total: i64,
    attachment_bytes: i64,
    done_bytes: i64,
    total_bytes: i64,
}

/// locals the upload of one attachment reads
struct UploadStep<'a> {
    original: &'a RiseOutgoingMediaFile,
    compressed: &'a OutgoingMediaCompressorResult,
    attachment: &'a RiseShareSendAttachment,
    upload_chat_id: &'a str,
    session_id: &'a str,
    account_id: &'a str,
    epoch: Option<String>,
    generation: Uuid,
    preparation_share: f64,
    progress_sender: mpsc::UnboundedSender<ShareSendFileTransfer>,
}

/// a files share of its ring slice and its bytes on the wire, both 0 while its prepared
#[derive(Clone, Copy, Debug)]
struct ShareSendFileTransfer {
    file_fraction: f64,
    sent_bytes: i64,
    file_bytes: i64,
}

struct ShareSendGeneratedFile(Option<PathBuf>);

impl Drop for ShareSendGeneratedFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = FileManager.remove_item(path);
        }
    }
}
