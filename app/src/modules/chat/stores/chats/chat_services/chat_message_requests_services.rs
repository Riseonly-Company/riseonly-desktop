use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::chat as key;
use rise_canon::rise_key::otp as otp_key;
use rise_canon::{RiseEvent, RiseLimits};
use rise_chat::MessageRequestState;
use serde_json::{Map, Value};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseChatListWireSnapshot, RiseConversationPresentation,
};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::effective_user_id_for_wire;
use crate::modules::chat::stores::chats::chat_actions::chat_message_requests_wire_types::RiseStoredMessageRequestPage;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

impl ChatServicesStore {
    pub fn message_requests_limit() -> i64 {
        RiseLimits::client_value("chat_services_store_message_requests_limit")
    }

    pub fn message_requests_cache_key(user_id: &str) -> String {
        SaiCache::new(["getMessageRequests", user_id]).key
    }

    fn message_requests_dto(&self, cx: &mut App) -> Option<Rc<RiseStoredMessageRequestPage>> {
        let user_id = effective_user_id_for_wire(cx)?;
        Self::observed_gateway_dto(&Self::message_requests_cache_key(&user_id), cx)
    }

    pub fn message_requests(&self, cx: &mut App) -> Vec<ChatInfo> {
        let presentation = RiseConversationPresentation::shared(cx).read(cx);
        if presentation.is_active() {
            return presentation.chats(RiseChatListKind::Requests).to_vec();
        }
        self.message_requests_dto(cx)
            .map(|dto| dto.chats.iter().flatten().map(|chat| chat.to_chat_info()).collect())
            .unwrap_or_default()
    }

    /// badge count for message requests on the chats menu
    /// server total from the first page while more pages remain but never below loaded rows, the
    /// loaded rows once the list is complete
    pub fn message_requests_badge_count(state: &RiseChatListWireSnapshot) -> i64 {
        if state.has_more {
            state.loaded_row_count.max(state.server_total_count.unwrap_or(0))
        } else {
            state.loaded_row_count
        }
    }

    pub fn message_requests_has_more(&self, cx: &mut App) -> bool {
        let presentation = RiseConversationPresentation::shared(cx).read(cx);
        if presentation.is_active()
            && let Some(state) = presentation.chat_state(RiseChatListKind::Requests)
        {
            return state.has_more;
        }
        self.message_requests_dto(cx).and_then(|dto| dto.has_more).unwrap_or(false)
    }

    pub fn message_requests_relative_id(&self, cx: &mut App) -> Option<String> {
        let presentation = RiseConversationPresentation::shared(cx).read(cx);
        if presentation.is_active()
            && let Some(state) = presentation.chat_state(RiseChatListKind::Requests)
        {
            return state.cursor.clone();
        }
        self.message_requests_dto(cx).and_then(|dto| dto.relative_id.clone())
    }

    pub fn message_request_raw_row(&self, chat_id: &str, cx: &mut App) -> Option<Map<String, Value>> {
        let dto = self.message_requests_dto(cx)?;
        let wire = dto.chats.iter().flatten().find(|chat| chat.id == chat_id)?;
        SaiJSONCodec::encode(wire)
    }

    pub fn remove_message_request(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let _ = SaiUpdater::remove_row(
            &Self::message_requests_cache_key(&user_id),
            key::CHATS,
            key::ID,
            chat_id,
            SaiUpdaterWriteScope::All,
            cx,
        );
        cx.notify();
    }

    pub fn restore_message_request(&mut self, row: Map<String, Value>, cx: &mut Context<Self>) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let _ = SaiUpdater::insert_row_if_missing(
            &Self::message_requests_cache_key(&user_id),
            key::CHATS,
            key::ID,
            row,
            true,
            SaiUpdaterWriteScope::All,
            cx,
        );
        cx.notify();
    }

    pub fn message_requests_root_snapshot(&self, cx: &mut App) -> Option<Map<String, Value>> {
        let dto = self.message_requests_dto(cx)?;
        SaiJSONCodec::encode(&*dto)
    }

    pub fn clear_message_requests(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let mut root = Map::new();
        root.insert(key::CHATS.to_owned(), Value::Array(Vec::new()));
        root.insert(otp_key::HAS_MORE.to_owned(), Value::Bool(false));
        let _ = SaiUpdater::replace_root(
            &Self::message_requests_cache_key(&user_id),
            root,
            SaiUpdaterWriteScope::All,
            cx,
        );
        cx.notify();
    }

    pub fn restore_message_requests_root_merging_current(&mut self, root: Map<String, Value>, cx: &mut Context<Self>) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let Some(snapshot) = SaiJSONCodec::decode::<RiseStoredMessageRequestPage>(Some(&Value::Object(root))) else {
            return;
        };

        let mut merged_rows: Vec<RiseStoredChat> =
            self.message_requests_dto(cx).and_then(|dto| dto.chats.clone()).unwrap_or_default();
        let mut ids: HashSet<String> = merged_rows.iter().map(|chat| chat.id.clone()).collect();
        let mut resolved_ids: HashSet<String> =
            self.current_chat_infos(cx).into_iter().map(|chat| chat.id).collect();
        resolved_ids.extend(self.confirmed_removed_chat_ids.iter().cloned());
        for row in snapshot.chats.iter().flatten() {
            if !resolved_ids.contains(&row.id) && ids.insert(row.id.clone()) {
                merged_rows.push(row.clone());
            }
        }
        let merged = RiseStoredMessageRequestPage {
            chats: Some(merged_rows),
            has_more: snapshot.has_more,
            relative_id: snapshot.relative_id,
            total_count: snapshot.total_count,
        };
        let Some(payload) = SaiJSONCodec::encode(&merged) else {
            return;
        };
        let _ = SaiUpdater::replace_root(
            &Self::message_requests_cache_key(&user_id),
            payload,
            SaiUpdaterWriteScope::All,
            cx,
        );
        cx.notify();
    }

    pub fn accept_message_request(&mut self, chat: ChatInfo, cx: &mut Context<Self>) {
        let mut accepted =
            self.current_chat_infos(cx).into_iter().find(|existing| existing.id == chat.id).unwrap_or(chat);
        accepted.is_message_request = false;
        accepted.message_request_state = None;
        self.left_chat_ids.remove(&accepted.id);
        self.confirmed_removed_chat_ids.remove(&accepted.id);
        self.upsert_chat(accepted, cx);
        cx.notify();
    }

    pub fn remove_message_request_from_realtime(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.remove_message_request(chat_id, cx);
    }

    pub fn message_request_accepted_handler(&mut self, value: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::MessageRequestAccepted(event)) =
            RiseEvent::decode_payload(rise_events::name::MESSAGE_REQUEST_ACCEPTED, value)
        else {
            return;
        };
        let wire = RiseStoredChat::from_canon_proto(&event);
        self.remove_message_request(&wire.id, cx);
        self.accept_message_request(wire.to_chat_info(), cx);
    }

    pub fn message_request_created_handler(&mut self, value: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::MessageRequestCreated(event)) =
            RiseEvent::decode_payload(rise_events::name::MESSAGE_REQUEST_CREATED, value)
        else {
            return;
        };
        let mut wire = RiseStoredChat::from_canon_proto(&event);
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        let existing = self.message_requests(cx).into_iter().find(|chat| chat.id == wire.id);
        let incoming_message_id = wire.last_message.as_ref().and_then(|preview| preview.id.clone());
        let existing_message_id =
            existing.as_ref().and_then(|chat| chat.last_message.as_ref()).map(|message| message.id.clone());
        let is_new_message = incoming_message_id.as_deref().is_some_and(|id| !id.is_empty())
            && incoming_message_id != existing_message_id;
        let realtime_count = existing.as_ref().map_or(0, |chat| chat.unread_count) + i64::from(is_new_message);
        let floor = if existing.is_none() { 1 } else { 0 };
        wire.unread_count = Some(wire.unread_count.unwrap_or(0).max(realtime_count.max(floor)));
        self.confirmed_removed_chat_ids.remove(&wire.id);
        wire.is_message_request = Some(true);
        wire.message_request_state = Some(MessageRequestState::Pending.wire().to_owned());
        let Some(row) = SaiJSONCodec::encode(&wire) else {
            return;
        };
        let cache_key = Self::message_requests_cache_key(&user_id);
        if self.message_requests_dto(cx).is_none() {
            let mut root = Map::new();
            root.insert(key::CHATS.to_owned(), Value::Array(vec![Value::Object(row)]));
            root.insert(otp_key::HAS_MORE.to_owned(), Value::Bool(false));
            let _ = SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::All, cx);
        } else {
            let _ = SaiUpdater::insert_row_if_missing(
                &cache_key,
                key::CHATS,
                key::ID,
                row.clone(),
                true,
                SaiUpdaterWriteScope::All,
                cx,
            );
            let _ = SaiUpdater::patch_row(
                &cache_key,
                key::CHATS,
                key::ID,
                &wire.id,
                &row,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }
        cx.notify();
    }
}
