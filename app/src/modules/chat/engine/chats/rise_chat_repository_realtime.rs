use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::time::Duration;

use rise_canon::{ChatLeaveReason, ChatMessageContentType, ChatMessageRequestState, RiseEvent, rise_key, rise_request, rise_view};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RisePositionKey, RisePositionKeyOrder,
};
use rise_engine::rise_store::rise_store_models::RiseStoreWriteBatch;
use rise_engine::rise_sync::RiseSyncPushHandler;
use rise_engine::rise_wire::rise_wire_contracts::RiseWireTimestamp;
use rise_engine::{RiseJSONValue, RiseWirePushEnvelope};
use rise_events::name;
use rise_i18n::date_format::iso8601_date_formatter::ISO8601DateFormatter;

use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::modules::chat::engine::chat_engine_hooks::{ChatEngineRemovedChatName, chat_engine_hooks};
use crate::modules::chat::engine::chats::rise_chat_repository::{RiseChatDisplacedPreview, RiseChatRepository};
use crate::modules::chat::engine::chats::rise_chat_repository_models::{
    RiseChatAggregateDelta, RiseChatMessageBatchHook, RiseChatRealtimeMessageContext, RiseChatRealtimeMessageKind,
    RiseChatRepositoryError, RiseChatResourceInvalidation, RiseChatResourceInvalidationHook, RiseChatStoredEntity,
    RiseChatTombstone, RiseChatWireCodec, RiseConversationTopicPresence, RiseStoreWriteBatchConversationExt,
};
use crate::modules::chat::engine::chats::rise_normalized_key::RiseNormalizedKey;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseChatEntityRecord, RiseConversationEntitySource,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseChatListKind;
use crate::modules::chat::engine::core::rise_conversation_rpc::RiseConversationEndpoints;
use crate::modules::chat::engine::core::rise_conversation_store_layout::RiseConversationStoreLayout;
use crate::modules::chat::engine::messages::rise_message_contracts::RiseMessagePreviewChange;
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    RiseStoredChat, RiseStoredChatDetail, RiseStoredChatPreview,
};
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageCallTerminalState, MessageSystemKind,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;

const REQUEST_PREVIEW_RETRY_LIMIT: u32 = 3;
const REQUEST_PREVIEW_RETRY_STEP: Duration = Duration::from_millis(1500);
const SERVER_ID_KEY: &str = "serverId";
const CLIENT_ID_KEY: &str = "clientId";
const TEMPORARY_ID_KEY: &str = "temporaryId";

struct RiseChatRepositoryIncomingResolution {
    local_id: String,
    wire: RiseStoredChat,
    kind: RiseChatListKind,
    aliases: Vec<RiseEntityAlias>,
    duplicate_local_ids: Vec<String>,
    previous: Option<RiseChatStoredEntity>,
}

struct RiseChatRepositoryRequestMessageState {
    unread: i64,
    advances_preview: bool,
    needs_authoritative_count: bool,
}

struct RiseChatUnreadFlagRollbackGuard<'a> {
    repository: &'a RiseChatRepository,
    local_id: &'a str,
    armed: bool,
}

impl Drop for RiseChatUnreadFlagRollbackGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.repository.drop_projection(self.local_id);
        self.repository.state.with(|state| state.projected_aggregates.clear());
    }
}

impl RiseChatRepository {
    /// handlers only build batches, sync commits chat and message changes in one sqlite transaction
    /// so observers never see a preview without its timeline change
    pub fn push_handlers(
        &self,
        message_batch_hook: Option<RiseChatMessageBatchHook>,
        resource_invalidation_hook: Option<RiseChatResourceInvalidationHook>,
    ) -> Vec<RiseSyncPushHandler> {
        let mut handlers: Vec<RiseSyncPushHandler> = Self::realtime_event_types()
            .into_iter()
            .map(|event_type| {
                self.chat_event_handler(event_type, message_batch_hook.clone(), resource_invalidation_hook.clone())
            })
            .collect();
        if let Some(hook) = resource_invalidation_hook {
            handlers.extend(self.resource_event_handlers(hook));
        }
        handlers
    }

    pub fn chat_push_handlers(
        &self,
        resource_invalidation_hook: Option<RiseChatResourceInvalidationHook>,
    ) -> Vec<RiseSyncPushHandler> {
        let mut handlers: Vec<RiseSyncPushHandler> = Self::CHAT_EVENT_TYPES
            .into_iter()
            .map(|event_type| self.chat_event_handler(event_type, None, resource_invalidation_hook.clone()))
            .collect();
        if let Some(hook) = resource_invalidation_hook {
            handlers.extend(self.resource_event_handlers(hook));
        }
        handlers
    }

    fn chat_event_handler(
        &self,
        event_type: &'static str,
        message_batch_hook: Option<RiseChatMessageBatchHook>,
        resource_invalidation_hook: Option<RiseChatResourceInvalidationHook>,
    ) -> RiseSyncPushHandler {
        let this = self.this.clone();
        RiseSyncPushHandler::new(event_type, move |envelope: RiseWirePushEnvelope| {
            let this = this.clone();
            let message_batch_hook = message_batch_hook.clone();
            let resource_invalidation_hook = resource_invalidation_hook.clone();
            async move {
                let Some(this) = this.upgrade() else {
                    return Ok(None);
                };
                let batch = this.make_realtime_batch(&envelope, message_batch_hook).await?;
                if let Some(hook) = resource_invalidation_hook {
                    let invalidations = this.chat_event_invalidations(&envelope);
                    if !invalidations.is_empty() {
                        hook(invalidations).await;
                    }
                }
                Ok::<_, anyhow::Error>(batch)
            }
        })
    }

    fn resource_event_handlers(&self, hook: RiseChatResourceInvalidationHook) -> Vec<RiseSyncPushHandler> {
        Self::RESOURCE_EVENT_TYPES
            .into_iter()
            .map(|event_type| {
                let account_id = self.account_id.clone();
                let hook = hook.clone();
                RiseSyncPushHandler::new(event_type, move |envelope: RiseWirePushEnvelope| {
                    let account_id = account_id.clone();
                    let hook = hook.clone();
                    async move {
                        let invalidations = Self::resource_invalidations(&envelope, &account_id);
                        if invalidations.is_empty() {
                            return Ok(None);
                        }
                        hook(invalidations).await;
                        Ok::<Option<RiseStoreWriteBatch>, anyhow::Error>(None)
                    }
                })
            })
            .collect()
    }

    /// batch hook for domains that add their own reducers to the chat and message transaction
    pub async fn make_realtime_batch(
        &self,
        envelope: &RiseWirePushEnvelope,
        message_batch_hook: Option<RiseChatMessageBatchHook>,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let event = envelope.r#type.as_str();
        if event == RiseChatRealtimeMessageKind::Created.raw_value() {
            return self.reduce_message(envelope, RiseChatRealtimeMessageKind::Created, message_batch_hook).await;
        }
        if event == RiseChatRealtimeMessageKind::Edited.raw_value() {
            return self.reduce_message(envelope, RiseChatRealtimeMessageKind::Edited, message_batch_hook).await;
        }
        if event == RiseChatRealtimeMessageKind::Deleted.raw_value() {
            return self.reduce_message(envelope, RiseChatRealtimeMessageKind::Deleted, message_batch_hook).await;
        }
        match event {
            name::CHAT_CREATED => self.reduce_chat_created(envelope).await,
            name::CHAT_UPDATED => self.reduce_chat_updated(envelope).await,
            name::CHAT_DELETED => self.reduce_removed_chat(envelope, RiseChatTombstone::ConfirmedDeleted).await,
            name::CHAT_ARCHIVED => self.reduce_archive(envelope, true).await,
            name::CHAT_UNARCHIVED => self.reduce_archive(envelope, false).await,
            name::MESSAGE_REQUEST_CREATED => self.reduce_message_request(envelope, false).await,
            name::MESSAGE_REQUEST_ACCEPTED => self.reduce_message_request(envelope, true).await,
            name::MEMBER_JOINED => self.reduce_member(envelope, true).await,
            name::MEMBER_LEFT => self.reduce_member(envelope, false).await,
            name::CHAT_USER_ROLE_CHANGED => self.reduce_own_role(envelope).await,
            _ => Ok(None),
        }
    }

    async fn reduce_message(
        &self,
        envelope: &RiseWirePushEnvelope,
        kind: RiseChatRealtimeMessageKind,
        message_batch_hook: Option<RiseChatMessageBatchHook>,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let root = Self::payload_object(&envelope.data)?;
        let message_object =
            Self::nested_object(&root, &[rise_key::chat::MESSAGE]).unwrap_or_else(|| root.clone());
        let chat_id = Self::string(&root, &[RiseNormalizedKey::chat_id()])
            .or_else(|| Self::string(&message_object, &[RiseNormalizedKey::chat_id()]))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RiseChatRepositoryError::InvalidPush(envelope.r#type.clone()))?;

        let topic = Self::topic_presence(&root, &message_object);
        let message = match kind {
            RiseChatRealtimeMessageKind::Created | RiseChatRealtimeMessageKind::Edited => {
                let mut normalized = message_object.clone();
                if !normalized.contains_key(RiseNormalizedKey::chat_id()) {
                    normalized.insert(RiseNormalizedKey::chat_id().to_owned(), RiseJSONValue::String(chat_id.clone()));
                }
                if kind == RiseChatRealtimeMessageKind::Created
                    && !normalized.contains_key(RiseNormalizedKey::created_at())
                    && !normalized.contains_key(rise_key::chat::TIMESTAMP)
                    && let Some(timestamp) = Self::envelope_timestamp(envelope)
                {
                    normalized.insert(RiseNormalizedKey::created_at().to_owned(), RiseJSONValue::Number(timestamp));
                }
                Some(Self::decode_message(normalized)?)
            }
            RiseChatRealtimeMessageKind::Deleted => None,
        };
        let message_ids = Self::message_ids(&root, &message_object, message.as_ref());
        let version = self.next_version();
        let context = RiseChatRealtimeMessageContext {
            kind,
            envelope: envelope.clone(),
            chat_id: chat_id.clone(),
            message_ids: message_ids.clone(),
            message: message.clone(),
            topic: topic.clone(),
            version,
        };

        let preview_change = match kind {
            RiseChatRealtimeMessageKind::Created => {
                let message = message.ok_or_else(|| RiseChatRepositoryError::InvalidPush(envelope.r#type.clone()))?;
                RiseMessagePreviewChange::Created {
                    scope: RiseMessageScope::new(chat_id.clone(), topic.value().map(str::to_owned), None),
                    message,
                    was_already_stored: false,
                }
            }
            RiseChatRealtimeMessageKind::Edited => {
                let message = message.ok_or_else(|| RiseChatRepositoryError::InvalidPush(envelope.r#type.clone()))?;
                RiseMessagePreviewChange::Edited {
                    scope: RiseMessageScope::new(chat_id.clone(), topic.value().map(str::to_owned), None),
                    message,
                }
            }
            RiseChatRealtimeMessageKind::Deleted => {
                RiseMessagePreviewChange::Deleted { chat_id: chat_id.clone(), server_ids: message_ids, fallback: None }
            }
        };
        let chat_batch = self.make_message_preview_batch(preview_change, version).await?;

        let message_batch = match message_batch_hook {
            Some(hook) => hook(context).await?,
            None => RiseStoreWriteBatch::empty(),
        };
        let combined = RiseStoreWriteBatch::conversation_merging(vec![chat_batch, message_batch]);
        if combined.mutations.is_empty() && combined.invalidated_views.is_empty() {
            return Ok(None);
        }
        Ok(Some(combined))
    }

    /// batch only chat projection used by the message repository
    /// never writes alone so timeline, preview and archive commit together
    pub async fn make_message_preview_batch(
        &self,
        change: RiseMessagePreviewChange,
        version: i64,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let chat_id = match &change {
            RiseMessagePreviewChange::Created { scope, .. } | RiseMessagePreviewChange::Edited { scope, .. } => {
                scope.chat_id.clone()
            }
            RiseMessagePreviewChange::Deleted { chat_id, .. }
            | RiseMessagePreviewChange::HistoryCleared { chat_id }
            | RiseMessagePreviewChange::Read { chat_id, .. }
            | RiseMessagePreviewChange::Receipt { chat_id, .. }
            | RiseMessagePreviewChange::SendFailed { chat_id, .. } => chat_id.clone(),
        };
        let Some(stored) = self.stored_chat_id(&chat_id, false).await? else {
            if let RiseMessagePreviewChange::Created { scope, message, .. } = &change
                && RiseConversationStoreLayout::canonical_message_server_id(message).is_some()
            {
                self.stash_orphan_preview(&chat_id, scope, message)?;
                self.notify_unknown_chat(&chat_id).await;
            }
            return Ok(RiseStoreWriteBatch::empty());
        };
        if let Some(tombstone) = stored.record.tombstone {
            if let RiseMessagePreviewChange::Created { message, .. } = &change
                && tombstone.is_confirmed()
                && RiseConversationStoreLayout::canonical_message_server_id(message).is_some()
            {
                self.notify_unknown_chat(&chat_id).await;
            }
            return Ok(RiseStoreWriteBatch::empty());
        }
        if stored.record.list_kind.is_none()
            && let RiseMessagePreviewChange::Created { message, .. } = &change
            && RiseConversationStoreLayout::canonical_message_server_id(message).is_some()
        {
            self.notify_unknown_chat(&chat_id).await;
        }
        let next_after_entity =
            if stored.entity.version == i64::MAX { i64::MAX } else { stored.entity.version + 1 };
        let version = version.max(next_after_entity).max(Self::now_milliseconds());
        self.state.with(|state| state.version_clock = state.version_clock.max(version));

        let old_wire = &stored.record.wire;
        let updated_wire;
        let aggregate_kind = stored.record.list_kind;
        let moves_aggregate = aggregate_kind == Some(RiseChatListKind::Archive)
            || (aggregate_kind == Some(RiseChatListKind::Inbox) && old_wire.counts_toward_unread_badge());
        let mut aggregate_delta = RiseChatAggregateDelta::zero();
        match &change {
            RiseMessagePreviewChange::Created { scope, message, was_already_stored } => {
                let topic = match message.topic_id.as_ref().or(scope.topic_id.as_ref()) {
                    Some(topic_id) => RiseConversationTopicPresence::Value(topic_id.clone()),
                    None => RiseConversationTopicPresence::Absent,
                };
                let sender_id = Self::sender_id(message);
                let incoming = !sender_id.is_empty() && sender_id != self.account_id;
                let timestamp = RiseChatWireCodec::timestamp(message);
                let same_preview_identity = Self::last_message(
                    old_wire.last_message.as_ref(),
                    &Self::message_identity_values(message),
                );
                let unread_delta: i64 =
                    if incoming && !*was_already_stored && !same_preview_identity && Self::charges_unread(message) {
                        1
                    } else {
                        0
                    };
                let adds_photo = unread_delta > 0 && Self::is_photo_message(message);
                let mut updates: BTreeMap<String, RiseJSONValue> = BTreeMap::new();
                if Self::should_advance_preview(old_wire.last_message.as_ref(), message) {
                    let preview = RiseChatWireCodec::last_message(message, &topic);
                    self.note_displaced_preview(&chat_id, old_wire, &preview, message, incoming, same_preview_identity);
                    updates.insert(
                        RiseNormalizedKey::last_message().to_owned(),
                        RiseChatWireCodec::last_message_value(Some(&preview))?,
                    );
                    updates.insert(RiseNormalizedKey::updated_at().to_owned(), RiseJSONValue::Number(timestamp));
                    updates.insert(RiseNormalizedKey::last_activity().to_owned(), RiseJSONValue::Number(timestamp));
                    if let Some(live) = Self::live_voice_change(message) {
                        updates.insert(RiseNormalizedKey::has_live_voice().to_owned(), RiseJSONValue::Bool(live));
                    }
                }
                if unread_delta > 0 {
                    updates.insert(
                        RiseNormalizedKey::unread_count().to_owned(),
                        RiseJSONValue::Integer((old_wire.resolved_unread_count() + unread_delta).max(0)),
                    );
                    if adds_photo {
                        updates.insert(
                            RiseNormalizedKey::unread_media_count().to_owned(),
                            RiseJSONValue::Integer(old_wire.resolved_unread_media_count() + 1),
                        );
                    }
                }
                if updates.is_empty() {
                    return Ok(RiseStoreWriteBatch::empty());
                }
                updated_wire = RiseChatWireCodec::patch(old_wire, updates)?;
                if moves_aggregate && unread_delta > 0 {
                    aggregate_delta = RiseChatAggregateDelta {
                        unread: unread_delta,
                        unread_chats: if old_wire.resolved_unread_count() == 0 { 1 } else { 0 },
                        unread_media: if adds_photo { 1 } else { 0 },
                        ..RiseChatAggregateDelta::default()
                    };
                }
            }

            RiseMessagePreviewChange::Edited { scope, message } => {
                let ids: Vec<String> = [&message.server_id, &message.id, &message.temp_id]
                    .into_iter()
                    .filter_map(|value| value.clone())
                    .collect();
                if !Self::last_message(old_wire.last_message.as_ref(), &ids) {
                    return Ok(RiseStoreWriteBatch::empty());
                }
                let topic = message
                    .topic_id
                    .as_ref()
                    .or(scope.topic_id.as_ref())
                    .map_or(RiseConversationTopicPresence::Absent, |topic_id| {
                        RiseConversationTopicPresence::Value(topic_id.clone())
                    });
                let RiseJSONValue::Object(incoming) = RiseJSONValue::encode(message)? else {
                    return Err(RiseChatRepositoryError::CorruptedChat(chat_id).into());
                };
                let edited = Self::edited_last_message(old_wire.last_message.as_ref(), &incoming, message, &topic)?;
                updated_wire = RiseChatWireCodec::patch(
                    old_wire,
                    BTreeMap::from([(
                        RiseNormalizedKey::last_message().to_owned(),
                        RiseChatWireCodec::last_message_value(Some(&edited))?,
                    )]),
                )?;
            }

            RiseMessagePreviewChange::Deleted { server_ids, fallback, .. } => {
                RiseChatRetractedPreviews::note(&chat_id, server_ids);
                let displaced = self.take_displaced_preview(&chat_id, old_wire.last_message.as_ref(), server_ids);
                tracing::warn!(
                    target: "riseonly::X1777",
                    "preview delete chat={chat_id} preview={:?} ids={server_ids:?} matched={} displaced={} fallback={:?} topics={:?}",
                    old_wire.last_message.as_ref().and_then(|value| value.id.as_deref()),
                    Self::last_message(old_wire.last_message.as_ref(), server_ids),
                    displaced.is_some(),
                    fallback.as_ref().and_then(|value| value.server_id.as_deref().or(value.id.as_deref())),
                    old_wire.topics_enabled,
                );
                if !Self::last_message(old_wire.last_message.as_ref(), server_ids) {
                    return Ok(RiseStoreWriteBatch::empty());
                }
                if let Some(displaced) = displaced {
                    let number = |value: Option<f64>| value.map_or(RiseJSONValue::Null, RiseJSONValue::Number);
                    updated_wire = RiseChatWireCodec::patch(
                        old_wire,
                        BTreeMap::from([
                            (
                                RiseNormalizedKey::last_message().to_owned(),
                                RiseChatWireCodec::last_message_value(displaced.last_message.as_ref())?,
                            ),
                            (RiseNormalizedKey::updated_at().to_owned(), number(displaced.updated_at)),
                            (RiseNormalizedKey::last_activity().to_owned(), number(displaced.last_activity)),
                        ]),
                    )?;
                } else if let Some(fallback) = fallback {
                    let timestamp = RiseChatWireCodec::timestamp(fallback);
                    let preview = RiseChatWireCodec::last_message(fallback, &RiseConversationTopicPresence::Absent);
                    updated_wire = RiseChatWireCodec::patch(
                        old_wire,
                        BTreeMap::from([
                            (
                                RiseNormalizedKey::last_message().to_owned(),
                                RiseChatWireCodec::last_message_value(Some(&preview))?,
                            ),
                            (RiseNormalizedKey::updated_at().to_owned(), RiseJSONValue::Number(timestamp)),
                            (RiseNormalizedKey::last_activity().to_owned(), RiseJSONValue::Number(timestamp)),
                        ]),
                    )?;
                } else {
                    updated_wire = RiseChatWireCodec::patch(
                        old_wire,
                        BTreeMap::from([(RiseNormalizedKey::last_message().to_owned(), RiseJSONValue::Null)]),
                    )?;
                    if let Some(kind) = stored.record.list_kind {
                        self.schedule_preview_refetch(kind);
                    }
                }
            }

            RiseMessagePreviewChange::HistoryCleared { .. } => {
                self.state.with(|state| state.displaced_previews.remove(&chat_id));
                let unread = old_wire.resolved_unread_count();
                updated_wire = RiseChatWireCodec::patch(
                    old_wire,
                    BTreeMap::from([
                        (RiseNormalizedKey::last_message().to_owned(), RiseJSONValue::Null),
                        (RiseNormalizedKey::unread_count().to_owned(), RiseJSONValue::Integer(0)),
                        (RiseNormalizedKey::unread_media_count().to_owned(), RiseJSONValue::Integer(0)),
                    ]),
                )?;
                if moves_aggregate && unread > 0 {
                    aggregate_delta = RiseChatAggregateDelta::row(old_wire, -1, false);
                }
            }

            RiseMessagePreviewChange::Read { topic_id, authoritative_unread, clears_marks, .. } => {
                let previous_unread = old_wire.resolved_unread_count();
                let remaining_unread = match (authoritative_unread, topic_id) {
                    (Some(authoritative), _) => Some((*authoritative).max(0)),
                    (None, None) => Some(0),
                    (None, Some(_)) => None,
                };
                let last_message = old_wire.last_message.as_ref();
                let advances_preview = (topic_id.is_none()
                    || last_message.and_then(|value| value.topic_id.as_deref()) == topic_id.as_deref())
                    && last_message.and_then(|value| value.sender_id.as_deref()) != Some(self.account_id.as_str());
                let drops_marks = *clears_marks
                    && (old_wire.resolved_unread_mentions_count() > 0
                        || old_wire.resolved_unread_replies_count() > 0
                        || old_wire.resolved_unread_reactions_count() > 0);
                if !advances_preview && remaining_unread == Some(previous_unread) && !drops_marks {
                    return Ok(RiseStoreWriteBatch::empty());
                }
                let mut updates: BTreeMap<String, RiseJSONValue> = BTreeMap::new();
                if drops_marks {
                    updates.insert(RiseNormalizedKey::unread_mentions_count().to_owned(), RiseJSONValue::Integer(0));
                    updates.insert(RiseNormalizedKey::unread_replies_count().to_owned(), RiseJSONValue::Integer(0));
                    updates.insert(RiseNormalizedKey::unread_reactions_count().to_owned(), RiseJSONValue::Integer(0));
                }
                let previous_media = old_wire.resolved_unread_media_count();
                let mut remaining_media = previous_media;
                if let Some(remaining) = remaining_unread
                    && remaining != previous_unread
                {
                    updates.insert(RiseNormalizedKey::unread_count().to_owned(), RiseJSONValue::Integer(remaining));
                    remaining_media = previous_media.min(remaining);
                    if remaining_media != previous_media {
                        updates.insert(
                            RiseNormalizedKey::unread_media_count().to_owned(),
                            RiseJSONValue::Integer(remaining_media),
                        );
                    }
                }
                let base = if advances_preview { Self::mark_last_message_read(old_wire)? } else { old_wire.clone() };
                updated_wire = RiseChatWireCodec::patch(&base, updates)?;
                if moves_aggregate
                    && let Some(remaining) = remaining_unread
                    && remaining != previous_unread
                {
                    aggregate_delta = RiseChatAggregateDelta {
                        unread: remaining - previous_unread,
                        unread_chats: Self::unread_chats_delta(previous_unread, remaining),
                        unread_media: remaining_media - previous_media,
                        ..RiseChatAggregateDelta::default()
                    };
                }
            }

            RiseMessagePreviewChange::Receipt { server_ids, .. } => {
                if !Self::last_message(old_wire.last_message.as_ref(), server_ids) {
                    return Ok(RiseStoreWriteBatch::empty());
                }
                updated_wire = Self::mark_last_message_read(old_wire)?;
            }

            RiseMessagePreviewChange::SendFailed { local_id, .. } => {
                let Some(mut last_message) = old_wire.last_message.clone() else {
                    return Ok(RiseStoreWriteBatch::empty());
                };
                if last_message.id.as_deref() != Some(local_id.as_str()) || last_message.send_failed == Some(true) {
                    return Ok(RiseStoreWriteBatch::empty());
                }
                last_message.send_failed = Some(true);
                updated_wire = RiseChatWireCodec::patch(
                    old_wire,
                    BTreeMap::from([(
                        RiseNormalizedKey::last_message().to_owned(),
                        RiseChatWireCodec::last_message_value(Some(&last_message))?,
                    )]),
                )?;
            }
        }

        if RiseChatWireCodec::object(&updated_wire)? == RiseChatWireCodec::object(old_wire)? {
            return Ok(RiseStoreWriteBatch::empty());
        }
        let aliases = RiseConversationStoreLayout::chat_aliases(&updated_wire);
        let entity_batch = self.make_upsert_batch(
            RiseChatEntityRecord {
                wire: updated_wire,
                source: RiseConversationEntitySource::Realtime,
                locally_created: stored.record.locally_created,
                list_kind: stored.record.list_kind,
                tombstone: None,
            },
            &stored.entity.local_id,
            aliases,
            stored.record.list_kind,
            version,
            false,
            Vec::new(),
        )?;
        let Some(aggregate_kind) = aggregate_kind else {
            return Ok(entity_batch);
        };
        if aggregate_delta.is_zero() {
            return Ok(entity_batch);
        }
        self.merging_aggregates(entity_batch, version, vec![(aggregate_kind, aggregate_delta)]).await
    }

    pub async fn replace_hidden_topic_preview(
        &self,
        chat_id: &str,
        topic_ids: &HashSet<String>,
        fallback: Option<&MessageWireDTO>,
    ) -> anyhow::Result<()> {
        let normalized = |raw: Option<&str>| -> Option<String> {
            let value = trimming_whitespaces_and_newlines(raw?);
            (!value.is_empty()).then(|| value.to_lowercase())
        };
        let hidden: HashSet<String> = topic_ids.iter().filter_map(|topic_id| normalized(Some(topic_id))).collect();
        let Some(stored) = self.stored_chat_id(chat_id, false).await? else {
            return Ok(());
        };
        if stored.record.tombstone.is_some() {
            return Ok(());
        }
        let old_wire = &stored.record.wire;
        let shows_hidden = old_wire
            .last_message
            .as_ref()
            .and_then(|preview| normalized(preview.topic_id.as_deref()))
            .is_some_and(|topic_id| hidden.contains(&topic_id));
        if !shows_hidden {
            return Ok(());
        }
        self.state.with(|state| state.displaced_previews.remove(chat_id));
        let mut updates: BTreeMap<String, RiseJSONValue> = BTreeMap::new();
        match fallback {
            Some(message) => {
                let topic = match message.topic_id.as_ref() {
                    Some(topic_id) => RiseConversationTopicPresence::Value(topic_id.clone()),
                    None => RiseConversationTopicPresence::Absent,
                };
                let preview = RiseChatWireCodec::last_message(message, &topic);
                let timestamp = RiseChatWireCodec::timestamp(message);
                updates.insert(
                    RiseNormalizedKey::last_message().to_owned(),
                    RiseChatWireCodec::last_message_value(Some(&preview))?,
                );
                updates.insert(RiseNormalizedKey::updated_at().to_owned(), RiseJSONValue::Number(timestamp));
            }
            None => {
                updates.insert(RiseNormalizedKey::last_message().to_owned(), RiseJSONValue::Null);
            }
        }
        let updated_wire = RiseChatWireCodec::patch(old_wire, updates)?;
        let next_after_entity =
            if stored.entity.version == i64::MAX { i64::MAX } else { stored.entity.version + 1 };
        let version = self.next_version().max(next_after_entity);
        self.state.with(|state| state.version_clock = state.version_clock.max(version));
        let aliases = RiseConversationStoreLayout::chat_aliases(&updated_wire);
        let batch = self.make_upsert_batch(
            RiseChatEntityRecord {
                wire: updated_wire,
                source: RiseConversationEntitySource::Realtime,
                locally_created: stored.record.locally_created,
                list_kind: stored.record.list_kind,
                tombstone: None,
            },
            &stored.entity.local_id,
            aliases,
            stored.record.list_kind,
            version,
            false,
            Vec::new(),
        )?;
        self.engine.write(batch).await?;
        Ok(())
    }

    async fn notify_unknown_chat(&self, chat_id: &str) {
        let handler = self.state.with(|state| state.unknown_chat_handler.clone());
        if let Some(handler) = handler {
            handler(chat_id.to_owned()).await;
        }
    }

    /// aggregates a row leaves behind when it stops being listed
    /// archive loses it whole, the inbox badge only if it counted
    fn removal_deltas(record: &RiseChatEntityRecord) -> Vec<(RiseChatListKind, RiseChatAggregateDelta)> {
        match record.list_kind {
            Some(RiseChatListKind::Archive) => {
                vec![(RiseChatListKind::Archive, RiseChatAggregateDelta::row(&record.wire, -1, true))]
            }
            Some(RiseChatListKind::Inbox) if record.wire.counts_toward_unread_badge() => {
                vec![(RiseChatListKind::Inbox, RiseChatAggregateDelta::row(&record.wire, -1, false))]
            }
            _ => Vec::new(),
        }
    }

    async fn reduce_chat_created(
        &self,
        envelope: &RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let root = Self::payload_object(&envelope.data)?;
        let mut incoming = Self::nested_object(&root, &[rise_key::chat::CHAT, RiseNormalizedKey::chat_info()])
            .unwrap_or_else(|| root.clone());
        if let Ok(data) = envelope.data.data(false)
            && let Ok(RiseEvent::ChatCreated(chat)) = RiseEvent::decode(&envelope.r#type, &data)
            && !chat.id.is_empty()
            && let Ok(typed) = RiseChatWireCodec::object(&RiseStoredChat::from_canon_proto(&chat))
        {
            incoming.extend(typed);
        }
        let chat_id = Self::string(&incoming, &[rise_key::chat::ID, RiseNormalizedKey::chat_id()])
            .or_else(|| Self::string(&root, &[RiseNormalizedKey::chat_id()]))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RiseChatRepositoryError::InvalidPush(envelope.r#type.clone()))?;
        let resolution = self.resolve_incoming_chat(&chat_id, &incoming, RiseChatListKind::Inbox, true).await?;
        let mut wire = resolution.wire;
        if wire.last_message.is_none()
            && let Some(previous) = &resolution.previous
            && previous.record.tombstone.is_none()
            && previous.record.wire.last_message.is_some()
        {
            wire = Self::keeping_stored_preview(&wire, &previous.record.wire)?;
        }
        if let Some(previous) = &resolution.previous
            && previous.record.tombstone.is_none()
        {
            wire = Self::keeping_stored_unread_count(wire, &previous.record.wire)?;
        }
        let wire = self.applying_orphan_preview(wire, &chat_id)?;
        let version = self.next_version();
        self.clear_pending_mutation(&resolution.local_id);
        let source = if resolution.duplicate_local_ids.is_empty() {
            RiseConversationEntitySource::Realtime
        } else {
            RiseConversationEntitySource::Acknowledgement
        };
        let batch = self.make_upsert_batch(
            RiseChatEntityRecord {
                wire,
                source,
                locally_created: false,
                list_kind: Some(resolution.kind),
                tombstone: None,
            },
            &resolution.local_id,
            resolution.aliases,
            Some(resolution.kind),
            version,
            true,
            resolution.duplicate_local_ids,
        )?;
        Ok(Some(batch))
    }

    async fn reduce_chat_updated(
        &self,
        envelope: &RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let root = Self::payload_object(&envelope.data)?;
        let incoming = Self::nested_object(&root, &[rise_key::chat::CHAT, RiseNormalizedKey::chat_info()])
            .unwrap_or_else(|| root.clone());
        let chat_id_keys = [RiseNormalizedKey::chat_id(), rise_key::chat::ID];
        let Some(chat_id) = Self::string(&root, &chat_id_keys).or_else(|| Self::string(&incoming, &chat_id_keys)) else {
            return Ok(None);
        };
        let Some(stored) = self.stored_chat_id(&chat_id, false).await? else {
            return Ok(None);
        };
        if stored.record.tombstone.is_some() {
            return Ok(None);
        }
        let excluded = Self::key_set(&[
            RiseNormalizedKey::chat_id(),
            rise_key::chat::ID,
            SERVER_ID_KEY,
            RiseNormalizedKey::temp_id(),
            CLIENT_ID_KEY,
            rise_key::events::PAYLOAD,
            rise_key::chat::CHAT,
            RiseNormalizedKey::chat_info(),
            RiseNormalizedKey::settings_changed(),
        ]);
        let mut patch: BTreeMap<String, RiseJSONValue> =
            incoming.into_iter().filter(|(_, value)| *value != RiseJSONValue::Null).collect();
        if let Ok(data) = envelope.data.data(false)
            && let Ok(RiseEvent::ChatUpdated(update)) = RiseEvent::decode(&envelope.r#type, &data)
        {
            let mut put = |key: &str, value: Option<RiseJSONValue>| {
                if let Some(value) = value {
                    patch.insert(key.to_owned(), value);
                }
            };
            put(rise_key::chat::TITLE, update.title.map(RiseJSONValue::String));
            put(rise_key::chat::DESCRIPTION, update.description.map(RiseJSONValue::String));
            put(RiseNormalizedKey::logo_url(), update.logo_url.map(RiseJSONValue::String));
            put(RiseNormalizedKey::banner_url(), update.banner_url.map(RiseJSONValue::String));
            put(rise_key::chat::USERNAME, update.username.map(RiseJSONValue::String));
            put(rise_key::chat::TAG, update.tag.map(RiseJSONValue::String));
            put(rise_key::chat::ROLE, update.role.map(RiseJSONValue::String));
            put(RiseNormalizedKey::is_public(), update.is_public.map(RiseJSONValue::Bool));
            put(RiseNormalizedKey::topics_enabled(), update.topics_enabled.map(RiseJSONValue::Bool));
            put(RiseNormalizedKey::forward_in_chat_enabled(), update.forward_in_chat_enabled.map(RiseJSONValue::Bool));
            put(
                RiseNormalizedKey::auto_delete_messages_after(),
                update.auto_delete_messages_after.map(RiseJSONValue::Integer),
            );
            put(RiseNormalizedKey::can_accept_join_requests(), update.can_accept_join_requests.map(RiseJSONValue::Bool));
            put(RiseNormalizedKey::can_pin_messages(), update.can_pin_messages.map(RiseJSONValue::Bool));
            put(RiseNormalizedKey::can_manage_chat(), update.can_manage_chat.map(RiseJSONValue::Bool));
        }
        let wire = RiseChatWireCodec::overlay(&stored.record.wire, &patch, &excluded)?;
        let version = self.next_version();
        if Self::contains_any(
            &patch,
            &[
                RiseNormalizedKey::is_pinned(),
                RiseNormalizedKey::pin_position(),
                RiseNormalizedKey::is_muted(),
                RiseNormalizedKey::mute_until(),
                RiseNormalizedKey::mute_scopes(),
                RiseNormalizedKey::notifications_enabled(),
            ],
        ) {
            self.clear_pending_mutation(&stored.entity.local_id);
        }
        let aliases = RiseConversationStoreLayout::chat_aliases(&wire);
        let batch = self.make_upsert_batch(
            RiseChatEntityRecord {
                wire: wire.clone(),
                source: RiseConversationEntitySource::Realtime,
                locally_created: stored.record.locally_created,
                list_kind: stored.record.list_kind,
                tombstone: None,
            },
            &stored.entity.local_id,
            aliases,
            stored.record.list_kind,
            version,
            false,
            Vec::new(),
        )?;
        if stored.record.list_kind != Some(RiseChatListKind::Inbox)
            || wire.resolved_unread_count() <= 0
            || stored.record.wire.counts_toward_unread_badge() == wire.counts_toward_unread_badge()
        {
            return Ok(Some(batch));
        }
        let sign = if wire.counts_toward_unread_badge() { 1 } else { -1 };
        let merged = self
            .merging_aggregates(
                batch,
                version,
                vec![(RiseChatListKind::Inbox, RiseChatAggregateDelta::row(&wire, sign, false))],
            )
            .await?;
        Ok(Some(merged))
    }

    async fn reduce_removed_chat(
        &self,
        envelope: &RiseWirePushEnvelope,
        tombstone: RiseChatTombstone,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let root = Self::payload_object(&envelope.data)?;
        let Some(chat_id) = Self::string(&root, &[RiseNormalizedKey::chat_id(), rise_key::chat::ID]) else {
            return Ok(None);
        };
        let Some(stored) = self.stored_chat_id(&chat_id, false).await? else {
            return Ok(None);
        };
        let version = self.next_version();
        self.clear_pending_mutation(&stored.entity.local_id);
        let batch = self.make_tombstone_batch(&stored, tombstone, version)?;
        let merged = self.merging_aggregates(batch, version, Self::removal_deltas(&stored.record)).await?;
        Ok(Some(merged))
    }

    async fn reduce_archive(
        &self,
        envelope: &RiseWirePushEnvelope,
        fallback_archived: bool,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let root = Self::payload_object(&envelope.data)?;
        let Some(chat_id) = Self::string(&root, &[RiseNormalizedKey::chat_id(), rise_key::chat::ID]) else {
            return Ok(None);
        };
        let Some(stored) = self.stored_chat_id(&chat_id, false).await? else {
            return Ok(None);
        };
        if stored.record.tombstone.is_some() {
            return Ok(None);
        }
        let archived = Self::bool(&root, &[RiseNormalizedKey::is_archived()]).unwrap_or(fallback_archived);
        let destination = if archived { RiseChatListKind::Archive } else { RiseChatListKind::Inbox };
        let previous_kind = stored.record.list_kind;
        let mut wire = stored.record.wire.clone();
        if archived {
            wire = RiseChatWireCodec::patch(
                &wire,
                BTreeMap::from([
                    (RiseNormalizedKey::is_pinned().to_owned(), RiseJSONValue::Bool(false)),
                    (RiseNormalizedKey::pin_position().to_owned(), RiseJSONValue::Null),
                ]),
            )?;
        }
        let version = self.next_version();
        self.clear_pending_mutation(&stored.entity.local_id);
        let aliases = RiseConversationStoreLayout::chat_aliases(&wire);
        let mut batch = self.make_upsert_batch(
            RiseChatEntityRecord {
                wire: wire.clone(),
                source: RiseConversationEntitySource::Realtime,
                locally_created: stored.record.locally_created,
                list_kind: Some(destination),
                tombstone: None,
            },
            &stored.entity.local_id,
            aliases,
            Some(destination),
            version,
            false,
            Vec::new(),
        )?;
        if previous_kind != Some(destination) {
            let sign = if archived { 1 } else { -1 };
            let mut deltas = vec![(RiseChatListKind::Archive, RiseChatAggregateDelta::row(&wire, sign, true))];
            if wire.counts_toward_unread_badge() {
                deltas.push((RiseChatListKind::Inbox, RiseChatAggregateDelta::row(&wire, -sign, false)));
            }
            batch = self.merging_aggregates(batch, version, deltas).await?;
        }
        Ok(Some(batch))
    }

    async fn reduce_message_request(
        &self,
        envelope: &RiseWirePushEnvelope,
        accepted: bool,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let root = Self::payload_object(&envelope.data)?;
        let incoming = Self::nested_object(&root, &[rise_key::chat::CHAT, RiseNormalizedKey::chat_info()])
            .unwrap_or_else(|| root.clone());
        let Some(chat_id) = Self::string(&incoming, &[rise_key::chat::ID, RiseNormalizedKey::chat_id()])
            .or_else(|| Self::string(&root, &[RiseNormalizedKey::chat_id()]))
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let destination = if accepted { RiseChatListKind::Inbox } else { RiseChatListKind::Requests };
        tracing::warn!(target: "riseonly::X641", "request frame chat={chat_id} accepted={accepted}");
        let resolution = self.resolve_incoming_chat(&chat_id, &incoming, destination, true).await?;
        let mut updates: BTreeMap<String, RiseJSONValue> = BTreeMap::from([
            (RiseNormalizedKey::is_message_request().to_owned(), RiseJSONValue::Bool(!accepted)),
            (
                RiseNormalizedKey::message_request_state().to_owned(),
                if accepted {
                    RiseJSONValue::Null
                } else {
                    RiseJSONValue::String(ChatMessageRequestState::Pending.wire().to_owned())
                },
            ),
        ]);
        let mut base_wire = resolution.wire.clone();
        if accepted && Self::real_last_message(base_wire.last_message.as_ref()).is_none() {
            let previous_wire = resolution
                .previous
                .as_ref()
                .filter(|value| value.record.tombstone.is_none())
                .map(|value| &value.record.wire)
                .filter(|wire| Self::real_last_message(wire.last_message.as_ref()).is_some());
            match previous_wire {
                Some(previous_wire) => base_wire = Self::keeping_stored_preview(&base_wire, previous_wire)?,
                None => self.schedule_preview_refetch(RiseChatListKind::Inbox),
            }
        }
        if !accepted {
            let previous = resolution
                .previous
                .as_ref()
                .filter(|value| value.record.tombstone.is_none())
                .map(|value| value.record.wire.clone());
            let mut state = Self::request_message_state(base_wire.last_message.as_ref(), previous.as_ref());
            if Self::real_last_message(base_wire.last_message.as_ref()).is_none()
                && Self::real_last_message(previous.as_ref().and_then(|value| value.last_message.as_ref())).is_none()
            {
                self.schedule_preview_refetch(RiseChatListKind::Requests);
            }
            if state.needs_authoritative_count {
                let reply: rise_view::chat::ChatById = self
                    .rpc
                    .call(
                        RiseConversationEndpoints::get_chat_by_id(),
                        rise_request::chat::GetChatById { chat_id: chat_id.clone() },
                    )
                    .await?;
                let authoritative = RiseStoredChatDetail::from_canon(&reply)
                    .chat
                    .filter(|value| value.id == chat_id && value.is_message_request == Some(true));
                let Some(authoritative) = authoritative else {
                    tracing::warn!(target: "riseonly::X641", "request frame chat={chat_id} point read not a request");
                    self.schedule_preview_refetch(RiseChatListKind::Requests);
                    return Ok(None);
                };
                let current = self.stored_chat_id(&chat_id, false).await?;
                let baseline = resolution.previous.as_ref();
                if current.as_ref().map(|value| value.entity.local_id.as_str())
                    != baseline.map(|value| value.entity.local_id.as_str())
                    || current.as_ref().map(|value| value.entity.version) != baseline.map(|value| value.entity.version)
                    || current.as_ref().and_then(|value| value.record.tombstone)
                        != baseline.and_then(|value| value.record.tombstone)
                {
                    self.schedule_preview_refetch(RiseChatListKind::Requests);
                    return Ok(None);
                }
                state = Self::request_message_state(authoritative.last_message.as_ref(), previous.as_ref());
                if let Some(last) = Self::real_last_message(authoritative.last_message.as_ref())
                    && let Some(old_last) =
                        Self::real_last_message(previous.as_ref().and_then(|value| value.last_message.as_ref()))
                    && last.id != old_last.id
                    && Self::preview_timestamp(last) == Self::preview_timestamp(old_last)
                {
                    state.advances_preview = true;
                }
                updates.insert(
                    RiseNormalizedKey::unread_count().to_owned(),
                    RiseJSONValue::Integer(authoritative.resolved_unread_count().max(0)),
                );
                base_wire = authoritative;
            } else {
                updates.insert(RiseNormalizedKey::unread_count().to_owned(), RiseJSONValue::Integer(state.unread));
            }
            if !state.advances_preview
                && let Some(previous) = &previous
            {
                updates.insert(
                    RiseNormalizedKey::last_message().to_owned(),
                    RiseChatWireCodec::last_message_value(previous.last_message.as_ref())?,
                );
                updates.insert(
                    RiseNormalizedKey::updated_at().to_owned(),
                    previous.updated_at.map_or(RiseJSONValue::Null, RiseJSONValue::Number),
                );
                updates.insert(
                    RiseNormalizedKey::last_activity().to_owned(),
                    previous.last_activity.map_or(RiseJSONValue::Null, RiseJSONValue::Number),
                );
            }
        }
        let mut wire = RiseChatWireCodec::patch(&base_wire, updates)?;
        if !accepted && wire.last_message.is_none() {
            wire = self.applying_orphan_preview(wire, &chat_id)?;
        }
        tracing::warn!(
            target: "riseonly::X567",
            "request frame chat={chat_id} preview={} unread={}",
            wire.last_message.is_some(),
            wire.resolved_unread_count()
        );
        let version = self.next_version();
        self.clear_pending_mutation(&resolution.local_id);
        let batch = self.make_upsert_batch(
            RiseChatEntityRecord {
                wire,
                source: RiseConversationEntitySource::Realtime,
                locally_created: false,
                list_kind: Some(destination),
                tombstone: None,
            },
            &resolution.local_id,
            resolution.aliases,
            Some(destination),
            version,
            true,
            resolution.duplicate_local_ids,
        )?;
        Ok(Some(batch))
    }

    async fn reduce_member(
        &self,
        envelope: &RiseWirePushEnvelope,
        joined: bool,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let root = Self::payload_object(&envelope.data)?;
        let (Some(chat_id), Some(user_id)) = (
            Self::string(&root, &[RiseNormalizedKey::chat_id()]),
            Self::string(&root, &[RiseNormalizedKey::user_id()]),
        ) else {
            return Ok(None);
        };
        if chat_id.is_empty() || user_id.is_empty() {
            return Ok(None);
        }

        if user_id == self.account_id {
            if joined {
                let nested_chat = Self::nested_object(&root, &[rise_key::chat::CHAT, RiseNormalizedKey::chat_info()]);
                if nested_chat.is_none() && self.stored_chat_id(&chat_id, false).await?.is_none() {
                    return Ok(None);
                }
                let incoming = nested_chat.unwrap_or_else(|| {
                    BTreeMap::from([(rise_key::chat::ID.to_owned(), RiseJSONValue::String(chat_id.clone()))])
                });
                let resolution = self.resolve_incoming_chat(&chat_id, &incoming, RiseChatListKind::Inbox, true).await?;
                let version = self.next_version();
                self.clear_pending_mutation(&resolution.local_id);
                let wire = Self::granting_membership(&resolution.wire)?;
                let batch = self.make_upsert_batch(
                    RiseChatEntityRecord {
                        wire,
                        source: RiseConversationEntitySource::Realtime,
                        locally_created: false,
                        list_kind: Some(RiseChatListKind::Inbox),
                        tombstone: None,
                    },
                    &resolution.local_id,
                    resolution.aliases,
                    Some(RiseChatListKind::Inbox),
                    version,
                    true,
                    resolution.duplicate_local_ids,
                )?;
                return Ok(Some(batch));
            }
            let Some(stored) = self.stored_chat_id(&chat_id, false).await? else {
                return Ok(None);
            };
            let version = self.next_version();
            self.clear_pending_mutation(&stored.entity.local_id);
            let batch = self.make_tombstone_batch(&stored, RiseChatTombstone::ConfirmedLeft, version)?;
            let merged = self.merging_aggregates(batch, version, Self::removal_deltas(&stored.record)).await?;
            return Ok(Some(merged));
        }

        let Some(stored) = self.stored_chat_id(&chat_id, false).await? else {
            return Ok(None);
        };
        if stored.record.tombstone.is_some() {
            return Ok(None);
        }
        let count = (stored.record.wire.member_count.unwrap_or(0) + if joined { 1 } else { -1 }).max(0);
        let wire = RiseChatWireCodec::patch(
            &stored.record.wire,
            BTreeMap::from([(RiseNormalizedKey::member_count().to_owned(), RiseJSONValue::Integer(count))]),
        )?;
        let version = self.next_version();
        let aliases = RiseConversationStoreLayout::chat_aliases(&wire);
        let batch = self.make_upsert_batch(
            RiseChatEntityRecord {
                wire,
                source: RiseConversationEntitySource::Realtime,
                locally_created: stored.record.locally_created,
                list_kind: stored.record.list_kind,
                tombstone: None,
            },
            &stored.entity.local_id,
            aliases,
            stored.record.list_kind,
            version,
            false,
            Vec::new(),
        )?;
        Ok(Some(batch))
    }

    pub async fn confirm_removed_by_server<Announce, Announced>(
        &self,
        chat_id: &str,
        announce: Announce,
    ) -> anyhow::Result<bool>
    where
        Announce: Fn(ChatEngineRemovedChatName) -> Announced + Send + Sync,
        Announced: Future<Output = ()> + Send,
    {
        let Some(stored) = self.stored_chat_id(chat_id, false).await? else {
            return Ok(false);
        };
        let Some(chat_type) = stored.record.wire.r#type.as_deref().and_then(ChatType::matching) else {
            return Ok(false);
        };
        if !chat_type.is_community() {
            return Ok(false);
        }
        let mut chat_name = ChatEngineRemovedChatName {
            title: stored.record.wire.title.clone().unwrap_or_default(),
            is_channel: chat_type == ChatType::Channel,
            announces: true,
        };
        if stored.record.tombstone == Some(RiseChatTombstone::ConfirmedLeft) {
            chat_name.announces = false;
            announce(chat_name).await;
            return Ok(true);
        }
        let joined = chat_engine_hooks()
            .is_some_and(|hooks| (hooks.is_joined_member_role)(stored.record.wire.role.as_deref()));
        if stored.record.tombstone.is_some() || !joined {
            return Ok(false);
        }
        announce(chat_name).await;
        let version = self.next_version();
        self.clear_pending_mutation(&stored.entity.local_id);
        let batch = self.make_tombstone_batch(&stored, RiseChatTombstone::ConfirmedLeft, version)?;
        let merged = self.merging_aggregates(batch, version, Self::removal_deltas(&stored.record)).await?;
        self.engine.write(merged).await?;
        Ok(true)
    }

    async fn reduce_own_role(
        &self,
        envelope: &RiseWirePushEnvelope,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let root = Self::payload_object(&envelope.data)?;
        if Self::string(&root, &[RiseNormalizedKey::target_user_id()]).as_deref() != Some(self.account_id.as_str()) {
            return Ok(None);
        }
        let (Some(chat_id), Some(role)) = (
            Self::string(&root, &[RiseNormalizedKey::chat_id()]),
            Self::string(&root, &[RiseNormalizedKey::new_role()]),
        ) else {
            return Ok(None);
        };
        if role.is_empty() {
            return Ok(None);
        }
        let Some(stored) = self.stored_chat_id(&chat_id, false).await? else {
            return Ok(None);
        };
        if stored.record.tombstone.is_some() || stored.record.wire.role.as_deref() == Some(role.as_str()) {
            return Ok(None);
        }
        let wire = RiseChatWireCodec::patch(
            &stored.record.wire,
            BTreeMap::from([(rise_key::chat::ROLE.to_owned(), RiseJSONValue::String(role))]),
        )?;
        let version = self.next_version();
        let aliases = RiseConversationStoreLayout::chat_aliases(&wire);
        let batch = self.make_upsert_batch(
            RiseChatEntityRecord {
                wire,
                source: RiseConversationEntitySource::Realtime,
                locally_created: stored.record.locally_created,
                list_kind: stored.record.list_kind,
                tombstone: None,
            },
            &stored.entity.local_id,
            aliases,
            stored.record.list_kind,
            version,
            false,
            Vec::new(),
        )?;
        Ok(Some(batch))
    }

    async fn resolve_incoming_chat(
        &self,
        chat_id: &str,
        incoming: &BTreeMap<String, RiseJSONValue>,
        default_kind: RiseChatListKind,
        explicitly_restores_membership: bool,
    ) -> anyhow::Result<RiseChatRepositoryIncomingResolution> {
        fn append(aliases: &mut Vec<RiseEntityAlias>, seen: &mut HashSet<RiseEntityAlias>, alias: RiseEntityAlias) {
            if seen.insert(alias.clone()) {
                aliases.push(alias);
            }
        }
        let mut aliases: Vec<RiseEntityAlias> = Vec::new();
        let mut seen: HashSet<RiseEntityAlias> = HashSet::new();
        let normalized_chat_id = trimming_whitespaces_and_newlines(chat_id);
        let primary_kind = if GetSaiServerId::is_temp_id(Some(normalized_chat_id)) {
            RiseEntityAliasKind::temporary_id()
        } else {
            RiseEntityAliasKind::server_id()
        };
        append(&mut aliases, &mut seen, RiseEntityAlias::new(primary_kind, normalized_chat_id));
        if let Some(temp_id) = Self::string(incoming, &[RiseNormalizedKey::temp_id(), TEMPORARY_ID_KEY])
            && !temp_id.is_empty()
        {
            append(&mut aliases, &mut seen, RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), temp_id));
        }
        if let Some(client_id) = Self::string(incoming, &[CLIENT_ID_KEY, RiseNormalizedKey::request_id()])
            && !client_id.is_empty()
        {
            append(&mut aliases, &mut seen, RiseEntityAlias::new(RiseEntityAliasKind::client_id(), client_id));
        }
        if let Some(server_id) = Self::string(incoming, &[SERVER_ID_KEY])
            && !server_id.is_empty()
        {
            append(&mut aliases, &mut seen, RiseEntityAlias::new(RiseEntityAliasKind::server_id(), server_id));
        }

        let mut existing_by_local_id: HashMap<String, RiseChatStoredEntity> = HashMap::new();
        for alias in &aliases {
            let Some(entity) =
                self.engine.entity_alias(RiseConversationStoreLayout::chats(), alias.clone()).await?
            else {
                continue;
            };
            existing_by_local_id.insert(entity.local_id.clone(), self.decode_stored(entity)?);
        }
        let preferred = existing_by_local_id
            .values()
            .find(|value| GetSaiServerId::is_temp_id(Some(value.entity.local_id.as_str())))
            .or_else(|| existing_by_local_id.values().next())
            .cloned();

        if preferred.as_ref().is_some_and(|value| value.record.tombstone.is_some()) && !explicitly_restores_membership
        {
            return Err(RiseChatRepositoryError::ChatIsTombstoned(chat_id.to_owned()).into());
        }
        let base_wire = match &preferred {
            Some(value) => value.record.wire.clone(),
            None => RiseChatWireCodec::minimal_wire(chat_id)?,
        };
        let mut wire = RiseChatWireCodec::overlay(
            &base_wire,
            incoming,
            &Self::key_set(&[
                rise_key::events::PAYLOAD,
                rise_key::chat::CHAT,
                RiseNormalizedKey::chat_info(),
                RiseNormalizedKey::temp_id(),
                TEMPORARY_ID_KEY,
                CLIENT_ID_KEY,
                RiseNormalizedKey::request_id(),
            ]),
        )?;
        if wire.id.is_empty() {
            wire = RiseChatWireCodec::patch(
                &wire,
                BTreeMap::from([(rise_key::chat::ID.to_owned(), RiseJSONValue::String(chat_id.to_owned()))]),
            )?;
        }
        for alias in RiseConversationStoreLayout::chat_aliases(&wire) {
            append(&mut aliases, &mut seen, alias);
        }
        let local_id = preferred
            .as_ref()
            .map_or_else(|| RiseConversationStoreLayout::local_chat_id(&wire), |value| value.entity.local_id.clone());
        let kind = preferred.as_ref().and_then(|value| value.record.list_kind).unwrap_or(default_kind);
        let duplicate_local_ids = existing_by_local_id.into_keys().filter(|value| *value != local_id).collect();
        Ok(RiseChatRepositoryIncomingResolution {
            local_id,
            wire,
            kind,
            aliases,
            duplicate_local_ids,
            previous: preferred,
        })
    }

    fn clear_pending_mutation(&self, local_id: &str) {
        self.state.with(|state| {
            if let Some(mutation_id) = state.pending_mutation_by_chat.remove(local_id) {
                state.pending_mutations_by_id.remove(&mutation_id);
            }
        });
    }

    fn realtime_event_types() -> Vec<&'static str> {
        RiseChatRealtimeMessageKind::all_cases()
            .iter()
            .map(|kind| kind.raw_value())
            .chain(Self::CHAT_EVENT_TYPES)
            .collect()
    }

    const CHAT_EVENT_TYPES: [&'static str; 10] = [
        name::CHAT_CREATED,
        name::CHAT_UPDATED,
        name::CHAT_DELETED,
        name::CHAT_ARCHIVED,
        name::CHAT_UNARCHIVED,
        name::MESSAGE_REQUEST_CREATED,
        name::MESSAGE_REQUEST_ACCEPTED,
        name::MEMBER_JOINED,
        name::MEMBER_LEFT,
        name::CHAT_USER_ROLE_CHANGED,
    ];

    const RESOURCE_EVENT_TYPES: [&'static str; 14] = [
        name::TOPIC_CREATED,
        name::TOPIC_UPDATED,
        name::TOPIC_DELETED,
        name::TOPIC_OVERWRITE_CHANGED,
        name::ROLE_CREATED,
        name::ROLE_UPDATED,
        name::ROLE_DELETED,
        name::ROLES_REORDERED,
        name::MEMBER_ROLES_CHANGED,
        name::SECTION_CREATED,
        name::SECTION_UPDATED,
        name::SECTION_DELETED,
        name::SECTION_OVERWRITE_CHANGED,
        name::SECTIONS_REORDERED,
    ];

    fn chat_event_invalidations(&self, envelope: &RiseWirePushEnvelope) -> HashSet<RiseChatResourceInvalidation> {
        let mut invalidations = HashSet::new();
        if let Some(roster) = self.roster_invalidation(envelope) {
            invalidations.insert(roster);
        }
        if let Some(settings) = Self::settings_invalidation(envelope) {
            invalidations.insert(settings);
        }
        if envelope.r#type == name::CHAT_USER_ROLE_CHANGED
            && let Ok(root) = Self::payload_object(&envelope.data)
            && let Some(chat_id) = Self::string(&root, &[RiseNormalizedKey::chat_id()])
            && !chat_id.is_empty()
        {
            let target_id = Self::string(&root, &[RiseNormalizedKey::target_user_id()]);
            if target_id.as_deref() == Some(self.account_id.as_str()) {
                invalidations.insert(RiseChatResourceInvalidation::Preview {
                    chat_id: chat_id.clone(),
                    topic_id: None,
                    reread: false,
                });
            }
            if let Some(target_id) = target_id
                && !target_id.is_empty()
                && let Some(role) = Self::string(&root, &[RiseNormalizedKey::new_role()])
                && !role.is_empty()
            {
                invalidations.insert(RiseChatResourceInvalidation::MemberRole {
                    chat_id: chat_id.clone(),
                    user_id: target_id,
                    role: Some(role),
                    role_ids: None,
                    display_color: None,
                });
            }
            invalidations.insert(RiseChatResourceInvalidation::Members { chat_id });
        }
        if envelope.r#type == name::MEMBER_LEFT
            && let Ok(root) = Self::payload_object(&envelope.data)
            && Self::string(&root, &[RiseNormalizedKey::user_id()]).as_deref() == Some(self.account_id.as_str())
            && let Some(reason) = Self::string(&root, &[rise_key::chat::REASON])
            && (reason == ChatLeaveReason::Kicked.raw_value() || reason == ChatLeaveReason::Banned.raw_value())
            && let Some(chat_id) = Self::string(&root, &[RiseNormalizedKey::chat_id()])
            && !chat_id.is_empty()
        {
            invalidations.insert(RiseChatResourceInvalidation::RemovedByAdmin { chat_id });
        }
        invalidations
    }

    fn settings_invalidation(envelope: &RiseWirePushEnvelope) -> Option<RiseChatResourceInvalidation> {
        if envelope.r#type != name::CHAT_UPDATED {
            return None;
        }
        let root = Self::payload_object(&envelope.data).ok()?;
        if root.get(RiseNormalizedKey::settings_changed()) != Some(&RiseJSONValue::Bool(true)) {
            return None;
        }
        let chat_id = Self::string(&root, &[RiseNormalizedKey::chat_id()]).filter(|value| !value.is_empty())?;
        Some(RiseChatResourceInvalidation::Settings { chat_id })
    }

    fn roster_invalidation(&self, envelope: &RiseWirePushEnvelope) -> Option<RiseChatResourceInvalidation> {
        let joined = match envelope.r#type.as_str() {
            name::MEMBER_JOINED => true,
            name::MEMBER_LEFT => false,
            _ => return None,
        };
        let root = Self::payload_object(&envelope.data).ok()?;
        let chat_id = Self::string(&root, &[RiseNormalizedKey::chat_id()])?;
        let user_id = Self::string(&root, &[RiseNormalizedKey::user_id()])?;
        if chat_id.is_empty() || user_id.is_empty() || user_id == self.account_id {
            return None;
        }
        Some(RiseChatResourceInvalidation::Roster { chat_id, user_id, joined })
    }

    fn resource_invalidations(
        envelope: &RiseWirePushEnvelope,
        account_id: &str,
    ) -> HashSet<RiseChatResourceInvalidation> {
        let Ok(root) = Self::payload_object(&envelope.data) else {
            return HashSet::new();
        };
        let Some(chat_id) = Self::string(&root, &[RiseNormalizedKey::chat_id()]) else {
            return HashSet::new();
        };
        let mut invalidations: HashSet<RiseChatResourceInvalidation> = HashSet::new();
        if envelope.r#type.starts_with("topic_") || envelope.r#type.starts_with("section_") {
            invalidations.insert(RiseChatResourceInvalidation::Topics { chat_id: chat_id.clone() });
            invalidations.insert(RiseChatResourceInvalidation::Forum { chat_id: chat_id.clone() });
        } else if envelope.r#type.contains("role") {
            invalidations.insert(RiseChatResourceInvalidation::Roles { chat_id: chat_id.clone() });
            invalidations.insert(RiseChatResourceInvalidation::Members { chat_id: chat_id.clone() });
            if envelope.r#type == name::MEMBER_ROLES_CHANGED
                && let Some(user_id) = Self::string(&root, &[RiseNormalizedKey::user_id()])
                && !user_id.is_empty()
            {
                let role = Self::string(&root, &[rise_key::chat::ROLE]).filter(|value| !value.is_empty());
                let role_ids = match root.get(RiseNormalizedKey::role_ids()) {
                    Some(RiseJSONValue::Array(values)) => {
                        Some(values.iter().filter_map(|value| value.string_value().map(str::to_owned)).collect())
                    }
                    _ => None,
                };
                let display_color = Self::string(&root, &[RiseNormalizedKey::display_color()])
                    .and_then(|value| value.parse::<i64>().ok());
                if role.is_some() || role_ids.is_some() || display_color.is_some() {
                    invalidations.insert(RiseChatResourceInvalidation::MemberRole {
                        chat_id: chat_id.clone(),
                        user_id,
                        role,
                        role_ids,
                        display_color,
                    });
                }
            }
        }
        if let Some(preview) = Self::preview_invalidation(&envelope.r#type, &root, &chat_id, account_id) {
            invalidations.insert(preview);
        }
        if envelope.r#type == name::TOPIC_DELETED
            && let Some(topic_id) = Self::string(&root, &[RiseNormalizedKey::topic_id()])
            && !topic_id.is_empty()
        {
            invalidations.insert(RiseChatResourceInvalidation::TopicDeleted {
                chat_id,
                topic_id,
                deleted_by: Self::string(&root, &[RiseNormalizedKey::deleted_by()]),
            });
        }
        invalidations
    }

    fn preview_invalidation(
        event_type: &str,
        root: &BTreeMap<String, RiseJSONValue>,
        chat_id: &str,
        account_id: &str,
    ) -> Option<RiseChatResourceInvalidation> {
        let preview = |topic_id: Option<String>, reread: bool| {
            Some(RiseChatResourceInvalidation::Preview { chat_id: chat_id.to_owned(), topic_id, reread })
        };
        match event_type {
            name::TOPIC_OVERWRITE_CHANGED => preview(Self::string(root, &[RiseNormalizedKey::topic_id()]), false),
            name::TOPIC_DELETED => preview(Self::string(root, &[RiseNormalizedKey::topic_id()]), true),
            name::TOPIC_CREATED => preview(None, true),
            name::SECTION_OVERWRITE_CHANGED | name::ROLE_UPDATED | name::ROLE_DELETED => preview(None, false),
            name::MEMBER_ROLES_CHANGED => {
                if Self::string(root, &[RiseNormalizedKey::user_id()]).as_deref() != Some(account_id) {
                    return None;
                }
                preview(None, false)
            }
            _ => None,
        }
    }

    fn payload_object(value: &RiseJSONValue) -> anyhow::Result<BTreeMap<String, RiseJSONValue>> {
        let RiseJSONValue::Object(raw) = value else {
            return Err(RiseChatRepositoryError::InvalidPush("non_object_payload".to_owned()).into());
        };
        let normalized = RiseChatWireCodec::normalized_object(raw);
        if let Some(RiseJSONValue::Object(payload)) = normalized.get(rise_key::events::PAYLOAD) {
            return Ok(RiseChatWireCodec::normalized_object(payload));
        }
        Ok(normalized)
    }

    fn nested_object(
        object: &BTreeMap<String, RiseJSONValue>,
        keys: &[&str],
    ) -> Option<BTreeMap<String, RiseJSONValue>> {
        for key in keys {
            if let Some(RiseJSONValue::Object(value)) = object.get(*key) {
                return Some(RiseChatWireCodec::normalized_object(value));
            }
        }
        None
    }

    fn string(object: &BTreeMap<String, RiseJSONValue>, keys: &[&str]) -> Option<String> {
        for key in keys {
            match object.get(*key) {
                Some(RiseJSONValue::String(string)) => return Some(string.clone()),
                Some(RiseJSONValue::Integer(number)) => return Some(number.to_string()),
                Some(RiseJSONValue::UnsignedInteger(number)) => return Some(number.to_string()),
                _ => continue,
            }
        }
        None
    }

    fn bool(object: &BTreeMap<String, RiseJSONValue>, keys: &[&str]) -> Option<bool> {
        for key in keys {
            match object.get(*key) {
                Some(RiseJSONValue::Bool(flag)) => return Some(*flag),
                Some(RiseJSONValue::Integer(number)) => return Some(*number != 0),
                Some(RiseJSONValue::UnsignedInteger(number)) => return Some(*number != 0),
                _ => continue,
            }
        }
        None
    }

    fn contains_any(object: &BTreeMap<String, RiseJSONValue>, keys: &[&str]) -> bool {
        keys.iter().any(|key| object.contains_key(*key))
    }

    fn key_set(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|key| (*key).to_owned()).collect()
    }

    fn decode_message(object: BTreeMap<String, RiseJSONValue>) -> anyhow::Result<MessageWireDTO> {
        Ok(RiseJSONValue::Object(object).decode_as::<MessageWireDTO>()?)
    }

    fn topic_presence(
        root: &BTreeMap<String, RiseJSONValue>,
        message: &BTreeMap<String, RiseJSONValue>,
    ) -> RiseConversationTopicPresence {
        for object in [message, root] {
            match object.get(RiseNormalizedKey::topic_id()) {
                None => continue,
                Some(RiseJSONValue::Null) => return RiseConversationTopicPresence::Null,
                Some(RiseJSONValue::String(topic)) => return RiseConversationTopicPresence::Value(topic.clone()),
                Some(_) => return RiseConversationTopicPresence::Absent,
            }
        }
        RiseConversationTopicPresence::Absent
    }

    fn message_ids(
        root: &BTreeMap<String, RiseJSONValue>,
        message: &BTreeMap<String, RiseJSONValue>,
        decoded: Option<&MessageWireDTO>,
    ) -> Vec<String> {
        fn append(result: &mut Vec<String>, seen: &mut HashSet<String>, value: Option<&str>) {
            let Some(value) = value.map(trimming_whitespaces_and_newlines) else {
                return;
            };
            if !value.is_empty() && seen.insert(value.to_owned()) {
                result.push(value.to_owned());
            }
        }
        let mut result: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        if let Some(decoded) = decoded {
            append(&mut result, &mut seen, decoded.server_id.as_deref());
            append(&mut result, &mut seen, decoded.id.as_deref());
            append(&mut result, &mut seen, decoded.temp_id.as_deref());
        }
        for object in [message, root] {
            let identity = Self::string(
                object,
                &[RiseNormalizedKey::message_id(), SERVER_ID_KEY, rise_key::chat::ID, RiseNormalizedKey::temp_id()],
            );
            append(&mut result, &mut seen, identity.as_deref());
            for key in [RiseNormalizedKey::message_ids(), RiseNormalizedKey::deleted_message_ids()] {
                let Some(RiseJSONValue::Array(values)) = object.get(key) else {
                    continue;
                };
                for value in values {
                    append(&mut result, &mut seen, value.string_value());
                }
            }
        }
        result
    }

    fn note_displaced_preview(
        &self,
        chat_id: &str,
        old_wire: &RiseStoredChat,
        preview: &RiseStoredChatPreview,
        message: &MessageWireDTO,
        incoming: bool,
        same_preview_identity: bool,
    ) {
        let unsent = !incoming && RiseConversationStoreLayout::canonical_message_server_id(message).is_none();
        self.state.with(|state| {
            if !unsent {
                state.displaced_previews.remove(chat_id);
                return;
            }
            if same_preview_identity {
                return;
            }
            let Some(preview_id) = preview.id.clone() else {
                return;
            };
            state.displaced_previews.entry(chat_id.to_owned()).or_default().insert(
                preview_id,
                RiseChatDisplacedPreview {
                    last_message: old_wire.last_message.clone(),
                    updated_at: old_wire.updated_at,
                    last_activity: old_wire.last_activity,
                },
            );
        });
    }

    fn take_displaced_preview(
        &self,
        chat_id: &str,
        current: Option<&RiseStoredChatPreview>,
        deleted_ids: &[String],
    ) -> Option<RiseChatDisplacedPreview> {
        self.state.with(|state| {
            let claims = state.displaced_previews.get_mut(chat_id)?;
            let mut removed: HashMap<String, RiseChatDisplacedPreview> = HashMap::new();
            for id in deleted_ids {
                if let Some(claim) = claims.remove(id) {
                    removed.insert(id.clone(), claim);
                }
            }
            let mut orphaned: Vec<String> = Vec::new();
            for (key, claim) in claims.iter_mut() {
                let Some(pointed) = claim.last_message.as_ref().and_then(|preview| preview.id.clone()) else {
                    continue;
                };
                if !deleted_ids.contains(&pointed) {
                    continue;
                }
                match Self::resolve_displaced_claim(&removed, deleted_ids, &pointed) {
                    Some(inherited) => *claim = inherited,
                    None => orphaned.push(key.clone()),
                }
            }
            for key in &orphaned {
                claims.remove(key);
            }
            let result = current
                .and_then(|preview| preview.id.as_deref())
                .filter(|id| deleted_ids.iter().any(|deleted| deleted == id))
                .and_then(|id| Self::resolve_displaced_claim(&removed, deleted_ids, id));
            if claims.is_empty() {
                state.displaced_previews.remove(chat_id);
            }
            result
        })
    }

    fn resolve_displaced_claim(
        removed: &HashMap<String, RiseChatDisplacedPreview>,
        deleted_ids: &[String],
        id: &str,
    ) -> Option<RiseChatDisplacedPreview> {
        let mut claim = removed.get(id)?.clone();
        for _ in 0..=removed.len() {
            let Some(pointed) = claim.last_message.as_ref().and_then(|preview| preview.id.clone()) else {
                return Some(claim);
            };
            if !deleted_ids.contains(&pointed) {
                return Some(claim);
            }
            claim = removed.get(&pointed)?.clone();
        }
        None
    }

    fn last_message(message: Option<&RiseStoredChatPreview>, ids: &[String]) -> bool {
        let Some(id) = message.and_then(|value| value.id.as_ref()) else {
            return false;
        };
        ids.contains(id)
    }

    /// coalesced, a burst of deletions costs one list read per kind
    /// a deletion during the read gets its own read after
    pub fn schedule_preview_refetch(&self, kind: RiseChatListKind) {
        self.state.with(|state| {
            state.pending_preview_refetch_kinds.insert(kind);
            if state.preview_refetch_task.is_some() {
                return;
            }
            let this = self.this.clone();
            state.preview_refetch_task = Some(RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = this.upgrade() {
                    this.drain_preview_refetches().await;
                }
            }));
        });
    }

    pub(crate) fn schedule_request_preview_retry(&self, chat_id: &str) {
        let attempt = self.state.with(|state| {
            let attempts = state.request_preview_retries.entry(chat_id.to_owned()).or_insert(0);
            *attempts += 1;
            *attempts
        });
        if attempt > REQUEST_PREVIEW_RETRY_LIMIT {
            return;
        }
        let this = self.this.clone();
        RiseAccountRuntime::current().spawn(async move {
            if !RiseRuntimeTimer::sleep(REQUEST_PREVIEW_RETRY_STEP * attempt).await {
                return;
            }
            if let Some(this) = this.upgrade() {
                this.schedule_preview_refetch(RiseChatListKind::Requests);
            }
        });
    }

    async fn drain_preview_refetches(&self) {
        loop {
            let next = self.state.with(|state| {
                while let Some(kind) = state.pending_preview_refetch_kinds.iter().next().copied() {
                    state.pending_preview_refetch_kinds.remove(&kind);
                    if state.loading_kinds.contains(&kind) {
                        state.preview_refetch_after_load_kinds.insert(kind);
                        continue;
                    }
                    return Some(kind);
                }
                state.preview_refetch_task = None;
                None
            });
            let Some(kind) = next else {
                return;
            };
            let _ = self.refresh(kind).await;
        }
    }

    /// a system line means no messages yet and is not an unread message
    fn real_last_message(message: Option<&RiseStoredChatPreview>) -> Option<&RiseStoredChatPreview> {
        let message = message?;
        if message.is_system_message == Some(true)
            || message.content_type.as_deref().map(str::to_lowercase).as_deref()
                == Some(ChatMessageContentType::System.wire())
        {
            return None;
        }
        Some(message)
    }

    fn preview_timestamp(preview: &RiseStoredChatPreview) -> f64 {
        RiseConversationStoreLayout::normalized_timestamp(preview.created_at.or(preview.timestamp).unwrap_or(0.0))
    }

    fn request_message_state(
        incoming: Option<&RiseStoredChatPreview>,
        previous: Option<&RiseStoredChat>,
    ) -> RiseChatRepositoryRequestMessageState {
        let unread = previous.map_or(0, RiseStoredChat::resolved_unread_count).max(0);
        let state = |unread: i64, advances_preview: bool, needs_authoritative_count: bool| {
            RiseChatRepositoryRequestMessageState { unread, advances_preview, needs_authoritative_count }
        };
        let Some(incoming) = Self::real_last_message(incoming) else {
            return state(if previous.is_none() { 1 } else { unread }, false, false);
        };
        let incoming_id = incoming.id.as_deref().map_or("", trimming_whitespaces_and_newlines);
        let Some(existing) = Self::real_last_message(previous.and_then(|value| value.last_message.as_ref())) else {
            return state(unread.max(1), true, incoming_id.is_empty());
        };
        let existing_id = existing.id.as_deref().map_or("", trimming_whitespaces_and_newlines);
        if !incoming_id.is_empty() && incoming_id == existing_id {
            return state(unread, false, false);
        }
        let advances = Self::preview_timestamp(incoming) > Self::preview_timestamp(existing);
        if incoming_id.is_empty() || existing_id.is_empty() || !advances {
            return state(unread, advances, true);
        }
        state(if unread == i64::MAX { unread } else { unread + 1 }, true, false)
    }

    fn message_identity_values(message: &MessageWireDTO) -> Vec<String> {
        let mut values: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for raw in [&message.server_id, &message.id, &message.temp_id] {
            let Some(value) = raw.as_deref().map(trimming_whitespaces_and_newlines) else {
                continue;
            };
            if !value.is_empty() && seen.insert(value.to_owned()) {
                values.push(value.to_owned());
            }
        }
        values
    }

    fn sender_id(message: &MessageWireDTO) -> String {
        message
            .sender_id
            .clone()
            .or_else(|| message.sender.as_ref().and_then(|sender| sender.id.clone()))
            .unwrap_or_default()
    }

    fn stash_orphan_preview(
        &self,
        chat_id: &str,
        scope: &RiseMessageScope,
        message: &MessageWireDTO,
    ) -> anyhow::Result<()> {
        let topic = message
            .topic_id
            .as_ref()
            .or(scope.topic_id.as_ref())
            .map_or(RiseConversationTopicPresence::Absent, |topic_id| {
                RiseConversationTopicPresence::Value(topic_id.clone())
            });
        let timestamp = RiseChatWireCodec::timestamp(message);
        let sender_id = Self::sender_id(message);
        let charges = !sender_id.is_empty() && sender_id != self.account_id && Self::charges_unread(message);
        let last_message =
            RiseChatWireCodec::last_message_value(Some(&RiseChatWireCodec::last_message(message, &topic)))?;
        let is_photo = charges && Self::is_photo_message(message);
        let server_id = RiseConversationStoreLayout::canonical_message_server_id(message);
        let projection_limit = self.projection_limit;
        self.state.with(|state| {
            if !state.orphan_created_previews.contains_key(chat_id)
                && state.orphan_created_previews.len() as i64 >= projection_limit
            {
                state.orphan_created_previews.clear();
            }
            let entry = state.orphan_created_previews.entry(chat_id.to_owned()).or_insert_with(|| {
                RiseChatOrphanPreview {
                    last_message: RiseJSONValue::Null,
                    timestamp: f64::MIN,
                    charged_ids: HashSet::new(),
                    unread_media: 0,
                }
            });
            if charges
                && let Some(server_id) = server_id
                && entry.charged_ids.insert(server_id)
                && is_photo
            {
                entry.unread_media += 1;
            }
            if entry.timestamp <= timestamp {
                entry.last_message = last_message;
                entry.timestamp = timestamp;
            }
        });
        Ok(())
    }

    /// puts stashed messages on a row that came without a preview
    /// eg a chat_created card, a request frame or a requests page
    pub(crate) fn applying_orphan_preview(
        &self,
        wire: RiseStoredChat,
        chat_id: &str,
    ) -> anyhow::Result<RiseStoredChat> {
        let orphan = self.state.with(|state| state.orphan_created_previews.remove(chat_id));
        let Some(orphan) = orphan else {
            return Ok(wire);
        };
        if wire.last_message.is_some() {
            return Ok(wire);
        }
        let mut updates: BTreeMap<String, RiseJSONValue> = BTreeMap::from([
            (RiseNormalizedKey::last_message().to_owned(), orphan.last_message),
            (RiseNormalizedKey::updated_at().to_owned(), RiseJSONValue::Number(orphan.timestamp)),
            (RiseNormalizedKey::last_activity().to_owned(), RiseJSONValue::Number(orphan.timestamp)),
        ]);
        let unread = orphan.charged_ids.len() as i64;
        if unread > 0 && wire.resolved_unread_count() < unread {
            updates.insert(RiseNormalizedKey::unread_count().to_owned(), RiseJSONValue::Integer(unread));
            if orphan.unread_media > wire.resolved_unread_media_count() {
                updates.insert(
                    RiseNormalizedKey::unread_media_count().to_owned(),
                    RiseJSONValue::Integer(orphan.unread_media),
                );
            }
        }
        RiseChatWireCodec::patch(&wire, updates)
    }

    /// photo message for the badge second number, an album counts once per message like the server ledger
    fn is_photo_message(message: &MessageWireDTO) -> bool {
        matches!(message.content_type.as_deref().map(str::to_lowercase).as_deref(), Some("image" | "photo"))
    }

    /// group voice chat posts a line when the first joins and the last leaves
    /// so the list badge moves at once and doesnt wait for the voice subscription
    fn live_voice_change(message: &MessageWireDTO) -> Option<bool> {
        match message.system_message_type.as_deref().and_then(MessageSystemKind::from_raw) {
            Some(MessageSystemKind::VoiceCallStarted) => Some(true),
            Some(MessageSystemKind::VoiceCallEnded) => Some(false),
            _ => None,
        }
    }

    /// same rule as the server, only a missed call counts as unread
    /// without it the badge flashed on every answered call
    fn charges_unread(message: &MessageWireDTO) -> bool {
        if RiseMessageRepository::is_device_only_row(message) {
            return false;
        }
        if message.content_type.as_deref().map(str::to_lowercase).as_deref()
            != Some(ChatMessageContentType::Call.wire())
        {
            return true;
        }
        let Some(call) = message.metadata.as_ref().and_then(|metadata| metadata.to_model()) else {
            return true;
        };
        call.answered_at_ms.is_none() && call.terminal_state != MessageCallTerminalState::Declined
    }

    fn should_advance_preview(existing: Option<&RiseStoredChatPreview>, incoming: &MessageWireDTO) -> bool {
        let Some(existing) = existing else {
            return true;
        };
        if let Some(existing_id) = &existing.id
            && Self::message_identity_values(incoming).contains(existing_id)
        {
            return true;
        }
        let incoming_timestamp =
            RiseConversationStoreLayout::normalized_timestamp(incoming.created_at.or(incoming.timestamp).unwrap_or(0.0));
        let existing_timestamp = Self::preview_timestamp(existing);
        if incoming_timestamp != existing_timestamp {
            return incoming_timestamp > existing_timestamp;
        }
        let incoming_id = RiseConversationStoreLayout::canonical_message_server_id(incoming)
            .or_else(|| incoming.id.clone())
            .or_else(|| incoming.temp_id.clone())
            .unwrap_or_default();
        let existing_id = existing.id.as_deref().unwrap_or("");
        if incoming_id == existing_id {
            return true;
        }
        let incoming_position = RisePositionKey::make(|key| {
            key.append_string_with_order(&incoming_id, RisePositionKeyOrder::Descending);
        });
        let existing_position = RisePositionKey::make(|key| {
            key.append_string_with_order(existing_id, RisePositionKeyOrder::Descending);
        });
        incoming_position > existing_position
    }

    fn edited_last_message(
        existing: Option<&RiseStoredChatPreview>,
        incoming: &BTreeMap<String, RiseJSONValue>,
        decoded: &MessageWireDTO,
        topic: &RiseConversationTopicPresence,
    ) -> anyhow::Result<RiseStoredChatPreview> {
        let Some(existing) = existing else {
            return Ok(RiseChatWireCodec::last_message(decoded, topic));
        };
        let RiseJSONValue::Object(mut object) = RiseJSONValue::encode(existing)? else {
            return Ok(RiseChatWireCodec::last_message(decoded, topic));
        };
        let allowed = Self::key_set(&[
            rise_key::chat::ID,
            RiseNormalizedKey::chat_id(),
            RiseNormalizedKey::topic_id(),
            RiseNormalizedKey::sender_id(),
            RiseNormalizedKey::sender_name(),
            rise_key::chat::CONTENT,
            rise_key::chat::CAPTION,
            RiseNormalizedKey::content_type(),
            rise_key::chat::TIMESTAMP,
            RiseNormalizedKey::created_at(),
            RiseNormalizedKey::has_attachments(),
            RiseNormalizedKey::has_reactions(),
            RiseNormalizedKey::is_pinned(),
            RiseNormalizedKey::is_read(),
            RiseNormalizedKey::is_system_message(),
            RiseNormalizedKey::system_message_type(),
            rise_key::message::METADATA,
        ]);
        for (key, value) in RiseChatWireCodec::normalized_object(incoming) {
            if !allowed.contains(&key) {
                continue;
            }
            if key == RiseNormalizedKey::topic_id() && *topic == RiseConversationTopicPresence::Absent {
                continue;
            }
            object.insert(key, value);
        }
        if let Some(server_id) = RiseConversationStoreLayout::canonical_message_server_id(decoded) {
            object.insert(rise_key::chat::ID.to_owned(), RiseJSONValue::String(server_id));
        }
        match topic {
            RiseConversationTopicPresence::Absent => {}
            RiseConversationTopicPresence::Null => {
                object.insert(RiseNormalizedKey::topic_id().to_owned(), RiseJSONValue::Null);
            }
            RiseConversationTopicPresence::Value(value) => {
                object.insert(RiseNormalizedKey::topic_id().to_owned(), RiseJSONValue::String(value.clone()));
            }
        }
        Ok(RiseJSONValue::Object(object).decode_as::<RiseStoredChatPreview>()?)
    }

    fn unread_chats_delta(previous: i64, remaining: i64) -> i64 {
        if previous > 0 && remaining == 0 {
            return -1;
        }
        if previous == 0 && remaining > 0 {
            return 1;
        }
        0
    }

    fn mark_last_message_read(wire: &RiseStoredChat) -> anyhow::Result<RiseStoredChat> {
        let Some(last_message) = &wire.last_message else {
            return Ok(wire.clone());
        };
        let RiseJSONValue::Object(mut object) = RiseJSONValue::encode(last_message)? else {
            return Ok(wire.clone());
        };
        object.insert(RiseNormalizedKey::is_read().to_owned(), RiseJSONValue::Bool(true));
        let updated = RiseJSONValue::Object(object).decode_as::<RiseStoredChatPreview>()?;
        RiseChatWireCodec::patch(
            wire,
            BTreeMap::from([(
                RiseNormalizedKey::last_message().to_owned(),
                RiseChatWireCodec::last_message_value(Some(&updated))?,
            )]),
        )
    }

    fn envelope_timestamp(envelope: &RiseWirePushEnvelope) -> Option<f64> {
        match &envelope.timestamp {
            Some(RiseWireTimestamp::UnixSeconds(value)) => {
                Some(RiseConversationStoreLayout::normalized_timestamp(*value))
            }
            Some(RiseWireTimestamp::Text(value)) => {
                ISO8601DateFormatter::new().date_from(value).map(|date| date.time_interval_since_1970())
            }
            None => None,
        }
    }

    /// moves the unread mention, reply and reaction counters of the chat row
    /// absolute replaces all, otherwise deltas are added and clamped at zero
    pub async fn make_unread_marks_batch(
        &self,
        chat_id: &str,
        mentions_delta: i64,
        replies_delta: i64,
        reactions_delta: i64,
        absolute: Option<(i64, i64, i64)>,
        version: Option<i64>,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let Some(stored) = self.stored_chat_id(chat_id, false).await? else {
            return Ok(RiseStoreWriteBatch::empty());
        };
        if stored.record.tombstone.is_some() {
            return Ok(RiseStoreWriteBatch::empty());
        }
        let old_wire = &stored.record.wire;
        let (mentions, replies, reactions) = match absolute {
            Some((mentions, replies, reactions)) => (mentions.max(0), replies.max(0), reactions.max(0)),
            None => (
                (old_wire.resolved_unread_mentions_count() + mentions_delta).max(0),
                (old_wire.resolved_unread_replies_count() + replies_delta).max(0),
                (old_wire.resolved_unread_reactions_count() + reactions_delta).max(0),
            ),
        };
        if mentions == old_wire.resolved_unread_mentions_count()
            && replies == old_wire.resolved_unread_replies_count()
            && reactions == old_wire.resolved_unread_reactions_count()
            && old_wire.unread_mentions_count.is_some()
            && old_wire.unread_replies_count.is_some()
            && old_wire.unread_reactions_count.is_some()
        {
            return Ok(RiseStoreWriteBatch::empty());
        }
        let next_after_entity =
            if stored.entity.version == i64::MAX { i64::MAX } else { stored.entity.version + 1 };
        let version = version.unwrap_or(0).max(next_after_entity).max(Self::now_milliseconds());
        self.state.with(|state| state.version_clock = state.version_clock.max(version));
        let updated_wire = RiseChatWireCodec::patch(
            old_wire,
            BTreeMap::from([
                (RiseNormalizedKey::unread_mentions_count().to_owned(), RiseJSONValue::Integer(mentions)),
                (RiseNormalizedKey::unread_replies_count().to_owned(), RiseJSONValue::Integer(replies)),
                (RiseNormalizedKey::unread_reactions_count().to_owned(), RiseJSONValue::Integer(reactions)),
            ]),
        )?;
        let aliases = RiseConversationStoreLayout::chat_aliases(&updated_wire);
        self.make_upsert_batch(
            RiseChatEntityRecord {
                wire: updated_wire,
                source: RiseConversationEntitySource::Realtime,
                locally_created: stored.record.locally_created,
                list_kind: stored.record.list_kind,
                tombstone: None,
            },
            &stored.entity.local_id,
            aliases,
            stored.record.list_kind,
            version,
            false,
            Vec::new(),
        )
    }

    pub async fn apply_unread_marks(
        &self,
        chat_id: &str,
        mentions_delta: i64,
        replies_delta: i64,
        reactions_delta: i64,
        absolute: Option<(i64, i64, i64)>,
    ) {
        let Ok(batch) =
            self.make_unread_marks_batch(chat_id, mentions_delta, replies_delta, reactions_delta, absolute, None).await
        else {
            return;
        };
        if batch.mutations.is_empty() {
            return;
        }
        let _ = self.engine.write(batch).await;
    }

    pub async fn apply_chat_unread_flag(&self, chat_id: &str, unread_count: i64) {
        let Ok(Some(stored)) = self.stored_chat_id(chat_id, true).await else {
            return;
        };
        if stored.record.tombstone.is_some() {
            return;
        }
        let old_wire = &stored.record.wire;
        let previous_unread = old_wire.resolved_unread_count();
        let remaining = unread_count.max(0);
        if remaining == previous_unread {
            self.drop_projection(&stored.entity.local_id);
            return;
        }
        let next_after_entity =
            if stored.entity.version == i64::MAX { i64::MAX } else { stored.entity.version + 1 };
        let version = next_after_entity.max(self.next_version());
        self.state.with(|state| state.version_clock = state.version_clock.max(version));
        let previous_media = old_wire.resolved_unread_media_count();
        let remaining_media = previous_media.min(remaining);
        let mut updates: BTreeMap<String, RiseJSONValue> =
            BTreeMap::from([(RiseNormalizedKey::unread_count().to_owned(), RiseJSONValue::Integer(remaining))]);
        if remaining_media != previous_media {
            updates
                .insert(RiseNormalizedKey::unread_media_count().to_owned(), RiseJSONValue::Integer(remaining_media));
        }
        let mut rollback = RiseChatUnreadFlagRollbackGuard {
            repository: self,
            local_id: &stored.entity.local_id,
            armed: true,
        };
        let written: anyhow::Result<()> = async {
            let updated_wire = RiseChatWireCodec::patch(old_wire, updates)?;
            let aliases = RiseConversationStoreLayout::chat_aliases(&updated_wire);
            let entity_batch = self.make_upsert_batch(
                RiseChatEntityRecord {
                    wire: updated_wire,
                    source: RiseConversationEntitySource::Realtime,
                    locally_created: stored.record.locally_created,
                    list_kind: stored.record.list_kind,
                    tombstone: None,
                },
                &stored.entity.local_id,
                aliases,
                stored.record.list_kind,
                version,
                false,
                Vec::new(),
            )?;
            let aggregate_kind = stored.record.list_kind;
            let moves_aggregate = aggregate_kind == Some(RiseChatListKind::Archive)
                || (aggregate_kind == Some(RiseChatListKind::Inbox) && old_wire.counts_toward_unread_badge());
            let mut batch = entity_batch;
            if moves_aggregate && let Some(aggregate_kind) = aggregate_kind {
                batch = self
                    .merging_aggregates(
                        batch,
                        version,
                        vec![(
                            aggregate_kind,
                            RiseChatAggregateDelta {
                                unread: remaining - previous_unread,
                                unread_chats: Self::unread_chats_delta(previous_unread, remaining),
                                unread_media: remaining_media - previous_media,
                                ..RiseChatAggregateDelta::default()
                            },
                        )],
                    )
                    .await?;
            }
            self.engine.write(batch).await?;
            Ok(())
        }
        .await;
        if written.is_ok() {
            rollback.armed = false;
        }
    }
}

/// a confirmed message that arrived before its chat row existed
#[derive(Clone, Debug)]
pub struct RiseChatOrphanPreview {
    pub last_message: RiseJSONValue,
    pub timestamp: f64,
    pub charged_ids: HashSet<String>,
    pub unread_media: i64,
}

/// tracks previews of deleted messages so a search page read before a delete cant bring them back
/// search asks here if a preview names a message deleted on this device
pub struct RiseChatRetractedPreviews;

static RETRACTED_PREVIEWS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, HashSet<String>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

impl RiseChatRetractedPreviews {
    const CHAT_LIMIT: usize = 512;
    const IDS_PER_CHAT: usize = 64;

    pub fn note(chat_id: &str, message_ids: &[String]) {
        if message_ids.is_empty() {
            return;
        }
        let Ok(mut retracted) = RETRACTED_PREVIEWS.lock() else {
            return;
        };
        if retracted.len() >= Self::CHAT_LIMIT && !retracted.contains_key(chat_id) {
            retracted.clear();
        }
        let ids = retracted.entry(chat_id.to_owned()).or_default();
        if ids.len() + message_ids.len() > Self::IDS_PER_CHAT {
            ids.clear();
        }
        ids.extend(message_ids.iter().cloned());
    }

    /// true when the message was deleted after this process started
    pub fn contains(chat_id: &str, message_id: &str) -> bool {
        RETRACTED_PREVIEWS
            .lock()
            .ok()
            .is_some_and(|retracted| retracted.get(chat_id).is_some_and(|ids| ids.contains(message_id)))
    }
}
