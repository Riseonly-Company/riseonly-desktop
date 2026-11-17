use std::collections::{HashMap, HashSet};
use std::iter::once;
use std::sync::LazyLock;
use std::time::Duration;

use parking_lot::Mutex;
use rise_canon::rise_proto::message::{ProtoAddReactionResponse, ProtoRemoveReactionResponse};
use rise_canon::rise_request::message::{AddReaction, RemoveReaction};
use rise_canon::rise_view::message::Created;
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_date::Date;
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RiseEntityViewMembership, RiseEntityViewRetention, RiseEntityWritePolicy,
    RiseStoreEntity,
};
use rise_engine::rise_store::rise_store_models::{RiseStoreMutation, RiseStoreWriteBatch};
use rise_engine::rise_sync::{
    RiseSyncDurableContext, RiseSyncDurableEndpoint, RiseSyncDurableFailure, RiseSyncDurableFailureDisposition,
};
use rise_engine::{RiseJSONValue, RiseWireMethod, RiseWireMethodDescriptor, RiseWireRemoteError, RiseWireReplayPolicy};
use rise_errors::{ErrorCategory, ErrorCode, ErrorParam};
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_errors::CanonErrors;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseConversationEntitySource, RiseMessageEntityRecord, RiseMessageLocalPhase,
};
use crate::modules::chat::engine::core::rise_conversation_store_layout::{
    RiseConversationStoreLayout, RiseEntityAliasKindExt,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_stored_canon::create_message_from_stored;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    CreateMessageGatewayDTO, MessageWireDTO, RiseStoredOutgoingMessage,
};

const MESSAGE_WIRE_TIMEOUT: Duration = Duration::from_secs(30);

pub enum RiseCreateMessageWireMethod {}

impl RiseWireMethod for RiseCreateMessageWireMethod {
    type Request = RiseJSONValue;
    type Response = Created;

    fn descriptor() -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(rise_wire_methods::message::SERVICE, rise_wire_methods::message::CREATE_MESSAGE)
            .with_timeout(MESSAGE_WIRE_TIMEOUT)
            .with_replay_policy(RiseWireReplayPolicy::RequestID)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseCreateMessageDurablePayload {
    pub body: RiseStoredOutgoingMessage,
    pub optimistic_wire: MessageWireDTO,
    pub scope: RiseMessageScope,
    pub materialized_scopes: Vec<RiseMessageScope>,
    #[serde(rename = "localID")]
    pub local_id: String,
    #[serde(rename = "clientRequestID")]
    pub client_request_id: String,
    pub version: i64,
    pub retention_limit: i64,
}

pub enum RiseCreateMessageDurableEndpoint {}

impl RiseSyncDurableEndpoint for RiseCreateMessageDurableEndpoint {
    type Payload = RiseCreateMessageDurablePayload;
    type Method = RiseCreateMessageWireMethod;

    fn kind() -> &'static str {
        Self::KIND
    }

    fn classify_failure(
        failure: &RiseSyncDurableFailure,
        context: &RiseSyncDurableContext,
    ) -> RiseSyncDurableFailureDisposition {
        if let RiseSyncDurableFailure::Remote(error) = failure {
            RiseSlowModeRefusals::note(error, &context.idempotency_key);
            if let Some(entry) = CanonErrors::entry(Some(&error.code))
                && entry.category == ErrorCategory::Unavailable
                && entry.retryable
            {
                return RiseSyncDurableFailureDisposition::Retry { code: error.code.clone() };
            }
        }
        RiseSyncDurableFailureDisposition::standard(failure)
    }

    fn make_request(payload: &Self::Payload, _context: &RiseSyncDurableContext) -> anyhow::Result<RiseJSONValue> {
        let mut body = payload.body.clone();
        body.temp_id = Some(payload.local_id.clone());
        body.chat_id = body.chat_id.or_else(|| Some(payload.scope.chat_id.clone()));
        body.topic_id = body.topic_id.or_else(|| payload.scope.topic_id.clone());
        Ok(RiseJSONValue::encode(&create_message_from_stored(&body))?)
    }

    fn reconcile(
        payload: &Self::Payload,
        created: &Created,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let response = CreateMessageGatewayDTO::from_canon(created)?;
        let wire = RiseMessageWireCodec::acknowledged_wire(
            &payload.optimistic_wire,
            &response,
            &payload.local_id,
            &payload.scope,
        )?;
        Self::acknowledgement_batch(payload, &wire)
    }

    fn terminal_reconciliation(
        payload: &Self::Payload,
        code: &str,
        context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        RiseSlowModeRefusals::settle(&context.idempotency_key, &payload.scope.chat_id);
        Self::failed_batch(payload, code)
    }
}

impl RiseCreateMessageDurableEndpoint {
    pub const KIND: &str = "conversation.message.create.v1";

    pub fn acknowledgement_batch(
        payload: &RiseCreateMessageDurablePayload,
        wire: &MessageWireDTO,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let namespace = RiseConversationStoreLayout::messages();
        let now = Self::now_milliseconds();
        let mut record = RiseMessageEntityRecord::new(wire.clone(), RiseConversationEntitySource::Acknowledgement);
        record.local_phase = RiseMessageLocalPhase::Remote;
        record.client_request_id = Some(payload.client_request_id.clone());
        record.scope_storage_key = Some(payload.scope.storage_key());
        let data = RiseMessageWireCodec::record_data(&record)?;
        let mut mutations = vec![RiseStoreMutation::UpsertEntity {
            namespace: namespace.clone(),
            entity: RiseStoreEntity::new(payload.local_id.clone(), data.clone(), payload.version, now),
            policy: RiseEntityWritePolicy::IgnoreIfNotNewer,
        }];

        if let Some(server_id) = RiseConversationStoreLayout::canonical_message_server_id(wire)
            && server_id != payload.local_id
        {
            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: namespace.clone(),
                entity: RiseStoreEntity::new(server_id.clone(), data, payload.version, now),
                policy: RiseEntityWritePolicy::IgnoreIfNotNewer,
            });
            mutations.push(RiseStoreMutation::MergeEntities {
                namespace: namespace.clone(),
                survivor_local_id: payload.local_id.clone(),
                duplicate_local_id: server_id,
            });
        }

        let mut aliases = RiseConversationStoreLayout::message_aliases(wire);
        aliases.push(RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), payload.local_id.clone()));
        aliases.push(RiseEntityAlias::new(RiseEntityAliasKind::request_id(), payload.client_request_id.clone()));
        for alias in aliases.into_iter().collect::<HashSet<_>>() {
            mutations.push(RiseStoreMutation::SetEntityAlias {
                namespace: namespace.clone(),
                alias,
                local_id: payload.local_id.clone(),
            });
        }
        mutations.push(RiseStoreMutation::RemoveEntityAlias {
            namespace: namespace.clone(),
            alias: RiseEntityAlias::new(RiseEntityAliasKind::retained(), payload.local_id.clone()),
        });

        let position = RiseConversationStoreLayout::message_position(wire, &payload.local_id, RiseMessageLocalPhase::Remote);
        for scope in Self::scopes(payload) {
            let view = RiseConversationStoreLayout::message_view(scope);
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: namespace.clone(),
                view: view.clone(),
                membership: RiseEntityViewMembership::new(payload.local_id.clone(), position.clone(), payload.version),
            });
            mutations.push(RiseStoreMutation::TrimEntityView {
                namespace: namespace.clone(),
                view,
                keeping: Self::retention(payload),
                if_version_at_most: None,
            });
        }
        mutations.push(RiseStoreMutation::PruneUnreferencedEntities { namespace, limit: payload.retention_limit });
        Ok(RiseStoreWriteBatch::new(mutations, HashSet::new()))
    }

    fn failed_batch(payload: &RiseCreateMessageDurablePayload, code: &str) -> anyhow::Result<RiseStoreWriteBatch> {
        let namespace = RiseConversationStoreLayout::messages();
        let now = Self::now_milliseconds();
        let mut record =
            RiseMessageEntityRecord::new(payload.optimistic_wire.clone(), RiseConversationEntitySource::Optimistic);
        record.local_phase = RiseMessageLocalPhase::Failed;
        record.client_request_id = Some(payload.client_request_id.clone());
        record.last_error_code = Some(code.to_string());
        record.scope_storage_key = Some(payload.scope.storage_key());
        record.is_replayable = Some(true);
        let data = RiseMessageWireCodec::record_data(&record)?;
        let failure_version = payload.version - 1;
        let membership_version = payload.version;
        let mut mutations = vec![
            RiseStoreMutation::UpsertEntity {
                namespace: namespace.clone(),
                entity: RiseStoreEntity::new(payload.local_id.clone(), data, failure_version, now),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            },
            RiseStoreMutation::SetEntityAlias {
                namespace: namespace.clone(),
                alias: RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), payload.local_id.clone()),
                local_id: payload.local_id.clone(),
            },
            RiseStoreMutation::SetEntityAlias {
                namespace: namespace.clone(),
                alias: RiseEntityAlias::new(RiseEntityAliasKind::request_id(), payload.client_request_id.clone()),
                local_id: payload.local_id.clone(),
            },
            RiseStoreMutation::SetEntityAlias {
                namespace: namespace.clone(),
                alias: RiseEntityAlias::new(RiseEntityAliasKind::retained(), payload.local_id.clone()),
                local_id: payload.local_id.clone(),
            },
        ];
        let position = RiseConversationStoreLayout::message_position(
            &payload.optimistic_wire,
            &payload.local_id,
            RiseMessageLocalPhase::Failed,
        );
        for scope in Self::scopes(payload) {
            let view = RiseConversationStoreLayout::message_view(scope);
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: namespace.clone(),
                view: view.clone(),
                membership: RiseEntityViewMembership::new(payload.local_id.clone(), position.clone(), membership_version),
            });
            mutations.push(RiseStoreMutation::TrimEntityView {
                namespace: namespace.clone(),
                view,
                keeping: Self::retention(payload),
                if_version_at_most: None,
            });
        }
        mutations.push(RiseStoreMutation::PruneUnreferencedEntities { namespace, limit: payload.retention_limit });
        Ok(RiseStoreWriteBatch::new(mutations, HashSet::new()))
    }

    fn scopes(payload: &RiseCreateMessageDurablePayload) -> HashSet<&RiseMessageScope> {
        payload.materialized_scopes.iter().chain(once(&payload.scope)).collect()
    }

    fn retention(payload: &RiseCreateMessageDurablePayload) -> RiseEntityViewRetention {
        RiseEntityViewRetention::Latest(usize::try_from(payload.retention_limit).unwrap_or(0))
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}

#[derive(Default)]
struct RiseSlowModeRefusalsState {
    by_request: HashMap<String, i64>,
    by_chat: HashMap<String, Date>,
}

static RISE_SLOW_MODE_REFUSALS_STATE: LazyLock<Mutex<RiseSlowModeRefusalsState>> = LazyLock::new(Mutex::default);

/// server own slow mode wait for a refused send
/// local countdown only knows sends from this device so retry_after_s of the refusal is the truth
pub enum RiseSlowModeRefusals {}

impl RiseSlowModeRefusals {
    pub fn note(error: &RiseWireRemoteError, request_key: &str) {
        if error.code != ErrorCode::ChatSlowModeActive.wire() {
            return;
        }
        let Some(RiseJSONValue::Object(fields)) = &error.details else {
            return;
        };
        let Some(seconds) = Self::whole_seconds(fields.get(ErrorParam::RetryAfterS.wire())) else {
            return;
        };
        if seconds <= 0 {
            return;
        }
        RISE_SLOW_MODE_REFUSALS_STATE.lock().by_request.insert(request_key.to_string(), seconds);
    }

    pub fn settle(request_key: &str, chat_id: &str) {
        let mut state = RISE_SLOW_MODE_REFUSALS_STATE.lock();
        let Some(seconds) = state.by_request.remove(request_key) else {
            return;
        };
        state.by_chat.insert(chat_id.to_string(), Date::now().adding_time_interval(seconds as f64));
    }

    pub fn take_wait_end(chat_id: &str) -> Option<Date> {
        RISE_SLOW_MODE_REFUSALS_STATE.lock().by_chat.remove(chat_id)
    }

    fn whole_seconds(value: Option<&RiseJSONValue>) -> Option<i64> {
        match value? {
            RiseJSONValue::Integer(number) => Some(*number),
            RiseJSONValue::UnsignedInteger(number) => i64::try_from(*number).ok(),
            RiseJSONValue::Number(number) => {
                let rounded = number.ceil();
                (rounded.is_finite() && rounded >= i64::MIN as f64 && rounded < i64::MAX as f64).then_some(rounded as i64)
            }
            RiseJSONValue::String(text) => text.parse().ok(),
            _ => None,
        }
    }
}

pub enum RiseAddReactionWireMethod {}

impl RiseWireMethod for RiseAddReactionWireMethod {
    type Request = RiseJSONValue;
    type Response = ProtoAddReactionResponse;

    fn descriptor() -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(rise_wire_methods::message::SERVICE, rise_wire_methods::message::ADD_REACTION)
            .with_timeout(MESSAGE_WIRE_TIMEOUT)
            .with_replay_policy(RiseWireReplayPolicy::RequestID)
    }
}

pub enum RiseRemoveReactionWireMethod {}

impl RiseWireMethod for RiseRemoveReactionWireMethod {
    type Request = RiseJSONValue;
    type Response = ProtoRemoveReactionResponse;

    fn descriptor() -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(rise_wire_methods::message::SERVICE, rise_wire_methods::message::REMOVE_REACTION)
            .with_timeout(MESSAGE_WIRE_TIMEOUT)
            .with_replay_policy(RiseWireReplayPolicy::RequestID)
    }
}

/// a reaction chosen offline, the row already shows it
/// add and remove are idempotent on the server so a replay after relaunch is safe
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RiseReactionDurablePayload {
    #[serde(rename = "messageServerID")]
    pub message_server_id: String,
    pub reaction: String,
}

pub enum RiseAddReactionDurableEndpoint {}

impl RiseAddReactionDurableEndpoint {
    pub const KIND: &str = "conversation.message.reaction.add.v1";
}

impl RiseSyncDurableEndpoint for RiseAddReactionDurableEndpoint {
    type Payload = RiseReactionDurablePayload;
    type Method = RiseAddReactionWireMethod;

    fn kind() -> &'static str {
        Self::KIND
    }

    fn make_request(payload: &Self::Payload, _context: &RiseSyncDurableContext) -> anyhow::Result<RiseJSONValue> {
        Ok(RiseJSONValue::encode(&AddReaction {
            message_id: payload.message_server_id.clone(),
            reaction: payload.reaction.clone(),
        })?)
    }

    fn reconcile(
        payload: &Self::Payload,
        response: &ProtoAddReactionResponse,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        tracing::warn!(
            target: "riseonly::X2228",
            "reaction add reply message={} reaction={} success={} error={:?} reply_message={:?} reply_reaction={:?}",
            payload.message_server_id,
            payload.reaction,
            response.success,
            response.error,
            response.message_id,
            response.reaction
        );
        Ok(RiseStoreWriteBatch::empty())
    }

    fn terminal_reconciliation(
        _payload: &Self::Payload,
        _code: &str,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(RiseStoreWriteBatch::empty())
    }
}

pub enum RiseRemoveReactionDurableEndpoint {}

impl RiseRemoveReactionDurableEndpoint {
    pub const KIND: &str = "conversation.message.reaction.remove.v1";
}

impl RiseSyncDurableEndpoint for RiseRemoveReactionDurableEndpoint {
    type Payload = RiseReactionDurablePayload;
    type Method = RiseRemoveReactionWireMethod;

    fn kind() -> &'static str {
        Self::KIND
    }

    fn make_request(payload: &Self::Payload, _context: &RiseSyncDurableContext) -> anyhow::Result<RiseJSONValue> {
        Ok(RiseJSONValue::encode(&RemoveReaction {
            message_id: payload.message_server_id.clone(),
            reaction: payload.reaction.clone(),
        })?)
    }

    fn reconcile(
        _payload: &Self::Payload,
        _response: &ProtoRemoveReactionResponse,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(RiseStoreWriteBatch::empty())
    }

    fn terminal_reconciliation(
        _payload: &Self::Payload,
        _code: &str,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(RiseStoreWriteBatch::empty())
    }
}
