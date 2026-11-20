use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};

use futures::FutureExt;
use gpui::AsyncApp;
use rise_canon::rise_key;
use rise_core::foundation::foundation_date::Date;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RiseEntityViewMembership, RiseEntityViewRetention, RiseEntityWindow,
    RiseEntityWindowQuery, RiseEntityWritePolicy, RisePositionKey, RisePositionKeyOrder, RiseStoreEntity,
};
use rise_engine::rise_store::rise_store_models::{RiseStoreMutation, RiseStoreWriteBatch};
use rise_engine::rise_sync::RiseSyncPushHandler;
use rise_engine::rise_upload::rise_upload_progress::RiseUploadRealtimeProgress;
use rise_engine::rise_wire::rise_wire_contracts::RiseWireTimestamp;
use rise_engine::{RiseJSONValue, RiseWirePushEnvelope};
use rise_events::name;
use rise_i18n::date_format::iso8601_date_formatter::ISO8601DateFormatter;

use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseColdOpenTrace;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::chat_engine_hooks::chat_engine_hooks;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseConversationEntitySource, RiseMessageEntityRecord, RiseMessageLocalPhase,
};
use crate::modules::chat::engine::core::rise_conversation_store_layout::RiseConversationStoreLayout;
use crate::modules::chat::engine::messages::rise_message_contracts::{
    RiseMessageAuxiliaryInvalidation, RiseMessageAuxiliaryInvalidationPinnedMessagePatch,
    RiseMessageAuxiliaryProjection, RiseMessagePreviewChange, RiseMessageReadReceiptHint,
};
use crate::modules::chat::engine::messages::rise_message_repository::{
    RiseMessageRepository, RiseMessageRepositoryEncodedRow,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_types::MessageContentType;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    CommenterPreviewWireDTO, MessageWireDTO,
};

const PROGRESS_FALLBACK_KEY: &str = "progress";
const TIMESTAMP_FALLBACK_KEY: &str = "timestamp";
const ID_FALLBACK_KEY: &str = "id";
const NAME_FALLBACK_KEY: &str = "name";
const SERVER_ID_KEY: &str = "server_id";
const QUOTING_PROBE_LIMIT: usize = 16;
const RECENT_COMMENTERS_LIMIT: usize = 3;

static OWN_REACTION_INTENTS: LazyLock<Mutex<HashMap<String, HashMap<String, bool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// own reaction picks still in flight, by server message id
pub enum RiseOwnReactionIntents {}

impl RiseOwnReactionIntents {
    pub fn set(server_message_id: &str, reaction: &str, wanted: bool) {
        if server_message_id.is_empty() {
            return;
        }
        let mut intents = OWN_REACTION_INTENTS.lock().unwrap_or_else(PoisonError::into_inner);
        intents.entry(server_message_id.to_owned()).or_default().insert(reaction.to_owned(), wanted);
    }

    pub fn clear(server_message_id: &str, reaction: &str) {
        let mut intents = OWN_REACTION_INTENTS.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(picks) = intents.get_mut(server_message_id) else {
            return;
        };
        picks.remove(reaction);
        if picks.is_empty() {
            intents.remove(server_message_id);
        }
    }

    pub fn clear_all() {
        OWN_REACTION_INTENTS.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }

    fn pending(server_message_id: &str) -> Vec<(String, bool)> {
        OWN_REACTION_INTENTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(server_message_id)
            .map(|picks| picks.iter().map(|(reaction, wanted)| (reaction.clone(), *wanted)).collect())
            .unwrap_or_default()
    }

    fn keep(
        wire: MessageWireDTO,
        intents: &[(String, bool)],
        account_id: &str,
        own_reaction_limit: Option<i64>,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut wire = wire;
        for (reaction, wanted) in intents {
            let is_mine = wire
                .reactions
                .as_ref()
                .and_then(|reactions| reactions.iter().find(|candidate| &candidate.reaction == reaction))
                .is_some_and(|candidate| candidate.reacted_by_you == Some(true));
            if is_mine == *wanted {
                continue;
            }
            let payload = BTreeMap::from([
                (rise_key::events::REACTION.to_owned(), RiseJSONValue::String(reaction.clone())),
                (rise_key::events::USER_ID.to_owned(), RiseJSONValue::String(account_id.to_owned())),
            ]);
            wire = RiseMessageWireCodec::applying_reaction(&wire, &payload, account_id, *wanted, own_reaction_limit)?;
        }
        Ok(wire)
    }
}

/// raw values must match the canon wire names
/// they are the canon constants and the set is checked at startup
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMessageRepositoryRealtimeEvent {
    Created,
    Edited,
    LinkPreviewResolved,
    Deleted,
    Pinned,
    Unpinned,
    Read,
    Typing,
    MediaPlayed,
    PollUpdated,
    ReactionAdded,
    ReactionRemoved,
    CommentsUpdated,
    SparksUpdated,
    ViewCountChanged,
    UploadProgress,
}

impl RiseMessageRepositoryRealtimeEvent {
    pub fn all_cases() -> &'static [Self] {
        &[
            Self::Created,
            Self::Edited,
            Self::LinkPreviewResolved,
            Self::Deleted,
            Self::Pinned,
            Self::Unpinned,
            Self::Read,
            Self::Typing,
            Self::MediaPlayed,
            Self::PollUpdated,
            Self::ReactionAdded,
            Self::ReactionRemoved,
            Self::CommentsUpdated,
            Self::SparksUpdated,
            Self::ViewCountChanged,
            Self::UploadProgress,
        ]
    }

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Created => name::MESSAGE_CREATED,
            Self::Edited => name::MESSAGE_EDITED,
            Self::LinkPreviewResolved => name::MESSAGE_LINK_PREVIEW_RESOLVED,
            Self::Deleted => name::MESSAGE_DELETED,
            Self::Pinned => name::MESSAGE_PINNED,
            Self::Unpinned => name::MESSAGE_UNPINNED,
            Self::Read => name::CHAT_READ,
            Self::Typing => name::MESSAGE_TYPING,
            Self::MediaPlayed => name::MEDIA_PLAYED,
            Self::PollUpdated => name::POLL_UPDATED,
            Self::ReactionAdded => name::REACTION_ADDED,
            Self::ReactionRemoved => name::REACTION_REMOVED,
            Self::CommentsUpdated => name::COMMENTS_COUNT_UPDATED,
            Self::SparksUpdated => name::MESSAGE_SPARKS_UPDATED,
            Self::ViewCountChanged => name::MESSAGE_VIEW_COUNT_CHANGED,
            Self::UploadProgress => name::UPLOAD_PROGRESS,
        }
    }
}

struct RiseMessageDecodedCacheEvictionGuard<'a>(&'a RiseMessageRepository);

impl Drop for RiseMessageDecodedCacheEvictionGuard<'_> {
    fn drop(&mut self) {
        self.0.evict_decoded_cache_if_needed();
    }
}

/// true for optimistic or ack sources, rows with a client request id or a non remote phase
fn is_local_submission(record: &RiseMessageEntityRecord) -> bool {
    record.source == RiseConversationEntitySource::Optimistic
        || record.source == RiseConversationEntitySource::Acknowledgement
        || record.client_request_id.is_some()
        || record.local_phase != RiseMessageLocalPhase::Remote
}

fn sender_id_of(wire: &MessageWireDTO) -> Option<&str> {
    wire.sender_id.as_deref().or_else(|| wire.sender.as_ref().and_then(|sender| sender.id.as_deref()))
}

fn is_own_auto_forward(object: &BTreeMap<String, RiseJSONValue>) -> bool {
    let flag = |metadata: &BTreeMap<String, RiseJSONValue>| {
        matches!(metadata.get(rise_key::payload::OWN_AUTO_FORWARD), Some(RiseJSONValue::Bool(true)))
    };
    match RiseMessageWireCodec::value(object, &[rise_key::message::METADATA]) {
        Some(RiseJSONValue::Object(metadata)) => flag(metadata),
        Some(RiseJSONValue::String(raw)) => match RiseJSONValue::decode(raw.as_bytes()) {
            Ok(RiseJSONValue::Object(metadata)) => flag(&metadata),
            _ => false,
        },
        _ => false,
    }
}

/// patched copy of old where only the wire and the source change
fn realtime_record(old: &RiseMessageEntityRecord, wire: MessageWireDTO) -> RiseMessageEntityRecord {
    RiseMessageEntityRecord {
        wire,
        source: RiseConversationEntitySource::Realtime,
        local_phase: old.local_phase,
        client_request_id: old.client_request_id.clone(),
        last_error_code: old.last_error_code.clone(),
        scope_storage_key: old.scope_storage_key.clone(),
        is_replayable: old.is_replayable,
        upload_retry: old.upload_retry.clone(),
        forward_retry: old.forward_retry.clone(),
    }
}

impl RiseMessageRepository {
    pub const TOMBSTONE_VIEW: &'static str = "message.tombstones.v1";

    pub fn realtime_push_handlers(&self) -> Vec<RiseSyncPushHandler> {
        RiseMessageRepositoryRealtimeEvent::all_cases()
            .iter()
            .map(|&event| {
                let repository = self.this.clone();
                RiseSyncPushHandler::new(event.raw_value(), move |push: RiseWirePushEnvelope| {
                    let repository = repository.clone();
                    async move {
                        let Some(repository) = repository.upgrade() else {
                            return Ok(None);
                        };
                        repository.reduce(&push, event).await
                    }
                })
            })
            .collect()
    }

    pub async fn reduce(
        &self,
        push: &RiseWirePushEnvelope,
        event: RiseMessageRepositoryRealtimeEvent,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let payload = push.data.object_value().cloned().unwrap_or_default();
        let root = RiseMessageWireCodec::canon_root(push, &payload);
        if event == RiseMessageRepositoryRealtimeEvent::Typing {
            self.reduce_typing(&root).await;
            return Ok(None);
        }
        if event == RiseMessageRepositoryRealtimeEvent::UploadProgress {
            self.reduce_upload_progress(&root, &push.extras).await;
            return Ok(None);
        }

        let version = self.next_version();
        match event {
            RiseMessageRepositoryRealtimeEvent::Created => self.reduce_created(push, &root, version, false).await,
            RiseMessageRepositoryRealtimeEvent::Edited => {
                if let Some(restored) = self.restored_message_root(&root).await? {
                    return self.reduce_created(push, &restored, version, true).await;
                }
                self.reduce_patched_message(
                    &root,
                    version,
                    RiseMessageAuxiliaryProjection::all(),
                    None,
                    true,
                    true,
                    false,
                    |existing: &MessageWireDTO| RiseMessageWireCodec::editing(existing, &root, true),
                )
                .await
            }
            RiseMessageRepositoryRealtimeEvent::LinkPreviewResolved => {
                if let Some(identifier) = self.message_id(&root)
                    && self.message_entity(&identifier).await?.is_none()
                {
                    let pending = self.state.with(|state| state.reduced_created_rows.contains_key(&identifier));
                    if pending {
                        return self
                            .reduce_patched_message(
                                &root,
                                version,
                                HashSet::new(),
                                None,
                                false,
                                false,
                                true,
                                |existing: &MessageWireDTO| RiseMessageWireCodec::resolving_link_preview(existing, &root),
                            )
                            .await;
                    }
                    self.hold_early_link_preview(&root, &identifier);
                    return Ok(None);
                }
                self.reduce_patched_message(
                    &root,
                    version,
                    HashSet::new(),
                    None,
                    false,
                    false,
                    false,
                    |existing: &MessageWireDTO| RiseMessageWireCodec::resolving_link_preview(existing, &root),
                )
                .await
            }
            RiseMessageRepositoryRealtimeEvent::Deleted => {
                self.reduce_deleted(&root, version, RiseMessageAuxiliaryProjection::all()).await
            }
            RiseMessageRepositoryRealtimeEvent::Pinned => {
                self.reduce_patched_message(
                    &root,
                    version,
                    RiseMessageAuxiliaryProjection::pin_state(),
                    Some(true),
                    false,
                    false,
                    false,
                    |existing: &MessageWireDTO| {
                        RiseMessageWireCodec::setting_boolean(existing, rise_key::message::IS_PINNED, true)
                    },
                )
                .await
            }
            RiseMessageRepositoryRealtimeEvent::Unpinned => {
                self.reduce_patched_message(
                    &root,
                    version,
                    RiseMessageAuxiliaryProjection::pin_state(),
                    Some(false),
                    false,
                    false,
                    false,
                    |existing: &MessageWireDTO| {
                        RiseMessageWireCodec::setting_boolean(existing, rise_key::message::IS_PINNED, false)
                    },
                )
                .await
            }
            RiseMessageRepositoryRealtimeEvent::Read => self.reduce_read(&root, version).await,
            RiseMessageRepositoryRealtimeEvent::MediaPlayed => {
                self.reduce_patched_message(
                    &root,
                    version,
                    HashSet::new(),
                    None,
                    false,
                    false,
                    false,
                    |existing: &MessageWireDTO| RiseMessageWireCodec::merging_played_by(existing, &root),
                )
                .await
            }
            RiseMessageRepositoryRealtimeEvent::PollUpdated => {
                let Some(content) = RiseMessageWireCodec::poll_content(&root) else {
                    return Ok(None);
                };
                self.reduce_patched_message(
                    &root,
                    version,
                    HashSet::new(),
                    None,
                    false,
                    false,
                    false,
                    |existing: &MessageWireDTO| RiseMessageWireCodec::setting_poll(existing, &content, true),
                )
                .await
            }
            RiseMessageRepositoryRealtimeEvent::ReactionAdded => {
                let own_reaction_limit = self.reaction_limit_for_active_account().await;
                let leaves_unnamed_reactor = AtomicBool::new(false);
                let account_id = self.account_id.as_str();
                let intents = self.message_id(&root).map(|id| RiseOwnReactionIntents::pending(&id)).unwrap_or_default();
                let batch = self
                    .reduce_patched_message(
                        &root,
                        version,
                        HashSet::new(),
                        None,
                        false,
                        true,
                        false,
                        |existing: &MessageWireDTO| {
                            let patched = RiseMessageWireCodec::applying_reaction(
                                existing,
                                &root,
                                account_id,
                                true,
                                own_reaction_limit,
                            )?;
                            let patched =
                                RiseOwnReactionIntents::keep(patched, &intents, account_id, own_reaction_limit)?;
                            let unnamed = RiseMessageWireCodec::has_unnamed_reactor(&patched);
                            leaves_unnamed_reactor.store(unnamed, Ordering::Relaxed);
                            Ok(patched)
                        },
                    )
                    .await?;
                if batch.is_some()
                    && leaves_unnamed_reactor.load(Ordering::Relaxed)
                    && let Some(message_id) = self.message_id(&root)
                {
                    let repository = self.this.clone();
                    RiseAccountRuntime::current().spawn(async move {
                        if let Some(repository) = repository.upgrade() {
                            repository.hydrate_reactor_identities(&message_id).await;
                        }
                    });
                }
                Ok(batch)
            }
            RiseMessageRepositoryRealtimeEvent::ReactionRemoved => {
                let account_id = self.account_id.as_str();
                let intents = self.message_id(&root).map(|id| RiseOwnReactionIntents::pending(&id)).unwrap_or_default();
                let own_reaction_limit =
                    if intents.is_empty() { None } else { self.reaction_limit_for_active_account().await };
                self.reduce_patched_message(
                    &root,
                    version,
                    HashSet::new(),
                    None,
                    false,
                    true,
                    false,
                    |existing: &MessageWireDTO| {
                        let patched = RiseMessageWireCodec::applying_reaction(existing, &root, account_id, false, None)?;
                        RiseOwnReactionIntents::keep(patched, &intents, account_id, own_reaction_limit)
                    },
                )
                .await
            }
            RiseMessageRepositoryRealtimeEvent::CommentsUpdated => {
                self.announce_thread_total(&root).await;
                self.reduce_patched_message(
                    &root,
                    version,
                    HashSet::new(),
                    None,
                    false,
                    false,
                    false,
                    |existing: &MessageWireDTO| RiseMessageWireCodec::setting_comments(existing, &root),
                )
                .await
            }
            RiseMessageRepositoryRealtimeEvent::SparksUpdated => {
                self.reduce_patched_message(
                    &root,
                    version,
                    HashSet::new(),
                    None,
                    false,
                    false,
                    false,
                    |existing: &MessageWireDTO| RiseMessageWireCodec::raising_sparks(existing, &root),
                )
                .await
            }
            RiseMessageRepositoryRealtimeEvent::ViewCountChanged => {
                self.reduce_patched_message(
                    &root,
                    version,
                    HashSet::new(),
                    None,
                    false,
                    false,
                    false,
                    |existing: &MessageWireDTO| RiseMessageWireCodec::raising_view_count(existing, &root),
                )
                .await
            }
            RiseMessageRepositoryRealtimeEvent::Typing | RiseMessageRepositoryRealtimeEvent::UploadProgress => Ok(None),
        }
    }

    pub async fn raise_comments_total(
        &self,
        channel_id: &str,
        post_id: &str,
        count: i64,
        commenters: Vec<CommenterPreviewWireDTO>,
    ) -> anyhow::Result<()> {
        let Some(entity) = self.message_entity(post_id).await? else {
            return Ok(());
        };
        if RiseMessageWireCodec::is_tombstone(&entity.data) {
            return Ok(());
        }
        let stored = RiseMessageWireCodec::record(&entity.data)?;
        if stored.wire.comments_count.unwrap_or(0) >= count {
            return Ok(());
        }
        let mut root: BTreeMap<String, RiseJSONValue> = BTreeMap::new();
        root.insert(rise_key::message::MESSAGE_ID.to_owned(), RiseJSONValue::String(post_id.to_owned()));
        root.insert(rise_key::message::CHAT_ID.to_owned(), RiseJSONValue::String(channel_id.to_owned()));
        let version = self.next_version();
        let batch = self
            .reduce_patched_message(
                &root,
                version,
                HashSet::new(),
                None,
                false,
                false,
                false,
                move |existing: &MessageWireDTO| {
                    let mut next = existing.clone();
                    next.comments_count = Some(count);
                    if !commenters.is_empty() {
                        next.recent_commenters =
                            Some(Self::merged_commenters(&commenters, existing.recent_commenters.as_deref()));
                    }
                    Ok(next)
                },
            )
            .await?;
        if let Some(batch) = batch {
            self.engine.write(batch).await?;
        }
        Ok(())
    }

    fn merged_commenters(
        newest: &[CommenterPreviewWireDTO],
        previous: Option<&[CommenterPreviewWireDTO]>,
    ) -> Vec<CommenterPreviewWireDTO> {
        let previous = previous.unwrap_or_default();
        let mut merged: Vec<CommenterPreviewWireDTO> = Vec::new();
        for commenter in newest.iter().chain(previous.iter()) {
            if merged.len() >= RECENT_COMMENTERS_LIMIT {
                break;
            }
            if merged.iter().any(|known| known.name == commenter.name) {
                continue;
            }
            let mut kept = commenter.clone();
            if kept.logo.as_deref().is_none_or(str::is_empty)
                && let Some(known) = previous
                    .iter()
                    .find(|old| old.name == kept.name && old.logo.as_deref().is_some_and(|logo| !logo.is_empty()))
            {
                kept.logo = known.logo.clone();
            }
            merged.push(kept);
        }
        merged
    }

    async fn announce_thread_total(&self, root: &BTreeMap<String, RiseJSONValue>) {
        let Some(root_id) = RiseMessageWireCodec::string(root, &[rise_key::message::DISCUSSION_MESSAGE_ID]) else {
            return;
        };
        let Some(count) = RiseMessageWireCodec::double(root, &[rise_key::message::COMMENTS_COUNT]) else {
            return;
        };
        if !count.is_finite() || count < 0.0 {
            return;
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        let viewer = AuthServicesStore::viewer_mirror();
                        if viewer.effective_user_id_for_wire.as_deref() != Some(account_id.as_str()) {
                            return;
                        }
                        if let Some(hooks) = chat_engine_hooks() {
                            (hooks.apply_thread_total)(&root_id, count as i64, cx);
                        }
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    async fn reduce_upload_progress(
        &self,
        root: &BTreeMap<String, RiseJSONValue>,
        extras: &BTreeMap<String, RiseJSONValue>,
    ) {
        let Some(upload_id) = RiseMessageWireCodec::string(extras, &[rise_key::events::UPLOAD_ID])
            .or_else(|| RiseMessageWireCodec::string(root, &[rise_key::events::UPLOAD_ID]))
        else {
            return;
        };
        let Some(percentage) =
            RiseMessageWireCodec::double(root, &[rise_key::events::PROGRESS_PERCENTAGE, PROGRESS_FALLBACK_KEY])
        else {
            return;
        };
        if !percentage.is_finite() {
            return;
        }

        self.upload_realtime_progress
            .publish(RiseUploadRealtimeProgress::new(
                upload_id,
                percentage / 100.0,
                RiseMessageWireCodec::string(root, &[rise_key::events::STAGE]),
                RiseMessageWireCodec::string(root, &[rise_key::bot::MESSAGE]),
            ))
            .await;
    }

    pub async fn reduce_created(
        &self,
        push: &RiseWirePushEnvelope,
        root: &BTreeMap<String, RiseJSONValue>,
        version: i64,
        lifts_tombstone: bool,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let mut object = RiseMessageWireCodec::canon_created_object(push)
            .unwrap_or_else(|| RiseMessageWireCodec::push_object(push));
        if RiseMessageWireCodec::value(&object, &[rise_key::message::CREATED_AT, TIMESTAMP_FALLBACK_KEY]).is_none()
            && let Some(timestamp) = Self::push_timestamp(push)
        {
            object.insert(rise_key::bot::CREATED_AT.to_owned(), RiseJSONValue::Number(timestamp));
        }
        let mut authoritative_wire = RiseMessageWireCodec::wire(&object)?;
        if authoritative_wire.forward_info.is_some()
            && authoritative_wire.media_items.as_ref().is_none_or(|items| items.is_empty())
            && authoritative_wire.media_info.is_none()
            && authoritative_wire.content_type.as_deref().is_some_and(|kind| MessageContentType::from_raw_value(kind) != MessageContentType::Text)
        {
            let media_raw = object
                .get(rise_key::message::MEDIA_ITEMS)
                .map(|value| format!("{value:?}").chars().take(600).collect::<String>());
            tracing::warn!(
                target: "riseonly::X1946",
                id = ?authoritative_wire.id,
                content_type = ?authoritative_wire.content_type,
                keys = ?object.keys().collect::<Vec<_>>(),
                media_items = ?media_raw,
                "forwarded media frame without media"
            );
        }
        if is_own_auto_forward(&object) {
            authoritative_wire.sender_id = Some(self.account_id.clone());
        }
        let Some(chat_id) = self
            .normalized(authoritative_wire.chat_id.as_deref())
            .or_else(|| RiseMessageWireCodec::string(root, &[rise_key::message::CHAT_ID]))
        else {
            return Ok(None);
        };

        let authoritative_aliases = RiseConversationStoreLayout::message_aliases(&authoritative_wire);
        let mut existing: Vec<RiseStoreEntity> = Vec::new();
        let mut lifted_tombstones: Vec<String> = Vec::new();
        for alias in &authoritative_aliases {
            if let Some(entity) =
                self.engine.entity_alias(RiseConversationStoreLayout::messages(), alias.clone()).await?
                && !existing.iter().any(|known| known.local_id == entity.local_id)
            {
                if RiseMessageWireCodec::is_tombstone(&entity.data) {
                    if !lifts_tombstone {
                        return Ok(None);
                    }
                    lifted_tombstones.push(entity.local_id.clone());
                }
                self.state.with(|state| state.version_clock = state.version_clock.max(entity.version));
                existing.push(entity);
            }
        }
        let existing_records: Vec<(String, RiseMessageEntityRecord)> = existing
            .iter()
            .filter(|entity| !RiseMessageWireCodec::is_tombstone(&entity.data))
            .filter_map(|entity| {
                RiseMessageWireCodec::record(&entity.data).ok().map(|record| (entity.local_id.clone(), record))
            })
            .collect();
        let local_id = self.preferred_local_id(&existing, &authoritative_wire);
        let local_submission = existing_records
            .iter()
            .find(|(identifier, record)| *identifier == local_id && is_local_submission(record))
            .or_else(|| existing_records.iter().find(|(_, record)| is_local_submission(record)))
            .map(|(_, record)| record);
        let reconciles_local_submission = local_submission.is_some();
        let mut wire = match local_submission {
            Some(submission) => RiseMessageWireCodec::reconciling_authoritative_wire(
                &submission.wire,
                &authoritative_wire,
                &local_id,
            )?,
            None => authoritative_wire,
        };
        if let Some(held) = self.take_early_link_preview(&wire)
            && let Ok(resolved) = RiseMessageWireCodec::resolving_link_preview(&wire, &held)
        {
            wire = resolved;
        }
        let mut aliases = authoritative_aliases;
        let mut alias_set: HashSet<RiseEntityAlias> = aliases.iter().cloned().collect();
        for alias in RiseConversationStoreLayout::message_aliases(&wire) {
            if alias_set.insert(alias.clone()) {
                aliases.push(alias);
            }
        }
        let was_already_reduced = self.register_created_aliases(&aliases);
        let was_already_stored = !existing.is_empty() || was_already_reduced;
        if let Some(sender_id) = self.normalized(sender_id_of(&wire))
            && sender_id != self.account_id
        {
            self.typing_hub
                .update(&self.known_scopes_with_chat(&chat_id), &sender_id, "", false, None, None)
                .await;
        }
        let record = RiseMessageEntityRecord {
            wire: wire.clone(),
            source: RiseConversationEntitySource::Realtime,
            local_phase: RiseMessageLocalPhase::Remote,
            client_request_id: local_submission.and_then(|submission| submission.client_request_id.clone()).or_else(
                || existing_records.iter().find_map(|(_, record)| record.client_request_id.clone()),
            ),
            last_error_code: None,
            scope_storage_key: local_submission.and_then(|submission| submission.scope_storage_key.clone()).or_else(
                || existing_records.iter().find_map(|(_, record)| record.scope_storage_key.clone()),
            ),
            is_replayable: None,
            upload_retry: None,
            forward_retry: None,
        };
        let data = RiseMessageWireCodec::record_data(&record)?;
        let row = RiseMessageRepositoryEncodedRow {
            local_id: local_id.clone(),
            data,
            position: RiseConversationStoreLayout::message_position(&wire, &local_id, RiseMessageLocalPhase::Remote),
            aliases,
            duplicate_local_ids: existing
                .iter()
                .map(|entity| entity.local_id.clone())
                .filter(|identifier| *identifier != local_id)
                .collect(),
            entity_version: 0,
            is_unchanged: false,
        };
        self.remember_reduced_created(&row, &wire, version);
        let explicit_thread_root = RiseMessageWireCodec::string(root, &[rise_key::message::THREAD_ROOT_ID]);
        let reply_root = object
            .get(rise_key::bot::REPLY_TO)
            .and_then(RiseJSONValue::object_value)
            .and_then(|reply| RiseMessageWireCodec::string(reply, &[rise_key::message::MESSAGE_ID]));
        let mut thread_root = explicit_thread_root.or_else(|| {
            reply_root.as_ref().filter(|candidate| {
                let scope =
                    RiseMessageScope::new(chat_id.clone(), None, Some((*candidate).clone()));
                self.state.with(|state| state.known_scopes.contains(&scope))
            }).cloned()
        });
        if thread_root.is_none()
            && let Some(reply_root) = &reply_root
        {
            thread_root = self.observed_thread_root(&chat_id, reply_root).await;
        }
        let scopes = self.scopes_for_message(&chat_id, wire.topic_id.as_deref(), thread_root.as_deref());
        let membership_scopes = self.scopes_accepting_realtime_tail(&scopes).await;
        RiseColdOpenTrace::mark(|| {
            format!(
                "a3a1 rt create scopes={} tail={} v={version} stored={was_already_stored} merged={} tag={}",
                scopes.len(),
                membership_scopes.len(),
                row.duplicate_local_ids.len(),
                RiseColdOpenTrace::tag(&row.local_id)
            )
        });
        let mut batch = self.entity_upsert_batch(&row, &scopes, version, Some(&membership_scopes));
        if !lifted_tombstones.is_empty() {
            let mutations = lifted_tombstones
                .iter()
                .map(|local_id| RiseStoreMutation::RemoveEntityViewMembership {
                    namespace: RiseConversationStoreLayout::messages(),
                    view: Self::TOMBSTONE_VIEW.to_owned(),
                    local_id: local_id.clone(),
                    if_version_at_most: Some(version),
                })
                .collect();
            batch = self.combining(batch, RiseStoreWriteBatch::new(mutations, HashSet::new()));
        }
        let preview = self
            .preview_hook
            .call(
                RiseMessagePreviewChange::Created {
                    scope: scopes
                        .first()
                        .cloned()
                        .unwrap_or_else(|| RiseMessageScope::new(chat_id.clone(), None, None)),
                    message: wire.clone(),
                    was_already_stored,
                },
                version,
            )
            .await?;
        batch = self.combining(batch, preview);
        if !was_already_stored
            || !lifted_tombstones.is_empty()
            || (reconciles_local_submission && !was_already_reduced)
        {
            let topic_id = self.normalized(wire.topic_id.as_deref());
            let invalidation = RiseMessageAuxiliaryInvalidation::new(
                &chat_id,
                topic_id.as_deref(),
                RiseMessageAuxiliaryProjection::content(),
            );
            let auxiliary = self.auxiliary_invalidation_batch(invalidation).await;
            batch = self.combining(batch, auxiliary);
        }
        if !was_already_stored
            && !reconciles_local_submission
            && wire.shared_post.is_none()
            && wire
                .content_type
                .as_deref()
                .is_some_and(|raw| matches!(MessageContentType::from_raw_value(raw), MessageContentType::PostShort))
            && let Some(message_id) = self.normalized(wire.id.as_deref())
        {
            let repository = self.this.clone();
            RiseAccountRuntime::current().spawn(async move {
                if let Some(repository) = repository.upgrade() {
                    repository.hydrate_shared_post(&message_id).await;
                }
            });
        }
        Ok(Some(batch))
    }

    async fn observed_thread_root(&self, chat_id: &str, reply_id: &str) -> Option<String> {
        let mut threads: Vec<RiseMessageScope> = self.state.with(|state| {
            state
                .known_scopes
                .iter()
                .filter(|scope| scope.chat_id == chat_id && scope.thread_root_id.is_some())
                .cloned()
                .collect()
        });
        threads.sort_by_key(RiseMessageScope::storage_key);
        if threads.is_empty() {
            return None;
        }
        let target = self.message_entity(reply_id).await.ok().flatten()?;
        if RiseMessageWireCodec::is_tombstone(&target.data) {
            return None;
        }
        for scope in threads {
            if let Ok(Some(_)) = self
                .engine
                .entity_view_position(
                    RiseConversationStoreLayout::messages(),
                    RiseConversationStoreLayout::message_view(&scope),
                    target.local_id.clone(),
                )
                .await
            {
                return scope.thread_root_id;
            }
        }
        None
    }

    async fn hydrate_shared_post(&self, message_id: &str) {
        let Ok(response) = self.remote.message_by_id(message_id.to_owned()).await else {
            return;
        };
        let Some(message) = response
            .object_value()
            .and_then(|object| object.get(rise_key::bot::MESSAGE))
            .and_then(RiseJSONValue::object_value)
        else {
            return;
        };
        let Some(shared_post) = message.get(rise_key::payload::SHARED_POST) else {
            return;
        };
        if shared_post.object_value().is_none() {
            return;
        }
        let root = BTreeMap::from([(
            rise_key::bot::MESSAGE_ID.to_owned(),
            RiseJSONValue::String(message_id.to_owned()),
        )]);
        let patched = self
            .reduce_patched_message(
                &root,
                self.next_version(),
                HashSet::new(),
                None,
                false,
                false,
                true,
                |existing: &MessageWireDTO| {
                    if existing.shared_post.is_some() {
                        return Ok(existing.clone());
                    }
                    let mut object = RiseMessageWireCodec::object(existing)?;
                    object.insert(rise_key::payload::SHARED_POST.to_owned(), shared_post.clone());
                    RiseMessageWireCodec::wire(&object)
                },
            )
            .await;
        if let Ok(Some(batch)) = patched {
            let _ = self.engine.write(batch).await;
        }
    }

    async fn hydrate_reactor_identities(&self, message_id: &str) {
        let Ok(response) = self.remote.message_by_id(message_id.to_owned()).await else {
            return;
        };
        let Some(message) = response
            .object_value()
            .and_then(|object| object.get(rise_key::bot::MESSAGE))
            .and_then(RiseJSONValue::object_value)
        else {
            return;
        };
        let root = BTreeMap::from([(
            rise_key::bot::MESSAGE_ID.to_owned(),
            RiseJSONValue::String(message_id.to_owned()),
        )]);
        let patched = self
            .reduce_patched_message(
                &root,
                self.next_version(),
                HashSet::new(),
                None,
                false,
                true,
                false,
                |existing: &MessageWireDTO| RiseMessageWireCodec::filling_reactor_identities(existing, message),
            )
            .await;
        if let Ok(Some(batch)) = patched {
            let _ = self.engine.write(batch).await;
        }
    }

    async fn restored_message_root(
        &self,
        root: &BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<Option<BTreeMap<String, RiseJSONValue>>> {
        let Some(message_id) = self.message_id(root) else {
            return Ok(None);
        };
        let Some(RiseJSONValue::Object(message)) = root.get(rise_key::bot::MESSAGE) else {
            return Ok(None);
        };
        if message.is_empty() {
            return Ok(None);
        }
        if let Some(entity) = self.message_entity(&message_id).await?
            && !RiseMessageWireCodec::is_tombstone(&entity.data)
        {
            return Ok(None);
        }
        Ok(Some(message.clone()))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn reduce_patched_message(
        &self,
        root: &BTreeMap<String, RiseJSONValue>,
        version: i64,
        invalidation_projections: HashSet<RiseMessageAuxiliaryProjection>,
        pinned_state: Option<bool>,
        requotes_replies: bool,
        patches_pinned_copy: bool,
        reads_reduced_created: bool,
        transform: impl Fn(&MessageWireDTO) -> anyhow::Result<MessageWireDTO> + Send + Sync,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let Some(message_id) = self.message_id(root) else {
            return Ok(self.realtime_auxiliary_invalidation_batch(root, invalidation_projections, pinned_state).await);
        };
        let stored = self.message_entity(&message_id).await?;
        let pending = if stored.is_none() && reads_reduced_created {
            self.state.with(|state| state.reduced_created_rows.get(&message_id).cloned())
        } else {
            None
        };
        let is_pending = pending.is_some();
        let Some(entity) = stored.or(pending).filter(|entity| !RiseMessageWireCodec::is_tombstone(&entity.data)) else {
            return Ok(self.realtime_auxiliary_invalidation_batch(root, invalidation_projections, pinned_state).await);
        };
        self.state.with(|state| state.version_clock = state.version_clock.max(entity.version));
        let old = RiseMessageWireCodec::record(&entity.data)?;
        let wire = transform(&old.wire)?;
        let record = realtime_record(&old, wire.clone());
        let aliases = RiseConversationStoreLayout::message_aliases(&wire);
        let row = RiseMessageRepositoryEncodedRow {
            local_id: entity.local_id.clone(),
            data: RiseMessageWireCodec::record_data(&record)?,
            position: RiseConversationStoreLayout::message_position(&wire, &entity.local_id, old.local_phase),
            aliases: aliases.clone(),
            duplicate_local_ids: Vec::new(),
            entity_version: 0,
            is_unchanged: false,
        };
        if is_pending {
            self.remember_reduced_created(&row, &wire, version);
        }
        let chat_id = self
            .normalized(wire.chat_id.as_deref())
            .or_else(|| RiseMessageWireCodec::string(root, &[rise_key::message::CHAT_ID]));
        let mut scopes: Vec<RiseMessageScope> = Vec::new();
        if let Some(chat_id) = &chat_id {
            let thread_root = RiseMessageWireCodec::string(root, &[rise_key::message::THREAD_ROOT_ID]);
            scopes = self.scopes_for_message(chat_id, wire.topic_id.as_deref(), thread_root.as_deref());
        }
        let mut batch = self.entity_upsert_batch(&row, &scopes, version, None);
        if requotes_replies && let Some(chat_id) = &chat_id {
            let quoted: HashSet<String> = aliases.iter().map(|alias| alias.value.clone()).collect();
            let mutations = self.quoting_row_mutations(chat_id, quoted, Some(&wire), &HashSet::new()).await;
            batch = self.combining(batch, RiseStoreWriteBatch::new(mutations, HashSet::new()));
        }
        if let Some(chat_id) = &chat_id {
            let preview = self
                .preview_hook
                .call(
                    RiseMessagePreviewChange::Edited {
                        scope: scopes
                            .first()
                            .cloned()
                            .unwrap_or_else(|| RiseMessageScope::new(chat_id.clone(), None, None)),
                        message: wire.clone(),
                    },
                    version,
                )
                .await?;
            batch = self.combining(batch, preview);
            let pinned_copy_patch = (patches_pinned_copy && wire.is_pinned == Some(true))
                .then(|| RiseMessageAuxiliaryInvalidationPinnedMessagePatch::from_message(wire.clone(), true));
            let topic_id = self.normalized(wire.topic_id.as_deref());
            if !invalidation_projections.is_empty() {
                let mut invalidation =
                    RiseMessageAuxiliaryInvalidation::new(chat_id, topic_id.as_deref(), invalidation_projections);
                invalidation.pinned_message_patch = pinned_state
                    .map(|is_pinned| RiseMessageAuxiliaryInvalidationPinnedMessagePatch::from_message(wire.clone(), is_pinned))
                    .or(pinned_copy_patch);
                let auxiliary = self.auxiliary_invalidation_batch(invalidation).await;
                batch = self.combining(batch, auxiliary);
            } else if let Some(pinned_copy_patch) = pinned_copy_patch {
                let mut invalidation = RiseMessageAuxiliaryInvalidation::new(
                    chat_id,
                    topic_id.as_deref(),
                    HashSet::from([RiseMessageAuxiliaryProjection::Pinned]),
                );
                invalidation.pinned_message_patch = Some(pinned_copy_patch);
                let auxiliary = self.auxiliary_invalidation_batch(invalidation).await;
                batch = self.combining(batch, auxiliary);
            }
        }
        Ok(Some(batch))
    }

    async fn realtime_auxiliary_invalidation_batch(
        &self,
        root: &BTreeMap<String, RiseJSONValue>,
        projections: HashSet<RiseMessageAuxiliaryProjection>,
        pinned_state: Option<bool>,
    ) -> Option<RiseStoreWriteBatch> {
        if projections.is_empty() {
            return None;
        }
        let empty = BTreeMap::new();
        let message = root.get(rise_key::bot::MESSAGE).and_then(RiseJSONValue::object_value).unwrap_or(&empty);
        let chat_id_raw = RiseMessageWireCodec::string(root, &[rise_key::message::CHAT_ID])
            .or_else(|| RiseMessageWireCodec::string(message, &[rise_key::message::CHAT_ID]));
        let chat_id = self.normalized(chat_id_raw.as_deref())?;
        let topic_id_raw = RiseMessageWireCodec::string(root, &[rise_key::message::TOPIC_ID])
            .or_else(|| RiseMessageWireCodec::string(message, &[rise_key::message::TOPIC_ID]));
        let topic_id = self.normalized(topic_id_raw.as_deref());
        let mut invalidation = RiseMessageAuxiliaryInvalidation::new(&chat_id, topic_id.as_deref(), projections);
        invalidation.pinned_message_patch = if pinned_state == Some(false) {
            self.message_id(root).map(|identifier| {
                RiseMessageAuxiliaryInvalidationPinnedMessagePatch::from_message_identifier(&identifier, false)
            })
        } else {
            None
        };
        let batch = self.auxiliary_invalidation_batch(invalidation).await;
        if batch.mutations.is_empty() && batch.invalidated_views.is_empty() {
            None
        } else {
            Some(batch)
        }
    }

    pub async fn reduce_deleted(
        &self,
        root: &BTreeMap<String, RiseJSONValue>,
        version: i64,
        invalidation_projections: HashSet<RiseMessageAuxiliaryProjection>,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let Some(chat_id) = RiseMessageWireCodec::string(root, &[rise_key::message::CHAT_ID]) else {
            return Ok(None);
        };
        let mut identifiers = RiseMessageWireCodec::strings(
            root,
            &[rise_key::message::MESSAGE_IDS, rise_key::message::DELETED_MESSAGE_IDS],
        );
        if identifiers.is_empty()
            && let Some(identifier) = self.message_id(root)
        {
            identifiers = vec![identifier];
        }
        if identifiers.is_empty() {
            return Ok(None);
        }

        let namespace = RiseConversationStoreLayout::messages();
        let now = Self::now_milliseconds();
        let mut mutations: Vec<RiseStoreMutation> = Vec::new();
        let mut deleted_server_ids: Vec<String> = Vec::new();
        let mut quoted_ids: HashSet<String> = HashSet::new();
        let mut affected_topic_ids: HashSet<String> = HashSet::new();
        let root_topic_raw = RiseMessageWireCodec::string(root, &[rise_key::message::TOPIC_ID]);
        if let Some(topic_id) = self.normalized(root_topic_raw.as_deref()) {
            affected_topic_ids.insert(topic_id);
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mut deletion_scopes: HashSet<RiseMessageScope> = self.known_scopes_with_chat(&chat_id).into_iter().collect();
        deletion_scopes.insert(RiseMessageScope::new(chat_id.clone(), None, None));
        deletion_scopes.insert(RiseMessageScope::new(chat_id.clone(), Some(String::new()), None));
        for topic_id in &affected_topic_ids {
            deletion_scopes.insert(RiseMessageScope::new(chat_id.clone(), Some(topic_id.clone()), None));
        }
        for identifier in &identifiers {
            let entity: RiseStoreEntity;
            let old: RiseMessageEntityRecord;
            if let Some(found) = self.message_entity(identifier).await? {
                self.state.with(|state| state.version_clock = state.version_clock.max(found.version));
                old = RiseMessageWireCodec::record(&found.data)?;
                entity = found;
            } else {
                let wire = RiseMessageWireCodec::wire(&BTreeMap::from([
                    (rise_key::bot::ID.to_owned(), RiseJSONValue::String(identifier.clone())),
                    (SERVER_ID_KEY.to_owned(), RiseJSONValue::String(identifier.clone())),
                    (rise_key::bot::CHAT_ID.to_owned(), RiseJSONValue::String(chat_id.clone())),
                    (
                        rise_key::bot::CREATED_AT.to_owned(),
                        RiseJSONValue::Number(Date::now().time_interval_since_1970()),
                    ),
                ]))?;
                old = RiseMessageEntityRecord::new(wire, RiseConversationEntitySource::Realtime);
                entity = RiseStoreEntity::new(identifier.clone(), RiseMessageWireCodec::record_data(&old)?, 0, now);
            }
            if !seen.insert(entity.local_id.clone()) {
                continue;
            }
            if let Some(topic_id) = self.normalized(old.wire.topic_id.as_deref()) {
                deletion_scopes.insert(RiseMessageScope::new(chat_id.clone(), Some(topic_id.clone()), None));
                affected_topic_ids.insert(topic_id);
            }
            RiseColdOpenTrace::mark(|| {
                format!(
                    "a3a1 rt delete views={} entityVersion={} tag={}",
                    deletion_scopes.len(),
                    entity.version,
                    RiseColdOpenTrace::tag(&entity.local_id)
                )
            });
            let aliases = RiseConversationStoreLayout::message_aliases(&old.wire);
            let tombstone = RiseMessageEntityRecord {
                wire: old.wire.clone(),
                source: RiseConversationEntitySource::Realtime,
                local_phase: RiseMessageLocalPhase::Remote,
                client_request_id: old.client_request_id.clone(),
                last_error_code: None,
                scope_storage_key: old.scope_storage_key.clone(),
                is_replayable: None,
                upload_retry: None,
                forward_retry: None,
            };
            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: namespace.clone(),
                entity: RiseStoreEntity::new(
                    entity.local_id.clone(),
                    RiseMessageWireCodec::tombstone_data(&tombstone, now)?,
                    version,
                    now,
                ),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            });
            for alias in &aliases {
                mutations.push(RiseStoreMutation::SetEntityAlias {
                    namespace: namespace.clone(),
                    alias: alias.clone(),
                    local_id: entity.local_id.clone(),
                });
            }
            for scope in &deletion_scopes {
                mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                    namespace: namespace.clone(),
                    view: RiseConversationStoreLayout::message_view(scope),
                    local_id: entity.local_id.clone(),
                    if_version_at_most: Some(version),
                });
            }
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: namespace.clone(),
                view: Self::TOMBSTONE_VIEW.to_owned(),
                membership: RiseEntityViewMembership::new(
                    entity.local_id.clone(),
                    RisePositionKey::make(|key| {
                        key.append_signed_int64_with_order(version, RisePositionKeyOrder::Ascending);
                        key.append_string_with_order(&entity.local_id, RisePositionKeyOrder::Ascending);
                    }),
                    version,
                ),
            });
            deleted_server_ids.push(identifier.clone());
            deleted_server_ids.extend(Self::preview_identities(&entity));
            quoted_ids.insert(identifier.clone());
            quoted_ids.extend(aliases.into_iter().map(|alias| alias.value));
        }
        if mutations.is_empty() {
            return Ok(None);
        }
        mutations.push(RiseStoreMutation::TrimEntityView {
            namespace: namespace.clone(),
            view: Self::TOMBSTONE_VIEW.to_owned(),
            keeping: RiseEntityViewRetention::Latest(self.retention_limit as usize),
            if_version_at_most: None,
        });
        mutations.push(RiseStoreMutation::PruneUnreferencedEntities {
            namespace,
            limit: self.retention_limit as i64,
        });
        mutations.extend(self.quoting_row_mutations(&chat_id, quoted_ids, None, &seen).await);
        let mut batch = RiseStoreWriteBatch::new(mutations, HashSet::new());
        tracing::warn!(target: "riseonly::X1777", "rt delete chat={chat_id} deleted={deleted_server_ids:?} rows={}", seen.len());
        let preview = self
            .preview_hook
            .call(
                RiseMessagePreviewChange::Deleted {
                    chat_id: chat_id.clone(),
                    server_ids: deleted_server_ids.clone(),
                    fallback: self
                        .newest_preview_fallback(
                            &chat_id,
                            &seen.iter().chain(deleted_server_ids.iter()).cloned().collect::<HashSet<String>>(),
                        )
                        .await,
                },
                version,
            )
            .await?;
        batch = self.combining(batch, preview);
        if !invalidation_projections.is_empty() {
            let mut invalidation = RiseMessageAuxiliaryInvalidation::new(&chat_id, None, invalidation_projections);
            invalidation.topic_ids = affected_topic_ids;
            invalidation.removed_message_ids = identifiers.into_iter().chain(deleted_server_ids).collect();
            let auxiliary = self.auxiliary_invalidation_hook.call(invalidation).await;
            batch = self.combining(batch, auxiliary);
        }
        Ok(Some(batch))
    }

    pub async fn reduce_read(
        &self,
        root: &BTreeMap<String, RiseJSONValue>,
        version: i64,
    ) -> anyhow::Result<Option<RiseStoreWriteBatch>> {
        let Some(chat_id) = RiseMessageWireCodec::string(root, &[rise_key::message::CHAT_ID]) else {
            return Ok(None);
        };
        let Some(reader_id) = RiseMessageWireCodec::string(root, &[rise_key::message::USER_ID]) else {
            return Ok(None);
        };
        if reader_id.is_empty() {
            return Ok(None);
        }
        let is_local_reader = reader_id == self.account_id;

        let has_topic_value = RiseMessageWireCodec::value(root, &[rise_key::message::TOPIC_ID]).is_some();
        let topic_id = RiseMessageWireCodec::string(root, &[rise_key::message::TOPIC_ID]);
        let authoritative_unread = if is_local_reader {
            RiseMessageWireCodec::double(root, &[rise_key::message::UNREAD_COUNT]).map(|count| count.max(0.0) as i64)
        } else {
            None
        };
        let last_read_id = RiseMessageWireCodec::string(root, &[rise_key::chat::LAST_READ_MESSAGE_ID]);
        let mut cutoff = RiseMessageWireCodec::double(root, &[rise_key::events::LAST_READ_CREATED_AT])
            .map(RiseConversationStoreLayout::normalized_timestamp);
        if cutoff.is_none()
            && let Some(last_read_id) = &last_read_id
            && let Some(entity) = self.message_entity(last_read_id).await?
            && let Ok(record) = RiseMessageWireCodec::record(&entity.data)
        {
            cutoff = Some(RiseConversationStoreLayout::normalized_timestamp(
                record.wire.created_at.or(record.wire.timestamp).unwrap_or(0.0),
            ));
        }
        let explicit: HashSet<String> =
            RiseMessageWireCodec::strings(root, &[rise_key::message::READ_MESSAGE_IDS]).into_iter().collect();
        let mut direct_receipt_ids = explicit.clone();
        if let Some(last_read_id) = &last_read_id
            && !last_read_id.is_empty()
        {
            direct_receipt_ids.insert(last_read_id.clone());
        }
        let same_millisecond: HashSet<String> =
            RiseMessageWireCodec::strings(root, &[rise_key::events::LAST_READ_SAME_MILLISECOND_MESSAGE_IDS])
                .into_iter()
                .collect();
        let has_read_boundary = cutoff.is_some() || !explicit.is_empty();
        if !has_read_boundary {
            let preview = if is_local_reader {
                self.preview_hook
                    .call(
                        RiseMessagePreviewChange::Read {
                            chat_id: chat_id.clone(),
                            topic_id: topic_id.clone(),
                            authoritative_unread,
                            clears_marks: false,
                        },
                        version,
                    )
                    .await?
            } else if !direct_receipt_ids.is_empty() {
                let receipt_ids: Vec<String> = direct_receipt_ids.iter().cloned().collect();
                self.record_read_receipt_hint(&chat_id, receipt_ids.clone()).await;
                self.preview_hook
                    .call(RiseMessagePreviewChange::Receipt { chat_id: chat_id.clone(), server_ids: receipt_ids }, version)
                    .await?
            } else {
                return Ok(None);
            };
            return Ok(if preview.mutations.is_empty() && preview.invalidated_views.is_empty() {
                None
            } else {
                Some(preview)
            });
        }

        let read_topic = has_topic_value.then(|| topic_id.clone().unwrap_or_default());
        let mut candidate_scopes = self.known_scopes_with_chat(&chat_id);
        if let Some(topic) = &read_topic {
            let named = RiseMessageScope::new(chat_id.clone(), Some(topic.clone()), None);
            if !candidate_scopes.contains(&named) {
                candidate_scopes.push(named);
            }
        }
        let mut entities: HashMap<String, RiseStoreEntity> = HashMap::new();
        for scope in &candidate_scopes {
            let window = self
                .engine
                .entity_window(RiseEntityWindowQuery::new(
                    RiseConversationStoreLayout::messages(),
                    RiseConversationStoreLayout::message_view(scope),
                    RiseEntityWindow::Latest { limit: self.retention_limit as usize },
                ))
                .await?;
            for item in window.items {
                entities.insert(item.entity.local_id.clone(), item.entity);
            }
        }

        let cutoff_milliseconds = cutoff.map(|cutoff| (cutoff * 1_000.0) as i64);
        let mut mutations: Vec<RiseStoreMutation> = Vec::new();
        let mut receipt_server_ids: HashSet<String> =
            if is_local_reader { HashSet::new() } else { direct_receipt_ids.clone() };
        for entity in entities.values() {
            if RiseMessageWireCodec::is_tombstone(&entity.data) {
                continue;
            }
            let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
                continue;
            };
            if record.local_phase != RiseMessageLocalPhase::Remote {
                continue;
            }
            if read_topic.as_deref().is_some_and(|topic| record.wire.topic_id.as_deref().unwrap_or_default() != topic) {
                continue;
            }
            let sender_id = sender_id_of(&record.wire).unwrap_or_default();
            let is_counterpart =
                if is_local_reader { sender_id != self.account_id } else { sender_id == self.account_id };
            if sender_id.is_empty() || !is_counterpart {
                continue;
            }
            let aliases: HashSet<String> = RiseConversationStoreLayout::message_aliases(&record.wire)
                .into_iter()
                .map(|alias| alias.value)
                .collect();
            let is_explicit = !aliases.is_disjoint(&explicit);
            let timestamp = RiseConversationStoreLayout::normalized_timestamp(
                record.wire.created_at.or(record.wire.timestamp).unwrap_or(0.0),
            );
            let timestamp_milliseconds = (timestamp * 1_000.0) as i64;
            let is_before =
                timestamp_milliseconds > 0 && cutoff_milliseconds.is_some_and(|cutoff| timestamp_milliseconds < cutoff);
            let is_exact_boundary =
                cutoff_milliseconds == Some(timestamp_milliseconds) && !aliases.is_disjoint(&same_millisecond);
            if !(is_explicit || is_before || is_exact_boundary) {
                continue;
            }
            if !is_local_reader {
                receipt_server_ids.extend(aliases);
            }
            if record.wire.is_read == Some(true) {
                continue;
            }
            let wire = RiseMessageWireCodec::setting_boolean(&record.wire, rise_key::message::IS_READ, true)?;
            let updated = realtime_record(&record, wire);
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
        let mut batch = RiseStoreWriteBatch::new(mutations, HashSet::new());
        if is_local_reader {
            let preview = self
                .preview_hook
                .call(
                    RiseMessagePreviewChange::Read {
                        chat_id: chat_id.clone(),
                        topic_id: topic_id.clone(),
                        authoritative_unread,
                        clears_marks: false,
                    },
                    version,
                )
                .await?;
            batch = self.combining(batch, preview);
        } else if !receipt_server_ids.is_empty() {
            let receipt_ids: Vec<String> = receipt_server_ids.into_iter().collect();
            self.record_read_receipt_hint(&chat_id, receipt_ids.clone()).await;
            let preview = self
                .preview_hook
                .call(RiseMessagePreviewChange::Receipt { chat_id: chat_id.clone(), server_ids: receipt_ids }, version)
                .await?;
            batch = self.combining(batch, preview);
        }
        Ok(if batch.mutations.is_empty() && batch.invalidated_views.is_empty() {
            None
        } else {
            Some(batch)
        })
    }

    async fn record_read_receipt_hint(&self, chat_id: &str, message_ids: Vec<String>) {
        if message_ids.is_empty() {
            return;
        }
        self.read_receipt_hint_hook
            .call(RiseMessageReadReceiptHint {
                chat_id: chat_id.to_owned(),
                message_ids,
                read_at_milliseconds: Self::now_milliseconds(),
            })
            .await;
    }

    pub async fn reduce_typing(&self, root: &BTreeMap<String, RiseJSONValue>) {
        let Some(chat_id) = RiseMessageWireCodec::string(root, &[rise_key::message::CHAT_ID]) else {
            return;
        };
        let Some(user_id) = RiseMessageWireCodec::string(root, &[rise_key::message::USER_ID]) else {
            return;
        };
        if user_id == self.account_id {
            return;
        }
        let user = RiseMessageWireCodec::value(root, &[rise_key::events::USER_INFO])
            .and_then(|value| value.object_value().cloned())
            .unwrap_or_default();
        let name = RiseMessageWireCodec::string(&user, &[rise_key::bot::NAME])
            .or_else(|| {
                RiseMessageWireCodec::string(
                    root,
                    &[rise_key::message::SENDER_NAME, rise_key::push::USER_NAME, NAME_FALLBACK_KEY],
                )
            })
            .unwrap_or_default();
        let topic_id = RiseMessageWireCodec::string(root, &[rise_key::message::TOPIC_ID]).unwrap_or_default();
        let known = self.state.with(|state| state.known_scopes.clone());
        let scopes = Self::typing_scopes(&chat_id, &topic_id, &known);
        self.typing_hub
            .update(
                &scopes,
                &user_id,
                &name,
                RiseMessageWireCodec::bool(root, &[rise_key::events::TYPING, rise_key::message::IS_TYPING])
                    .unwrap_or(false),
                RiseMessageWireCodec::string(root, &[rise_key::events::ACTION]),
                RiseMessageWireCodec::string(root, &[rise_key::message::STATUS_TEXT]),
            )
            .await;
    }

    pub fn typing_scopes(chat_id: &str, topic_id: &str, known: &HashSet<RiseMessageScope>) -> Vec<RiseMessageScope> {
        let mut scopes: HashSet<RiseMessageScope> = HashSet::from([
            RiseMessageScope::new(chat_id.to_owned(), None, None),
            RiseMessageScope::new(chat_id.to_owned(), Some(topic_id.to_owned()), None),
        ]);
        for scope in known.iter().filter(|scope| scope.chat_id == chat_id) {
            if let Some(scope_topic) = &scope.topic_id {
                if scope_topic == topic_id {
                    scopes.insert(scope.clone());
                }
            } else if scope.thread_root_id.is_some() && topic_id.is_empty() {
                scopes.insert(scope.clone());
            }
        }
        scopes.into_iter().collect()
    }

    /// known scopes of the chat plus the plain chat scope
    fn known_scopes_with_chat(&self, chat_id: &str) -> Vec<RiseMessageScope> {
        let mut scopes: HashSet<RiseMessageScope> = self.state.with(|state| {
            state.known_scopes.iter().filter(|scope| scope.chat_id == chat_id).cloned().collect()
        });
        scopes.insert(RiseMessageScope::new(chat_id.to_owned(), None, None));
        scopes.into_iter().collect()
    }

    pub fn push_timestamp(push: &RiseWirePushEnvelope) -> Option<f64> {
        match push.timestamp.as_ref()? {
            RiseWireTimestamp::UnixSeconds(value) => Some(RiseConversationStoreLayout::normalized_timestamp(*value)),
            RiseWireTimestamp::Text(value) => {
                ISO8601DateFormatter::new().date_from(value).map(|date| date.time_interval_since_1970())
            }
        }
    }

    pub async fn scopes_accepting_realtime_tail(&self, scopes: &[RiseMessageScope]) -> Vec<RiseMessageScope> {
        let mut accepted: Vec<RiseMessageScope> = Vec::new();
        for scope in scopes {
            if let Ok(Some(state)) = self.view_state(scope).await
                && state.has_more_after
            {
                continue;
            }
            accepted.push(scope.clone());
        }
        accepted
    }

    pub fn entity_upsert_batch(
        &self,
        row: &RiseMessageRepositoryEncodedRow,
        scopes: &[RiseMessageScope],
        version: i64,
        membership_scopes: Option<&[RiseMessageScope]>,
    ) -> RiseStoreWriteBatch {
        let namespace = RiseConversationStoreLayout::messages();
        let mut mutations: Vec<RiseStoreMutation> = vec![RiseStoreMutation::UpsertEntity {
            namespace: namespace.clone(),
            entity: RiseStoreEntity::new(row.local_id.clone(), row.data.clone(), version, Self::now_milliseconds()),
            policy: RiseEntityWritePolicy::ServerAuthoritative,
        }];
        for duplicate in &row.duplicate_local_ids {
            mutations.push(RiseStoreMutation::MergeEntities {
                namespace: namespace.clone(),
                survivor_local_id: row.local_id.clone(),
                duplicate_local_id: duplicate.clone(),
            });
        }
        for alias in &row.aliases {
            mutations.push(RiseStoreMutation::SetEntityAlias {
                namespace: namespace.clone(),
                alias: alias.clone(),
                local_id: row.local_id.clone(),
            });
        }
        let unique_scopes: HashSet<&RiseMessageScope> = membership_scopes.unwrap_or(scopes).iter().collect();
        for scope in unique_scopes {
            let view = RiseConversationStoreLayout::message_view(scope);
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: namespace.clone(),
                view: view.clone(),
                membership: RiseEntityViewMembership::new(row.local_id.clone(), row.position.clone(), version),
            });
            mutations.push(RiseStoreMutation::TrimEntityView {
                namespace: namespace.clone(),
                view,
                keeping: RiseEntityViewRetention::Latest(self.retention_limit as usize),
                if_version_at_most: None,
            });
        }
        mutations.push(RiseStoreMutation::PruneUnreferencedEntities {
            namespace,
            limit: self.retention_limit as i64,
        });
        RiseStoreWriteBatch::new(mutations, HashSet::new())
    }

    pub async fn message_entity(&self, identifier: &str) -> anyhow::Result<Option<RiseStoreEntity>> {
        let namespace = RiseConversationStoreLayout::messages();
        if let Some(entity) = self
            .engine
            .entity_alias(namespace.clone(), RiseEntityAlias::new(RiseEntityAliasKind::server_id(), identifier))
            .await?
        {
            return Ok(Some(entity));
        }
        if let Some(entity) = self
            .engine
            .entity_alias(namespace.clone(), RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), identifier))
            .await?
        {
            return Ok(Some(entity));
        }
        self.engine.entity_local_id(namespace, identifier.to_owned()).await
    }

    /// replies keep their own copy of the quoted message and the server sends nothing when it changes
    /// so held replies follow it, an edit redraws the quote and a delete for everyone drops it
    pub async fn quoting_row_mutations(
        &self,
        chat_id: &str,
        quoted: HashSet<String>,
        source: Option<&MessageWireDTO>,
        excluding: &HashSet<String>,
    ) -> Vec<RiseStoreMutation> {
        let quoted: HashSet<String> = quoted.iter().filter_map(|value| self.normalized(Some(value.as_str()))).collect();
        if quoted.is_empty() {
            return Vec::new();
        }
        let namespace = RiseConversationStoreLayout::messages();
        let probes: Vec<&[u8]> = if quoted.len() <= QUOTING_PROBE_LIMIT {
            quoted.iter().map(|value| value.as_bytes()).collect()
        } else {
            Vec::new()
        };
        let mut scopes: HashSet<RiseMessageScope> = self.state.with(|state| {
            state.known_scopes.iter().filter(|scope| scope.chat_id == chat_id).cloned().collect()
        });
        scopes.insert(RiseMessageScope::new(chat_id.to_owned(), None, None));
        let mut seen = excluding.clone();
        let mut mutations: Vec<RiseStoreMutation> = Vec::new();
        let _eviction = RiseMessageDecodedCacheEvictionGuard(self);
        for scope in &scopes {
            let Ok(result) = self
                .engine
                .entity_window(RiseEntityWindowQuery::new(
                    namespace.clone(),
                    RiseConversationStoreLayout::message_view(scope),
                    RiseEntityWindow::Latest { limit: self.retention_limit as usize },
                ))
                .await
            else {
                continue;
            };
            for item in result.items {
                if !seen.insert(item.entity.local_id.clone()) {
                    continue;
                }
                let entity = item.entity;
                let names_a_quoted_id = probes.is_empty()
                    || probes.iter().any(|probe| entity.data.windows(probe.len()).any(|bytes| bytes == *probe));
                if !names_a_quoted_id || RiseMessageWireCodec::is_tombstone(&entity.data) {
                    continue;
                }
                let Ok(old) = self.decoded_record(&entity) else {
                    continue;
                };
                let reply_id = old.wire.reply_to.as_ref().and_then(|reply| reply.message_id.as_deref());
                let Some(reply_id) = self.normalized(reply_id) else {
                    continue;
                };
                if !quoted.contains(&reply_id) {
                    continue;
                }
                let Ok(wire) = RiseMessageWireCodec::requoting(&old.wire, source) else {
                    continue;
                };
                let Ok(data) = RiseMessageWireCodec::record_data(&RiseMessageEntityRecord {
                    wire,
                    source: old.source,
                    local_phase: old.local_phase,
                    client_request_id: old.client_request_id.clone(),
                    last_error_code: old.last_error_code.clone(),
                    scope_storage_key: old.scope_storage_key.clone(),
                    is_replayable: old.is_replayable,
                    upload_retry: old.upload_retry.clone(),
                    forward_retry: old.forward_retry.clone(),
                }) else {
                    continue;
                };
                if data == entity.data {
                    continue;
                }
                mutations.push(RiseStoreMutation::UpsertEntity {
                    namespace: namespace.clone(),
                    entity: RiseStoreEntity::new(
                        entity.local_id.clone(),
                        data,
                        self.next_version_floor(entity.version),
                        Self::now_milliseconds(),
                    ),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                });
            }
        }
        mutations
    }

    pub fn hold_early_link_preview(&self, root: &BTreeMap<String, RiseJSONValue>, identifier: &str) {
        let limit = self.recent_created_alias_limit as usize;
        self.state.with(|state| {
            if state.early_link_previews.insert(identifier.to_owned(), root.clone()).is_none() {
                state.early_link_preview_order.push(identifier.to_owned());
            }
            while state.early_link_preview_order.len() > limit {
                let oldest = state.early_link_preview_order.remove(0);
                state.early_link_previews.remove(&oldest);
            }
        });
    }

    pub fn take_early_link_preview(&self, wire: &MessageWireDTO) -> Option<BTreeMap<String, RiseJSONValue>> {
        let identifiers: Vec<String> = [wire.server_id.as_deref(), wire.id.as_deref(), wire.temp_id.as_deref()]
            .into_iter()
            .filter_map(|raw| self.normalized(raw))
            .collect();
        self.state.with(|state| {
            if state.early_link_previews.is_empty() {
                return None;
            }
            for identifier in &identifiers {
                if let Some(held) = state.early_link_previews.remove(identifier) {
                    state.early_link_preview_order.retain(|known| known != identifier);
                    return Some(held);
                }
            }
            None
        })
    }

    pub fn remember_reduced_created(&self, row: &RiseMessageRepositoryEncodedRow, wire: &MessageWireDTO, version: i64) {
        let entity =
            RiseStoreEntity::new(row.local_id.clone(), row.data.clone(), version, Self::now_milliseconds());
        let identifiers: HashSet<String> = [wire.server_id.as_deref(), wire.id.as_deref(), wire.temp_id.as_deref()]
            .into_iter()
            .filter_map(|raw| self.normalized(raw))
            .collect();
        let limit = self.recent_created_alias_limit as usize;
        self.state.with(|state| {
            for identifier in identifiers {
                if state.reduced_created_rows.insert(identifier.clone(), entity.clone()).is_none() {
                    state.reduced_created_order.push(identifier);
                }
            }
            while state.reduced_created_order.len() > limit {
                let oldest = state.reduced_created_order.remove(0);
                state.reduced_created_rows.remove(&oldest);
            }
        });
    }

    /// applies a link card that arrived before the send ack committed
    pub async fn apply_held_link_preview(&self, local_id: &str) {
        if self.state.with(|state| state.early_link_previews.is_empty()) {
            return;
        }
        let Ok(Some(entity)) = self.message_entity(local_id).await else {
            return;
        };
        if RiseMessageWireCodec::is_tombstone(&entity.data) {
            return;
        }
        let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
            return;
        };
        let Some(held) = self.take_early_link_preview(&record.wire) else {
            return;
        };
        let patched = self
            .reduce_patched_message(
                &held,
                self.next_version(),
                HashSet::new(),
                None,
                false,
                false,
                false,
                |existing: &MessageWireDTO| RiseMessageWireCodec::resolving_link_preview(existing, &held),
            )
            .await;
        if let Ok(Some(batch)) = patched {
            let _ = self.engine.write(batch).await;
        }
    }

    pub fn message_id(&self, root: &BTreeMap<String, RiseJSONValue>) -> Option<String> {
        RiseMessageWireCodec::string(root, &[rise_key::message::MESSAGE_ID, ID_FALLBACK_KEY])
    }

    pub fn combining(&self, first: RiseStoreWriteBatch, second: RiseStoreWriteBatch) -> RiseStoreWriteBatch {
        let mut mutations = first.mutations;
        mutations.extend(second.mutations);
        let mut invalidated_views = first.invalidated_views;
        invalidated_views.extend(second.invalidated_views);
        RiseStoreWriteBatch::new(mutations, invalidated_views)
    }

    pub async fn clear_unread_mark_flags(
        &self,
        mention_ids: HashSet<String>,
        reply_ids: HashSet<String>,
        reaction_ids: HashSet<String>,
    ) {
        let mut mutations: Vec<RiseStoreMutation> = Vec::new();
        let version = self.next_version();
        let identifiers: HashSet<&String> = mention_ids.iter().chain(&reply_ids).chain(&reaction_ids).collect();
        for identifier in identifiers {
            let Ok(Some(entity)) = self.message_entity(identifier).await else {
                continue;
            };
            if RiseMessageWireCodec::is_tombstone(&entity.data) {
                continue;
            }
            let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
                continue;
            };
            let mut wire = record.wire.clone();
            if mention_ids.contains(identifier) && wire.is_mentioned_you == Some(true) {
                let Ok(next) =
                    RiseMessageWireCodec::setting_boolean(&wire, rise_key::message::IS_MENTIONED_YOU, false)
                else {
                    continue;
                };
                wire = next;
            }
            if reply_ids.contains(identifier) && wire.is_replied_you == Some(true) {
                let Ok(next) = RiseMessageWireCodec::setting_boolean(&wire, rise_key::message::IS_REPLIED_YOU, false)
                else {
                    continue;
                };
                wire = next;
            }
            if reaction_ids.contains(identifier) && wire.reactions_unreaded == Some(true) {
                let Ok(next) =
                    RiseMessageWireCodec::setting_boolean(&wire, rise_key::message::REACTIONS_UNREADED, false)
                else {
                    continue;
                };
                wire = next;
            }
            let Ok(data) = RiseMessageWireCodec::record_data(&RiseMessageEntityRecord {
                wire,
                source: record.source,
                local_phase: record.local_phase,
                client_request_id: record.client_request_id.clone(),
                last_error_code: record.last_error_code.clone(),
                scope_storage_key: record.scope_storage_key.clone(),
                is_replayable: record.is_replayable,
                upload_retry: record.upload_retry.clone(),
                forward_retry: record.forward_retry.clone(),
            }) else {
                continue;
            };
            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: RiseConversationStoreLayout::messages(),
                entity: RiseStoreEntity::new(entity.local_id.clone(), data, version, Self::now_milliseconds()),
                policy: RiseEntityWritePolicy::ServerAuthoritative,
            });
        }
        if mutations.is_empty() {
            return;
        }
        let _ = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await;
    }
}
