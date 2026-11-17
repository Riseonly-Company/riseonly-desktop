use std::collections::{BTreeMap, HashMap, HashSet};

use futures::FutureExt as _;
use gpui::AsyncApp;
use rise_canon::RiseIdentityCanon;
use rise_canon::rise_key::message as key;
use rise_canon::rise_request::message::forward_messages::DestinationsElement;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_core::rise_identifiers::RiseRequestIDAllocator;
use rise_engine::rise_store::rise_store_models::RiseStoreWriteBatch;
use rise_engine::RiseJSONValue;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseConversationEntitySource, RiseMessageForwardRetry, RiseMessageLocalPhase,
};
use crate::modules::chat::engine::messages::rise_message_contracts::RiseAuthoritativeMessage;
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    ForwardMessagesBody, ForwardMessagesGatewayDTO, MessageWireDTO,
};
use crate::modules::share_extension::send_handoff::share_send_handoff_models::RiseStoredSharePayloadReply;

#[derive(Clone, Debug)]
pub struct OptimisticFanoutRow {
    pub id: String,
    pub scope: RiseMessageScope,
    pub scopes: Vec<RiseMessageScope>,
    pub local_id: String,
    pub wire: MessageWireDTO,
    pub request_string: String,
    pub forward_retry: Option<RiseMessageForwardRetry>,
}

async fn log_fanout(line: String) {
    let Some(main) = RiseEngineMainHop::installed() else {
        return;
    };
    let _ = main
        .run(move |cx: AsyncApp| {
            async move { Ok(cx.update(|cx| Sai::log("Message", move || line, SaiLogKind::Network, cx))) }.boxed_local()
        })
        .await;
}

impl RiseMessageRepository {
    pub async fn install_forward_optimistic_rows(&self, body: &ForwardMessagesBody) -> Vec<OptimisticFanoutRow> {
        let Some(idempotency_key) = RiseMessageWireCodec::normalized(body.idempotency_key.as_deref()) else {
            return Vec::new();
        };
        let Some(destinations) = body.destinations.as_ref().filter(|destinations| !destinations.is_empty()) else {
            return Vec::new();
        };
        let Some(message_ids) = body.message_ids.as_ref().filter(|message_ids| !message_ids.is_empty()) else {
            return Vec::new();
        };

        let mut sources: Vec<(&str, MessageWireDTO)> = Vec::new();
        for identifier in message_ids {
            let Ok(Some(entity)) = self.message_entity(identifier).await else {
                continue;
            };
            if RiseMessageWireCodec::is_tombstone(&entity.data) {
                continue;
            }
            let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
                continue;
            };
            sources.push((identifier.as_str(), record.wire));
        }
        if sources.is_empty() {
            return Vec::new();
        }

        let retry = RiseMessageForwardRetry {
            idempotency_key: idempotency_key.clone(),
            message_ids: message_ids.clone(),
            caption: body.caption.clone(),
        };
        let now = RiseClock::now().time_interval_since_1970();
        let mut rows = Vec::new();
        for destination in destinations {
            let Some(scope) = Self::fanout_scope(destination.chat_id.as_deref(), destination.topic_id.as_deref()) else {
                return Vec::new();
            };
            for (identifier, source) in &sources {
                let local_id = Self::fanout_local_id("fwd", &idempotency_key, identifier, &scope);
                let Ok(wire) =
                    RiseMessageWireCodec::forwarded_optimistic_wire(source, &scope, &local_id, &self.account_id, now)
                else {
                    continue;
                };
                rows.push(self.make_fanout_row(
                    Self::fanout_row_id(identifier, &scope),
                    scope.clone(),
                    local_id,
                    wire,
                    Some(retry.clone()),
                ));
            }
        }
        self.install_optimistic_fanout_rows(&rows).await;
        rows
    }

    /// send again on a failed forward copy, every failed copy in this destination goes back to sending
    /// the request replays the original key narrowed to this destination
    /// none when nothing is left failed here
    pub async fn reinstall_failed_forward_rows(
        &self,
        retry: &RiseMessageForwardRetry,
        scope: &RiseMessageScope,
    ) -> Option<(ForwardMessagesBody, Vec<OptimisticFanoutRow>)> {
        let target = Self::fanout_scope(Some(scope.chat_id.as_str()), scope.topic_id.as_deref())?;
        let now = RiseClock::now().time_interval_since_1970();
        let mut rows = Vec::new();
        for identifier in &retry.message_ids {
            let local_id = Self::fanout_local_id("fwd", &retry.idempotency_key, identifier, &target);
            let Ok(Some(entity)) = self.message_entity(&local_id).await else {
                continue;
            };
            if RiseMessageWireCodec::is_tombstone(&entity.data) {
                continue;
            }
            let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
                continue;
            };
            if !matches!(record.local_phase, RiseMessageLocalPhase::Failed) {
                continue;
            }
            let wire = RiseMessageWireCodec::restamped(&record.wire, now).unwrap_or(record.wire);
            rows.push(self.make_fanout_row(
                Self::fanout_row_id(identifier, &target),
                target.clone(),
                local_id,
                wire,
                Some(retry.clone()),
            ));
        }
        if rows.is_empty() {
            return None;
        }
        self.install_optimistic_fanout_rows(&rows).await;
        let body = ForwardMessagesBody {
            caption: retry.caption.clone(),
            destinations: Some(vec![DestinationsElement {
                chat_id: Some(target.chat_id.clone()),
                topic_id: target.topic_id.clone(),
            }]),
            idempotency_key: Some(retry.idempotency_key.clone()),
            message_ids: Some(retry.message_ids.clone()),
            target_chat_ids: None,
        };
        Some((body, rows))
    }

    pub async fn install_share_payload_optimistic_rows(&self, body: &RiseJSONValue) -> Vec<OptimisticFanoutRow> {
        let Some(object) = body.object_value() else {
            return Vec::new();
        };
        let Some(idempotency_key) = RiseMessageWireCodec::string(object, &[key::IDEMPOTENCY_KEY]) else {
            return Vec::new();
        };
        let Some(RiseJSONValue::Array(destinations)) = object.get(key::DESTINATIONS) else {
            return Vec::new();
        };
        if destinations.is_empty() {
            return Vec::new();
        }

        let content = RiseMessageWireCodec::string(object, &[key::CONTENT]).unwrap_or_default();
        let content_type = RiseMessageWireCodec::string(object, &[key::CONTENT_TYPE]).unwrap_or_else(|| "text".to_owned());
        let media_items = match object.get(key::MEDIA_ITEMS) {
            Some(RiseJSONValue::Array(items)) if !items.is_empty() => Some(RiseJSONValue::Array(items.clone())),
            _ => None,
        };
        if content.is_empty() && media_items.is_none() {
            return Vec::new();
        }

        let now = RiseClock::now().time_interval_since_1970();
        let mut rows = Vec::new();
        for value in destinations {
            let Some(destination) = value.object_value() else {
                return Vec::new();
            };
            let Some(scope) = Self::fanout_scope(
                RiseMessageWireCodec::string(destination, &[key::CHAT_ID]).as_deref(),
                RiseMessageWireCodec::string(destination, &[key::TOPIC_ID]).as_deref(),
            ) else {
                return Vec::new();
            };
            let mut message_object: BTreeMap<String, RiseJSONValue> = BTreeMap::from([
                (key::CONTENT.to_owned(), RiseJSONValue::String(content.clone())),
                (key::ORIGINAL_CONTENT.to_owned(), RiseJSONValue::String(content.clone())),
                (key::CONTENT_TYPE.to_owned(), RiseJSONValue::String(content_type.clone())),
                (key::SENDER_ID.to_owned(), RiseJSONValue::String(self.account_id.clone())),
                (key::CREATED_AT.to_owned(), RiseJSONValue::Number(now)),
                (key::TIMESTAMP.to_owned(), RiseJSONValue::Number(now)),
                (key::IS_READ.to_owned(), RiseJSONValue::Bool(false)),
                (key::IS_PINNED.to_owned(), RiseJSONValue::Bool(false)),
                (key::HAS_REACTIONS.to_owned(), RiseJSONValue::Bool(false)),
                (key::REACTIONS.to_owned(), RiseJSONValue::Array(Vec::new())),
                (key::REACTED_BY.to_owned(), RiseJSONValue::Array(Vec::new())),
                (key::PLAYED_BY.to_owned(), RiseJSONValue::Array(Vec::new())),
            ]);
            if let Some(media_items) = &media_items {
                message_object.insert(key::MEDIA_ITEMS.to_owned(), media_items.clone());
                message_object.insert(key::HAS_ATTACHMENTS.to_owned(), RiseJSONValue::Bool(true));
            }
            if let Some(entities) = RiseMessageWireCodec::string(object, &[key::ENTITIES]) {
                message_object.insert(key::ENTITIES.to_owned(), RiseJSONValue::String(entities));
            }
            let local_id = Self::fanout_local_id("share", &idempotency_key, "", &scope);
            let Ok(seed) = RiseMessageWireCodec::wire(&message_object) else {
                return Vec::new();
            };
            let Ok(wire) = RiseMessageWireCodec::installing_local_identity(&seed, &scope, &local_id, now) else {
                return Vec::new();
            };
            rows.push(self.make_fanout_row(Self::fanout_row_id("", &scope), scope, local_id, wire, None));
        }
        self.install_optimistic_fanout_rows(&rows).await;
        rows
    }

    pub async fn apply_forward_results(&self, response: &ForwardMessagesGatewayDTO, rows: &[OptimisticFanoutRow]) {
        let mut pending = Self::pending_fanout_rows(rows);
        let mut authoritative = Vec::new();
        let mut rejected = Vec::new();
        let mut refused = Vec::new();
        for result in response.results.as_deref().unwrap_or_default() {
            let destination = result.destination.as_ref();
            let Some(scope) = Self::fanout_scope(
                destination.map(|destination| destination.chat_id.as_str()),
                destination.and_then(|destination| destination.topic_id.as_deref()),
            ) else {
                continue;
            };
            let row = pending.remove(&Self::fanout_row_id(&result.source_message_id, &scope));
            let Some(wire) = result.message.clone().filter(|_| result.success) else {
                if let Some(row) = row {
                    if result.refusal().is_some() && !result.retryable {
                        refused.push(row);
                    } else {
                        rejected.push(row);
                    }
                }
                continue;
            };
            authoritative.push(RiseAuthoritativeMessage { wire, scope, local_id: row.map(|row| row.local_id) });
        }
        self.ingest_fanout_copies(&authoritative, rows, "forward_unacknowledged").await;
        self.drop_refused_fanout_rows(&refused).await;
        self.fail_optimistic_fanout_rows(&rejected, "forward_rejected").await;
        let unacknowledged: Vec<OptimisticFanoutRow> = pending.into_values().collect();
        self.fail_optimistic_fanout_rows(&unacknowledged, "forward_unacknowledged").await;
        self.release_live_fanout_rows(rows);
    }

    pub async fn apply_share_payload_results(
        &self,
        response: &RiseStoredSharePayloadReply,
        rows: &[OptimisticFanoutRow],
    ) {
        let mut pending = Self::pending_fanout_rows(rows);
        let mut authoritative = Vec::new();
        let mut rejected = Vec::new();
        let mut refused = Vec::new();
        for result in &response.results {
            let Some(scope) = Self::fanout_scope(Some(result.chat_id.as_str()), result.topic_id.as_deref()) else {
                continue;
            };
            let row = pending.remove(&Self::fanout_row_id("", &scope));
            let Some(wire) = result.message.clone().filter(|_| result.success) else {
                if let Some(row) = row {
                    if result.refusal().is_some() && !result.retryable {
                        refused.push(row);
                    } else {
                        rejected.push(row);
                    }
                }
                continue;
            };
            authoritative.push(RiseAuthoritativeMessage { wire, scope, local_id: row.map(|row| row.local_id) });
        }
        let unacknowledged: Vec<OptimisticFanoutRow> = pending.into_values().collect();
        let scopes_of = |rows: &[OptimisticFanoutRow]| rows.iter().map(|row| row.scope.storage_key()).collect::<Vec<_>>();
        tracing::warn!(
            target: "riseonly::X1057",
            results = response.results.len(),
            rows = rows.len(),
            authoritative = authoritative.len(),
            refused = ?scopes_of(refused.as_slice()),
            rejected = ?scopes_of(rejected.as_slice()),
            unacknowledged = ?scopes_of(unacknowledged.as_slice()),
            "share results applied"
        );
        self.ingest_fanout_copies(&authoritative, rows, "share_unacknowledged").await;
        self.drop_refused_fanout_rows(&refused).await;
        self.fail_optimistic_fanout_rows(&rejected, "share_rejected").await;
        self.fail_optimistic_fanout_rows(&unacknowledged, "share_unacknowledged").await;
        self.release_live_fanout_rows(rows);
    }

    fn pending_fanout_rows(rows: &[OptimisticFanoutRow]) -> HashMap<String, OptimisticFanoutRow> {
        rows.iter().map(|row| (row.id.clone(), row.clone())).collect()
    }

    /// a store failure while ingesting must not leave rows with a clock forever
    /// so every row still optimistic after the failed write turns failed and can be resent
    async fn ingest_fanout_copies(
        &self,
        authoritative: &[RiseAuthoritativeMessage],
        rows: &[OptimisticFanoutRow],
        code: &str,
    ) {
        let Err(error) = self.ingest_authoritative(authoritative).await else {
            return;
        };
        tracing::warn!(target: "riseonly::X1057", code, error = %error, "fan-out ingest failed: stranded rows fail");
        log_fanout(format!("fan-out ingest failed: {}", Self::error_code(&error))).await;
        let ingested: HashSet<&str> =
            authoritative.iter().filter_map(|message| message.local_id.as_deref()).collect();
        let mut stranded = Vec::new();
        for row in rows {
            if !ingested.contains(row.local_id.as_str()) {
                continue;
            }
            let Ok(Some(entity)) = self.message_entity(&row.local_id).await else {
                continue;
            };
            let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
                continue;
            };
            if !matches!(record.source, RiseConversationEntitySource::Optimistic) {
                continue;
            }
            stranded.push(row.clone());
        }
        self.fail_optimistic_fanout_rows(&stranded, code).await;
    }

    /// a destination that refused the copy outright leaves nothing to resend
    /// sheet already names the reason so no failed bubble
    /// a refusal with no code keeps its failed row
    async fn drop_refused_fanout_rows(&self, rows: &[OptimisticFanoutRow]) {
        for row in rows {
            let Ok(Some(entity)) = self.message_entity(&row.local_id).await else {
                continue;
            };
            if RiseMessageWireCodec::is_tombstone(&entity.data) {
                continue;
            }
            let _ = self.remove_outgoing_locally(&entity, &row.scope, None).await;
        }
    }

    pub async fn fail_optimistic_fanout_rows(&self, rows: &[OptimisticFanoutRow], code: &str) {
        for row in rows {
            let _ = self
                .write_create_failure(
                    &row.wire,
                    &row.scope,
                    &row.scopes,
                    &row.local_id,
                    &row.request_string,
                    code,
                    self.next_version(),
                    None,
                    row.forward_retry.clone(),
                )
                .await;
        }
        self.release_live_fanout_rows(rows);
    }

    fn release_live_fanout_rows(&self, rows: &[OptimisticFanoutRow]) {
        self.state.with(|state| {
            for row in rows {
                state.live_fanout_local_ids.remove(&row.local_id);
            }
        });
    }

    pub fn is_fanout_local_id(local_id: &str) -> bool {
        let prefix = RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX;
        local_id.strip_prefix(prefix).is_some_and(|rest| rest.starts_with("fwd_") || rest.starts_with("share_"))
    }

    pub fn settle_orphaned_fanout_if_needed(&self, local_id: &str) {
        let is_settling = self.state.with(|state| {
            !state.live_fanout_local_ids.contains(local_id) && state.settled_orphan_fanout_ids.insert(local_id.to_owned())
        });
        if !is_settling {
            return;
        }
        let Some(this) = self.this.upgrade() else {
            return;
        };
        let local_id = local_id.to_owned();
        RiseAccountRuntime::current().spawn(async move {
            this.fail_orphaned_fanout(&local_id).await;
        });
    }

    async fn fail_orphaned_fanout(&self, local_id: &str) {
        let Ok(Some(entity)) = self.message_entity(local_id).await else {
            return;
        };
        if RiseMessageWireCodec::is_tombstone(&entity.data) {
            return;
        }
        let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
            return;
        };
        if !matches!(record.local_phase, RiseMessageLocalPhase::Sending)
            || !matches!(record.source, RiseConversationEntitySource::Optimistic)
        {
            return;
        }
        let Some(request_string) = record.client_request_id.clone().filter(|request| !request.is_empty()) else {
            return;
        };
        let is_live = self.state.with(|state| {
            state.live_fanout_local_ids.contains(&entity.local_id) || state.live_fanout_local_ids.contains(local_id)
        });
        if is_live {
            return;
        }
        let fallback_chat_id = record.wire.chat_id.clone().unwrap_or_default();
        let scope = record.scope_storage_key.as_deref().and_then(RiseMessageScope::parse).or_else(|| {
            if fallback_chat_id.is_empty() { None } else { Some(RiseMessageScope::new(fallback_chat_id, None, None)) }
        });
        let Some(scope) = scope else {
            return;
        };
        tracing::warn!(target: "riseonly::X1057", local_id = %entity.local_id, "orphaned fan-out row failed (not live)");
        let is_forward = record.forward_retry.is_some()
            || entity.local_id.starts_with(&format!("{}fwd_", RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX));
        let scopes =
            self.scopes_for_message(&scope.chat_id, scope.topic_id.as_deref(), scope.thread_root_id.as_deref());
        let _ = self
            .write_create_failure(
                &record.wire,
                &scope,
                &scopes,
                &entity.local_id,
                &request_string,
                if is_forward { "forward_unacknowledged" } else { "share_unacknowledged" },
                self.next_version_floor(entity.version),
                None,
                record.forward_retry.clone(),
            )
            .await;
    }

    async fn install_optimistic_fanout_rows(&self, rows: &[OptimisticFanoutRow]) {
        if rows.is_empty() {
            return;
        }
        let mut batch = RiseStoreWriteBatch::empty();
        for row in rows {
            self.state.with(|state| {
                state.known_scopes.insert(row.scope.clone());
                state.live_fanout_local_ids.insert(row.local_id.clone());
            });
            let Ok(next) = self
                .create_optimistic_batch(
                    &row.wire,
                    &row.scope,
                    &row.scopes,
                    &row.local_id,
                    &row.request_string,
                    RiseMessageLocalPhase::Sending,
                    self.next_version(),
                    None,
                    row.forward_retry.clone(),
                )
                .await
            else {
                continue;
            };
            if let Err(error) = self.accumulate(next.clone(), &mut batch).await {
                log_fanout(format!("fan-out optimistic rows not installed: {}", Self::error_code(&error))).await;
                batch = next;
            }
        }
        if batch.mutations.is_empty() && batch.invalidated_views.is_empty() {
            return;
        }
        let _ = self.engine.write(batch).await;
    }

    fn make_fanout_row(
        &self,
        id: String,
        scope: RiseMessageScope,
        local_id: String,
        wire: MessageWireDTO,
        forward_retry: Option<RiseMessageForwardRetry>,
    ) -> OptimisticFanoutRow {
        let scopes =
            self.scopes_for_message(&scope.chat_id, scope.topic_id.as_deref(), scope.thread_root_id.as_deref());
        OptimisticFanoutRow {
            id,
            scope,
            scopes,
            local_id,
            wire,
            request_string: RiseRequestIDAllocator::shared().next(RiseClock::now()).raw_value().to_string(),
            forward_retry,
        }
    }

    fn fanout_scope(chat_id: Option<&str>, topic_id: Option<&str>) -> Option<RiseMessageScope> {
        let chat_id = RiseMessageWireCodec::normalized(chat_id)?;
        Some(RiseMessageScope::new(chat_id, RiseMessageWireCodec::normalized(topic_id), None))
    }

    /// stable across a retry with the same idempotency key
    /// so send again after a partial failure reuses the row and doesnt stack a second bubble
    fn fanout_local_id(kind: &str, idempotency_key: &str, discriminator: &str, scope: &RiseMessageScope) -> String {
        format!(
            "{}{}_{}_{}_{}",
            RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX,
            kind,
            idempotency_key,
            discriminator.to_lowercase(),
            scope.storage_key(),
        )
    }

    fn fanout_row_id(discriminator: &str, scope: &RiseMessageScope) -> String {
        format!("{}\u{0}{}", trimming_whitespaces_and_newlines(discriminator).to_lowercase(), scope.storage_key())
    }
}
