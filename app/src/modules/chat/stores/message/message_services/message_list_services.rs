use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use gpui::{App, Context};
use rise_canon::RiseWireKey;
use rise_canon::rise_key::message as message_key;
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::rise_core::rise_identifiers::{RiseRequestID, RiseRequestIDAllocator};
use serde_json::{Map, Value};

use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::{
    SaiFetchJsonEncoder, SaiFetchKeyEncodingStrategy,
};
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::ui::lottie_sticker::lottie_sticker_prefetch::LottieStickerPrefetch;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageContentType, MessageHistoryDirection, MessageHistoryPageState, MessageSystemKind, MessageThreadOrder,
    MessageThreadOrderKey,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    GetMessagesAroundGatewayDTO, GetMessagesGatewayDTO, MessagePayloadCodec, MessageWireDTO,
};
use crate::modules::chat::stores::message::message_services::message_pagination_trace::MessagePaginationTrace;
use crate::modules::chat::stores::message::message_services::message_presentation_window_projector::MessagePresentationWindowProjector;
use crate::modules::chat::stores::message::message_services::message_services::{
    MessageSeedRefreshFence, MessageServicesStore,
};

type MessageRow = Map<String, Value>;

const TEMP_ID_PREFIX: &str = "temp_";
const HISTORY_PAGINATION_KEY: &str = "history_pagination";

static IS_TEMP_FLAG_KEY: LazyLock<String> = LazyLock::new(|| RiseWireKey::camel(RiseLocalKey::IS_TEMP));
static CAMEL_MESSAGE_ID_KEY: LazyLock<String> = LazyLock::new(|| RiseWireKey::camel(message_key::MESSAGE_ID));

fn trimmed_string<'a>(row: &'a MessageRow, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).map(trimming_whitespaces_and_newlines).unwrap_or_default()
}

fn rows_value(rows: Vec<MessageRow>) -> Value {
    Value::Array(rows.into_iter().map(Value::Object).collect())
}

fn row_count(rows: &[MessageRow]) -> Option<i64> {
    i64::try_from(rows.len()).ok()
}

fn optional_text(value: Option<&str>) -> &str {
    value.unwrap_or("nil")
}

fn is_contact_joined_row(row: &MessageRow) -> bool {
    row.get(message_key::SYSTEM_MESSAGE_TYPE).and_then(Value::as_str)
        == Some(MessageSystemKind::ContactJoined.raw_value())
}

fn keep_newest(mut rows: Vec<MessageRow>, limit: usize) -> Vec<MessageRow> {
    let start = rows.len().saturating_sub(limit);
    rows.split_off(start)
}

fn presentation_window_limit() -> usize {
    usize::try_from(MessagePresentationWindowProjector::limit()).unwrap_or(0)
}

fn merge_sorted<T>(mut items: Vec<T>, precedes: &impl Fn(&T, &T) -> bool) -> Vec<T> {
    if items.len() < 2 {
        return items;
    }
    let right = items.split_off(items.len() / 2);
    let left = merge_sorted(items, precedes);
    let right = merge_sorted(right, precedes);
    let mut merged = Vec::with_capacity(left.len() + right.len());
    let mut left = left.into_iter().peekable();
    let mut right = right.into_iter().peekable();
    while let (Some(first), Some(second)) = (left.peek(), right.peek()) {
        if precedes(second, first) {
            merged.extend(right.next());
        } else {
            merged.extend(left.next());
        }
    }
    merged.extend(left);
    merged.extend(right);
    merged
}

struct IndexedMessageRow {
    index: usize,
    row: MessageRow,
    timestamp: f64,
    id: Option<String>,
}

impl MessageServicesStore {
    pub fn bump_chat_list(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if RiseConversationPresentation::shared(cx).read(cx).is_active() || chat_id.is_empty() {
            return;
        }
        let Some(last) = self.messages(chat_id, cx).pop() else {
            return;
        };
        ChatServicesStore::shared(cx).update(cx, |services, cx| {
            services.apply_optimistic_last_message(chat_id, last, cx);
        });
    }

    pub fn reconcile_outgoing_messages_cache(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let cache_key = Self::messages_cache_key(chat_id);
        let Some(payload) = SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        ) else {
            return;
        };
        let Some(mut root) = MessagePayloadCodec::dictionary(Some(&payload)) else {
            return;
        };

        let mut did_change = false;
        let rows = MessagePayloadCodec::array_of_dictionaries(root.get(message_key::MESSAGES));
        let reconciled: Vec<MessageRow> = rows
            .iter()
            .map(|row| {
                let (row, changed) = self.reconcile_outgoing_message_row(row);
                did_change = did_change || changed;
                row
            })
            .collect();
        if !did_change {
            return;
        }
        root.insert(message_key::MESSAGES.to_owned(), rows_value(reconciled));
        let did_patch = SaiWsResponseCache::apply_sai_updater_payload(
            Value::Object(root),
            &cache_key,
            SaiUpdaterWriteScope::Cache,
            false,
            cx,
        )
        .is_ok();
        if did_patch {
            self.mark_messages_cache_for_disk_persistence(chat_id, cx);
        }
    }

    fn reconcile_outgoing_message_row(&self, row: &MessageRow) -> (MessageRow, bool) {
        let mut out = row.clone();
        let mut changed = false;
        let id = row.get(message_key::ID).and_then(Value::as_str).unwrap_or_default();
        let has_server_id = !trimmed_string(row, RiseLocalKey::SERVER_ID).is_empty();
        let temp_flag = row.get(IS_TEMP_FLAG_KEY.as_str()).and_then(Value::as_bool);

        let was_temp = temp_flag == Some(true);
        if has_server_id || temp_flag == Some(false) {
            if temp_flag != Some(false) {
                out.insert(IS_TEMP_FLAG_KEY.clone(), Value::Bool(false));
                changed = true;
            }
            if was_temp
                && let Some(Value::Array(media)) = out.get_mut(message_key::MEDIA_ITEMS)
                && media.iter().all(Value::is_object)
            {
                for item in media.iter_mut().filter_map(Value::as_object_mut) {
                    item.insert(RiseLocalKey::IS_UPLOADING.to_owned(), Value::Bool(false));
                    item.insert(RiseLocalKey::IS_COMPRESSING.to_owned(), Value::Bool(false));
                    item.insert(RiseLocalKey::UPLOAD_PROGRESS.to_owned(), Value::from(100));
                    item.remove(RiseLocalKey::UPLOADED_BYTES);
                    item.remove(RiseLocalKey::TOTAL_BYTES);
                    item.remove(RiseLocalKey::COMPRESSION_PROGRESS);
                }
                changed = true;
            }
        }

        if MessageThreadOrder::timestamp_seconds(&out).is_none() {
            if let Some(timestamp) = self.timestamp_from_temp_message_id(id) {
                out.insert(message_key::CREATED_AT.to_owned(), Value::from(timestamp));
                out.insert(message_key::TIMESTAMP.to_owned(), Value::from(timestamp));
                changed = true;
            } else if out.get(message_key::CONTENT_TYPE).and_then(Value::as_str)
                == Some(MessageContentType::System.raw_value())
            {
                let now = RiseClock::now().time_interval_since_1970();
                out.insert(message_key::CREATED_AT.to_owned(), Value::from(now));
                out.insert(message_key::TIMESTAMP.to_owned(), Value::from(now));
                changed = true;
            }
        }
        (out, changed)
    }

    pub fn timestamp_from_temp_message_id(&self, id: &str) -> Option<TimeInterval> {
        if !GetSaiServerId::is_temp_id(Some(id)) {
            return None;
        }
        let body = id.get(TEMP_ID_PREFIX.len()..)?;
        let wire_part = body.split('_').find(|part| !part.is_empty())?;
        let wire_id = wire_part.parse::<u64>().ok()?;
        let request_id = RiseRequestID::new(wire_id)?;
        Some(RiseRequestIDAllocator::timestamp_seconds(request_id) as TimeInterval)
    }

    pub fn is_create_message_wire_payload(&self, any: Option<&Value>) -> bool {
        let Some(dict) = MessagePayloadCodec::dictionary(any) else {
            return false;
        };
        dict.contains_key(message_key::MESSAGE)
            || dict.contains_key(message_key::MESSAGE_ID)
            || dict.contains_key(CAMEL_MESSAGE_ID_KEY.as_str())
    }

    pub fn get_messages_success_handler(
        &mut self,
        any: Option<&Value>,
        chat_id: &str,
        is_first_page: bool,
        reconcile_cache: bool,
        cx: &mut Context<Self>,
    ) {
        if !is_first_page {
            return;
        }
        self.finish_initial_messages_request(chat_id, cx);
        if reconcile_cache {
            self.reconcile_outgoing_messages_cache(chat_id, cx);
        }

        let page: Option<GetMessagesGatewayDTO> = self.decode_gateway(any);
        let fresh_messages = page.and_then(|page| page.messages).unwrap_or_default();
        let urls: Vec<String> =
            fresh_messages.iter().filter_map(|message| message.to_message().lottie_sticker_prefetch_url()).collect();
        LottieStickerPrefetch::warm(&urls, cx);
    }

    pub fn get_messages_error_handler(
        &mut self,
        error: &anyhow::Error,
        chat_id: &str,
        direction: Option<MessageHistoryDirection>,
        request_token: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        if let Some(direction) = direction {
            self.finish_message_history_request(chat_id, direction, request_token, true, cx);
        } else {
            self.fail_initial_messages_request(error, chat_id, cx);
        }
        let line = format!("get_messages({chat_id}) error: {error}");
        Sai::log("Message", move || line, SaiLogKind::Network, cx);
    }

    pub fn install_initial_messages_page(&mut self, any: Option<&Value>, chat_id: &str, cx: &mut Context<Self>) {
        let Some(page) = self.decode_gateway::<GetMessagesGatewayDTO>(any) else {
            return;
        };
        let Some(fresh_rows) = self.decoded_message_rows(any, page.messages.as_deref()) else {
            return;
        };
        let incoming_count = row_count(&fresh_rows);
        let rows = self.chronological_message_rows(self.deduplicated_message_rows(fresh_rows));
        let older = self.validated_history_page_state(&page, None, cx);
        let newer = MessageHistoryPageState {
            relative_id: self.boundary_relative_id(&rows, MessageHistoryDirection::Newer),
            has_more: false,
        };
        let result_count = row_count(&rows);
        self.patch_messages_root_with_history(
            chat_id,
            older.relative_id.as_deref(),
            Some(older.has_more),
            HashMap::from([
                (MessageHistoryDirection::Older, older.clone()),
                (MessageHistoryDirection::Newer, newer),
            ]),
            rows,
            cx,
        );
        MessagePaginationTrace::record(
            "merge.initial",
            chat_id,
            Some(MessageHistoryDirection::Older),
            None,
            older.relative_id.as_deref(),
            None,
            None,
            incoming_count,
            result_count,
            Some(older.has_more),
            None,
            cx,
        );
    }

    pub fn snapshot_latest_messages_page(
        &mut self,
        any: Option<&Value>,
        chat_id: &str,
        refresh_fence: Option<&MessageSeedRefreshFence>,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.decode_gateway::<GetMessagesGatewayDTO>(any) else {
            return;
        };
        let Some(fresh_rows) = self.decoded_message_rows(any, page.messages.as_deref()) else {
            return;
        };

        let latest_cache_key = Self::latest_messages_cache_key(chat_id);
        let existing_root = SaiWsResponseCache::memory_only(&latest_cache_key, cx);
        let existing_root = MessagePayloadCodec::dictionary(existing_root.as_ref());
        let existing_rows = MessagePayloadCodec::array_of_dictionaries(
            existing_root.as_ref().and_then(|root| root.get(message_key::MESSAGES)),
        );
        let canonical_root = SaiWsResponseCache::memory_only(&Self::messages_cache_key(chat_id), cx);
        let canonical_root = MessagePayloadCodec::dictionary(canonical_root.as_ref());
        let limit = presentation_window_limit();
        let canonical_candidates = self.newest_candidate_rows(
            MessagePayloadCodec::array_of_dictionaries(
                canonical_root.as_ref().and_then(|root| root.get(message_key::MESSAGES)),
            ),
            limit,
        );
        let preservation_rows = self.deduplicated_message_rows([existing_rows, canonical_candidates].concat());
        let concurrent_rows = self.rows_arriving_after_seed_fence(&preservation_rows, &fresh_rows, refresh_fence);
        let local_rows: Vec<MessageRow> = preservation_rows
            .into_iter()
            .filter(|row| self.is_unresolved_optimistic_message_row(row) || is_contact_joined_row(row))
            .collect();
        let chronological = self
            .chronological_message_rows(self.deduplicated_message_rows([concurrent_rows, local_rows, fresh_rows].concat()));
        let rows = keep_newest(chronological, limit);
        let older = self.validated_history_page_state(&page, None, cx);
        let newer = MessageHistoryPageState {
            relative_id: self.boundary_relative_id(&rows, MessageHistoryDirection::Newer),
            has_more: false,
        };
        let mut root = MessagePayloadCodec::dictionary(any).unwrap_or_default();
        root.insert(message_key::CHAT_ID.to_owned(), Value::from(chat_id));
        root.insert(message_key::MESSAGES.to_owned(), rows_value(rows));
        match &older.relative_id {
            Some(relative_id) => {
                root.insert(message_key::RELATIVE_ID.to_owned(), Value::from(relative_id.as_str()));
            }
            None => {
                root.remove(message_key::RELATIVE_ID);
            }
        }
        root.insert(message_key::IS_HAVE_MORE.to_owned(), Value::Bool(older.has_more));
        let mut older_lane = Map::new();
        older_lane.insert(message_key::HAS_MORE.to_owned(), Value::Bool(older.has_more));
        if let Some(relative_id) = &older.relative_id {
            older_lane.insert(message_key::RELATIVE_ID.to_owned(), Value::from(relative_id.as_str()));
        }
        let mut newer_lane = Map::new();
        newer_lane.insert(message_key::HAS_MORE.to_owned(), Value::Bool(false));
        if let Some(relative_id) = &newer.relative_id {
            newer_lane.insert(message_key::RELATIVE_ID.to_owned(), Value::from(relative_id.as_str()));
        }
        let mut history_pagination = Map::new();
        history_pagination.insert(MessageHistoryDirection::Older.raw_value().to_owned(), Value::Object(older_lane));
        history_pagination.insert(MessageHistoryDirection::Newer.raw_value().to_owned(), Value::Object(newer_lane));
        root.insert(HISTORY_PAGINATION_KEY.to_owned(), Value::Object(history_pagination));

        let did_snapshot =
            SaiUpdater::replace_root(&latest_cache_key, root, SaiUpdaterWriteScope::Cache, cx).is_ok();
        if did_snapshot {
            self.mark_latest_messages_cache_for_disk_persistence(chat_id, cx);
        }
    }

    pub fn install_latest_messages_snapshot(&mut self, chat_id: &str, cx: &mut Context<Self>) -> bool {
        let latest_cache_key = Self::latest_messages_cache_key(chat_id);
        let payload = SaiWsResponseCache::current_payload(
            &latest_cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        let Some(root) = MessagePayloadCodec::dictionary(payload.as_ref()) else {
            if self.message_history_page_state(chat_id, MessageHistoryDirection::Newer, cx).has_more
                || self.cached_message_count(chat_id, cx) <= 0
            {
                return false;
            }
            let _ = self.project_message_presentation_to_latest(chat_id, cx);
            return true;
        };

        let canonical_cache_key = Self::messages_cache_key(chat_id);
        let latest_rows = MessagePayloadCodec::array_of_dictionaries(root.get(message_key::MESSAGES));
        let canonical_payload = SaiWsResponseCache::current_payload(
            &canonical_cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        let canonical_root = MessagePayloadCodec::dictionary(canonical_payload.as_ref());
        let canonical_rows = MessagePayloadCodec::array_of_dictionaries(
            canonical_root.as_ref().and_then(|root| root.get(message_key::MESSAGES)),
        );
        if latest_rows.is_empty() && !canonical_rows.is_empty() {
            if self.message_history_page_state(chat_id, MessageHistoryDirection::Newer, cx).has_more {
                return false;
            }
            let _ = self.project_message_presentation_to_latest(chat_id, cx);
            return true;
        }
        if self.canonical_cache_can_project_latest(chat_id, &canonical_rows, &latest_rows, cx) {
            let _ = self.project_message_presentation_to_latest(chat_id, cx);
            return true;
        }

        let latest_aliases: HashSet<String> =
            latest_rows.iter().flat_map(|row| self.message_row_aliases(row)).collect();
        let local_rows: Vec<MessageRow> = canonical_rows
            .into_iter()
            .filter(|row| {
                (self.is_unresolved_optimistic_message_row(row) || is_contact_joined_row(row))
                    && self.message_row_aliases(row).is_disjoint(&latest_aliases)
            })
            .collect();
        let installed_rows = keep_newest(
            self.chronological_message_rows(self.deduplicated_message_rows([local_rows, latest_rows].concat())),
            presentation_window_limit(),
        );
        let mut installed_root = root;
        installed_root.insert(message_key::MESSAGES.to_owned(), rows_value(installed_rows));

        let did_install =
            SaiUpdater::replace_root(&canonical_cache_key, installed_root, SaiUpdaterWriteScope::Cache, cx).is_ok();
        if !did_install {
            return false;
        }

        self.mark_messages_cache_for_disk_persistence(chat_id, cx);
        let scroll_position_key = self.scroll_position_key(chat_id);
        ChatScrollPositionStore::clear(&scroll_position_key, cx);
        let _ = self.project_message_presentation_to_latest(chat_id, cx);
        true
    }

    pub fn request_latest_messages_jump(&mut self, chat_id: &str, cx: &mut Context<Self>) -> bool {
        let installed = self.install_latest_messages_snapshot(chat_id, cx);
        if installed {
            self.pending_latest_jump_scopes.remove(chat_id);
        } else {
            self.pending_latest_jump_scopes.insert(chat_id.to_owned());
        }
        cx.notify();
        installed
    }

    pub fn finish_pending_latest_messages_jump(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if !self.pending_latest_jump_scopes.remove(chat_id) {
            return;
        }
        if !self.install_latest_messages_snapshot(chat_id, cx) {
            self.pending_latest_jump_scopes.insert(chat_id.to_owned());
            cx.notify();
            return;
        }
        let revision = self.latest_jump_completion_revisions.entry(chat_id.to_owned()).or_insert(0);
        *revision = revision.wrapping_add(1);
        cx.notify();
    }

    pub fn latest_jump_completion_revision(&self, chat_id: &str) -> u64 {
        self.latest_jump_completion_revisions.get(chat_id).copied().unwrap_or(0)
    }

    pub fn cancel_pending_latest_messages_jump(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.pending_latest_jump_scopes.remove(chat_id);
        cx.notify();
    }

    fn canonical_cache_can_project_latest(
        &mut self,
        chat_id: &str,
        canonical_rows: &[MessageRow],
        latest_rows: &[MessageRow],
        cx: &mut Context<Self>,
    ) -> bool {
        if canonical_rows.is_empty()
            || self.message_history_page_state(chat_id, MessageHistoryDirection::Newer, cx).has_more
        {
            return false;
        }
        let (Some(canonical_last), Some(latest_last)) =
            (self.newest_boundary_row(canonical_rows), self.newest_boundary_row(latest_rows))
        else {
            return false;
        };
        if !self.message_row_aliases(canonical_last).is_disjoint(&self.message_row_aliases(latest_last)) {
            return true;
        }
        let (Some(canonical_key), Some(latest_key)) =
            (self.message_row_order_key(canonical_last), self.message_row_order_key(latest_last))
        else {
            return false;
        };
        !MessageThreadOrder::precedes(&canonical_key, &latest_key)
    }

    fn newest_boundary_row<'a>(&self, rows: &'a [MessageRow]) -> Option<&'a MessageRow> {
        let (first, last) = (rows.first()?, rows.last()?);
        if rows.len() <= 1 {
            return Some(last);
        }
        let (Some(first_key), Some(last_key)) = (self.message_row_order_key(first), self.message_row_order_key(last))
        else {
            return Some(last);
        };
        Some(if MessageThreadOrder::precedes(&first_key, &last_key) { last } else { first })
    }

    fn newest_candidate_rows(&self, mut rows: Vec<MessageRow>, limit: usize) -> Vec<MessageRow> {
        if rows.len() <= limit {
            return rows;
        }
        let keys = rows
            .first()
            .and_then(|first| self.message_row_order_key(first))
            .zip(rows.last().and_then(|last| self.message_row_order_key(last)));
        let Some((first_key, last_key)) = keys else {
            return rows;
        };
        if MessageThreadOrder::precedes(&first_key, &last_key) {
            return keep_newest(rows, limit);
        }
        rows.truncate(limit);
        rows
    }

    pub fn install_messages_around_page(
        &mut self,
        any: Option<&Value>,
        chat_id: &str,
        requested_anchor: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(page) = self.decode_gateway::<GetMessagesAroundGatewayDTO>(any) else {
            return false;
        };
        if page.success == Some(false) {
            return false;
        }
        let Some(fresh_rows) = self.decoded_message_rows(any, page.messages.as_deref()) else {
            return false;
        };
        let incoming_count = row_count(&fresh_rows);
        let rows = self.chronological_message_rows(self.deduplicated_message_rows(fresh_rows));
        let resolved_anchor = page
            .anchor_message_id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|anchor| !anchor.is_empty())
            .unwrap_or(requested_anchor)
            .to_owned();
        if !rows.iter().any(|row| self.message_row(row, &resolved_anchor)) {
            return false;
        }

        let older = self.validated_around_history_page_state(
            page.older_relative_id.as_deref(),
            page.has_older.unwrap_or(false),
            MessageHistoryDirection::Older,
            cx,
        );
        let newer = self.validated_around_history_page_state(
            page.newer_relative_id.as_deref(),
            page.has_newer.unwrap_or(false),
            MessageHistoryDirection::Newer,
            cx,
        );
        let result_count = row_count(&rows);
        let detail = format!("hasOlder={} hasNewer={}", older.has_more, newer.has_more);
        self.patch_messages_root_with_history(
            chat_id,
            older.relative_id.as_deref(),
            Some(older.has_more),
            HashMap::from([
                (MessageHistoryDirection::Older, older.clone()),
                (MessageHistoryDirection::Newer, newer),
            ]),
            rows,
            cx,
        );
        self.reset_message_presentation_window(chat_id, &resolved_anchor, cx);
        MessagePaginationTrace::record(
            "merge.around",
            chat_id,
            None,
            None,
            None,
            Some(&resolved_anchor),
            None,
            incoming_count,
            result_count,
            None,
            Some(&detail),
            cx,
        );
        true
    }

    pub fn merge_messages_page(
        &mut self,
        any: Option<&Value>,
        chat_id: &str,
        direction: MessageHistoryDirection,
        requested_relative_id: Option<&str>,
        request_token: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        let failed = self.merge_messages_page_body(any, chat_id, direction, requested_relative_id, request_token, cx);
        self.finish_message_history_request(chat_id, direction, request_token, failed, cx);
    }

    fn merge_messages_page_body(
        &mut self,
        any: Option<&Value>,
        chat_id: &str,
        direction: MessageHistoryDirection,
        requested_relative_id: Option<&str>,
        request_token: Option<u64>,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(token) = request_token
            && !self.is_current_message_history_request(chat_id, direction, token)
        {
            MessagePaginationTrace::record(
                "response.drop.stale-lane",
                chat_id,
                Some(direction),
                request_token,
                requested_relative_id,
                None,
                None,
                None,
                None,
                None,
                None,
                cx,
            );
            return false;
        }
        let decoded = self.decode_gateway::<GetMessagesGatewayDTO>(any).and_then(|page| {
            let rows = self.decoded_message_rows(any, page.messages.as_deref())?;
            Some((page, rows))
        });
        let Some((page, fresh_rows)) = decoded else {
            MessagePaginationTrace::record(
                "response.reject",
                chat_id,
                Some(direction),
                request_token,
                requested_relative_id,
                None,
                None,
                None,
                None,
                None,
                Some("reason=decode-failed"),
                cx,
            );
            return true;
        };
        let incoming_count = row_count(&fresh_rows);
        let current_cursor = self.message_history_page_state(chat_id, direction, cx).relative_id;
        if requested_relative_id.is_some() && current_cursor.as_deref() != requested_relative_id {
            let detail = format!("current={}", optional_text(current_cursor.as_deref()));
            MessagePaginationTrace::record(
                "response.drop.cursor-changed",
                chat_id,
                Some(direction),
                request_token,
                requested_relative_id,
                None,
                None,
                incoming_count,
                None,
                None,
                Some(&detail),
                cx,
            );
            return false;
        }

        let cache_key = Self::messages_cache_key(chat_id);
        let payload = SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        let existing_rows = MessagePayloadCodec::dictionary(payload.as_ref())
            .map(|root| MessagePayloadCodec::array_of_dictionaries(root.get(message_key::MESSAGES)))
            .unwrap_or_default();

        let cached_count = row_count(&existing_rows);
        let merged = self.merged_chronological_message_rows(existing_rows, fresh_rows);
        let next_state = self.validated_history_page_state(&page, requested_relative_id, cx);

        let is_older = direction == MessageHistoryDirection::Older;
        let result_count = row_count(&merged);
        self.patch_messages_root_with_history(
            chat_id,
            if is_older { next_state.relative_id.as_deref() } else { None },
            is_older.then_some(next_state.has_more),
            HashMap::from([(direction, next_state.clone())]),
            merged,
            cx,
        );
        let detail = format!("requested={}", optional_text(requested_relative_id));
        MessagePaginationTrace::record(
            "merge.page",
            chat_id,
            Some(direction),
            request_token,
            next_state.relative_id.as_deref(),
            None,
            cached_count,
            incoming_count,
            result_count,
            Some(next_state.has_more),
            Some(&detail),
            cx,
        );
        false
    }

    pub fn merge_messages_refresh(
        &mut self,
        any: Option<&Value>,
        chat_id: &str,
        refresh_fence: Option<&MessageSeedRefreshFence>,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.decode_gateway::<GetMessagesGatewayDTO>(any) else {
            return;
        };
        let Some(fresh_decoded_rows) = self.decoded_message_rows(any, page.messages.as_deref()) else {
            return;
        };

        let previous_older = self.message_history_page_state(chat_id, MessageHistoryDirection::Older, cx);
        let cache_key = Self::messages_cache_key(chat_id);
        let payload = SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        let Some(root) = MessagePayloadCodec::dictionary(payload.as_ref()) else {
            return;
        };

        let old_rows = self.chronological_message_rows(self.deduplicated_message_rows(
            MessagePayloadCodec::array_of_dictionaries(root.get(message_key::MESSAGES)),
        ));
        let fresh_rows = self.chronological_message_rows(self.deduplicated_message_rows(fresh_decoded_rows));
        let fresh_aliases: HashSet<String> = fresh_rows.iter().flat_map(|row| self.message_row_aliases(row)).collect();
        let concurrent_rows = self.rows_arriving_after_seed_fence(&old_rows, &fresh_rows, refresh_fence);

        let preserved_rows: Vec<MessageRow> = if fresh_rows.is_empty() || page.is_have_more == Some(false) {
            Vec::new()
        } else {
            self.oldest_refresh_boundary(&old_rows, &fresh_rows)
                .map(|boundary| old_rows[..boundary].to_vec())
                .unwrap_or_default()
        };

        let unmatched_optimistic_rows: Vec<MessageRow> = old_rows
            .iter()
            .filter(|row| {
                self.is_unresolved_optimistic_message_row(row)
                    && self.message_row_aliases(row).is_disjoint(&fresh_aliases)
            })
            .cloned()
            .collect();
        let recipient_only_rows: Vec<MessageRow> =
            old_rows.iter().filter(|row| is_contact_joined_row(row)).cloned().collect();
        let kept_older_history = preserved_rows.iter().any(|row| !self.is_unresolved_optimistic_message_row(row));
        let detail = format!(
            "preserved={} concurrent={} optimistic={} overlap={}",
            preserved_rows.len(),
            concurrent_rows.len(),
            unmatched_optimistic_rows.len(),
            kept_older_history,
        );
        let kept_rows = self.deduplicated_message_rows(
            [preserved_rows, concurrent_rows, unmatched_optimistic_rows, recipient_only_rows].concat(),
        );
        let incoming_count = row_count(&fresh_rows);
        let merged =
            self.chronological_message_rows(self.deduplicated_message_rows([kept_rows, fresh_rows].concat()));
        let older = if kept_older_history {
            previous_older
        } else {
            self.validated_history_page_state(&page, None, cx)
        };
        let newer = MessageHistoryPageState {
            relative_id: self.boundary_relative_id(&merged, MessageHistoryDirection::Newer),
            has_more: false,
        };

        let result_count = row_count(&merged);
        self.patch_messages_root_with_history(
            chat_id,
            older.relative_id.as_deref(),
            Some(older.has_more),
            HashMap::from([
                (MessageHistoryDirection::Older, older.clone()),
                (MessageHistoryDirection::Newer, newer),
            ]),
            merged,
            cx,
        );
        MessagePaginationTrace::record(
            "merge.refresh",
            chat_id,
            Some(MessageHistoryDirection::Older),
            None,
            older.relative_id.as_deref(),
            None,
            row_count(&old_rows),
            incoming_count,
            result_count,
            Some(older.has_more),
            Some(&detail),
            cx,
        );
    }

    fn rows_arriving_after_seed_fence(
        &self,
        existing_rows: &[MessageRow],
        fresh_rows: &[MessageRow],
        refresh_fence: Option<&MessageSeedRefreshFence>,
    ) -> Vec<MessageRow> {
        let Some(refresh_fence) = refresh_fence else {
            return Vec::new();
        };
        let fresh_aliases: HashSet<String> = fresh_rows.iter().flat_map(|row| self.message_row_aliases(row)).collect();
        existing_rows
            .iter()
            .filter(|row| {
                if !self.message_row_aliases(row).is_disjoint(&fresh_aliases)
                    || self.is_unresolved_optimistic_message_row(row)
                    || is_contact_joined_row(row)
                {
                    return false;
                }
                let Some(boundary) = &refresh_fence.latest_boundary else {
                    return true;
                };
                self.message_row_order_key(row).is_some_and(|key| MessageThreadOrder::precedes(boundary, &key))
            })
            .cloned()
            .collect()
    }

    fn decoded_message_rows(&self, any: Option<&Value>, messages: Option<&[MessageWireDTO]>) -> Option<Vec<MessageRow>> {
        if let Some(dictionary) = MessagePayloadCodec::dictionary(any)
            && let Some(raw_messages) = dictionary.get(message_key::MESSAGES)
            && !raw_messages.is_null()
        {
            return Some(MessagePayloadCodec::array_of_dictionaries(Some(raw_messages)));
        }

        let messages = messages?;
        let encoder =
            SaiFetchJsonEncoder { key_encoding_strategy: SaiFetchKeyEncodingStrategy::ConvertToSnakeCase };
        let mut rows = Vec::with_capacity(messages.len());
        for message in messages {
            let object = encoder.encode(message).ok()?;
            let mut row = MessagePayloadCodec::dictionary(Some(&object))?;
            if let Some(is_temp) = row.remove(RiseLocalKey::IS_TEMP) {
                row.insert(IS_TEMP_FLAG_KEY.clone(), is_temp);
            }
            rows.push(row);
        }
        Some(rows)
    }

    fn message_row_aliases(&self, row: &MessageRow) -> HashSet<String> {
        [message_key::ID, RiseLocalKey::SERVER_ID, message_key::TEMP_ID]
            .into_iter()
            .map(|key| trimmed_string(row, key))
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn message_row(&self, row: &MessageRow, anchor: &str) -> bool {
        let Some(target) = MessagePresentationWindowProjector::normalize_anchor_id(Some(anchor)) else {
            return false;
        };
        self.message_row_aliases(row).iter().any(|alias| {
            MessagePresentationWindowProjector::normalize_anchor_id(Some(alias.as_str())).as_deref() == Some(target.as_str())
        })
    }

    fn is_unresolved_optimistic_message_row(&self, row: &MessageRow) -> bool {
        let id = trimmed_string(row, message_key::ID);
        let server_id = trimmed_string(row, RiseLocalKey::SERVER_ID);
        let temp_id = trimmed_string(row, message_key::TEMP_ID);
        let has_real_server_id = !server_id.is_empty() && !GetSaiServerId::is_temp_id(Some(server_id));
        let flagged_temp = row.get(IS_TEMP_FLAG_KEY.as_str()).and_then(Value::as_bool) == Some(true)
            || row.get(RiseLocalKey::IS_TEMP).and_then(Value::as_bool) == Some(true);
        !has_real_server_id
            && (flagged_temp || GetSaiServerId::is_temp_id(Some(id)) || GetSaiServerId::is_temp_id(Some(temp_id)))
    }

    fn deduplicated_message_rows(&self, rows: Vec<MessageRow>) -> Vec<MessageRow> {
        let mut slots: Vec<Option<MessageRow>> = Vec::new();
        let mut aliases_by_slot: Vec<HashSet<String>> = Vec::new();
        let mut slot_by_alias: HashMap<String, usize> = HashMap::new();

        for row in rows {
            let aliases = self.message_row_aliases(&row);
            let matched_slots: HashSet<usize> =
                aliases.iter().filter_map(|alias| slot_by_alias.get(alias).copied()).collect();
            let Some(target) = matched_slots.iter().copied().min() else {
                let index = slots.len();
                slots.push(Some(row));
                for alias in &aliases {
                    slot_by_alias.insert(alias.clone(), index);
                }
                aliases_by_slot.push(aliases);
                continue;
            };

            let mut combined_aliases: HashSet<String> = aliases_by_slot[target].union(&aliases).cloned().collect();
            for duplicate in matched_slots {
                if duplicate != target {
                    combined_aliases.extend(std::mem::take(&mut aliases_by_slot[duplicate]));
                    slots[duplicate] = None;
                }
            }
            slots[target] = Some(row);
            for alias in &combined_aliases {
                slot_by_alias.insert(alias.clone(), target);
            }
            aliases_by_slot[target] = combined_aliases;
        }

        slots.into_iter().flatten().collect()
    }

    fn chronological_message_rows(&self, rows: Vec<MessageRow>) -> Vec<MessageRow> {
        let timestamps: Vec<Option<f64>> = rows.iter().map(MessageThreadOrder::timestamp_seconds).collect();
        if timestamps.iter().any(Option::is_none) {
            return rows;
        }
        let indexed: Vec<IndexedMessageRow> = rows
            .into_iter()
            .zip(timestamps)
            .enumerate()
            .map(|(index, (row, timestamp))| IndexedMessageRow {
                index,
                id: self.stable_message_row_id(&row),
                row,
                timestamp: timestamp.unwrap_or(0.0),
            })
            .collect();
        merge_sorted(indexed, &|lhs: &IndexedMessageRow, rhs: &IndexedMessageRow| {
            if lhs.timestamp != rhs.timestamp {
                return lhs.timestamp < rhs.timestamp;
            }
            if let (Some(left_id), Some(right_id)) = (&lhs.id, &rhs.id)
                && left_id != right_id
            {
                return MessageThreadOrder::precedes(
                    &MessageThreadOrderKey { timestamp: lhs.timestamp, id: left_id.clone() },
                    &MessageThreadOrderKey { timestamp: rhs.timestamp, id: right_id.clone() },
                );
            }
            lhs.index < rhs.index
        })
        .into_iter()
        .map(|indexed| indexed.row)
        .collect()
    }

    fn merged_chronological_message_rows(
        &self,
        existing_rows: Vec<MessageRow>,
        incoming_rows: Vec<MessageRow>,
    ) -> Vec<MessageRow> {
        if existing_rows.is_empty() {
            return self.chronological_message_rows(self.deduplicated_message_rows(incoming_rows));
        }
        if incoming_rows.is_empty() {
            return existing_rows;
        }

        let canonical_existing = self.deduplicated_message_rows(existing_rows);
        let canonical_incoming = self.chronological_message_rows(self.deduplicated_message_rows(incoming_rows));
        let incoming_aliases: HashSet<String> =
            canonical_incoming.iter().flat_map(|row| self.message_row_aliases(row)).collect();
        let preserved_existing: Vec<MessageRow> = canonical_existing
            .into_iter()
            .filter(|row| self.message_row_aliases(row).is_disjoint(&incoming_aliases))
            .collect();

        if !self.message_rows_are_chronological(&preserved_existing)
            || !self.message_rows_are_chronological(&canonical_incoming)
        {
            return self.chronological_message_rows(
                self.deduplicated_message_rows([preserved_existing, canonical_incoming].concat()),
            );
        }

        let mut merged = Vec::with_capacity(preserved_existing.len() + canonical_incoming.len());
        let mut existing = preserved_existing.into_iter().peekable();
        let mut incoming = canonical_incoming.into_iter().peekable();
        while let (Some(next_existing), Some(next_incoming)) = (existing.peek(), incoming.peek()) {
            if self.message_row_is_ordered_before(next_incoming, next_existing) {
                merged.extend(incoming.next());
            } else {
                merged.extend(existing.next());
            }
        }
        merged.extend(existing);
        merged.extend(incoming);
        merged
    }

    fn message_rows_are_chronological(&self, rows: &[MessageRow]) -> bool {
        let mut previous: Option<MessageThreadOrderKey> = None;
        for row in rows {
            let Some(key) = self.message_row_order_key(row) else {
                return false;
            };
            if previous.as_ref().is_some_and(|previous| MessageThreadOrder::precedes(&key, previous)) {
                return false;
            }
            previous = Some(key);
        }
        true
    }

    fn message_row_is_ordered_before(&self, lhs: &MessageRow, rhs: &MessageRow) -> bool {
        let (Some(left), Some(right)) = (self.message_row_order_key(lhs), self.message_row_order_key(rhs)) else {
            return false;
        };
        MessageThreadOrder::precedes(&left, &right)
    }

    fn message_row_order_key(&self, row: &MessageRow) -> Option<MessageThreadOrderKey> {
        let timestamp = MessageThreadOrder::timestamp_seconds(row)?;
        let id = self.stable_message_row_id(row)?;
        Some(MessageThreadOrderKey { timestamp, id })
    }

    fn validated_history_page_state(
        &self,
        page: &GetMessagesGatewayDTO,
        requested_relative_id: Option<&str>,
        cx: &mut App,
    ) -> MessageHistoryPageState {
        let normalized_relative_id = page
            .relative_id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|relative_id| !relative_id.is_empty());
        let mut has_more = page.is_have_more.unwrap_or(false);
        if has_more && (normalized_relative_id.is_none() || normalized_relative_id == requested_relative_id) {
            has_more = false;
            Sai::log(
                "Message",
                || "get_messages pagination cursor did not advance; lane closed",
                SaiLogKind::Network,
                cx,
            );
        }
        MessageHistoryPageState { relative_id: normalized_relative_id.map(str::to_owned), has_more }
    }

    fn validated_around_history_page_state(
        &self,
        relative_id: Option<&str>,
        has_more: bool,
        direction: MessageHistoryDirection,
        cx: &mut App,
    ) -> MessageHistoryPageState {
        let normalized_relative_id =
            relative_id.map(trimming_whitespaces_and_newlines).filter(|relative_id| !relative_id.is_empty());
        if has_more && normalized_relative_id.is_none() {
            let line = format!("get_messages_around missing {} cursor; lane closed", direction.raw_value());
            Sai::log("Message", move || line, SaiLogKind::Network, cx);
            return MessageHistoryPageState { relative_id: None, has_more: false };
        }
        MessageHistoryPageState {
            relative_id: normalized_relative_id.filter(|_| has_more).map(str::to_owned),
            has_more,
        }
    }

    fn boundary_relative_id(&self, rows: &[MessageRow], direction: MessageHistoryDirection) -> Option<String> {
        let stable_real_id = |row: &MessageRow| {
            self.stable_message_row_id(row).filter(|id| !GetSaiServerId::is_temp_id(Some(id.as_str())))
        };
        match direction {
            MessageHistoryDirection::Older => rows.iter().find_map(stable_real_id),
            MessageHistoryDirection::Newer => rows.iter().rev().find_map(stable_real_id),
        }
    }

    fn stable_message_row_id(&self, row: &MessageRow) -> Option<String> {
        [RiseLocalKey::SERVER_ID, message_key::ID, message_key::TEMP_ID]
            .into_iter()
            .map(|key| trimmed_string(row, key))
            .find(|value| !value.is_empty())
            .map(str::to_owned)
    }

    fn oldest_refresh_boundary(&self, old_rows: &[MessageRow], fresh_rows: &[MessageRow]) -> Option<usize> {
        fresh_rows.iter().find_map(|fresh_row| {
            let aliases = self.message_row_aliases(fresh_row);
            if aliases.is_empty() {
                return None;
            }
            old_rows.iter().position(|row| !self.message_row_aliases(row).is_disjoint(&aliases))
        })
    }
}
