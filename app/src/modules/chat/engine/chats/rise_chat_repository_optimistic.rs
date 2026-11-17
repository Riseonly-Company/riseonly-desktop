use std::collections::{BTreeMap, HashSet};

use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::RiseJSONValue;
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityViewMembership, RiseEntityViewPosition, RiseEntityWritePolicy, RiseStoreEntity,
};
use rise_engine::rise_store::rise_store_models::{RiseStoreMutation, RiseStoreWriteBatch};

use crate::modules::chat::engine::chats::rise_chat_repository::RiseChatRepository;
use crate::modules::chat::engine::chats::rise_chat_repository_models::{
    RiseChatAggregateDelta, RiseChatOptimisticOperation, RiseChatOptimisticToken, RiseChatPendingMutation,
    RiseChatRepositoryError, RiseChatStoredEntity, RiseChatTombstone, RiseChatWireCodec,
};
use crate::modules::chat::engine::chats::rise_normalized_key::RiseNormalizedKey;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseChatEntityRecord, RiseConversationEntitySource,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseChatListKind;
use crate::modules::chat::engine::core::rise_conversation_store_layout::{
    RiseConversationStoreLayout, RiseConversationStoreLayoutChatView,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatMuteScopes;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;

struct RiseChatRepositoryPreparedOptimisticMutation {
    pending: RiseChatPendingMutation,
    batch: RiseStoreWriteBatch,
}

struct RiseChatOptimisticApplyGuard<'a> {
    repository: &'a RiseChatRepository,
    pending: Option<RiseChatPendingMutation>,
}

impl Drop for RiseChatOptimisticApplyGuard<'_> {
    fn drop(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        self.repository.state.with(|state| {
            state.pending_mutations_by_id.remove(&pending.token.id);
            state.pending_mutation_by_chat.remove(&pending.token.chat_local_id);
        });
        self.repository.restore_projection(&pending.previous);
        self.repository.state.with(|state| state.projected_aggregates.clear());
    }
}

/// guard around a rollback write, puts back the projection the failed write replaced
struct RiseChatProjectionRestoreGuard<'a> {
    repository: &'a RiseChatRepository,
    current: Option<RiseChatStoredEntity>,
}

impl Drop for RiseChatProjectionRestoreGuard<'_> {
    fn drop(&mut self) {
        let Some(current) = self.current.take() else {
            return;
        };
        let aliases = RiseConversationStoreLayout::chat_aliases(&current.record.wire);
        self.repository.install_projection(&current, aliases, true);
        self.repository.state.with(|state| state.projected_aggregates.clear());
    }
}

fn wire_updates<const N: usize>(entries: [(&str, RiseJSONValue); N]) -> BTreeMap<String, RiseJSONValue> {
    entries.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

impl RiseChatRepository {
    /// applies the local change and records enough state for an exact rollback
    /// only one in flight optimistic mutation per chat so two failures cant undo each other
    pub async fn apply_optimistic(
        &self,
        chat_id: &str,
        operation: RiseChatOptimisticOperation,
    ) -> anyhow::Result<RiseChatOptimisticToken> {
        let prepared = self.prepare_optimistic(chat_id, operation).await?;
        let token = prepared.pending.token.clone();
        self.register_pending(prepared.pending.clone());
        let mut guard = RiseChatOptimisticApplyGuard { repository: self, pending: Some(prepared.pending) };
        self.engine.write(prepared.batch).await?;
        guard.pending = None;
        Ok(token)
    }

    /// batch only hook for a parent domain that composes this with another repo in one transaction
    /// use apply_optimistic if no composing is needed
    pub async fn make_optimistic_batch(
        &self,
        chat_id: &str,
        operation: RiseChatOptimisticOperation,
    ) -> anyhow::Result<(RiseChatOptimisticToken, RiseStoreWriteBatch)> {
        let prepared = self.prepare_optimistic(chat_id, operation).await?;
        let token = prepared.pending.token.clone();
        self.register_pending(prepared.pending);
        Ok((token, prepared.batch))
    }

    /// read goes into the message reducer transaction so it stays silent when there is nothing to clear
    /// a no op token would block the single pending slot of the chat
    pub async fn make_read_optimistic_batch(
        &self,
        chat_id: &str,
    ) -> anyhow::Result<Option<(RiseChatOptimisticToken, RiseStoreWriteBatch)>> {
        if !self.admits_read(chat_id).await? {
            return Ok(None);
        }
        Ok(Some(self.make_optimistic_batch(chat_id, RiseChatOptimisticOperation::Read).await?))
    }

    /// topic scoped variant, clearing is the local estimate of what this read covers
    /// clamped to the current count so the badge only shrinks
    pub async fn make_topic_read_optimistic_batch(
        &self,
        chat_id: &str,
        clearing: i64,
    ) -> anyhow::Result<Option<(RiseChatOptimisticToken, RiseStoreWriteBatch)>> {
        if clearing <= 0 || !self.admits_read(chat_id).await? {
            return Ok(None);
        }
        Ok(Some(
            self.make_optimistic_batch(chat_id, RiseChatOptimisticOperation::ReadTopic { clearing }).await?,
        ))
    }

    /// call only when a parent transaction holding a make_optimistic_batch batch failed before commit
    pub fn discard_prepared_optimistic(&self, token: &RiseChatOptimisticToken) {
        let Some(pending) = self.take_pending(token) else {
            return;
        };
        self.restore_projection(&pending.previous);
        self.state.with(|state| state.projected_aggregates.clear());
    }

    /// resolves a successful request
    /// an optional authoritative row turns temp into server ack, local id stays
    pub async fn confirm_optimistic(&self, token: &RiseChatOptimisticToken) -> anyhow::Result<()> {
        self.confirm_optimistic_authoritative_chat(token, None).await
    }

    pub async fn confirm_optimistic_authoritative_chat(
        &self,
        token: &RiseChatOptimisticToken,
        authoritative_chat: Option<RiseStoredChat>,
    ) -> anyhow::Result<()> {
        let Some(pending) = self.take_pending(token) else {
            return Ok(());
        };
        let Some(current) = self.stored_chat_local_id(&token.chat_local_id).await? else {
            return Ok(());
        };

        let Some(authoritative_chat) = authoritative_chat else {
            let confirmed_tombstone = match pending.operation {
                RiseChatOptimisticOperation::Delete => Some(RiseChatTombstone::ConfirmedDeleted),
                RiseChatOptimisticOperation::Leave => Some(RiseChatTombstone::ConfirmedLeft),
                _ => None,
            };
            if let Some(confirmed_tombstone) = confirmed_tombstone {
                let batch = self.make_tombstone_batch(&current, confirmed_tombstone, self.next_version())?;
                self.engine.write(batch).await?;
            }
            return Ok(());
        };

        let authoritative_chat = if authoritative_chat.last_message.is_none()
            && current.record.tombstone.is_none()
            && current.record.wire.last_message.is_some()
        {
            let kept = RiseChatRepository::keeping_stored_preview(&authoritative_chat, &current.record.wire)?;
            RiseChatRepository::keeping_stored_unread_count(kept, &current.record.wire)?
        } else {
            authoritative_chat
        };
        let version = self.next_version();
        let aliases = RiseConversationStoreLayout::chat_aliases(&authoritative_chat);
        let mut duplicate_ids = HashSet::new();
        for alias in &aliases {
            let duplicate =
                self.engine.entity_alias(RiseConversationStoreLayout::chats(), alias.clone()).await?;
            if let Some(duplicate) = duplicate {
                if duplicate.local_id != current.entity.local_id {
                    duplicate_ids.insert(duplicate.local_id);
                }
            }
        }
        let kind = current.record.list_kind.or(pending.previous_kind);
        let clears_tombstone = current.record.tombstone.is_none();
        let record = RiseChatEntityRecord {
            wire: authoritative_chat,
            source: RiseConversationEntitySource::Acknowledgement,
            locally_created: false,
            list_kind: kind,
            tombstone: current.record.tombstone,
        };
        let batch = self.make_upsert_batch(
            record,
            &current.entity.local_id,
            aliases,
            kind,
            version,
            clears_tombstone,
            duplicate_ids.into_iter().collect(),
        )?;
        self.engine.write(batch).await?;
        Ok(())
    }

    /// versioned rollback, no op if a realtime event already replaced the optimistic version
    /// for read the cleared unread count is added back so newer messages survive
    pub async fn rollback_optimistic(&self, token: &RiseChatOptimisticToken) -> anyhow::Result<()> {
        let Some(pending) = self.take_pending(token) else {
            return Ok(());
        };
        let Some(current) = self.stored_chat_local_id(&token.chat_local_id).await? else {
            return Ok(());
        };

        if pending.operation.is_read() {
            return self.rollback_read(&pending, current).await;
        }

        if current.entity.version != token.applied_version {
            return Ok(());
        }
        let rollback_version = self.next_version();
        let namespace = RiseConversationStoreLayout::chats();
        let local_id = pending.previous.entity.local_id.clone();
        let restored_entity = RiseStoreEntity::new(
            local_id.clone(),
            pending.previous.entity.data.clone(),
            rollback_version,
            Self::now_milliseconds(),
        );
        let previous_aliases = RiseConversationStoreLayout::chat_aliases(&pending.previous.record.wire);
        let mut mutations = vec![RiseStoreMutation::UpsertEntity {
            namespace: namespace.clone(),
            entity: restored_entity.clone(),
            policy: RiseEntityWritePolicy::ServerAuthoritative,
        }];
        for alias in &previous_aliases {
            mutations.push(RiseStoreMutation::SetEntityAlias {
                namespace: namespace.clone(),
                alias: alias.clone(),
                local_id: local_id.clone(),
            });
        }
        for view in RiseConversationStoreLayoutChatView::all_cases() {
            mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                namespace: namespace.clone(),
                view: view.raw_value().to_owned(),
                local_id: local_id.clone(),
                if_version_at_most: Some(rollback_version),
            });
        }
        mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
            namespace: namespace.clone(),
            view: Self::TOMBSTONE_VIEW.to_owned(),
            local_id: local_id.clone(),
            if_version_at_most: Some(rollback_version),
        });
        if let Some(previous_kind) = pending.previous_kind {
            let position_key = match pending.previous_position.as_ref() {
                Some(position) => position.position_key.clone(),
                None => RiseConversationStoreLayout::chat_position(&pending.previous.record.wire),
            };
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: namespace.clone(),
                view: previous_kind.store_view().raw_value().to_owned(),
                membership: RiseEntityViewMembership::new(local_id.clone(), position_key, rollback_version),
            });
        }

        let negated = Self::aggregate_deltas(&pending)
            .into_iter()
            .map(|(kind, delta)| (kind, delta.negated()))
            .collect();
        let batch = self
            .merging_aggregates(RiseStoreWriteBatch::new(mutations, HashSet::new()), rollback_version, negated)
            .await?;
        self.install_projection(
            &RiseChatStoredEntity { entity: restored_entity, record: pending.previous.record.clone() },
            previous_aliases,
            true,
        );
        let mut guard = RiseChatProjectionRestoreGuard { repository: self, current: Some(current) };
        self.engine.write(batch).await?;
        guard.current = None;
        Ok(())
    }

    async fn rollback_read(
        &self,
        pending: &RiseChatPendingMutation,
        current: RiseChatStoredEntity,
    ) -> anyhow::Result<()> {
        if pending.cleared_unread_count <= 0 {
            return Ok(());
        }
        let current_unread = current.record.wire.resolved_unread_count();
        let restored_unread = current_unread + pending.cleared_unread_count;
        let restored_media = current.record.wire.resolved_unread_media_count() + pending.cleared_unread_media_count;
        let wire = RiseChatWireCodec::patch(
            &current.record.wire,
            wire_updates([
                (RiseNormalizedKey::unread_count(), RiseJSONValue::Integer(restored_unread)),
                (RiseNormalizedKey::unread_media_count(), RiseJSONValue::Integer(restored_media.min(restored_unread))),
            ]),
        )?;
        let version = self.next_version();
        let aliases = RiseConversationStoreLayout::chat_aliases(&wire);
        let list_kind = current.record.list_kind;
        let counts_toward_badge = wire.counts_toward_unread_badge();
        let mut batch = self.make_upsert_batch(
            RiseChatEntityRecord {
                wire,
                source: RiseConversationEntitySource::Optimistic,
                locally_created: current.record.locally_created,
                list_kind,
                tombstone: current.record.tombstone,
            },
            &current.entity.local_id,
            aliases,
            list_kind,
            version,
            false,
            Vec::new(),
        )?;
        if let Some(kind) = list_kind {
            if kind == RiseChatListKind::Archive || (kind == RiseChatListKind::Inbox && counts_toward_badge) {
                let delta = RiseChatAggregateDelta {
                    total: 0,
                    unread: pending.cleared_unread_count,
                    unread_chats: if current_unread == 0 { 1 } else { 0 },
                    unread_media: pending.cleared_unread_media_count,
                };
                batch = self.merging_aggregates(batch, version, vec![(kind, delta)]).await?;
            }
        }
        let mut guard = RiseChatProjectionRestoreGuard { repository: self, current: Some(current) };
        self.engine.write(batch).await?;
        guard.current = None;
        Ok(())
    }

    async fn admits_read(&self, chat_id: &str) -> anyhow::Result<bool> {
        let Some(stored) = self.stored_chat_id(chat_id, false).await? else {
            return Ok(false);
        };
        Ok(stored.record.tombstone.is_none()
            && stored.record.wire.resolved_unread_count() > 0
            && !self.state.with(|state| state.pending_mutation_by_chat.contains_key(&stored.entity.local_id)))
    }

    fn register_pending(&self, pending: RiseChatPendingMutation) {
        self.state.with(|state| {
            state.pending_mutation_by_chat.insert(pending.token.chat_local_id.clone(), pending.token.id);
            state.pending_mutations_by_id.insert(pending.token.id, pending);
        });
    }

    fn take_pending(&self, token: &RiseChatOptimisticToken) -> Option<RiseChatPendingMutation> {
        self.state.with(|state| {
            let pending = state.pending_mutations_by_id.remove(&token.id)?;
            if pending.token != *token {
                return None;
            }
            state.pending_mutation_by_chat.remove(&token.chat_local_id);
            Some(pending)
        })
    }

    async fn prepare_optimistic(
        &self,
        chat_id: &str,
        operation: RiseChatOptimisticOperation,
    ) -> anyhow::Result<RiseChatRepositoryPreparedOptimisticMutation> {
        let Some(stored) = self.stored_chat_id(chat_id, false).await? else {
            return Err(RiseChatRepositoryError::ChatNotFound(chat_id.to_owned()).into());
        };
        if stored.record.tombstone.is_some() {
            return Err(RiseChatRepositoryError::ChatIsTombstoned(chat_id.to_owned()).into());
        }
        if self.state.with(|state| state.pending_mutation_by_chat.contains_key(&stored.entity.local_id)) {
            return Err(RiseChatRepositoryError::OptimisticMutationAlreadyPending(chat_id.to_owned()).into());
        }
        let (previous_kind, previous_position) = self.current_location(&stored).await?;
        let previous_unread = stored.record.wire.resolved_unread_count();
        let previous_media = stored.record.wire.resolved_unread_media_count();
        let (cleared_unread, cleared_media) = match &operation {
            RiseChatOptimisticOperation::Read => (previous_unread, previous_media),
            RiseChatOptimisticOperation::ReadTopic { clearing } => {
                let cleared_unread = previous_unread.min((*clearing).max(0));
                (cleared_unread, (previous_media - (previous_unread - cleared_unread)).max(0))
            }
            _ => (0, 0),
        };
        let version = self.next_version();
        let token = RiseChatOptimisticToken::new(Uuid::new_v4(), stored.entity.local_id.clone(), version);

        let batch = match &operation {
            RiseChatOptimisticOperation::Delete => {
                self.make_tombstone_batch(&stored, RiseChatTombstone::PendingDelete, version)?
            }
            RiseChatOptimisticOperation::Leave => {
                self.make_tombstone_batch(&stored, RiseChatTombstone::PendingLeave, version)?
            }
            _ => {
                let transformed = Self::optimistic_wire(&stored.record.wire, &operation)?;
                let destination = match &operation {
                    RiseChatOptimisticOperation::Archive => Some(RiseChatListKind::Archive),
                    RiseChatOptimisticOperation::Unarchive => Some(RiseChatListKind::Inbox),
                    _ => previous_kind,
                };
                let aliases = RiseConversationStoreLayout::chat_aliases(&transformed);
                self.make_upsert_batch(
                    RiseChatEntityRecord {
                        wire: transformed,
                        source: RiseConversationEntitySource::Optimistic,
                        locally_created: stored.record.locally_created,
                        list_kind: destination,
                        tombstone: None,
                    },
                    &stored.entity.local_id,
                    aliases,
                    destination,
                    version,
                    false,
                    Vec::new(),
                )?
            }
        };

        let previous_membership_version = Some(stored.entity.version);
        let pending = RiseChatPendingMutation {
            token,
            operation,
            previous: stored,
            previous_kind,
            previous_position,
            previous_membership_version,
            cleared_unread_count: cleared_unread,
            cleared_unread_media_count: cleared_media,
        };
        let batch = self.merging_aggregates(batch, version, Self::aggregate_deltas(&pending)).await?;
        Ok(RiseChatRepositoryPreparedOptimisticMutation { pending, batch })
    }

    async fn current_location(
        &self,
        stored: &RiseChatStoredEntity,
    ) -> anyhow::Result<(Option<RiseChatListKind>, Option<RiseEntityViewPosition>)> {
        if let Some(kind) = stored.record.list_kind {
            let position = self
                .engine
                .entity_view_position(
                    RiseConversationStoreLayout::chats(),
                    kind.store_view().raw_value().to_owned(),
                    stored.entity.local_id.clone(),
                )
                .await?;
            return Ok((Some(kind), position));
        }
        for kind in RiseChatListKind::ALL_CASES {
            let position = self
                .engine
                .entity_view_position(
                    RiseConversationStoreLayout::chats(),
                    kind.store_view().raw_value().to_owned(),
                    stored.entity.local_id.clone(),
                )
                .await?;
            if position.is_some() {
                return Ok((Some(kind), position));
            }
        }
        Ok((None, None))
    }

    fn optimistic_wire(
        wire: &RiseStoredChat,
        operation: &RiseChatOptimisticOperation,
    ) -> anyhow::Result<RiseStoredChat> {
        match operation {
            RiseChatOptimisticOperation::Archive => RiseChatWireCodec::patch(
                wire,
                wire_updates([
                    (RiseNormalizedKey::is_pinned(), RiseJSONValue::Bool(false)),
                    (RiseNormalizedKey::pin_position(), RiseJSONValue::Null),
                ]),
            ),
            RiseChatOptimisticOperation::Unarchive
            | RiseChatOptimisticOperation::Delete
            | RiseChatOptimisticOperation::Leave => Ok(wire.clone()),
            RiseChatOptimisticOperation::Pin(pinned) => RiseChatWireCodec::patch(
                wire,
                wire_updates([
                    (RiseNormalizedKey::is_pinned(), RiseJSONValue::Bool(*pinned)),
                    (
                        RiseNormalizedKey::pin_position(),
                        if *pinned {
                            RiseJSONValue::Integer(wire.pin_position.unwrap_or(0))
                        } else {
                            RiseJSONValue::Null
                        },
                    ),
                ]),
            ),
            RiseChatOptimisticOperation::ReorderPinned { position } => RiseChatWireCodec::patch(
                wire,
                wire_updates([
                    (RiseNormalizedKey::is_pinned(), RiseJSONValue::Bool(position.is_some())),
                    (
                        RiseNormalizedKey::pin_position(),
                        position.map_or(RiseJSONValue::Null, RiseJSONValue::Integer),
                    ),
                ]),
            ),
            RiseChatOptimisticOperation::Mute { is_muted, until } => {
                let scopes = if *is_muted {
                    ChatMuteScopes::MUTED.encoded_json().map_or(RiseJSONValue::Null, RiseJSONValue::String)
                } else {
                    RiseJSONValue::Null
                };
                RiseChatWireCodec::patch(
                    wire,
                    wire_updates([
                        (RiseNormalizedKey::is_muted(), RiseJSONValue::Bool(*is_muted)),
                        (RiseNormalizedKey::notifications_enabled(), RiseJSONValue::Bool(!*is_muted)),
                        (RiseNormalizedKey::mute_until(), until.map_or(RiseJSONValue::Null, RiseJSONValue::Number)),
                        (RiseNormalizedKey::mute_scopes(), scopes),
                    ]),
                )
            }
            RiseChatOptimisticOperation::Notifications(preference) => {
                let scopes = if preference.is_all_enabled() {
                    RiseJSONValue::Null
                } else {
                    preference.encoded_json().map_or(RiseJSONValue::Null, RiseJSONValue::String)
                };
                RiseChatWireCodec::patch(
                    wire,
                    wire_updates([
                        (RiseNormalizedKey::is_muted(), RiseJSONValue::Bool(preference.is_fully_muted())),
                        (
                            RiseNormalizedKey::notifications_enabled(),
                            RiseJSONValue::Bool(!preference.is_fully_muted()),
                        ),
                        (RiseNormalizedKey::mute_scopes(), scopes),
                    ]),
                )
            }
            RiseChatOptimisticOperation::Patch(values) => RiseChatWireCodec::patch(wire, values.clone()),
            RiseChatOptimisticOperation::Read => RiseChatWireCodec::patch(
                wire,
                wire_updates([
                    (RiseNormalizedKey::unread_count(), RiseJSONValue::Integer(0)),
                    (RiseNormalizedKey::unread_media_count(), RiseJSONValue::Integer(0)),
                ]),
            ),
            RiseChatOptimisticOperation::ReadTopic { clearing } => {
                let remaining = (wire.resolved_unread_count() - (*clearing).max(0)).max(0);
                RiseChatWireCodec::patch(
                    wire,
                    wire_updates([
                        (RiseNormalizedKey::unread_count(), RiseJSONValue::Integer(remaining)),
                        (
                            RiseNormalizedKey::unread_media_count(),
                            RiseJSONValue::Integer(wire.resolved_unread_media_count().min(remaining)),
                        ),
                    ]),
                )
            }
        }
    }

    /// what applying the pending change does to each list wide aggregate
    /// archive counts every row, the inbox badge only rows with notifications on so a mute moves it too
    fn aggregate_deltas(pending: &RiseChatPendingMutation) -> Vec<(RiseChatListKind, RiseChatAggregateDelta)> {
        let wire = &pending.previous.record.wire;
        let unread = wire.resolved_unread_count();
        let previous_kind = pending.previous_kind;
        let inbox_counts = previous_kind == Some(RiseChatListKind::Inbox) && wire.counts_toward_unread_badge();
        match &pending.operation {
            RiseChatOptimisticOperation::Unarchive
            | RiseChatOptimisticOperation::Delete
            | RiseChatOptimisticOperation::Leave
                if previous_kind == Some(RiseChatListKind::Requests) =>
            {
                vec![(RiseChatListKind::Requests, RiseChatAggregateDelta { total: -1, ..RiseChatAggregateDelta::zero() })]
            }
            RiseChatOptimisticOperation::Archive if previous_kind != Some(RiseChatListKind::Archive) => {
                let mut deltas =
                    vec![(RiseChatListKind::Archive, RiseChatAggregateDelta::row(wire, 1, true))];
                if inbox_counts {
                    deltas.push((RiseChatListKind::Inbox, RiseChatAggregateDelta::row(wire, -1, false)));
                }
                deltas
            }
            RiseChatOptimisticOperation::Unarchive if previous_kind == Some(RiseChatListKind::Archive) => {
                let mut deltas =
                    vec![(RiseChatListKind::Archive, RiseChatAggregateDelta::row(wire, -1, true))];
                if wire.counts_toward_unread_badge() {
                    deltas.push((RiseChatListKind::Inbox, RiseChatAggregateDelta::row(wire, 1, false)));
                }
                deltas
            }
            RiseChatOptimisticOperation::Delete | RiseChatOptimisticOperation::Leave => {
                if previous_kind == Some(RiseChatListKind::Archive) {
                    return vec![(RiseChatListKind::Archive, RiseChatAggregateDelta::row(wire, -1, true))];
                }
                if inbox_counts {
                    vec![(RiseChatListKind::Inbox, RiseChatAggregateDelta::row(wire, -1, false))]
                } else {
                    Vec::new()
                }
            }
            RiseChatOptimisticOperation::Read | RiseChatOptimisticOperation::ReadTopic { .. } => {
                let cleared = pending.cleared_unread_count;
                if cleared <= 0 {
                    return Vec::new();
                }
                let delta = RiseChatAggregateDelta {
                    total: 0,
                    unread: -cleared,
                    unread_chats: if unread - cleared == 0 { -1 } else { 0 },
                    unread_media: -pending.cleared_unread_media_count,
                };
                if previous_kind == Some(RiseChatListKind::Archive) {
                    return vec![(RiseChatListKind::Archive, delta)];
                }
                if inbox_counts { vec![(RiseChatListKind::Inbox, delta)] } else { Vec::new() }
            }
            RiseChatOptimisticOperation::Mute { .. } | RiseChatOptimisticOperation::Notifications(_) => {
                if previous_kind != Some(RiseChatListKind::Inbox) || unread <= 0 {
                    return Vec::new();
                }
                let Ok(transformed) = Self::optimistic_wire(wire, &pending.operation) else {
                    return Vec::new();
                };
                if wire.counts_toward_unread_badge() == transformed.counts_toward_unread_badge() {
                    return Vec::new();
                }
                let sign = if transformed.counts_toward_unread_badge() { 1 } else { -1 };
                vec![(RiseChatListKind::Inbox, RiseChatAggregateDelta::row(wire, sign, false))]
            }
            _ => Vec::new(),
        }
    }
}
