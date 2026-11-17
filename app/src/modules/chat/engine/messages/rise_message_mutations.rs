use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::{Future, Ready};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use gpui::{App, AsyncApp};
use rise_canon::rise_wire_canon::key as wire_key;
use rise_canon::{RiseTimingCanon, rise_key, rise_request};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_core::rise_identifiers::{RiseRequestID, RiseRequestIDAllocator};
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RiseEntityViewMembership, RiseEntityViewRetention, RiseEntityWindow,
    RiseEntityWindowQuery, RiseEntityWritePolicy, RisePositionKey, RiseStoreEntity,
};
use rise_engine::rise_store::rise_store_models::{
    RiseDurableOperationState, RiseDurableOrderingKey, RiseStoreError, RiseStoreLimits, RiseStoreMutation,
    RiseStoreWriteBatch,
};
use rise_engine::rise_sync::RiseSyncSubmitOptions;
use rise_engine::rise_upload::rise_upload_progress::{
    RiseUploadCancellationSignal, RiseUploadKey, RiseUploadPhase, RiseUploadProgress,
};
use rise_engine::{CancellationError, RiseJSONValue, RiseWireError, RiseWirePushEnvelope};
use tokio::sync::oneshot;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseConversationEntitySource, RiseMessageEntityRecord, RiseMessageForwardRetry, RiseMessageLocalPhase,
};
use crate::modules::chat::engine::core::rise_conversation_rpc::{RiseConversationEndpoint, RiseConversationRPC};
use crate::modules::chat::engine::core::rise_conversation_store_layout::{
    RiseConversationStoreLayout, RiseEntityAliasKindExt,
};
use crate::modules::chat::engine::messages::rise_message_contracts::{
    RiseAuthoritativeMessage, RiseMessageAuxiliaryInvalidation, RiseMessageAuxiliaryInvalidationPinnedMessagePatch,
    RiseMessageAuxiliaryProjection, RiseMessageChatReadOptimisticHookPrepared, RiseMessageDraft,
    RiseMessagePreviewChange, RiseMessageRepositoryError, RiseMessageSubmission, RiseMessageSubmissionMode,
    RiseMessageUploadEvent, RiseMessageUploadEvents,
};
use crate::modules::chat::engine::messages::rise_message_durable_outbox::{
    RiseAddReactionDurableEndpoint, RiseCreateMessageDurableEndpoint, RiseCreateMessageDurablePayload,
    RiseCreateMessageWireMethod, RiseReactionDurablePayload, RiseRemoveReactionDurableEndpoint,
};
use crate::modules::chat::engine::messages::rise_message_realtime::RiseMessageRepositoryRealtimeEvent;
use crate::modules::chat::engine::messages::rise_message_repository::{
    RiseMessageRepository, RiseMessageRepositoryEncodedRow, RiseMessageSendOrderGate,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_share_fanout::OptimisticFanoutRow;
use crate::modules::chat::engine::messages::rise_message_upload_retry::{
    RiseMessageUploadRetry, RiseMessageUploadRetryCodec,
};
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_stored_canon::create_message_from_stored;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    CreateMessageGatewayDTO, DeleteAllMessagesFromUserGatewayDTO, ForwardMessagesBody, ForwardMessagesGatewayDTO,
    MessageWireDTO, RiseStoredOutgoingMessage, UnpinAllMessagesGatewayDTO,
};

const MESSAGE_LOG_NAME: &str = "Message";
const REMOTE_REJECTED_CODE: &str = "remote_rejected";
const ORDERING_KEY_CHARACTERS: usize = 256;
const COMPENSATION_POLL: Duration = Duration::from_millis(100);
const FORWARD_REPLAY_POLL: Duration = Duration::from_secs(2);
const FORWARD_REPLAY_RUNG: usize = 4;
const NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

type RiseMessageNoSessionValidation = fn() -> Ready<anyhow::Result<()>>;

#[derive(Clone, Debug)]
pub struct MessageMutationCheckpoint {
    pub entity: RiseStoreEntity,
    pub server_id: String,
    pub topic_id: Option<String>,
    pub optimistic_data: Vec<u8>,
    pub acknowledgement_version: i64,
}

pub struct SendOrderTurn {
    pub chat_id: String,
    pub predecessor: Option<RiseMessageSendOrderGate>,
    pub gate: RiseMessageSendOrderGate,
    pub release: oneshot::Sender<()>,
}

#[derive(Clone)]
pub struct SendOrderGate {
    pub task: RiseMessageSendOrderGate,
    pub carries_uploads: bool,
}

/// a mark_chat_as_read waiting for the read already on the wire for its chat
#[derive(Clone, Debug)]
pub struct RiseMessageQueuedRead {
    pub scope: RiseMessageScope,
    pub clearing_marks: bool,
    pub up_to_message_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BulkMessagePatchCheckpoint {
    pub originals: Vec<RiseStoreEntity>,
    pub optimistic: Vec<(String, Vec<u8>)>,
    pub acknowledgement_version: i64,
}

#[derive(Clone, Debug)]
pub struct MessageDeletionCheckpointPosition {
    pub view: String,
    pub key: RisePositionKey,
}

#[derive(Clone, Debug)]
pub struct MessageDeletionCheckpoint {
    pub entity: RiseStoreEntity,
    pub positions: Vec<MessageDeletionCheckpointPosition>,
}

/// entity plus view and position pairs, the element of the checkpoints delete keeps
type RiseMessageDeleteCheckpoint = (RiseStoreEntity, Vec<(String, RisePositionKey)>);

/// what create writes through its failure path, so both send modes share it
struct RiseMessageSendFailure<'a> {
    wire: &'a MessageWireDTO,
    scope: &'a RiseMessageScope,
    scopes: &'a [RiseMessageScope],
    local_id: &'a str,
    request_string: &'a str,
    version: i64,
    upload_retry: Option<RiseMessageUploadRetry>,
    upload_count: usize,
    has_uploads: bool,
}

struct SendOrderTurnRelease<'a> {
    repository: &'a RiseMessageRepository,
    turn: Option<SendOrderTurn>,
}

impl Drop for SendOrderTurnRelease<'_> {
    fn drop(&mut self) {
        if let Some(turn) = self.turn.take() {
            self.repository.release_send_order_turn(turn);
        }
    }
}

/// ends an owner claim on its uploading row
struct LiveUploadOwnerRelease<'a> {
    repository: &'a RiseMessageRepository,
    owner_id: String,
}

impl Drop for LiveUploadOwnerRelease<'_> {
    fn drop(&mut self) {
        self.repository.state.with(|state| {
            state.live_upload_owner_ids.remove(&self.owner_id);
        });
    }
}

struct ReadFlightRelease<'a> {
    repository: &'a RiseMessageRepository,
    chat_id: String,
}

impl Drop for ReadFlightRelease<'_> {
    fn drop(&mut self) {
        self.repository.state.with(|state| {
            state.read_flight_chat_ids.remove(&self.chat_id);
        });
    }
}

struct RiseMessageSendCancellationGuard {
    repository: Weak<RiseMessageRepository>,
    local_id: String,
    scope: RiseMessageScope,
    armed: bool,
    finishes_upload: bool,
    removes_row: bool,
}

impl Drop for RiseMessageSendCancellationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(repository) = self.repository.upgrade() else {
            return;
        };
        let local_id = std::mem::take(&mut self.local_id);
        let scope = self.scope.clone();
        let finishes_upload = self.finishes_upload;
        let removes_row = self.removes_row;
        let _ = RiseAccountRuntime::current().spawn(async move {
            repository.cancel_send(&local_id, &scope, finishes_upload, removes_row).await;
        });
    }
}

struct RiseMessageRollbackGuard {
    repository: Weak<RiseMessageRepository>,
    rollback: Option<Box<dyn FnOnce(Arc<RiseMessageRepository>) -> BoxFuture<'static, ()> + Send>>,
}

impl RiseMessageRollbackGuard {
    fn disarm(mut self) {
        self.rollback = None;
    }
}

impl Drop for RiseMessageRollbackGuard {
    fn drop(&mut self) {
        if let (Some(rollback), Some(repository)) = (self.rollback.take(), self.repository.upgrade()) {
            let _ = RiseAccountRuntime::current().spawn(rollback(repository));
        }
    }
}

/// runs work on the main thread, none only before the main hop is installed
async fn on_main<R, W>(work: W) -> Option<R>
where
    R: Send + 'static,
    W: FnOnce(&mut App) -> R + Send + 'static,
{
    let main = RiseEngineMainHop::installed()?;
    main.run(move |cx: AsyncApp| async move { Ok(cx.update(work)) }.boxed_local()).await.ok()
}

/// sleep for callers that may be awaiting on main, off the runtime
async fn sleep_on_runtime(duration: Duration) -> anyhow::Result<()> {
    RiseAccountRuntime::current()
        .spawn(async move { tokio::time::sleep(duration).await })
        .await
        .map_err(|_| anyhow::Error::from(CancellationError))
}

fn write_batch(mutations: Vec<RiseStoreMutation>) -> RiseStoreWriteBatch {
    RiseStoreWriteBatch::new(mutations, HashSet::new())
}

fn is_durable_state_precondition(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<RiseStoreError>(),
        Some(RiseStoreError::DurableOperationStatePreconditionFailed(_))
    )
}

fn upload_slot_progress(local_id: &str, index: i64, fraction: f64, phase: RiseUploadPhase) -> RiseUploadProgress {
    RiseUploadProgress::new(RiseUploadKey::new(local_id, index), fraction, 0, 0, phase)
}

fn upload_event_progress(local_id: &str, event: RiseMessageUploadEvent) -> RiseUploadProgress {
    match event {
        RiseMessageUploadEvent::Preparing { index, fraction, total_bytes } => RiseUploadProgress::new(
            RiseUploadKey::new(local_id, index),
            fraction,
            0,
            total_bytes,
            RiseUploadPhase::Preparing,
        ),
        RiseMessageUploadEvent::Uploading { index, fraction, completed_bytes, total_bytes } => {
            RiseUploadProgress::new(
                RiseUploadKey::new(local_id, index),
                fraction,
                completed_bytes,
                total_bytes,
                RiseUploadPhase::Uploading,
            )
        }
        RiseMessageUploadEvent::Processing { index, fraction, total_bytes } => RiseUploadProgress::new(
            RiseUploadKey::new(local_id, index),
            fraction,
            0,
            total_bytes,
            RiseUploadPhase::Processing,
        ),
        RiseMessageUploadEvent::Completed { index, total_bytes } => RiseUploadProgress::new(
            RiseUploadKey::new(local_id, index),
            1.0,
            total_bytes,
            total_bytes,
            RiseUploadPhase::Completed,
        ),
    }
}

impl RiseMessageRepository {
    pub const UNDELIVERABLE_ERROR_CODE: &'static str = "message_undeliverable";
    pub const INTERRUPTED_UPLOAD_ERROR_CODE: &'static str = "upload_interrupted";
    pub const DELETE_SLICE_SIZE: usize = 128;

    pub async fn create(
        &self,
        draft: RiseMessageDraft,
        scope: &RiseMessageScope,
        mode: RiseMessageSubmissionMode,
    ) -> anyhow::Result<RiseMessageSubmission> {
        self.state.with(|state| {
            state.known_scopes.insert(scope.clone());
        });
        let send_turn = self.take_send_order_turn(&scope.chat_id, !draft.local_uploads.is_empty());
        let predecessor = send_turn.predecessor.clone();
        let _turn_release = SendOrderTurnRelease { repository: self, turn: Some(send_turn) };
        let mut _waiting_row_release: Option<LiveUploadOwnerRelease<'_>> = None;
        let request_id = RiseRequestIDAllocator::shared().next(Date::now());
        let mut body = draft.body.clone();
        let local_id = RiseMessageWireCodec::normalized(body.temp_id.as_deref())
            .unwrap_or_else(|| format!("temp_{}", request_id.raw_value()));
        body.chat_id = body.chat_id.take().or_else(|| Some(scope.chat_id.clone()));
        body.topic_id = body.topic_id.take().or_else(|| scope.topic_id.clone());
        body.temp_id = Some(local_id.clone());
        let now: TimeInterval = Date::now().time_interval_since_1970();
        let mut reply_source: Option<MessageWireDTO> = None;
        if let Some(reply_id) = self.normalized(body.reply_to_id.as_deref())
            && let Some(entity) = self.message_entity(&reply_id).await?
            && !RiseMessageWireCodec::is_tombstone(&entity.data)
        {
            let record = RiseMessageWireCodec::record(&entity.data)?;
            if record.wire.chat_id.as_deref() == Some(scope.chat_id.as_str()) {
                reply_source = Some(record.wire);
            }
        }
        let initial_wire = RiseMessageWireCodec::optimistic_wire(
            &body,
            scope,
            &local_id,
            now,
            draft.optimistic_wire.as_ref(),
            reply_source.as_ref(),
            Some(self.account_id.as_str()),
        )?;
        let request_string = request_id.raw_value().to_string();
        let scopes =
            self.scopes_for_message(&scope.chat_id, scope.topic_id.as_deref(), scope.thread_root_id.as_deref());
        let has_uploads = !draft.local_uploads.is_empty();
        let upload_count = draft.local_uploads.len();
        if has_uploads {
            self.state.with(|state| {
                state.live_upload_owner_ids.insert(local_id.clone());
            });
        }
        let _upload_owner_release = (has_uploads && mode != RiseMessageSubmissionMode::AwaitingConversation)
            .then(|| LiveUploadOwnerRelease { repository: self, owner_id: local_id.clone() });
        let initial_version = self.next_version();
        if mode == RiseMessageSubmissionMode::AwaitingConversation {
            self.state.with(|state| {
                state.live_upload_owner_ids.insert(local_id.clone());
            });
            let commit = self
                .engine
                .write(
                    self.create_optimistic_batch(
                        &initial_wire,
                        scope,
                        &scopes,
                        &local_id,
                        &request_string,
                        RiseMessageLocalPhase::Uploading,
                        initial_version,
                        None,
                        None,
                    )
                    .await?,
                )
                .await?;
            self.invalidate_auxiliary_scope(scope, &RiseMessageAuxiliaryProjection::content()).await;
            return Ok(RiseMessageSubmission {
                request_id,
                local_id,
                entity_version: initial_version,
                store_revision: commit.revision,
            });
        }
        if mode == RiseMessageSubmissionMode::Undeliverable {
            self.engine
                .write(
                    self.create_optimistic_batch(
                        &initial_wire,
                        scope,
                        &scopes,
                        &local_id,
                        &request_string,
                        RiseMessageLocalPhase::Queued,
                        initial_version,
                        None,
                        None,
                    )
                    .await?,
                )
                .await?;
            let failure_version = self.next_version_floor(initial_version);
            let commit = self
                .engine
                .write(
                    self.failure_batch(
                        &initial_wire,
                        scope,
                        &scopes,
                        &local_id,
                        &request_string,
                        Self::UNDELIVERABLE_ERROR_CODE,
                        failure_version,
                        None,
                        None,
                    )
                    .await?,
                )
                .await?;
            self.invalidate_auxiliary_scope(scope, &RiseMessageAuxiliaryProjection::content()).await;
            return Ok(RiseMessageSubmission {
                request_id,
                local_id,
                entity_version: failure_version,
                store_revision: commit.revision,
            });
        }
        let initial_retry = if has_uploads { self.upload_retry(&draft, &body, scope) } else { None };
        let initial_batch = self
            .create_optimistic_batch(
                &initial_wire,
                scope,
                &scopes,
                &local_id,
                &request_string,
                if has_uploads { RiseMessageLocalPhase::Uploading } else { RiseMessageLocalPhase::Queued },
                initial_version,
                initial_retry,
                None,
            )
            .await?;

        let composed_body = body.clone();
        let mut durable_wire = initial_wire.clone();
        let mut durable_optimistic_batch = initial_batch.clone();
        let mut durable_optimistic_version = initial_version;
        let mut cancellation_guard = RiseMessageSendCancellationGuard {
            repository: self.this.clone(),
            local_id: local_id.clone(),
            scope: scope.clone(),
            armed: false,
            finishes_upload: has_uploads,
            removes_row: false,
        };
        if has_uploads {
            self.engine.write(initial_batch).await?;
            self.upload_cancellation_hub.begin(&local_id).await;
            self.upload_progress_hub.begin(&local_id).await;
            cancellation_guard.armed = true;
            cancellation_guard.removes_row = true;
            self.publish_upload_slots(&local_id, upload_count, 0.0, RiseUploadPhase::Preparing).await;
            let prepared: anyhow::Result<(RiseStoredOutgoingMessage, MessageWireDTO, i64, RiseStoreWriteBatch)> = async {
                let upload_pipeline = draft
                    .upload_pipeline
                    .clone()
                    .or_else(|| self.upload_pipeline.clone())
                    .ok_or(RiseMessageRepositoryError::UploadPipelineRequired)?;
                let mut upload_draft = draft.clone();
                upload_draft.body = body.clone();
                let progress_hub = self.upload_progress_hub.clone();
                let events_local_id = local_id.clone();
                let events: RiseMessageUploadEvents =
                    Arc::new(move |event: RiseMessageUploadEvent| -> BoxFuture<'static, ()> {
                        let progress = upload_event_progress(&events_local_id, event);
                        let progress_hub = progress_hub.clone();
                        async move { progress_hub.publish(progress).await }.boxed()
                    });
                let mut prepared_body = upload_pipeline
                    .prepare(upload_draft, local_id.clone(), self.upload_cancellation_hub.clone(), events)
                    .await?;
                prepared_body.chat_id = prepared_body.chat_id.take().or_else(|| Some(scope.chat_id.clone()));
                prepared_body.topic_id = prepared_body.topic_id.take().or_else(|| scope.topic_id.clone());
                prepared_body.temp_id = Some(local_id.clone());
                let cancelled_slot_indices = self.upload_cancellation_hub.cancelled_slot_indices(&local_id).await;
                let wire = RiseMessageWireCodec::updating_optimistic_wire(
                    &initial_wire,
                    &prepared_body,
                    scope,
                    &local_id,
                    now,
                    &cancelled_slot_indices,
                )?;
                let version = self.next_version_floor(initial_version);
                let batch = self
                    .create_optimistic_batch(
                        &wire,
                        scope,
                        &scopes,
                        &local_id,
                        &request_string,
                        RiseMessageLocalPhase::Queued,
                        version,
                        None,
                        None,
                    )
                    .await?;
                Ok((prepared_body, wire, version, batch))
            }
            .await;
            match prepared {
                Ok((prepared_body, wire, version, batch)) => {
                    body = prepared_body;
                    durable_wire = wire;
                    durable_optimistic_version = version;
                    durable_optimistic_batch = batch;
                    cancellation_guard.removes_row = false;
                }
                Err(error) if error.is::<CancellationError>() => {
                    cancellation_guard.armed = false;
                    self.cancel_send(&local_id, scope, true, true).await;
                    return Err(CancellationError.into());
                }
                Err(error) => {
                    cancellation_guard.armed = false;
                    self.upload_cancellation_hub.finish(&local_id).await;
                    let code = Self::error_code(&error);
                    let reason =
                        format!("Rise upload({}/{local_id}) failed [{code}]: {error}", scope.storage_key());
                    on_main(move |cx| Sai::log(MESSAGE_LOG_NAME, move || reason, SaiLogKind::Network, cx)).await;
                    let written = self
                        .write_create_failure(
                            &initial_wire,
                            scope,
                            &scopes,
                            &local_id,
                            &request_string,
                            &code,
                            self.next_version_floor(initial_version),
                            self.upload_retry(&draft, &composed_body, scope),
                            None,
                        )
                        .await;
                    let write_error = written.err().map(|error| format!("{error:#}")).unwrap_or_default();
                    tracing::warn!(
                        target: "riseonly::X1428",
                        "upload prepare failure row local_id={local_id} code={code} write_error={write_error}"
                    );
                    self.publish_upload_slots(&local_id, upload_count, 0.0, RiseUploadPhase::Failed { code }).await;
                    return Err(error);
                }
            }
            let refusal = match &draft.publish_refusal {
                Some(publish_refusal) => publish_refusal().await,
                None => None,
            };
            if let Some(refusal) = refusal {
                cancellation_guard.armed = false;
                let failure = RiseMessageSendFailure {
                    wire: &durable_wire,
                    scope,
                    scopes: &scopes,
                    local_id: &local_id,
                    request_string: &request_string,
                    version: self.next_version_floor(durable_optimistic_version),
                    upload_retry: self.upload_retry(&draft, &composed_body, scope),
                    upload_count,
                    has_uploads,
                };
                self.fail_send(failure, refusal.clone()).await;
                return Err(RiseMessageRepositoryError::RemoteFailure(refusal).into());
            }
        }

        if let Some(predecessor) = predecessor {
            if !has_uploads {
                self.state.with(|state| {
                    state.live_upload_owner_ids.insert(local_id.clone());
                });
                _waiting_row_release = Some(LiveUploadOwnerRelease { repository: self, owner_id: local_id.clone() });
                self.engine
                    .write(
                        self.create_optimistic_batch(
                            &initial_wire,
                            scope,
                            &scopes,
                            &local_id,
                            &request_string,
                            RiseMessageLocalPhase::Uploading,
                            initial_version,
                            RiseMessageUploadRetryCodec::text_retry(&body, scope),
                            None,
                        )
                        .await?,
                    )
                    .await?;
            }
            predecessor.await;
            if !has_uploads {
                let entity = self
                    .message_entity(&local_id)
                    .await?
                    .filter(|entity| !RiseMessageWireCodec::is_tombstone(&entity.data))
                    .ok_or(CancellationError)?;
                durable_optimistic_version = self.next_version_floor(entity.version);
                durable_optimistic_batch = self
                    .create_optimistic_batch(
                        &durable_wire,
                        scope,
                        &scopes,
                        &local_id,
                        &request_string,
                        RiseMessageLocalPhase::Queued,
                        durable_optimistic_version,
                        None,
                        None,
                    )
                    .await?;
            }
        }

        let acknowledgement_version = self.next_version_floor(durable_optimistic_version);
        let payload = RiseCreateMessageDurablePayload {
            body: body.clone(),
            optimistic_wire: durable_wire.clone(),
            scope: scope.clone(),
            materialized_scopes: scopes.clone(),
            local_id: local_id.clone(),
            client_request_id: request_string.clone(),
            version: acknowledgement_version,
            retention_limit: self.retention_limit,
        };
        cancellation_guard.armed = true;
        if mode == RiseMessageSubmissionMode::Immediate {
            let outcome: anyhow::Result<RiseMessageSubmission> = async {
                if has_uploads && self.upload_cancellation_hub.is_cancelled(&local_id, None).await {
                    return Err(CancellationError.into());
                }
                self.engine.write(durable_optimistic_batch).await?;
                let wire_request: rise_request::message::CreateMessage = create_message_from_stored(&body);
                let request = RiseJSONValue::encode(&wire_request)?;
                let response = RiseConversationRPC::new(self.engine.clone())
                    .call_method::<RiseCreateMessageWireMethod>(&request, Some(request_id))
                    .await?;
                let acknowledged = RiseMessageWireCodec::acknowledged_wire(
                    &durable_wire,
                    &CreateMessageGatewayDTO::from_canon(&response)?,
                    &local_id,
                    scope,
                )?;
                let acknowledgement = RiseCreateMessageDurableEndpoint::acknowledgement_batch(&payload, &acknowledged)?;
                let preview = self
                    .preview_hook
                    .call(
                        RiseMessagePreviewChange::Created {
                            scope: scope.clone(),
                            message: acknowledged,
                            was_already_stored: true,
                        },
                        acknowledgement_version,
                    )
                    .await?;
                let commit = self.engine.write(self.combining(acknowledgement, preview)).await?;
                self.apply_held_link_preview(&local_id).await;
                self.invalidate_auxiliary_scope(scope, &RiseMessageAuxiliaryProjection::content()).await;
                self.upload_progress_hub.remove_owner_id(&local_id).await;
                if has_uploads {
                    self.upload_cancellation_hub.finish(&local_id).await;
                }
                Ok(RiseMessageSubmission {
                    request_id,
                    local_id: local_id.clone(),
                    entity_version: acknowledgement_version,
                    store_revision: commit.revision,
                })
            }
            .await;
            cancellation_guard.armed = false;
            return match outcome {
                Ok(submission) => Ok(submission),
                Err(error) if error.is::<CancellationError>() => {
                    self.cancel_send(&local_id, scope, has_uploads, false).await;
                    Err(CancellationError.into())
                }
                Err(error) => {
                    let failure = RiseMessageSendFailure {
                        wire: &durable_wire,
                        scope,
                        scopes: &scopes,
                        local_id: &local_id,
                        request_string: &request_string,
                        version: self.next_version_floor(acknowledgement_version),
                        upload_retry: self
                            .upload_retry(&draft, &composed_body, scope)
                            .or_else(|| if has_uploads { None } else { RiseMessageUploadRetryCodec::text_retry(&body, scope) }),
                        upload_count,
                        has_uploads,
                    };
                    self.fail_send(failure, Self::error_code(&error)).await;
                    Err(error)
                }
            };
        }
        let outcome: anyhow::Result<RiseMessageSubmission> = async {
            if has_uploads && self.upload_cancellation_hub.is_cancelled(&local_id, None).await {
                return Err(CancellationError.into());
            }
            let ordering_key: String =
                format!("message.chat.{}", scope.chat_id).chars().take(ORDERING_KEY_CHARACTERS).collect();
            let receipt = self
                .engine
                .submit_durable_receipt::<RiseCreateMessageDurableEndpoint>(
                    payload,
                    RiseSyncSubmitOptions {
                        ordering_key: RiseDurableOrderingKey::new(ordering_key),
                        request_id: Some(request_id),
                        optimistic: durable_optimistic_batch,
                    },
                )
                .await?;
            self.invalidate_auxiliary_scope(scope, &RiseMessageAuxiliaryProjection::content()).await;
            self.upload_progress_hub.remove_owner_id(&local_id).await;
            if has_uploads {
                self.upload_cancellation_hub.finish(&local_id).await;
            }
            Ok(RiseMessageSubmission {
                request_id,
                local_id: local_id.clone(),
                entity_version: durable_optimistic_version,
                store_revision: receipt.commit.revision,
            })
        }
        .await;
        cancellation_guard.armed = false;
        match outcome {
            Ok(submission) => Ok(submission),
            Err(error) if error.is::<CancellationError>() => {
                self.cancel_send(&local_id, scope, has_uploads, false).await;
                Err(CancellationError.into())
            }
            Err(error) => {
                let failure = RiseMessageSendFailure {
                    wire: &durable_wire,
                    scope,
                    scopes: &scopes,
                    local_id: &local_id,
                    request_string: &request_string,
                    version: self.next_version_floor(acknowledgement_version),
                    upload_retry: self.upload_retry(&draft, &composed_body, scope),
                    upload_count,
                    has_uploads,
                };
                self.fail_send(failure, Self::error_code(&error)).await;
                Err(error)
            }
        }
    }

    /// the cancelled arms of create
    async fn cancel_send(&self, local_id: &str, scope: &RiseMessageScope, finishes_upload: bool, removes_row: bool) {
        if finishes_upload {
            self.upload_cancellation_hub.finish(local_id).await;
        }
        self.upload_progress_hub.remove_owner_id(local_id).await;
        if removes_row
            && let Ok(Some(entity)) = self.message_entity(local_id).await
            && !RiseMessageWireCodec::is_tombstone(&entity.data)
        {
            let _ = self.remove_outgoing_locally(&entity, scope, None).await;
        }
    }

    /// shared failure arm of create, the refused row then every slot marked failed
    async fn fail_send(&self, failure: RiseMessageSendFailure<'_>, code: String) {
        if failure.has_uploads {
            self.upload_cancellation_hub.finish(failure.local_id).await;
        }
        let written = self
            .write_create_failure(
                failure.wire,
                failure.scope,
                failure.scopes,
                failure.local_id,
                failure.request_string,
                &code,
                failure.version,
                failure.upload_retry,
                None,
            )
            .await;
        let write_error = written.err().map(|error| format!("{error:#}")).unwrap_or_default();
        tracing::warn!(
            target: "riseonly::X1492",
            "send failure row local_id={} code={code} write_error={write_error}",
            failure.local_id
        );
        self.publish_upload_slots(failure.local_id, failure.upload_count, 1.0, RiseUploadPhase::Failed { code }).await;
    }

    async fn publish_upload_slots(&self, local_id: &str, count: usize, fraction: f64, phase: RiseUploadPhase) {
        for index in 0..count {
            let index = i64::try_from(index).unwrap_or(i64::MAX);
            self.upload_progress_hub.publish(upload_slot_progress(local_id, index, fraction, phase.clone())).await;
        }
    }

    fn rollback_guard(
        &self,
        rollback: impl FnOnce(Arc<RiseMessageRepository>) -> BoxFuture<'static, ()> + Send + 'static,
    ) -> RiseMessageRollbackGuard {
        RiseMessageRollbackGuard { repository: self.this.clone(), rollback: Some(Box::new(rollback)) }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_optimistic_batch(
        &self,
        wire: &MessageWireDTO,
        scope: &RiseMessageScope,
        scopes: &[RiseMessageScope],
        local_id: &str,
        request_string: &str,
        phase: RiseMessageLocalPhase,
        version: i64,
        upload_retry: Option<RiseMessageUploadRetry>,
        forward_retry: Option<RiseMessageForwardRetry>,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let record = RiseMessageEntityRecord {
            wire: wire.clone(),
            source: RiseConversationEntitySource::Optimistic,
            local_phase: phase,
            client_request_id: Some(request_string.to_owned()),
            last_error_code: None,
            scope_storage_key: Some(scope.storage_key()),
            is_replayable: None,
            upload_retry,
            forward_retry,
        };
        let row = RiseMessageRepositoryEncodedRow {
            local_id: local_id.to_owned(),
            data: RiseMessageWireCodec::record_data(&record)?,
            position: RiseConversationStoreLayout::message_position(wire, local_id, phase),
            aliases: vec![
                RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), local_id),
                RiseEntityAlias::new(RiseEntityAliasKind::request_id(), request_string),
                RiseEntityAlias::new(RiseEntityAliasKind::retained(), local_id),
            ],
            duplicate_local_ids: Vec::new(),
            entity_version: 0,
            is_unchanged: false,
        };
        let preview = self
            .preview_hook
            .call(
                RiseMessagePreviewChange::Created {
                    scope: scope.clone(),
                    message: wire.clone(),
                    was_already_stored: false,
                },
                version,
            )
            .await?;
        Ok(self.combining(self.entity_upsert_batch(&row, scopes, version, None), preview))
    }

    fn upload_retry(
        &self,
        draft: &RiseMessageDraft,
        body: &RiseStoredOutgoingMessage,
        scope: &RiseMessageScope,
    ) -> Option<RiseMessageUploadRetry> {
        if draft.local_uploads.is_empty() {
            return None;
        }
        draft.upload_pipeline.as_ref().or(self.upload_pipeline.as_ref())?.retry(body, scope)
    }

    fn take_send_order_turn(&self, chat_id: &str, carries_uploads: bool) -> SendOrderTurn {
        let (release, released) = oneshot::channel::<()>();
        self.state.with(|state| {
            let open: &[SendOrderGate] = state.send_order_turns.get(chat_id).map(Vec::as_slice).unwrap_or_default();
            let last_text = open.iter().rev().find(|gate| !gate.carries_uploads).map(|gate| gate.task.clone());
            let last_upload = open.iter().rev().find(|gate| gate.carries_uploads).map(|gate| gate.task.clone());
            let last = open.last().map(|gate| gate.task.clone());
            let predecessor: Option<RiseMessageSendOrderGate> = match (last_upload, last) {
                (Some(last_upload), Some(last)) if carries_uploads && !last.ptr_eq(&last_upload) => Some(
                    async move {
                        last_upload.await;
                        last.await;
                    }
                    .boxed()
                    .shared(),
                ),
                (_, last) if carries_uploads => last,
                _ => last_text,
            };
            let waited = predecessor.clone();
            let gate = async move {
                if let Some(waited) = waited {
                    waited.await;
                }
                let _ = released.await;
            }
            .boxed()
            .shared();
            state
                .send_order_turns
                .entry(chat_id.to_owned())
                .or_default()
                .push(SendOrderGate { task: gate.clone(), carries_uploads });
            SendOrderTurn { chat_id: chat_id.to_owned(), predecessor, gate, release }
        })
    }

    fn release_send_order_turn(&self, turn: SendOrderTurn) {
        let SendOrderTurn { chat_id, gate, release, .. } = turn;
        drop(release);
        self.state.with(|state| {
            let Some(open) = state.send_order_turns.get_mut(&chat_id) else {
                return;
            };
            open.retain(|candidate| !candidate.task.ptr_eq(&gate));
            if open.is_empty() {
                state.send_order_turns.remove(&chat_id);
            }
        });
    }

    pub fn settle_orphaned_upload_if_needed(&self, local_id: &str) {
        let settles = self.state.with(|state| {
            !state.live_upload_owner_ids.contains(local_id) && state.settled_orphan_upload_ids.insert(local_id.to_owned())
        });
        if !settles {
            return;
        }
        let Some(this) = self.this.upgrade() else {
            return;
        };
        let local_id = local_id.to_owned();
        let _ = RiseAccountRuntime::current().spawn(async move {
            this.fail_orphaned_upload(&local_id).await;
        });
    }

    async fn fail_orphaned_upload(&self, local_id: &str) {
        let Ok(Some(entity)) = self.message_entity(local_id).await else {
            return;
        };
        if RiseMessageWireCodec::is_tombstone(&entity.data) {
            return;
        }
        let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
            return;
        };
        if record.local_phase != RiseMessageLocalPhase::Uploading {
            return;
        }
        let Some(request_string) = record.client_request_id.clone().filter(|value| !value.is_empty()) else {
            return;
        };
        let is_live = self.state.with(|state| {
            state.live_upload_owner_ids.contains(&entity.local_id) || state.live_upload_owner_ids.contains(local_id)
        });
        if is_live {
            return;
        }
        let fallback_chat_id = record.wire.chat_id.clone().unwrap_or_default();
        let parsed = record.scope_storage_key.as_deref().and_then(RiseMessageScope::parse);
        let Some(scope) = parsed.or_else(|| {
            (!fallback_chat_id.is_empty()).then(|| RiseMessageScope::new(fallback_chat_id.clone(), None, None))
        }) else {
            return;
        };
        let scopes =
            self.scopes_for_message(&scope.chat_id, scope.topic_id.as_deref(), scope.thread_root_id.as_deref());
        self.upload_progress_hub.remove_owner_id(&entity.local_id).await;
        let _ = self
            .write_create_failure(
                &record.wire,
                &scope,
                &scopes,
                &entity.local_id,
                &request_string,
                Self::INTERRUPTED_UPLOAD_ERROR_CODE,
                self.next_version_floor(entity.version),
                record.upload_retry.clone(),
                None,
            )
            .await;
    }

    pub async fn failed_upload_retry(&self, local_id: &str) -> Option<RiseMessageUploadRetry> {
        let entity = self.message_entity(local_id).await.ok().flatten()?;
        if RiseMessageWireCodec::is_tombstone(&entity.data) {
            return None;
        }
        let record = RiseMessageWireCodec::record(&entity.data).ok()?;
        if record.local_phase != RiseMessageLocalPhase::Failed {
            return None;
        }
        record.upload_retry
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn write_create_failure(
        &self,
        wire: &MessageWireDTO,
        scope: &RiseMessageScope,
        scopes: &[RiseMessageScope],
        local_id: &str,
        request_string: &str,
        code: &str,
        version: i64,
        upload_retry: Option<RiseMessageUploadRetry>,
        forward_retry: Option<RiseMessageForwardRetry>,
    ) -> anyhow::Result<()> {
        self.engine
            .write(
                self.failure_batch(
                    wire,
                    scope,
                    scopes,
                    local_id,
                    request_string,
                    code,
                    version,
                    upload_retry,
                    forward_retry,
                )
                .await?,
            )
            .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn failure_batch(
        &self,
        wire: &MessageWireDTO,
        scope: &RiseMessageScope,
        scopes: &[RiseMessageScope],
        local_id: &str,
        request_string: &str,
        code: &str,
        version: i64,
        upload_retry: Option<RiseMessageUploadRetry>,
        forward_retry: Option<RiseMessageForwardRetry>,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let wire = RiseMessageWireCodec::halting_media_uploads(wire).unwrap_or_else(|_| wire.clone());
        let position =
            RiseConversationStoreLayout::message_position(&wire, local_id, RiseMessageLocalPhase::Failed);
        let record = RiseMessageEntityRecord {
            wire,
            source: RiseConversationEntitySource::Optimistic,
            local_phase: RiseMessageLocalPhase::Failed,
            client_request_id: Some(request_string.to_owned()),
            last_error_code: Some(code.to_owned()),
            scope_storage_key: Some(scope.storage_key()),
            is_replayable: None,
            upload_retry,
            forward_retry,
        };
        let row = RiseMessageRepositoryEncodedRow {
            local_id: local_id.to_owned(),
            data: RiseMessageWireCodec::record_data(&record)?,
            position,
            aliases: vec![
                RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), local_id),
                RiseEntityAlias::new(RiseEntityAliasKind::request_id(), request_string),
                RiseEntityAlias::new(RiseEntityAliasKind::retained(), local_id),
            ],
            duplicate_local_ids: Vec::new(),
            entity_version: 0,
            is_unchanged: false,
        };
        let preview = self
            .preview_hook
            .call(
                RiseMessagePreviewChange::SendFailed {
                    chat_id: scope.chat_id.clone(),
                    local_id: local_id.to_owned(),
                },
                version,
            )
            .await
            .unwrap_or_else(|_| RiseStoreWriteBatch::empty());
        Ok(self.combining(self.entity_upsert_batch(&row, scopes, version, None), preview))
    }

    pub async fn write_durable_send_outcome_preview(&self, request_id: RiseRequestID) {
        let Ok(Some(telemetry)) = self.durable_telemetry_record(request_id).await else {
            return;
        };
        let change = match telemetry.local_phase {
            RiseMessageLocalPhase::Failed => RiseMessagePreviewChange::SendFailed {
                chat_id: telemetry.scope.chat_id.clone(),
                local_id: telemetry.local_id.clone(),
            },
            RiseMessageLocalPhase::Remote => {
                self.apply_held_link_preview(&telemetry.local_id).await;
                let Ok(Some(entity)) = self.message_entity(&telemetry.local_id).await else {
                    return;
                };
                if RiseMessageWireCodec::is_tombstone(&entity.data) {
                    return;
                }
                let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
                    return;
                };
                RiseMessagePreviewChange::Created {
                    scope: telemetry.scope.clone(),
                    message: record.wire,
                    was_already_stored: true,
                }
            }
            RiseMessageLocalPhase::Queued | RiseMessageLocalPhase::Uploading | RiseMessageLocalPhase::Sending => {
                return;
            }
        };
        let Ok(preview) = self.preview_hook.call(change, self.next_version()).await else {
            return;
        };
        if preview.mutations.is_empty() && preview.invalidated_views.is_empty() {
            return;
        }
        let _ = self.engine.write(preview).await;
    }

    pub async fn edit(
        &self,
        local_id: &str,
        scope: &RiseMessageScope,
        content: &str,
        entities: Option<&str>,
        document: Option<&str>,
        link_preview_type: Option<&str>,
    ) -> anyhow::Result<()> {
        let json_string = |value: Option<&str>| {
            value.map_or(RiseJSONValue::Null, |value| RiseJSONValue::String(value.to_owned()))
        };
        let mut patch = BTreeMap::from([
            (rise_key::events::CONTENT.to_owned(), RiseJSONValue::String(content.to_owned())),
            (rise_key::events::ENTITIES.to_owned(), json_string(entities)),
            (rise_key::events::DOCUMENT.to_owned(), json_string(document)),
        ]);
        if let Some(link_preview_type) = link_preview_type {
            patch.insert(
                rise_key::bot::LINK_PREVIEW_TYPE.to_owned(),
                RiseJSONValue::String(link_preview_type.to_owned()),
            );
        }
        let checkpoint = self
            .begin_patch(local_id, Some(&scope.chat_id), move |wire: &MessageWireDTO| {
                RiseMessageWireCodec::editing(wire, &patch, true)
            })
            .await?;
        let guard = {
            let checkpoint = checkpoint.clone();
            let chat_id = scope.chat_id.clone();
            self.rollback_guard(move |repository| {
                async move {
                    let _ = repository.rollback(&checkpoint, Some(&chat_id)).await;
                }
                .boxed()
            })
        };
        let outcome: anyhow::Result<()> = async {
            self.remote
                .edit(rise_request::message::EditMessage {
                    caption: None,
                    content: Some(content.to_owned()),
                    document: document.map(str::to_owned),
                    entities: entities.map(str::to_owned),
                    link_preview_type: link_preview_type.map(str::to_owned),
                    media_items: None,
                    message_id: checkpoint.server_id.clone(),
                    reply_markup: None,
                    web_app_data: None,
                })
                .await?;
            self.acknowledge(&checkpoint).await?;
            self.invalidate_auxiliary_scope(
                &RiseMessageScope::new(
                    scope.chat_id.clone(),
                    self.normalized(scope.topic_id.as_deref()).or_else(|| checkpoint.topic_id.clone()),
                    None,
                ),
                &RiseMessageAuxiliaryProjection::all(),
            )
            .await;
            Ok(())
        }
        .await;
        guard.disarm();
        if let Err(error) = outcome {
            let _ = self.rollback(&checkpoint, Some(&scope.chat_id)).await;
            return Err(error);
        }
        Ok(())
    }

    pub const DELETE_RECONNECT_WAIT_MS: u64 = 10_000;

    pub async fn await_transport_reconnect(timeout_ms: u64) -> bool {
        if on_main(RiseTransportStatus::is_connected).await == Some(true) {
            return true;
        }
        let (connected, on_connected) = oneshot::channel::<()>();
        let wait = on_main(move |cx: &mut App| {
            if RiseTransportStatus::is_connected(cx) {
                return Err(true);
            }
            let watch = RiseTransportStatus::on_next_connect(
                move |_: &mut App| {
                    let _ = connected.send(());
                },
                cx,
            );
            let (stop, on_stop) = oneshot::channel::<()>();
            cx.spawn(async move |_| {
                let _ = on_stop.await;
                watch.stop();
            })
            .detach();
            Ok((stop, RiseTransportStatus::begin_reconnect_demand(cx)))
        })
        .await;
        let wait = match wait {
            Some(Err(already)) => return already,
            Some(Ok(wait)) => Some(wait),
            None => None,
        };
        let timer = std::pin::pin!(sleep_on_runtime(Duration::from_millis(timeout_ms)));
        let did_connect = match futures::future::select(on_connected, timer).await {
            futures::future::Either::Left((fired, _)) => fired.is_ok(),
            futures::future::Either::Right(_) => false,
        };
        if let Some((stop, demand)) = wait {
            on_main(move |cx: &mut App| {
                let _ = stop.send(());
                RiseTransportStatus::end_reconnect_demand(demand, cx);
            })
            .await;
        }
        if did_connect {
            return true;
        }
        on_main(RiseTransportStatus::is_connected).await == Some(true)
    }

    /// validate is the live account session guard of the scoped action
    pub async fn delete<Validate, Validated>(
        &self,
        local_ids: &[String],
        scope: &RiseMessageScope,
        for_everyone: Option<bool>,
        validate_session: Option<Validate>,
    ) -> anyhow::Result<()>
    where
        Validate: Fn() -> Validated + Send + Sync,
        Validated: Future<Output = anyhow::Result<()>> + Send,
    {
        if let Some(validate_session) = &validate_session {
            validate_session().await?;
        }
        let unique: Vec<String> = local_ids.iter().cloned().collect::<HashSet<_>>().into_iter().collect();
        if unique.is_empty() {
            return Ok(());
        }
        let mut checkpoints: Vec<RiseMessageDeleteCheckpoint> = Vec::new();
        let mut server_ids: Vec<String> = Vec::new();
        let mut preview_ids: Vec<String> = Vec::new();
        let mut affected_topic_ids: HashSet<String> = HashSet::new();
        let mut unloaded_local_ids: HashSet<String> = HashSet::new();
        for identifier in &unique {
            let entity = if let Some(cached) = self.message_entity(identifier).await? {
                cached
            } else {
                let Some(validate_session) = validate_session.as_ref().filter(|_| {
                    !GetSaiServerId::is_temp_id(Some(identifier.as_str()))
                }) else {
                    return Err(RiseMessageRepositoryError::MissingServerID(identifier.clone()).into());
                };
                let response = self.remote.message_by_id(identifier.clone()).await?;
                validate_session().await?;
                let root = response.object_value();
                let remote_error = root
                    .and_then(|root| root.get(rise_key::events::ERROR))
                    .and_then(RiseJSONValue::string_value);
                if let Some(error) = self.normalized(remote_error) {
                    return Err(RiseMessageRepositoryError::RemoteFailure(error).into());
                }
                let refused =
                    matches!(root.and_then(|root| root.get(rise_key::message::SUCCESS)), Some(RiseJSONValue::Bool(false)));
                let message = root
                    .and_then(|root| root.get(rise_key::events::MESSAGE))
                    .and_then(RiseJSONValue::object_value);
                let (false, Some(message)) = (refused, message) else {
                    return Err(RiseMessageRepositoryError::InvalidWirePayload.into());
                };
                let wire = RiseMessageWireCodec::wire(message)?;
                Self::validate_deletion_scope(&wire, scope)?;
                let Some(server_id) = RiseConversationStoreLayout::canonical_message_server_id(&wire)
                    .filter(|server_id| server_id == identifier)
                else {
                    return Err(RiseMessageRepositoryError::InvalidWirePayload.into());
                };
                if let Some(cached) = self.message_entity(identifier).await? {
                    cached
                } else {
                    let record = RiseMessageEntityRecord::new(wire, RiseConversationEntitySource::ServerPage);
                    let entity = RiseStoreEntity::new(
                        server_id,
                        RiseMessageWireCodec::record_data(&record)?,
                        self.state.with(|state| state.version_clock),
                        Self::now_milliseconds(),
                    );
                    unloaded_local_ids.insert(entity.local_id.clone());
                    entity
                }
            };
            if RiseMessageWireCodec::is_tombstone(&entity.data) {
                continue;
            }
            let record = RiseMessageWireCodec::record(&entity.data)?;
            Self::validate_deletion_scope(&record.wire, scope)?;
            if let Some(topic_id) = self.normalized(record.wire.topic_id.as_deref()) {
                affected_topic_ids.insert(topic_id);
            }
            server_ids.push(self.server_id(&entity)?);
            preview_ids.extend(Self::preview_identities(&entity));
            let mut positions: Vec<(String, RisePositionKey)> = Vec::new();
            for candidate in self.deletion_scopes(scope) {
                let view = RiseConversationStoreLayout::message_view(&candidate);
                if let Some(position) = self
                    .engine
                    .entity_view_position(
                        RiseConversationStoreLayout::messages(),
                        view.clone(),
                        entity.local_id.clone(),
                    )
                    .await?
                {
                    positions.push((view, position.position_key));
                }
            }
            checkpoints.push((entity, positions));
        }
        if server_ids.is_empty() {
            return Ok(());
        }
        if let Some(validate_session) = &validate_session {
            validate_session().await?;
        }
        let optimistic_version = self.next_version();
        let acknowledgement_version = self.next_version_floor(optimistic_version);
        let entities: Vec<RiseStoreEntity> = checkpoints.iter().map(|(entity, _)| entity.clone()).collect();
        let memberships: Vec<(String, String)> = checkpoints
            .iter()
            .flat_map(|(entity, positions)| {
                positions.iter().map(|(view, _)| (entity.local_id.clone(), view.clone()))
            })
            .collect();
        self.write_delete(&entities, optimistic_version, &memberships).await?;
        let guard = {
            let checkpoints = checkpoints.clone();
            let unloaded_local_ids = unloaded_local_ids.clone();
            self.rollback_guard(move |repository| {
                async move {
                    repository.rollback_delete(&checkpoints, &unloaded_local_ids, acknowledgement_version).await;
                }
                .boxed()
            })
        };
        let outcome: anyhow::Result<()> = async {
            if let Some(validate_session) = &validate_session {
                validate_session().await?;
            }
            let body = rise_request::message::DeleteMessages { for_everyone, message_ids: Some(server_ids.clone()) };
            let response = match self.remote.delete(body.clone()).await {
                Ok(response) => response,
                Err(error) => {
                    if error.is::<CancellationError>()
                        || !CanonErrors::is_connection_loss(&error)
                        || !Self::await_transport_reconnect(Self::DELETE_RECONNECT_WAIT_MS).await
                    {
                        return Err(error);
                    }
                    if let Some(validate_session) = &validate_session {
                        validate_session().await?;
                    }
                    self.remote.delete(body).await?
                }
            };
            if let Some(validate_session) = &validate_session {
                validate_session().await?;
            }
            self.validate(Some(response.success), Some(&response.error))?;
            self.write_delete(&entities, acknowledgement_version, &memberships).await?;
            let excluded: HashSet<String> = entities
                .iter()
                .map(|entity| entity.local_id.clone())
                .chain(server_ids.iter().cloned())
                .chain(preview_ids.iter().cloned())
                .collect();
            let fallback = self.newest_preview_fallback(&scope.chat_id, &excluded).await;
            if let Ok(preview) = self
                .preview_hook
                .call(
                    RiseMessagePreviewChange::Deleted {
                        chat_id: scope.chat_id.clone(),
                        server_ids: preview_ids.clone(),
                        fallback,
                    },
                    acknowledgement_version,
                )
                .await
            {
                let _ = self.engine.write(preview).await;
            }
            let removed: HashSet<String> =
                unique.iter().chain(&server_ids).chain(&preview_ids).cloned().collect();
            if for_everyone == Some(true) {
                let mut quoted = removed.clone();
                for (entity, _) in &checkpoints {
                    let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
                        continue;
                    };
                    quoted.extend(
                        RiseConversationStoreLayout::message_aliases(&record.wire)
                            .into_iter()
                            .map(|alias| alias.value),
                    );
                }
                let excluding: HashSet<String> =
                    checkpoints.iter().map(|(entity, _)| entity.local_id.clone()).collect();
                let quotes = self.quoting_row_mutations(&scope.chat_id, quoted, None, &excluding).await;
                if !quotes.is_empty() {
                    let _ = self.engine.write(write_batch(quotes)).await;
                }
            }
            let mut invalidation = RiseMessageAuxiliaryInvalidation::with_topic_ids(
                &scope.chat_id,
                affected_topic_ids,
                RiseMessageAuxiliaryProjection::all(),
            );
            invalidation.removed_message_ids = removed;
            self.invalidate_auxiliary(invalidation).await;
            Ok(())
        }
        .await;
        guard.disarm();
        if let Err(error) = outcome {
            self.rollback_delete(&checkpoints, &unloaded_local_ids, acknowledgement_version).await;
            return Err(error);
        }
        Ok(())
    }

    async fn rollback_delete(
        &self,
        checkpoints: &[RiseMessageDeleteCheckpoint],
        unloaded_local_ids: &HashSet<String>,
        acknowledgement_version: i64,
    ) {
        let namespace = RiseConversationStoreLayout::messages();
        let mut mutations: Vec<RiseStoreMutation> = Vec::new();
        for (entity, positions) in checkpoints {
            if unloaded_local_ids.contains(&entity.local_id) {
                if let Ok(Some(current)) = self.message_entity(&entity.local_id).await
                    && RiseMessageWireCodec::is_tombstone(&current.data)
                    && current.version <= acknowledgement_version
                {
                    mutations.push(RiseStoreMutation::RemoveEntity {
                        namespace: namespace.clone(),
                        local_id: entity.local_id.clone(),
                    });
                }
                continue;
            }
            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: namespace.clone(),
                entity: RiseStoreEntity::new(
                    entity.local_id.clone(),
                    entity.data.clone(),
                    acknowledgement_version,
                    Self::now_milliseconds(),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            });
            for (view, position) in positions {
                mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                    namespace: namespace.clone(),
                    view: view.clone(),
                    membership: RiseEntityViewMembership::new(
                        entity.local_id.clone(),
                        position.clone(),
                        acknowledgement_version,
                    ),
                });
            }
            mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                namespace: namespace.clone(),
                view: Self::TOMBSTONE_VIEW.to_owned(),
                local_id: entity.local_id.clone(),
                if_version_at_most: Some(acknowledgement_version),
            });
        }
        let _ = self.engine.write(write_batch(mutations)).await;
    }

    fn deletion_scopes(&self, scope: &RiseMessageScope) -> HashSet<RiseMessageScope> {
        let mut scopes: HashSet<RiseMessageScope> = self.state.with(|state| {
            state.known_scopes.iter().filter(|candidate| candidate.chat_id == scope.chat_id).cloned().collect()
        });
        scopes.insert(scope.clone());
        scopes.insert(RiseMessageScope::new(scope.chat_id.clone(), None, None));
        scopes
    }

    pub fn validate_deletion_scope(wire: &MessageWireDTO, scope: &RiseMessageScope) -> anyhow::Result<()> {
        let same_chat = wire.chat_id.as_deref() == Some(scope.chat_id.as_str());
        let same_topic = scope.topic_id.is_none()
            || Self::deletion_topic_key(wire.topic_id.as_deref()) == Self::deletion_topic_key(scope.topic_id.as_deref());
        if !same_chat || !same_topic {
            return Err(RiseMessageRepositoryError::InvalidWirePayload.into());
        }
        Ok(())
    }

    /// general topic has three spellings
    /// no topic or empty string or the nil uuid the server stores
    /// other topics compare as uuids so case doesnt matter
    fn deletion_topic_key(topic_id: Option<&str>) -> String {
        let value = trimming_whitespaces_and_newlines(topic_id.unwrap_or_default());
        let Some(uuid) = Uuid::from_uuid_string(value) else {
            return value.to_lowercase();
        };
        let uuid_string = uuid.uuid_string();
        if uuid_string == NIL_UUID { String::new() } else { uuid_string }
    }

    /// reconciles server created messages through the same reducer as realtime pushes
    /// so forward, share and inline results update timeline and preview with no legacy cache rewrite
    pub async fn ingest_authoritative(
        &self,
        messages: impl AsRef<[RiseAuthoritativeMessage]> + Send + Sync,
    ) -> anyhow::Result<()> {
        if messages.as_ref().is_empty() {
            return Ok(());
        }
        let mut batch = RiseStoreWriteBatch::empty();
        for item in messages.as_ref() {
            self.state.with(|state| {
                state.known_scopes.insert(item.scope.clone());
            });
            let mut object = RiseMessageWireCodec::object(&item.wire)?;
            object.insert(rise_key::events::CHAT_ID.to_owned(), RiseJSONValue::String(item.scope.chat_id.clone()));
            if let Some(topic_id) = &item.scope.topic_id {
                object.insert(rise_key::events::TOPIC_ID.to_owned(), RiseJSONValue::String(topic_id.clone()));
            } else {
                object.remove(rise_key::message::TOPIC_ID);
            }
            if let Some(local_id) = &item.local_id {
                object.insert(rise_key::events::TEMP_ID.to_owned(), RiseJSONValue::String(local_id.clone()));
            }
            let mut root: BTreeMap<String, RiseJSONValue> = BTreeMap::from([
                (rise_key::events::MESSAGE.to_owned(), RiseJSONValue::Object(object)),
                (rise_key::events::CHAT_ID.to_owned(), RiseJSONValue::String(item.scope.chat_id.clone())),
            ]);
            if let Some(local_id) = &item.local_id {
                root.insert(rise_key::events::TEMP_ID.to_owned(), RiseJSONValue::String(local_id.clone()));
            }
            let push = RiseWirePushEnvelope {
                r#type: RiseMessageRepositoryRealtimeEvent::Created.raw_value().to_owned(),
                data: RiseJSONValue::Object(root.clone()),
                timestamp: None,
                extras: BTreeMap::new(),
            };
            if let Some(next) = self.reduce_created(&push, &root, self.next_version(), false).await? {
                self.accumulate(next, &mut batch).await?;
            }
        }
        if batch.mutations.is_empty() && batch.invalidated_views.is_empty() {
            return Ok(());
        }
        self.engine.write(batch).await?;
        Ok(())
    }

    /// forward of 50 messages to 4 chats is 200 rows of ten mutations, past the store per write bound
    /// so it commits what is gathered before next would cross the bound
    pub async fn accumulate(&self, next: RiseStoreWriteBatch, batch: &mut RiseStoreWriteBatch) -> anyhow::Result<()> {
        let gathered = batch.mutations.len().saturating_add(next.mutations.len());
        if !batch.mutations.is_empty()
            && i64::try_from(gathered).unwrap_or(i64::MAX) > RiseStoreLimits::PRODUCTION.maximum_batch_mutations
        {
            self.engine.write(std::mem::take(batch)).await?;
        }
        *batch = self.combining(std::mem::take(batch), next);
        Ok(())
    }

    pub async fn submit_external_message(
        &self,
        endpoint: RiseConversationEndpoint,
        body: &RiseJSONValue,
        optimistic_wire: &MessageWireDTO,
        local_id: &str,
        echo_temp_id: Option<&str>,
        scope: &RiseMessageScope,
    ) -> anyhow::Result<RiseJSONValue> {
        let request_id = RiseRequestIDAllocator::shared().next(Date::now());
        let request_string = request_id.raw_value().to_string();
        let installed = RiseMessageWireCodec::installing_local_identity(
            optimistic_wire,
            scope,
            local_id,
            Date::now().time_interval_since_1970(),
        )?;
        let scopes =
            self.scopes_for_message(&scope.chat_id, scope.topic_id.as_deref(), scope.thread_root_id.as_deref());
        let mut optimistic_batch = self
            .create_optimistic_batch(
                &installed,
                scope,
                &scopes,
                local_id,
                &request_string,
                RiseMessageLocalPhase::Sending,
                self.next_version(),
                None,
                None,
            )
            .await?;
        if let Some(echo_temp_id) = echo_temp_id.map(str::trim).filter(|value| !value.is_empty() && *value != local_id) {
            optimistic_batch.mutations.push(RiseStoreMutation::SetEntityAlias {
                namespace: RiseConversationStoreLayout::messages(),
                alias: RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), echo_temp_id),
                local_id: local_id.to_owned(),
            });
        }
        self.engine.write(optimistic_batch).await?;
        let guard = {
            let installed = installed.clone();
            let scope = scope.clone();
            let scopes = scopes.clone();
            let local_id = local_id.to_owned();
            let request_string = request_string.clone();
            self.rollback_guard(move |repository| {
                async move {
                    let code = Self::error_code(&CancellationError.into());
                    let _ = repository
                        .write_create_failure(
                            &installed,
                            &scope,
                            &scopes,
                            &local_id,
                            &request_string,
                            &code,
                            repository.next_version(),
                            None,
                            None,
                        )
                        .await;
                }
                .boxed()
            })
        };
        let outcome: anyhow::Result<RiseJSONValue> = async {
            let response = RiseConversationRPC::new(self.engine.clone())
                .call_body(endpoint, body, Some(request_id))
                .await?;
            if let Some(root) = response.object_value()
                && RiseMessageWireCodec::bool(root, &[rise_key::message::SUCCESS]) == Some(false)
            {
                return Err(RiseMessageRepositoryError::RemoteFailure(
                    RiseMessageWireCodec::string(root, &[wire_key::ERROR, wire_key::CODE])
                        .unwrap_or_else(|| REMOTE_REJECTED_CODE.to_owned()),
                )
                .into());
            }
            let candidate = response.subscript_key(rise_key::events::MESSAGE).or_else(|| {
                response.subscript_key(rise_key::events::DATA).and_then(|data| data.subscript_key(rise_key::events::MESSAGE))
            });
            let Some(object) = candidate.and_then(RiseJSONValue::object_value) else {
                return Err(RiseMessageRepositoryError::InvalidWirePayload.into());
            };
            self.ingest_authoritative(vec![RiseAuthoritativeMessage {
                wire: RiseMessageWireCodec::wire(object)?,
                scope: scope.clone(),
                local_id: Some(local_id.to_owned()),
            }])
            .await?;
            Ok(response)
        }
        .await;
        guard.disarm();
        if let Err(error) = &outcome {
            let _ = self
                .write_create_failure(
                    &installed,
                    scope,
                    &scopes,
                    local_id,
                    &request_string,
                    &Self::error_code(error),
                    self.next_version(),
                    None,
                    None,
                )
                .await;
        }
        outcome
    }

    pub async fn forward(&self, body: ForwardMessagesBody) -> anyhow::Result<ForwardMessagesGatewayDTO> {
        let rows = self.install_forward_optimistic_rows(&body).await;
        self.settle_forward(body, rows, true).await
    }

    async fn retry_forward(&self, retry: &RiseMessageForwardRetry, scope: &RiseMessageScope) -> anyhow::Result<()> {
        let Some((resend_body, resend_rows)) = self.reinstall_failed_forward_rows(retry, scope).await else {
            return Ok(());
        };
        self.settle_forward(resend_body, resend_rows, true).await?;
        Ok(())
    }

    async fn settle_forward(
        &self,
        body: ForwardMessagesBody,
        rows: Vec<OptimisticFanoutRow>,
        replays_when_connected: bool,
    ) -> anyhow::Result<ForwardMessagesGatewayDTO> {
        let guard = {
            let rows = rows.clone();
            self.rollback_guard(move |repository| {
                async move {
                    let code = Self::error_code(&CancellationError.into());
                    repository.fail_optimistic_fanout_rows(&rows, &code).await;
                }
                .boxed()
            })
        };
        let outcome: anyhow::Result<ForwardMessagesGatewayDTO> = async {
            let response = self.remote_forward_settling_timeout(&body).await?;
            self.validate(response.success, response.error.as_deref())?;
            self.apply_forward_results(&response, &rows).await;
            Ok(response)
        }
        .await;
        guard.disarm();
        match outcome {
            Ok(response) => Ok(response),
            Err(error) => {
                self.fail_optimistic_fanout_rows(&rows, &Self::error_code(&error)).await;
                if replays_when_connected && Self::is_forward_transport_failure(&error) {
                    self.schedule_forward_replay(body);
                }
                Err(error)
            }
        }
    }

    async fn remote_forward_settling_timeout(
        &self,
        body: &ForwardMessagesBody,
    ) -> anyhow::Result<ForwardMessagesGatewayDTO> {
        match self.remote.forward(body.clone()).await {
            Err(error) if matches!(error.downcast_ref::<RiseWireError>(), Some(RiseWireError::Timeout(_))) => {
                if !matches!(self.engine.is_ready_for_calls(false).await, Ok(true)) {
                    return Err(error);
                }
                self.remote.forward(body.clone()).await
            }
            outcome => outcome,
        }
    }

    fn forward_replay_window() -> Duration {
        let ladder = RiseTimingCanon::FRESHNESS_LADDER_MS;
        let milliseconds = ladder.get(FORWARD_REPLAY_RUNG).or(ladder.last()).copied().unwrap_or(0);
        Duration::from_millis(u64::try_from(milliseconds).unwrap_or(0))
    }

    fn is_forward_transport_failure(error: &anyhow::Error) -> bool {
        matches!(
            error.downcast_ref::<RiseWireError>(),
            Some(
                RiseWireError::Timeout(_)
                    | RiseWireError::NotConnected
                    | RiseWireError::Disconnected(_)
                    | RiseWireError::NoSocketSession
                    | RiseWireError::Suspended
                    | RiseWireError::StaleSession
            )
        )
    }

    fn schedule_forward_replay(&self, body: ForwardMessagesBody) {
        let this = self.this.clone();
        let _ = RiseAccountRuntime::current().spawn(async move {
            if let Some(this) = this.upgrade() {
                this.replay_forward_when_connected(body).await;
            }
        });
    }

    async fn replay_forward_when_connected(&self, body: ForwardMessagesBody) {
        let Some(idempotency_key) = self.normalized(body.idempotency_key.as_deref()) else {
            return;
        };
        let Some(message_ids) = body.message_ids.clone().filter(|message_ids| !message_ids.is_empty()) else {
            return;
        };
        let Some(destinations) = body.destinations.clone().filter(|destinations| !destinations.is_empty()) else {
            return;
        };
        let deadline = RiseClock::monotonic() + Self::forward_replay_window();
        loop {
            let Ok(ready) = self.engine.is_ready_for_calls(false).await else {
                return;
            };
            if ready {
                break;
            }
            if RiseClock::monotonic() >= deadline || sleep_on_runtime(FORWARD_REPLAY_POLL).await.is_err() {
                return;
            }
        }
        let retry = RiseMessageForwardRetry {
            idempotency_key: idempotency_key.clone(),
            message_ids: message_ids.clone(),
            caption: body.caption.clone(),
        };
        let mut rows: Vec<OptimisticFanoutRow> = Vec::new();
        let mut replay_destinations = Vec::new();
        for destination in destinations {
            let Some(chat_id) = self.normalized(destination.chat_id.as_deref()) else {
                continue;
            };
            let scope = RiseMessageScope::new(chat_id, self.normalized(destination.topic_id.as_deref()), None);
            if let Some((resend_body, resend_rows)) = self.reinstall_failed_forward_rows(&retry, &scope).await {
                rows.extend(resend_rows);
                replay_destinations.extend(resend_body.destinations.unwrap_or_default());
            }
        }
        if rows.is_empty() || replay_destinations.is_empty() {
            return;
        }
        let replay = rise_request::message::ForwardMessages {
            caption: body.caption.clone(),
            destinations: Some(replay_destinations),
            idempotency_key: Some(idempotency_key),
            message_ids: Some(message_ids),
            ..Default::default()
        };
        let _ = self.settle_forward(replay, rows, false).await;
    }

    pub async fn unpin_all(&self, chat_id: &str) -> anyhow::Result<UnpinAllMessagesGatewayDTO> {
        let loaded: Vec<RiseStoreEntity> = self
            .loaded_entities(chat_id)
            .await?
            .into_iter()
            .filter(|entity| {
                !RiseMessageWireCodec::is_tombstone(&entity.data)
                    && RiseMessageWireCodec::record(&entity.data)
                        .is_ok_and(|record| record.wire.is_pinned == Some(true))
            })
            .collect();
        let mut affected_topic_ids: HashSet<String> = self.state.with(|state| {
            state
                .known_scopes
                .iter()
                .filter(|scope| scope.chat_id == chat_id)
                .filter_map(|scope| self.normalized(scope.topic_id.as_deref()))
                .collect()
        });
        affected_topic_ids.extend(loaded.iter().filter_map(|entity| {
            let record = RiseMessageWireCodec::record(&entity.data).ok()?;
            self.normalized(record.wire.topic_id.as_deref())
        }));
        let checkpoint = self
            .begin_bulk_patch(loaded, |wire: &MessageWireDTO| {
                RiseMessageWireCodec::setting_boolean(wire, rise_key::message::IS_PINNED, false)
            })
            .await?;
        let guard = {
            let checkpoint = checkpoint.clone();
            self.rollback_guard(move |repository| {
                async move {
                    let _ = repository.rollback_bulk(&checkpoint).await;
                }
                .boxed()
            })
        };
        let outcome: anyhow::Result<UnpinAllMessagesGatewayDTO> = async {
            let response = self
                .remote
                .unpin_all(rise_request::message::UnpinAllMessages { chat_id: chat_id.to_owned() })
                .await?;
            self.validate(Some(response.success), Some(&response.error))?;
            self.acknowledge_bulk(&checkpoint).await?;

            let mut authoritative = RiseStoreWriteBatch::empty();
            for identifier in &response.unpinned_message_ids {
                let root = BTreeMap::from([
                    (rise_key::events::MESSAGE_ID.to_owned(), RiseJSONValue::String(identifier.clone())),
                    (rise_key::events::CHAT_ID.to_owned(), RiseJSONValue::String(chat_id.to_owned())),
                ]);
                if let Some(next) = self
                    .reduce_patched_message(
                        &root,
                        self.next_version(),
                        HashSet::new(),
                        None,
                        false,
                        false,
                        false,
                        |wire: &MessageWireDTO| {
                            RiseMessageWireCodec::setting_boolean(wire, rise_key::message::IS_PINNED, false)
                        },
                    )
                    .await?
                {
                    authoritative = self.combining(authoritative, next);
                }
            }
            if !authoritative.mutations.is_empty() || !authoritative.invalidated_views.is_empty() {
                self.engine.write(authoritative).await?;
            }
            self.invalidate_auxiliary(
                RiseMessageAuxiliaryInvalidation::with_topic_ids(
                    chat_id,
                    affected_topic_ids,
                    RiseMessageAuxiliaryProjection::pin_state(),
                )
                .with_pinned_message_patch(RiseMessageAuxiliaryInvalidationPinnedMessagePatch::clear_all_pinned(true)),
            )
            .await;
            Ok(response)
        }
        .await;
        guard.disarm();
        if outcome.is_err() {
            let _ = self.rollback_bulk(&checkpoint).await;
        }
        outcome
    }

    pub async fn delete_all_from_user(
        &self,
        chat_id: &str,
        target_user_id: &str,
    ) -> anyhow::Result<DeleteAllMessagesFromUserGatewayDTO> {
        let scope = RiseMessageScope::new(chat_id.to_owned(), None, None);
        let loaded: Vec<RiseStoreEntity> = self
            .loaded_entities(chat_id)
            .await?
            .into_iter()
            .filter(|entity| {
                if RiseMessageWireCodec::is_tombstone(&entity.data) {
                    return false;
                }
                let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
                    return false;
                };
                let sender_id = record
                    .wire
                    .sender_id
                    .as_deref()
                    .or_else(|| record.wire.sender.as_ref().and_then(|sender| sender.id.as_deref()));
                self.normalized(sender_id).as_deref() == Some(target_user_id)
            })
            .collect();
        let checkpoints = self.deletion_checkpoints(&loaded, &scope).await?;
        let optimistic_version = self.next_version();
        let acknowledgement_version = self.next_version_floor(optimistic_version);
        let mut affected_topic_ids: HashSet<String> = self.state.with(|state| {
            state
                .known_scopes
                .iter()
                .filter(|candidate| candidate.chat_id == chat_id)
                .filter_map(|candidate| self.normalized(candidate.topic_id.as_deref()))
                .collect()
        });
        affected_topic_ids.extend(checkpoints.iter().filter_map(|item| {
            let record = RiseMessageWireCodec::record(&item.entity.data).ok()?;
            self.normalized(record.wire.topic_id.as_deref())
        }));
        let guard = {
            let checkpoints = checkpoints.clone();
            self.rollback_guard(move |repository| {
                async move {
                    let _ = repository
                        .restore_deletion_checkpoints_in_slices(&checkpoints, acknowledgement_version)
                        .await;
                }
                .boxed()
            })
        };
        let outcome: anyhow::Result<DeleteAllMessagesFromUserGatewayDTO> = async {
            self.write_delete_in_slices(&checkpoints, optimistic_version).await?;
            let response = self
                .remote
                .delete_all_from_user(rise_request::message::DeleteAllMessagesFromUser {
                    chat_id: chat_id.to_owned(),
                    target_user_id: target_user_id.to_owned(),
                })
                .await?;
            self.validate(Some(response.success), Some(&response.error))?;
            let deleted: Vec<String> = if response.deleted_message_ids.is_empty() {
                checkpoints.iter().filter_map(|item| self.server_id(&item.entity).ok()).collect()
            } else {
                response.deleted_message_ids.clone()
            };
            self.write_delete_in_slices(&checkpoints, acknowledgement_version).await?;
            let reduced_version = self.next_version_floor(acknowledgement_version);
            for slice in deleted.chunks(Self::DELETE_SLICE_SIZE) {
                let root = BTreeMap::from([
                    (rise_key::events::CHAT_ID.to_owned(), RiseJSONValue::String(chat_id.to_owned())),
                    (
                        rise_key::events::MESSAGE_IDS.to_owned(),
                        RiseJSONValue::Array(slice.iter().cloned().map(RiseJSONValue::String).collect()),
                    ),
                ]);
                if let Some(authoritative) = self.reduce_deleted(&root, reduced_version, HashSet::new()).await? {
                    self.engine.write(authoritative).await?;
                }
            }
            self.invalidate_auxiliary(RiseMessageAuxiliaryInvalidation::with_topic_ids(
                chat_id,
                affected_topic_ids,
                RiseMessageAuxiliaryProjection::all(),
            ))
            .await;
            Ok(response)
        }
        .await;
        guard.disarm();
        if outcome.is_err() {
            let _ = self.restore_deletion_checkpoints_in_slices(&checkpoints, acknowledgement_version).await;
        }
        outcome
    }

    /// cancels a local create without letting the outbox send an invisible message
    /// unclaimed work is removed in the same transaction as the bubble
    /// if a worker owns it the bubble stays hidden and a server delete runs after ack
    pub async fn cancel_outgoing(&self, local_id: &str, scope: &RiseMessageScope) -> anyhow::Result<()> {
        let _owner_release = LiveUploadOwnerRelease { repository: self, owner_id: local_id.to_owned() };
        self.upload_cancellation_hub.cancel(local_id, None).await;
        let Some(entity) = self.message_entity(local_id).await? else {
            return Ok(());
        };
        if RiseMessageWireCodec::is_tombstone(&entity.data) {
            return Ok(());
        }
        let record = RiseMessageWireCodec::record(&entity.data)?;
        if record.local_phase == RiseMessageLocalPhase::Remote {
            return self.delete_for_everyone(local_id, scope).await;
        }
        let request_id = record
            .client_request_id
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .and_then(RiseRequestID::new);

        if let Some(request_id) = request_id
            && let Some(operation) = self.engine.durable_operation(request_id).await?
        {
            if !matches!(operation.state, RiseDurableOperationState::InFlight) {
                match self.remove_outgoing_locally(&entity, scope, Some(request_id)).await {
                    Ok(()) => return Ok(()),
                    Err(error) if is_durable_state_precondition(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            return self.compensate_cancelled_send(local_id, scope, request_id).await;
        }

        match record.local_phase {
            RiseMessageLocalPhase::Uploading | RiseMessageLocalPhase::Failed => {
                self.remove_outgoing_locally(&entity, scope, None).await?;
            }
            RiseMessageLocalPhase::Queued | RiseMessageLocalPhase::Sending => {
                let resolved: anyhow::Result<()> = async {
                    self.wait_for_submission_resolution(local_id).await?;
                    self.delete_for_everyone(local_id, scope).await
                }
                .await;
                if resolved.is_err()
                    && let Ok(Some(current)) = self.message_entity(local_id).await
                    && !RiseMessageWireCodec::is_tombstone(&current.data)
                {
                    let _ = self.remove_outgoing_locally(&current, scope, None).await;
                }
            }
            RiseMessageLocalPhase::Remote => {}
        }
        Ok(())
    }

    async fn delete_for_everyone(&self, local_id: &str, scope: &RiseMessageScope) -> anyhow::Result<()> {
        self.delete(&[local_id.to_owned()], scope, Some(true), None::<RiseMessageNoSessionValidation>).await
    }

    pub async fn retry_outgoing(&self, local_id: &str, scope: &RiseMessageScope) -> anyhow::Result<()> {
        let Some(entity) = self.message_entity(local_id).await? else {
            return Ok(());
        };
        if RiseMessageWireCodec::is_tombstone(&entity.data) {
            return Ok(());
        }
        let record = RiseMessageWireCodec::record(&entity.data)?;
        if record.local_phase == RiseMessageLocalPhase::Failed
            && let Some(forward_retry) = &record.forward_retry
        {
            return self.retry_forward(forward_retry, scope).await;
        }
        if record.local_phase != RiseMessageLocalPhase::Failed {
            return Ok(());
        }
        let Some(request_string) = record.client_request_id.clone() else {
            return Ok(());
        };
        let Some(request_id) = request_string.parse::<u64>().ok().and_then(RiseRequestID::new) else {
            return Ok(());
        };
        let Some(operation) = self.engine.durable_operation(request_id).await? else {
            return Ok(());
        };
        if !matches!(operation.state, RiseDurableOperationState::TerminalFailure) {
            return Ok(());
        }
        let Ok(reserved) = operation.decoded_payload::<RiseCreateMessageDurableEndpoint>() else {
            return Ok(());
        };

        let version = self.next_version_floor(entity.version);
        let resent_at = Date::now().time_interval_since_1970();
        let resent_wire = RiseMessageWireCodec::restamped(&record.wire, resent_at).unwrap_or_else(|_| record.wire.clone());
        let payload = RiseCreateMessageDurablePayload {
            body: reserved.body.clone(),
            optimistic_wire: RiseMessageWireCodec::restamped(&reserved.optimistic_wire, resent_at)
                .unwrap_or_else(|_| reserved.optimistic_wire.clone()),
            scope: reserved.scope.clone(),
            materialized_scopes: reserved.materialized_scopes.clone(),
            local_id: reserved.local_id.clone(),
            client_request_id: reserved.client_request_id.clone(),
            version: self.next_version_floor(version),
            retention_limit: reserved.retention_limit,
        };
        let queued = RiseMessageEntityRecord {
            wire: resent_wire.clone(),
            source: RiseConversationEntitySource::Optimistic,
            local_phase: RiseMessageLocalPhase::Queued,
            client_request_id: Some(request_string.clone()),
            last_error_code: None,
            scope_storage_key: record.scope_storage_key.clone().or_else(|| Some(scope.storage_key())),
            is_replayable: None,
            upload_retry: None,
            forward_retry: None,
        };
        let row = RiseMessageRepositoryEncodedRow {
            local_id: entity.local_id.clone(),
            data: RiseMessageWireCodec::record_data(&queued)?,
            position: RiseConversationStoreLayout::message_position(
                &resent_wire,
                &entity.local_id,
                RiseMessageLocalPhase::Queued,
            ),
            aliases: vec![
                RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), entity.local_id.clone()),
                RiseEntityAlias::new(RiseEntityAliasKind::request_id(), request_string.clone()),
                RiseEntityAlias::new(RiseEntityAliasKind::retained(), entity.local_id.clone()),
            ],
            duplicate_local_ids: Vec::new(),
            entity_version: 0,
            is_unchanged: false,
        };
        let scopes =
            self.scopes_for_message(&scope.chat_id, scope.topic_id.as_deref(), scope.thread_root_id.as_deref());
        let preview = self
            .preview_hook
            .call(
                RiseMessagePreviewChange::Created {
                    scope: scope.clone(),
                    message: resent_wire,
                    was_already_stored: true,
                },
                self.next_version(),
            )
            .await
            .unwrap_or_else(|_| RiseStoreWriteBatch::empty());
        let requeued = self.combining(self.entity_upsert_batch(&row, &scopes, version, None), preview);
        let mut mutations = vec![RiseStoreMutation::RemoveUnclaimedDurableOperation { request_id }];
        mutations.extend(requeued.mutations);
        self.engine
            .submit_durable_receipt::<RiseCreateMessageDurableEndpoint>(
                payload,
                RiseSyncSubmitOptions {
                    ordering_key: operation.ordering_key.clone(),
                    request_id: Some(request_id),
                    optimistic: RiseStoreWriteBatch::new(mutations, requeued.invalidated_views),
                },
            )
            .await?;
        Ok(())
    }

    pub async fn cancel_upload_slot(&self, local_id: &str, slot_index: i64) -> bool {
        if local_id.is_empty() || slot_index < 0 {
            return false;
        }
        let owner_id = match self.message_entity(local_id).await {
            Ok(Some(entity)) => entity.local_id,
            _ => local_id.to_owned(),
        };
        if !self.upload_cancellation_hub.cancel(&owner_id, Some(slot_index)).await {
            return false;
        }
        self.upload_progress_hub.remove(RiseUploadKey::new(owner_id, slot_index)).await;
        true
    }

    pub async fn is_upload_cancelled(&self, local_id: &str, slot_index: impl Into<Option<i64>>) -> bool {
        if local_id.is_empty() {
            return true;
        }
        self.upload_cancellation_hub.is_cancelled(local_id, slot_index.into()).await
    }

    pub async fn upload_cancellation_signal(
        &self,
        local_id: &str,
        slot_index: impl Into<Option<i64>>,
    ) -> RiseUploadCancellationSignal {
        self.upload_cancellation_hub.signal(local_id, slot_index.into()).await
    }

    pub async fn compensate_cancelled_send(
        &self,
        local_id: &str,
        scope: &RiseMessageScope,
        request_id: RiseRequestID,
    ) -> anyhow::Result<()> {
        loop {
            if let Some(operation) = self.engine.durable_operation(request_id).await? {
                if !matches!(operation.state, RiseDurableOperationState::InFlight)
                    && let Some(current) = self.message_entity(local_id).await?
                    && !RiseMessageWireCodec::is_tombstone(&current.data)
                {
                    match self.remove_outgoing_locally(&current, scope, Some(request_id)).await {
                        Ok(()) => return Ok(()),
                        Err(error) if is_durable_state_precondition(&error) => continue,
                        Err(error) => return Err(error),
                    }
                }
                sleep_on_runtime(COMPENSATION_POLL).await?;
                continue;
            }

            let Some(current) = self.message_entity(local_id).await? else {
                return Ok(());
            };
            if RiseMessageWireCodec::is_tombstone(&current.data) {
                return Ok(());
            }
            let record = RiseMessageWireCodec::record(&current.data)?;
            if record.local_phase == RiseMessageLocalPhase::Remote {
                self.delete_for_everyone(local_id, scope).await?;
            } else {
                self.remove_outgoing_locally(&current, scope, None).await?;
            }
            return Ok(());
        }
    }

    pub async fn remove_outgoing_locally(
        &self,
        entity: &RiseStoreEntity,
        scope: &RiseMessageScope,
        durable_request_id: Option<RiseRequestID>,
    ) -> anyhow::Result<()> {
        let checkpoints = self.deletion_checkpoints(std::slice::from_ref(entity), scope).await?;
        let version = self.next_version_floor(entity.version);
        let memberships: Vec<(String, String)> = checkpoints
            .iter()
            .flat_map(|item| item.positions.iter().map(|position| (item.entity.local_id.clone(), position.view.clone())))
            .collect();
        let mut batch = self.make_delete_batch(std::slice::from_ref(entity), version, &memberships)?;
        if let Some(request_id) = durable_request_id {
            batch.mutations.push(RiseStoreMutation::RemoveUnclaimedDurableOperation { request_id });
        }
        let server_ids = Self::preview_identities(entity);
        let excluded: HashSet<String> =
            std::iter::once(entity.local_id.clone()).chain(server_ids.iter().cloned()).collect();
        let fallback = self.newest_preview_fallback(&scope.chat_id, &excluded).await;
        let preview = self
            .preview_hook
            .call(
                RiseMessagePreviewChange::Deleted { chat_id: scope.chat_id.clone(), server_ids, fallback },
                version,
            )
            .await?;
        self.engine.write(self.combining(batch, preview)).await?;
        self.upload_progress_hub.remove_owner_id(&entity.local_id).await;
        Ok(())
    }

    pub async fn set_pinned(
        &self,
        pinned: bool,
        local_id: &str,
        scope: &RiseMessageScope,
        for_everyone: Option<bool>,
        silent: Option<bool>,
    ) -> anyhow::Result<()> {
        let checkpoint = self
            .begin_patch(local_id, None, move |wire: &MessageWireDTO| {
                RiseMessageWireCodec::setting_boolean(wire, rise_key::message::IS_PINNED, pinned)
            })
            .await?;
        let guard = {
            let checkpoint = checkpoint.clone();
            self.rollback_guard(move |repository| {
                async move {
                    repository.restore_pinned_flag(&checkpoint).await;
                }
                .boxed()
            })
        };
        let outcome: anyhow::Result<()> = async {
            let body = rise_request::message::PinMessage {
                chat_id: scope.chat_id.clone(),
                for_everyone,
                message_id: checkpoint.server_id.clone(),
                silent,
            };
            if pinned {
                let response = self.remote.pin(body).await?;
                self.validate(Some(response.success), Some(&response.error))?;
            } else {
                let response = self.remote.unpin(body).await?;
                self.validate(Some(response.success), Some(&response.error))?;
            }
            self.acknowledge(&checkpoint).await?;
            let acknowledged_wire = RiseMessageWireCodec::record(&checkpoint.optimistic_data)?.wire;
            self.invalidate_auxiliary(
                RiseMessageAuxiliaryInvalidation::new(
                    &scope.chat_id,
                    self.normalized(scope.topic_id.as_deref()).or_else(|| checkpoint.topic_id.clone()).as_deref(),
                    RiseMessageAuxiliaryProjection::pin_state(),
                )
                .with_pinned_message_patch(RiseMessageAuxiliaryInvalidationPinnedMessagePatch::from_message(
                    acknowledged_wire,
                    pinned,
                )),
            )
            .await;
            Ok(())
        }
        .await;
        guard.disarm();
        if let Err(error) = outcome {
            self.restore_pinned_flag(&checkpoint).await;
            return Err(error);
        }
        Ok(())
    }

    async fn restore_pinned_flag(&self, checkpoint: &MessageMutationCheckpoint) {
        let previous = RiseMessageWireCodec::record(&checkpoint.entity.data)
            .ok()
            .and_then(|record| record.wire.is_pinned)
            .unwrap_or(false);
        let restored = self
            .begin_patch(&checkpoint.entity.local_id, None, move |wire: &MessageWireDTO| {
                RiseMessageWireCodec::setting_boolean(wire, rise_key::message::IS_PINNED, previous)
            })
            .await;
        match restored {
            Ok(restored) => {
                let _ = self.acknowledge(&restored).await;
            }
            Err(_) => {
                let _ = self.rollback(checkpoint, None).await;
            }
        }
    }

    pub async fn reaction_limit_for_active_account(&self) -> Option<i64> {
        let account_id = self.account_id.clone();
        let is_premium: Option<bool> = on_main(move |cx| {
            let auth = AuthServicesStore::shared(cx);
            let auth = auth.read(cx);
            if auth.effective_user_id_for_wire().as_deref() != Some(account_id.as_str()) {
                return None;
            }
            let profile = auth.profile()?;
            (profile.id == account_id).then_some(profile.more.is_premium)
        })
        .await
        .flatten();
        Some(RiseMessageWireCodec::reaction_limit(is_premium?))
    }

    pub async fn set_reaction(&self, reaction: &str, selected: bool, local_id: &str) -> anyhow::Result<()> {
        let Some(own_reaction_limit) = self.reaction_limit_for_active_account().await else {
            return Err(CancellationError.into());
        };
        let checkpoint = self
            .begin_patch(local_id, None, |wire: &MessageWireDTO| {
                self.selecting_own_reaction(wire, reaction, selected, own_reaction_limit)
            })
            .await?;
        self.patch_pinned_copy(&checkpoint.optimistic_data, checkpoint.topic_id.as_deref()).await;
        let guard = {
            let checkpoint = checkpoint.clone();
            self.rollback_guard(move |repository| {
                async move {
                    let _ = repository.acknowledge_if_current(&checkpoint).await;
                }
                .boxed()
            })
        };
        let outcome: anyhow::Result<()> = async {
            if selected {
                let response = self
                    .remote
                    .add_reaction(rise_request::message::AddReaction {
                        message_id: checkpoint.server_id.clone(),
                        reaction: reaction.to_owned(),
                    })
                    .await?;
                self.validate(Some(response.success), Some(&response.error))?;
            } else {
                let response = self
                    .remote
                    .remove_reaction(rise_request::message::RemoveReaction {
                        message_id: checkpoint.server_id.clone(),
                        reaction: reaction.to_owned(),
                    })
                    .await?;
                self.validate(Some(response.success), Some(&response.error))?;
            }
            self.acknowledge(&checkpoint).await
        }
        .await;
        guard.disarm();
        if let Err(error) = outcome {
            if error.is::<CancellationError>() || CanonErrors::is_connection_loss(&error) {
                let _ = self.acknowledge_if_current(&checkpoint).await;
                return Err(error);
            }
            let _ = self.rollback(&checkpoint, None).await;
            self.patch_pinned_copy(&checkpoint.entity.data, checkpoint.topic_id.as_deref()).await;
            return Err(error);
        }
        Ok(())
    }

    async fn acknowledge_if_current(&self, checkpoint: &MessageMutationCheckpoint) -> anyhow::Result<()> {
        let current = self
            .engine
            .entity_local_id(RiseConversationStoreLayout::messages(), checkpoint.entity.local_id.clone())
            .await?;
        if !current.is_some_and(|current| current.data == checkpoint.optimistic_data) {
            return Ok(());
        }
        self.acknowledge(checkpoint).await
    }

    pub async fn preview_reaction(&self, reaction: &str, selected: bool, local_id: &str) -> anyhow::Result<()> {
        let Some(own_reaction_limit) = self.reaction_limit_for_active_account().await else {
            return Err(CancellationError.into());
        };
        let checkpoint = self
            .begin_patch(local_id, None, |wire: &MessageWireDTO| {
                self.selecting_own_reaction(wire, reaction, selected, own_reaction_limit)
            })
            .await?;
        self.acknowledge(&checkpoint).await?;
        self.patch_pinned_copy(&checkpoint.optimistic_data, checkpoint.topic_id.as_deref()).await;
        Ok(())
    }

    pub async fn queue_reaction(&self, reaction: &str, selected: bool, local_id: &str) -> anyhow::Result<()> {
        let Some(own_reaction_limit) = self.reaction_limit_for_active_account().await else {
            return Err(CancellationError.into());
        };
        let checkpoint = self
            .begin_patch(local_id, None, |wire: &MessageWireDTO| {
                self.selecting_own_reaction(wire, reaction, selected, own_reaction_limit)
            })
            .await?;
        self.acknowledge(&checkpoint).await?;
        self.patch_pinned_copy(&checkpoint.optimistic_data, checkpoint.topic_id.as_deref()).await;
        tracing::warn!(
            target: "riseonly::X2228",
            "reaction queue local={} server={} reaction={} selected={}",
            local_id,
            checkpoint.server_id,
            reaction,
            selected
        );
        let payload = RiseReactionDurablePayload {
            message_server_id: checkpoint.server_id.clone(),
            reaction: reaction.to_owned(),
        };
        let ordering_key: String = format!("message.reaction.{}", checkpoint.server_id)
            .chars()
            .take(ORDERING_KEY_CHARACTERS)
            .collect();
        let options = RiseSyncSubmitOptions {
            ordering_key: RiseDurableOrderingKey::new(ordering_key),
            ..Default::default()
        };
        if selected {
            self.engine.submit_durable::<RiseAddReactionDurableEndpoint>(payload, options).await?;
        } else {
            self.engine.submit_durable::<RiseRemoveReactionDurableEndpoint>(payload, options).await?;
        }
        Ok(())
    }

    fn selecting_own_reaction(
        &self,
        wire: &MessageWireDTO,
        reaction: &str,
        selected: bool,
        own_reaction_limit: i64,
    ) -> anyhow::Result<MessageWireDTO> {
        let is_mine = wire
            .reactions
            .as_ref()
            .and_then(|reactions| reactions.iter().find(|candidate| candidate.reaction == reaction))
            .is_some_and(|candidate| candidate.reacted_by_you == Some(true));
        if is_mine == selected {
            return Ok(wire.clone());
        }
        let payload = BTreeMap::from([
            (rise_key::events::REACTION.to_owned(), RiseJSONValue::String(reaction.to_owned())),
            (rise_key::events::USER_ID.to_owned(), RiseJSONValue::String(self.account_id.clone())),
        ]);
        RiseMessageWireCodec::applying_reaction(wire, &payload, &self.account_id, selected, Some(own_reaction_limit))
    }

    /// pinned screen keeps its own copies of messages
    /// a reaction on a pinned message must reach that copy or the old pills stay
    async fn patch_pinned_copy(&self, data: &[u8], topic_id: Option<&str>) {
        let Ok(record) = RiseMessageWireCodec::record(data) else {
            return;
        };
        let wire = record.wire;
        if wire.is_pinned != Some(true) {
            return;
        }
        let Some(chat_id) = self.normalized(wire.chat_id.as_deref()) else {
            return;
        };
        let topic = self.normalized(wire.topic_id.as_deref()).or_else(|| topic_id.map(str::to_owned));
        self.invalidate_auxiliary(
            RiseMessageAuxiliaryInvalidation::new(
                &chat_id,
                topic.as_deref(),
                HashSet::from([RiseMessageAuxiliaryProjection::Pinned]),
            )
            .with_pinned_message_patch(RiseMessageAuxiliaryInvalidationPinnedMessagePatch::from_message(wire, true)),
        )
        .await;
    }

    pub async fn mark_media_played(
        &self,
        local_id: &str,
        scope: &RiseMessageScope,
        closed: Option<bool>,
    ) -> anyhow::Result<()> {
        let payload = BTreeMap::from([(
            rise_key::events::PLAYED_BY.to_owned(),
            RiseJSONValue::Array(vec![RiseJSONValue::String(self.account_id.clone())]),
        )]);
        let checkpoint = self
            .begin_patch(local_id, None, move |wire: &MessageWireDTO| {
                RiseMessageWireCodec::merging_played_by(wire, &payload)
            })
            .await?;
        let guard = {
            let checkpoint = checkpoint.clone();
            self.rollback_guard(move |repository| {
                async move {
                    let _ = repository.rollback(&checkpoint, None).await;
                }
                .boxed()
            })
        };
        let outcome: anyhow::Result<()> = async {
            let response = self
                .remote
                .mark_media_played(rise_request::message::MarkMediaAsPlayed {
                    chat_id: scope.chat_id.clone(),
                    closed,
                    message_id: checkpoint.server_id.clone(),
                })
                .await?;
            self.validate(Some(response.success), Some(&response.error))?;
            self.acknowledge(&checkpoint).await
        }
        .await;
        guard.disarm();
        if let Err(error) = outcome {
            let _ = self.rollback(&checkpoint, None).await;
            return Err(error);
        }
        Ok(())
    }

    pub async fn mark_read(
        &self,
        scope: &RiseMessageScope,
        clearing_marks: bool,
        up_to_message_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let chat_id = scope.chat_id.clone();
        let started = self.state.with(|state| {
            if state.read_flight_chat_ids.insert(chat_id.clone()) {
                return true;
            }
            let already_queued = state.queued_reads.get(&chat_id).is_some_and(|pending| pending.contains_key(scope));
            let queued_full = already_queued
                && !state.queued_read_up_to.get(&chat_id).is_some_and(|pending| pending.contains_key(scope));
            let pending = state.queued_reads.entry(chat_id.clone()).or_default();
            let queued = pending.entry(scope.clone()).or_insert(false);
            *queued = *queued || clearing_marks;
            match up_to_message_id {
                Some(up_to_message_id) if !queued_full => {
                    state
                        .queued_read_up_to
                        .entry(chat_id.clone())
                        .or_default()
                        .insert(scope.clone(), up_to_message_id.to_owned());
                }
                _ => {
                    if let Some(pending) = state.queued_read_up_to.get_mut(&chat_id) {
                        pending.remove(scope);
                    }
                }
            }
            false
        });
        if !started {
            return Ok(());
        }
        let _flight_release = ReadFlightRelease { repository: self, chat_id: chat_id.clone() };
        let mut next = Some(RiseMessageQueuedRead {
            scope: scope.clone(),
            clearing_marks,
            up_to_message_id: up_to_message_id.map(str::to_owned),
        });
        let mut first_error: Option<anyhow::Error> = None;
        while let Some(read) = next {
            if let Err(error) =
                self.send_mark_read(&read.scope, read.clearing_marks, read.up_to_message_id.as_deref()).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
            next = self.dequeue_read(&chat_id);
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn dequeue_read(&self, chat_id: &str) -> Option<RiseMessageQueuedRead> {
        self.state.with(|state| {
            let first = state
                .queued_reads
                .get(chat_id)
                .and_then(|pending| pending.iter().next().map(|(scope, clearing)| (scope.clone(), *clearing)));
            let Some((first_scope, clearing_marks)) = first else {
                state.queued_reads.remove(chat_id);
                state.queued_read_up_to.remove(chat_id);
                return None;
            };
            if let Some(pending) = state.queued_reads.get_mut(chat_id) {
                pending.remove(&first_scope);
                if pending.is_empty() {
                    state.queued_reads.remove(chat_id);
                }
            }
            let up_to_message_id =
                state.queued_read_up_to.get_mut(chat_id).and_then(|pending| pending.remove(&first_scope));
            if state.queued_read_up_to.get(chat_id).is_some_and(HashMap::is_empty) {
                state.queued_read_up_to.remove(chat_id);
            }
            Some(RiseMessageQueuedRead { scope: first_scope, clearing_marks, up_to_message_id })
        })
    }

    async fn send_mark_read(
        &self,
        scope: &RiseMessageScope,
        clearing_marks: bool,
        up_to_message_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let window = self
            .engine
            .entity_window(RiseEntityWindowQuery::new(
                RiseConversationStoreLayout::messages(),
                RiseConversationStoreLayout::message_view(scope),
                RiseEntityWindow::Latest { limit: usize::try_from(self.retention_limit).unwrap_or(0) },
            ))
            .await?;
        let entities: Vec<RiseStoreEntity> = window.items.into_iter().map(|item| item.entity).collect();
        let unread = Self::incoming_unread_entities(&entities, &self.account_id);
        let originals: Vec<RiseStoreEntity> = if let Some(up_to_message_id) = up_to_message_id {
            let created_at =
                |record: &RiseMessageEntityRecord| record.wire.created_at.or(record.wire.timestamp).unwrap_or(0.0);
            let boundary = self
                .message_entity(up_to_message_id)
                .await?
                .and_then(|entity| RiseMessageWireCodec::record(&entity.data).ok())
                .map(|record| created_at(&record));
            match boundary {
                Some(limit) => unread
                    .into_iter()
                    .filter(|entity| {
                        RiseMessageWireCodec::record(&entity.data).is_ok_and(|record| created_at(&record) <= limit)
                    })
                    .collect(),
                None => Vec::new(),
            }
        } else {
            unread
        };
        let optimistic_version = self.next_version();
        let acknowledgement_version = self.next_version_floor(optimistic_version);
        let optimistic = self.read_mutations(&originals, true, optimistic_version)?;
        let prepared_chat_read: Option<RiseMessageChatReadOptimisticHookPrepared> = if scope.topic_id.is_none() && up_to_message_id.is_none() {
            self.chat_read_optimistic_hook.prepare(scope.chat_id.clone(), None).await.ok().flatten()
        } else {
            let clearable = Self::incoming_unread_count(&originals, &self.account_id);
            if clearable > 0 {
                self.chat_read_optimistic_hook.prepare(scope.chat_id.clone(), Some(clearable)).await.ok().flatten()
            } else {
                None
            }
        };
        let chat_read_batch = prepared_chat_read
            .as_ref()
            .map_or_else(RiseStoreWriteBatch::empty, |prepared| prepared.batch.clone());
        if let Err(error) = self.engine.write(self.combining(write_batch(optimistic), chat_read_batch)).await {
            if let Some(prepared) = prepared_chat_read {
                self.chat_read_optimistic_hook.discard(prepared.token).await;
            }
            return Err(error);
        }
        let guard = {
            let token = prepared_chat_read.as_ref().map(|prepared| prepared.token.clone());
            let originals = originals.clone();
            self.rollback_guard(move |repository| {
                async move {
                    if let Some(token) = token {
                        repository.chat_read_optimistic_hook.rollback(token).await;
                    }
                    repository.restore_read_originals(&originals, acknowledgement_version).await;
                }
                .boxed()
            })
        };
        let outcome: anyhow::Result<()> = async {
            let response = self
                .remote
                .mark_read(rise_request::message::MarkChatAsRead {
                    chat_id: scope.chat_id.clone(),
                    clear_marks: clearing_marks.then_some(true),
                    topic_id: scope.topic_id.clone(),
                    up_to_message_id: up_to_message_id.map(str::to_owned),
                })
                .await?;
            let message_batch = write_batch(self.read_mutations(&originals, true, acknowledgement_version)?);
            let preview_batch = self
                .preview_hook
                .call(
                    RiseMessagePreviewChange::Read {
                        chat_id: scope.chat_id.clone(),
                        topic_id: scope.topic_id.clone(),
                        authoritative_unread: Some(response.unread_count.max(0)),
                        clears_marks: clearing_marks,
                    },
                    acknowledgement_version,
                )
                .await?;
            self.engine.write(self.combining(message_batch, preview_batch)).await?;
            Ok(())
        }
        .await;
        guard.disarm();
        match outcome {
            Ok(()) => {
                if let Some(prepared) = prepared_chat_read {
                    self.chat_read_optimistic_hook.confirm(prepared.token).await;
                }
                Ok(())
            }
            Err(error) => {
                if let Some(prepared) = prepared_chat_read {
                    self.chat_read_optimistic_hook.rollback(prepared.token).await;
                }
                self.restore_read_originals(&originals, acknowledgement_version).await;
                Err(error)
            }
        }
    }

    /// rollback half of the mark read failure path
    async fn restore_read_originals(&self, originals: &[RiseStoreEntity], version: i64) {
        let rollback = originals
            .iter()
            .map(|entity| RiseStoreMutation::UpsertEntity {
                namespace: RiseConversationStoreLayout::messages(),
                entity: RiseStoreEntity::new(
                    entity.local_id.clone(),
                    entity.data.clone(),
                    version,
                    Self::now_milliseconds(),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            })
            .collect();
        let _ = self.engine.write(write_batch(rollback)).await;
    }

    pub async fn send_typing(&self, typing: bool, action: Option<&str>, scope: &RiseMessageScope) -> anyhow::Result<()> {
        self.remote
            .send_typing(rise_request::message::MessageTyping {
                action: action.map(str::to_owned),
                chat_id: Some(scope.chat_id.clone()),
                is_typing: Some(typing),
                topic_id: scope.topic_id.clone().filter(|topic_id| !topic_id.is_empty()),
                ..Default::default()
            })
            .await?;
        Ok(())
    }

    pub async fn loaded_entities(&self, chat_id: &str) -> anyhow::Result<Vec<RiseStoreEntity>> {
        let result = self
            .engine
            .entity_window(RiseEntityWindowQuery::new(
                RiseConversationStoreLayout::messages(),
                RiseConversationStoreLayout::message_view(&RiseMessageScope::new(chat_id.to_owned(), None, None)),
                RiseEntityWindow::Latest { limit: usize::try_from(self.retention_limit).unwrap_or(0) },
            ))
            .await?;
        Ok(result.items.into_iter().map(|item| item.entity).collect())
    }

    pub async fn begin_bulk_patch(
        &self,
        entities: Vec<RiseStoreEntity>,
        transform: impl Fn(&MessageWireDTO) -> anyhow::Result<MessageWireDTO> + Send,
    ) -> anyhow::Result<BulkMessagePatchCheckpoint> {
        if entities.is_empty() {
            return Ok(BulkMessagePatchCheckpoint {
                originals: Vec::new(),
                optimistic: Vec::new(),
                acknowledgement_version: self.next_version(),
            });
        }
        self.state.with(|state| {
            for entity in &entities {
                state.version_clock = state.version_clock.max(entity.version);
            }
        });
        let optimistic_version = self.next_version();
        let acknowledgement_version = self.next_version_floor(optimistic_version);
        let mut rows: Vec<(String, Vec<u8>)> = Vec::with_capacity(entities.len());
        let mut mutations: Vec<RiseStoreMutation> = Vec::with_capacity(entities.len());
        for entity in &entities {
            let old = RiseMessageWireCodec::record(&entity.data)?;
            let updated = RiseMessageEntityRecord {
                wire: transform(&old.wire)?,
                source: RiseConversationEntitySource::Optimistic,
                local_phase: old.local_phase,
                client_request_id: old.client_request_id,
                last_error_code: old.last_error_code,
                scope_storage_key: old.scope_storage_key,
                is_replayable: old.is_replayable,
                upload_retry: old.upload_retry,
                forward_retry: old.forward_retry,
            };
            let data = RiseMessageWireCodec::record_data(&updated)?;
            rows.push((entity.local_id.clone(), data.clone()));
            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: RiseConversationStoreLayout::messages(),
                entity: RiseStoreEntity::new(
                    entity.local_id.clone(),
                    data,
                    optimistic_version,
                    Self::now_milliseconds(),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            });
        }
        self.engine.write(write_batch(mutations)).await?;
        Ok(BulkMessagePatchCheckpoint { originals: entities, optimistic: rows, acknowledgement_version })
    }

    pub async fn acknowledge_bulk(&self, checkpoint: &BulkMessagePatchCheckpoint) -> anyhow::Result<()> {
        if checkpoint.optimistic.is_empty() {
            return Ok(());
        }
        let mutations = checkpoint
            .optimistic
            .iter()
            .map(|(local_id, data)| RiseStoreMutation::UpsertEntity {
                namespace: RiseConversationStoreLayout::messages(),
                entity: RiseStoreEntity::new(
                    local_id.clone(),
                    data.clone(),
                    checkpoint.acknowledgement_version,
                    Self::now_milliseconds(),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            })
            .collect();
        self.engine.write(write_batch(mutations)).await?;
        Ok(())
    }

    pub async fn rollback_bulk(&self, checkpoint: &BulkMessagePatchCheckpoint) -> anyhow::Result<()> {
        if checkpoint.originals.is_empty() {
            return Ok(());
        }
        let mutations = checkpoint
            .originals
            .iter()
            .map(|entity| RiseStoreMutation::UpsertEntity {
                namespace: RiseConversationStoreLayout::messages(),
                entity: RiseStoreEntity::new(
                    entity.local_id.clone(),
                    entity.data.clone(),
                    checkpoint.acknowledgement_version,
                    Self::now_milliseconds(),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            })
            .collect();
        self.engine.write(write_batch(mutations)).await?;
        Ok(())
    }

    pub async fn deletion_checkpoints(
        &self,
        entities: &[RiseStoreEntity],
        scope: &RiseMessageScope,
    ) -> anyhow::Result<Vec<MessageDeletionCheckpoint>> {
        let mut result = Vec::with_capacity(entities.len());
        let scopes = self.deletion_scopes(scope);
        for entity in entities {
            let mut positions = Vec::new();
            for candidate in &scopes {
                let view = RiseConversationStoreLayout::message_view(candidate);
                if let Some(position) = self
                    .engine
                    .entity_view_position(
                        RiseConversationStoreLayout::messages(),
                        view.clone(),
                        entity.local_id.clone(),
                    )
                    .await?
                {
                    positions.push(MessageDeletionCheckpointPosition { view, key: position.position_key });
                }
            }
            result.push(MessageDeletionCheckpoint { entity: entity.clone(), positions });
        }
        Ok(result)
    }

    pub async fn restore_deletion_checkpoints(
        &self,
        checkpoints: &[MessageDeletionCheckpoint],
        version: i64,
    ) -> anyhow::Result<()> {
        if checkpoints.is_empty() {
            return Ok(());
        }
        self.engine.write(self.make_restore_batch(checkpoints, version)).await?;
        Ok(())
    }

    pub fn make_restore_batch(&self, checkpoints: &[MessageDeletionCheckpoint], version: i64) -> RiseStoreWriteBatch {
        let namespace = RiseConversationStoreLayout::messages();
        let mut mutations: Vec<RiseStoreMutation> = Vec::new();
        for checkpoint in checkpoints {
            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: namespace.clone(),
                entity: RiseStoreEntity::new(
                    checkpoint.entity.local_id.clone(),
                    checkpoint.entity.data.clone(),
                    version,
                    Self::now_milliseconds(),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            });
            for position in &checkpoint.positions {
                mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                    namespace: namespace.clone(),
                    view: position.view.clone(),
                    membership: RiseEntityViewMembership::new(
                        checkpoint.entity.local_id.clone(),
                        position.key.clone(),
                        version,
                    ),
                });
            }
            mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                namespace: namespace.clone(),
                view: Self::TOMBSTONE_VIEW.to_owned(),
                local_id: checkpoint.entity.local_id.clone(),
                if_version_at_most: Some(version),
            });
        }
        write_batch(mutations)
    }

    /// requoting_in names the chat whose held replies redraw
    /// their quote of the patched message in the same write
    pub async fn begin_patch(
        &self,
        identifier: &str,
        requoting_in: Option<&str>,
        transform: impl FnOnce(&MessageWireDTO) -> anyhow::Result<MessageWireDTO> + Send,
    ) -> anyhow::Result<MessageMutationCheckpoint> {
        self.write_patch(identifier, false, requoting_in, transform).await
    }

    pub async fn begin_prepared_patch(
        &self,
        identifier: &str,
        transform: impl FnOnce(&MessageWireDTO) -> anyhow::Result<MessageWireDTO> + Send,
    ) -> anyhow::Result<MessageMutationCheckpoint> {
        self.write_patch(identifier, true, None, transform).await
    }

    async fn write_patch(
        &self,
        identifier: &str,
        begins_prepared_upload: bool,
        requoting_in: Option<&str>,
        transform: impl FnOnce(&MessageWireDTO) -> anyhow::Result<MessageWireDTO> + Send,
    ) -> anyhow::Result<MessageMutationCheckpoint> {
        let Some(entity) = self
            .message_entity(identifier)
            .await?
            .filter(|entity| !RiseMessageWireCodec::is_tombstone(&entity.data))
        else {
            return Err(RiseMessageRepositoryError::MessageNotFound(identifier.to_owned()).into());
        };
        self.state.with(|state| state.version_clock = state.version_clock.max(entity.version));
        let old = RiseMessageWireCodec::record(&entity.data)?;
        let Some(server_id) = RiseConversationStoreLayout::canonical_message_server_id(&old.wire) else {
            return Err(RiseMessageRepositoryError::MissingServerID(entity.local_id.clone()).into());
        };
        let wire = transform(&old.wire)?;
        let optimistic_version = self.next_version_floor(entity.version);
        let acknowledgement_version = self.next_version_floor(optimistic_version);
        let topic_id = self.normalized(old.wire.topic_id.as_deref());
        let updated = RiseMessageEntityRecord {
            wire: wire.clone(),
            source: RiseConversationEntitySource::Optimistic,
            local_phase: old.local_phase,
            client_request_id: old.client_request_id,
            last_error_code: old.last_error_code,
            scope_storage_key: old.scope_storage_key,
            is_replayable: old.is_replayable,
            upload_retry: old.upload_retry,
            forward_retry: old.forward_retry,
        };
        let data = RiseMessageWireCodec::record_data(&updated)?;
        let mut quotes: Vec<RiseStoreMutation> = Vec::new();
        if let Some(chat_id) = self.normalized(requoting_in) {
            let quoted: HashSet<String> = RiseConversationStoreLayout::message_aliases(&wire)
                .into_iter()
                .map(|alias| alias.value)
                .collect();
            let excluding = HashSet::from([entity.local_id.clone()]);
            quotes = self.quoting_row_mutations(&chat_id, quoted, Some(&wire), &excluding).await;
        }
        if begins_prepared_upload {
            self.begin_prepared_patch_progress(&entity.local_id).await;
        }
        let mut mutations = vec![RiseStoreMutation::UpsertEntity {
            namespace: RiseConversationStoreLayout::messages(),
            entity: RiseStoreEntity::new(
                entity.local_id.clone(),
                data.clone(),
                optimistic_version,
                Self::now_milliseconds(),
            ),
            policy: RiseEntityWritePolicy::ServerAuthoritative,
        }];
        mutations.extend(quotes);
        if let Err(error) = self.engine.write(write_batch(mutations)).await {
            if begins_prepared_upload {
                self.finish_prepared_patch(&entity.local_id).await;
            }
            return Err(error);
        }
        Ok(MessageMutationCheckpoint {
            entity,
            server_id,
            topic_id,
            optimistic_data: data,
            acknowledgement_version,
        })
    }

    pub async fn acknowledge(&self, checkpoint: &MessageMutationCheckpoint) -> anyhow::Result<()> {
        self.engine
            .write(write_batch(vec![RiseStoreMutation::UpsertEntity {
                namespace: RiseConversationStoreLayout::messages(),
                entity: RiseStoreEntity::new(
                    checkpoint.entity.local_id.clone(),
                    checkpoint.optimistic_data.clone(),
                    checkpoint.acknowledgement_version,
                    Self::now_milliseconds(),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            }]))
            .await?;
        Ok(())
    }

    pub async fn rollback(
        &self,
        checkpoint: &MessageMutationCheckpoint,
        requoting_in: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut quotes: Vec<RiseStoreMutation> = Vec::new();
        if let Some(chat_id) = self.normalized(requoting_in)
            && let Ok(restored) = RiseMessageWireCodec::record(&checkpoint.entity.data)
        {
            let quoted: HashSet<String> = RiseConversationStoreLayout::message_aliases(&restored.wire)
                .into_iter()
                .map(|alias| alias.value)
                .collect();
            let excluding = HashSet::from([checkpoint.entity.local_id.clone()]);
            quotes = self.quoting_row_mutations(&chat_id, quoted, Some(&restored.wire), &excluding).await;
        }
        let mut mutations = vec![RiseStoreMutation::UpsertEntity {
            namespace: RiseConversationStoreLayout::messages(),
            entity: RiseStoreEntity::new(
                checkpoint.entity.local_id.clone(),
                checkpoint.entity.data.clone(),
                checkpoint.acknowledgement_version,
                Self::now_milliseconds(),
            ),
            policy: RiseEntityWritePolicy::ServerAuthoritative,
        }];
        mutations.extend(quotes);
        self.engine.write(write_batch(mutations)).await?;
        Ok(())
    }

    /// rolls back a long local preparation only if its optimistic bytes are still current
    /// a realtime update that came meanwhile is never overwritten
    pub async fn rollback_prepared_patch(&self, checkpoint: MessageMutationCheckpoint) -> anyhow::Result<()> {
        let local_id = checkpoint.entity.local_id.clone();
        let rolled_back: anyhow::Result<()> = async {
            let current = self.engine.entity_local_id(RiseConversationStoreLayout::messages(), local_id.clone()).await?;
            let Some(current) = current.filter(|current| current.data == checkpoint.optimistic_data) else {
                return Ok(());
            };
            self.engine
                .write(write_batch(vec![RiseStoreMutation::UpsertEntity {
                    namespace: RiseConversationStoreLayout::messages(),
                    entity: RiseStoreEntity::new(
                        local_id.clone(),
                        checkpoint.entity.data.clone(),
                        self.next_version_floor(current.version),
                        Self::now_milliseconds(),
                    ),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                }]))
                .await?;
            Ok(())
        }
        .await;
        self.finish_prepared_patch(&local_id).await;
        rolled_back
    }

    pub async fn publish_prepared_patch_progress(&self, local_id: &str, fraction: f64) {
        self.upload_progress_hub.publish(upload_slot_progress(local_id, 0, fraction, RiseUploadPhase::Uploading)).await;
    }

    pub async fn begin_prepared_patch_progress(&self, local_id: &str) {
        self.upload_cancellation_hub.begin(local_id).await;
        self.upload_progress_hub.begin(local_id).await;
    }

    pub async fn finish_prepared_patch(&self, local_id: &str) {
        self.upload_progress_hub.remove_owner_id(local_id).await;
        self.upload_cancellation_hub.finish(local_id).await;
    }

    pub fn server_id(&self, entity: &RiseStoreEntity) -> anyhow::Result<String> {
        let record = RiseMessageWireCodec::record(&entity.data)?;
        RiseConversationStoreLayout::canonical_message_server_id(&record.wire)
            .ok_or_else(|| RiseMessageRepositoryError::MissingServerID(entity.local_id.clone()).into())
    }

    /// every id a chat or topic row can hold for this message as preview
    /// own send keeps its local id there until the ack, a delete by server id alone left the text
    pub fn preview_identities(entity: &RiseStoreEntity) -> Vec<String> {
        let mut values = vec![entity.local_id.clone()];
        if let Ok(record) = RiseMessageWireCodec::record(&entity.data) {
            let wire = record.wire;
            let canonical = RiseConversationStoreLayout::canonical_message_server_id(&wire);
            values.extend(
                [canonical.as_deref(), wire.server_id.as_deref(), wire.id.as_deref(), wire.temp_id.as_deref()]
                    .into_iter()
                    .filter_map(RiseMessageWireCodec::normalized),
            );
        }
        let mut seen = HashSet::new();
        values.retain(|value| seen.insert(value.clone()));
        values
    }

    /// sender with 500+ rows makes a tombstone batch of thousands of mutations past the store bound
    /// so it cuts by the canon bound
    pub async fn write_delete_in_slices(
        &self,
        checkpoints: &[MessageDeletionCheckpoint],
        version: i64,
    ) -> anyhow::Result<()> {
        let mut batch = RiseStoreWriteBatch::empty();
        for item in checkpoints {
            let memberships: Vec<(String, String)> =
                item.positions.iter().map(|position| (item.entity.local_id.clone(), position.view.clone())).collect();
            let next = self.make_delete_batch(std::slice::from_ref(&item.entity), version, &memberships)?;
            self.accumulate(next, &mut batch).await?;
        }
        if batch.mutations.is_empty() {
            return Ok(());
        }
        self.engine.write(batch).await?;
        Ok(())
    }

    pub async fn restore_deletion_checkpoints_in_slices(
        &self,
        checkpoints: &[MessageDeletionCheckpoint],
        version: i64,
    ) -> anyhow::Result<()> {
        let mut batch = RiseStoreWriteBatch::empty();
        for checkpoint in checkpoints {
            let next = self.make_restore_batch(std::slice::from_ref(checkpoint), version);
            self.accumulate(next, &mut batch).await?;
        }
        if batch.mutations.is_empty() {
            return Ok(());
        }
        self.engine.write(batch).await?;
        Ok(())
    }

    pub async fn write_delete(
        &self,
        entities: &[RiseStoreEntity],
        version: i64,
        memberships: &[(String, String)],
    ) -> anyhow::Result<()> {
        self.engine.write(self.make_delete_batch(entities, version, memberships)?).await?;
        Ok(())
    }

    pub fn make_delete_batch(
        &self,
        entities: &[RiseStoreEntity],
        version: i64,
        memberships: &[(String, String)],
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let namespace = RiseConversationStoreLayout::messages();
        let mut mutations: Vec<RiseStoreMutation> = Vec::new();
        for entity in entities {
            let record = RiseMessageWireCodec::record(&entity.data)?;
            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: namespace.clone(),
                entity: RiseStoreEntity::new(
                    entity.local_id.clone(),
                    RiseMessageWireCodec::tombstone_data(&record, Self::now_milliseconds())?,
                    version,
                    Self::now_milliseconds(),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            });
            for alias in RiseConversationStoreLayout::message_aliases(&record.wire) {
                mutations.push(RiseStoreMutation::SetEntityAlias {
                    namespace: namespace.clone(),
                    alias,
                    local_id: entity.local_id.clone(),
                });
            }
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: namespace.clone(),
                view: Self::TOMBSTONE_VIEW.to_owned(),
                membership: RiseEntityViewMembership::new(
                    entity.local_id.clone(),
                    RisePositionKey::make(|key| {
                        key.append_signed_int64(version);
                        key.append_string(&entity.local_id);
                    }),
                    version,
                ),
            });
        }
        for (local_id, view) in memberships {
            mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                namespace: namespace.clone(),
                view: view.clone(),
                local_id: local_id.clone(),
                if_version_at_most: Some(version),
            });
        }
        mutations.push(RiseStoreMutation::TrimEntityView {
            namespace,
            view: Self::TOMBSTONE_VIEW.to_owned(),
            keeping: RiseEntityViewRetention::Latest(usize::try_from(self.retention_limit).unwrap_or(0)),
            if_version_at_most: None,
        });
        Ok(write_batch(mutations))
    }

    /// lower bound of the chat aggregate a scoped read clears
    /// is_read means somebody read past it, so only others unread rows count
    /// can fall short but never over
    pub fn incoming_unread_count(entities: &[RiseStoreEntity], account_id: &str) -> i64 {
        i64::try_from(Self::incoming_unread_entities(entities, account_id).len()).unwrap_or(i64::MAX)
    }

    pub fn incoming_unread_entities(entities: &[RiseStoreEntity], account_id: &str) -> Vec<RiseStoreEntity> {
        entities
            .iter()
            .filter(|entity| {
                if RiseMessageWireCodec::is_tombstone(&entity.data) {
                    return false;
                }
                let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
                    return false;
                };
                if record.local_phase != RiseMessageLocalPhase::Remote || record.wire.is_read == Some(true) {
                    return false;
                }
                let sender_id = record
                    .wire
                    .sender_id
                    .as_deref()
                    .or_else(|| record.wire.sender.as_ref().and_then(|sender| sender.id.as_deref()))
                    .unwrap_or_default();
                !sender_id.is_empty() && sender_id != account_id
            })
            .cloned()
            .collect()
    }

    pub fn read_mutations(
        &self,
        entities: &[RiseStoreEntity],
        value: bool,
        version: i64,
    ) -> anyhow::Result<Vec<RiseStoreMutation>> {
        let mut mutations = Vec::with_capacity(entities.len());
        for entity in entities {
            let record = RiseMessageWireCodec::record(&entity.data)?;
            if record.wire.is_read == Some(value) {
                continue;
            }
            let wire = RiseMessageWireCodec::setting_boolean(&record.wire, rise_key::message::IS_READ, value)?;
            let updated = RiseMessageEntityRecord {
                wire,
                source: RiseConversationEntitySource::Optimistic,
                local_phase: record.local_phase,
                client_request_id: record.client_request_id,
                last_error_code: record.last_error_code,
                scope_storage_key: record.scope_storage_key,
                is_replayable: record.is_replayable,
                upload_retry: record.upload_retry,
                forward_retry: record.forward_retry,
            };
            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: RiseConversationStoreLayout::messages(),
                entity: RiseStoreEntity::new(
                    entity.local_id.clone(),
                    RiseMessageWireCodec::record_data(&updated)?,
                    version,
                    Self::now_milliseconds(),
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            });
        }
        Ok(mutations)
    }
}
