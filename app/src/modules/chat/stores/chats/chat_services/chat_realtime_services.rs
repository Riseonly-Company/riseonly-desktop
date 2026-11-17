use gpui::Context;
use rise_canon::RiseEvent;
use rise_canon::rise_key::{chat as chat_key, events as events_key, message as message_key};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_events::name;
use serde_json::{Map, Value};

use super::chat_roles_services::{defer_forum_structure_refresh, removed_chat_messages_cache_key};
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_payload_casing::ws_string;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftStore;
use crate::modules::chat::shared::utils::chat_list_utils::ChatListUtils;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_share_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    ChatWallpaperWireDTO, ChatWallpaperWireDTOExt, RiseStoredChat,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageContentType, MessageSystemKind, MessageThreadOrder,
};

fn current_user_id(cx: &mut Context<ChatServicesStore>) -> String {
    let auth = AuthServicesStore::shared(cx);
    let auth = auth.read(cx);
    auth.effective_user_id_for_wire()
        .or_else(|| auth.profile().map(|profile| profile.id.clone()))
        .unwrap_or_default()
}

fn row_count(row: &Map<String, Value>, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn last_message_matches(row: &Map<String, Value>, message_id: &str) -> bool {
    let Some(last_message) = AnyDictionaryBridge::dictionary(row.get(events_key::LAST_MESSAGE)) else {
        return false;
    };
    [events_key::ID, RiseLocalKey::SERVER_ID, events_key::MESSAGE_ID]
        .iter()
        .any(|key| last_message.get(*key).and_then(Value::as_str) == Some(message_id))
}

impl ChatServicesStore {
    pub fn message_created_success_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = AnyDictionaryBridge::dictionary(any) else {
            return;
        };
        let Some(chat_id) = ws_string(&payload, &[message_key::CHAT_ID]) else {
            return;
        };

        if self.left_chat_ids.contains(&chat_id) {
            if !self.confirmed_removed_chat_ids.contains(&chat_id) {
                return;
            }
            self.confirmed_removed_chat_ids.remove(&chat_id);
            self.left_chat_ids.remove(&chat_id);
            cx.notify();
        }

        let current_user_id = current_user_id(cx);
        let sender_id = ws_string(&payload, &[message_key::SENDER_ID]).unwrap_or_default();
        if !sender_id.is_empty() && sender_id != current_user_id {
            let chat_id = chat_id.clone();
            let sender_id = sender_id.clone();
            cx.defer(move |cx| {
                if let Some(hooks) = chat_stores_share_hooks() {
                    (hooks.donate_incoming_interaction)(&chat_id, &sender_id, cx);
                }
            });
        }

        let topic_id = ws_string(&payload, &[message_key::TOPIC_ID])
            .map(|topic_id| trimming_whitespaces_and_newlines(&topic_id).to_owned())
            .unwrap_or_default();
        let should_increment_unread = !sender_id.is_empty() && sender_id != current_user_id;
        let last_message_row = self.realtime_last_message_row(&payload);
        let updated_at = self.realtime_timestamp(&payload);
        if !sender_id.is_empty()
            && sender_id == current_user_id
            && ws_string(&payload, &[message_key::CONTENT_TYPE])
                .is_none_or(|raw| MessageContentType::from_raw_value(&raw) != MessageContentType::System)
        {
            self.start_slow_mode(&chat_id, Date::from_time_interval_since_1970(updated_at), cx);
        }
        self.apply_topic_message_created(
            &chat_id,
            &topic_id,
            last_message_row.clone(),
            updated_at,
            should_increment_unread,
            cx,
        );
        if self.apply_message_created_to_archived_chat(&payload, &chat_id, &sender_id, &current_user_id, &topic_id, cx) {
            return;
        }
        let live_voice = Self::live_voice_change(&payload);
        let did_mutate = self.mutate_cached_chat_row(
            &chat_id,
            true,
            |row: &mut Map<String, Value>| {
                row.insert(events_key::LAST_MESSAGE.to_owned(), Value::Object(last_message_row.clone()));
                row.insert(events_key::UPDATED_AT.to_owned(), Value::from(updated_at));
                if let Some(live_voice) = live_voice {
                    row.insert(chat_key::HAS_LIVE_VOICE.to_owned(), Value::Bool(live_voice));
                }
                if should_increment_unread {
                    let unread_count = row_count(row, events_key::UNREAD_COUNT);
                    row.insert(events_key::UNREAD_COUNT.to_owned(), Value::from((unread_count + 1).max(1)));
                }
            },
            cx,
        );
        if did_mutate {
            return;
        }

        let mut items = self.current_chat_infos(cx);
        let chat_payload = AnyDictionaryBridge::dictionary(payload.get(events_key::CHAT))
            .or_else(|| AnyDictionaryBridge::dictionary(payload.get(chat_key::CHAT_INFO)));
        let chat_dto = chat_payload
            .and_then(|chat_payload| SaiJSONCodec::decode::<RiseStoredChat>(Some(&Value::Object(chat_payload))));
        if let Some(chat_dto) = chat_dto {
            let mut info = chat_dto.to_chat_info();
            info.last_message = Some(ChatListUtils::map_inbound_message(&payload));
            info.updated_at = payload
                .get(events_key::TIMESTAMP)
                .and_then(Value::as_f64)
                .unwrap_or_else(|| RiseClock::now().time_interval_since_1970());
            if let Some(live_voice) = live_voice {
                info.has_live_voice = live_voice;
            }
            items.insert(0, info);
            self.set_chats_payload(ChatListUtils::sort(&items), None, None, None, cx);
        }
    }

    pub fn chat_created_success_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = AnyDictionaryBridge::dictionary(any) else {
            return;
        };
        let chat_dict = AnyDictionaryBridge::dictionary(payload.get(events_key::CHAT))
            .or_else(|| AnyDictionaryBridge::dictionary(payload.get(chat_key::CHAT_INFO)))
            .unwrap_or(payload);
        let Some(dto) = SaiJSONCodec::decode::<RiseStoredChat>(Some(&Value::Object(chat_dict))) else {
            return;
        };
        if dto.id.is_empty() {
            return;
        }

        let info = dto.to_chat_info();
        self.confirmed_removed_chat_ids.remove(&info.id);
        self.left_chat_ids.remove(&info.id);
        cx.notify();

        let my_id = current_user_id(cx);
        if let Some(temp_id) = self.pending_create_chat_temp_id.clone()
            && !my_id.is_empty()
            && info.creator_info.as_deref() == Some(my_id.as_str())
        {
            self.temp_chat_id_to_real.insert(temp_id.clone(), info.id.clone());
            self.pending_create_chat_temp_id = None;
            let mut items = self.current_chat_infos(cx);
            if let Some(index) = items.iter().position(|item| item.id == temp_id) {
                items[index] = info;
            } else if !items.iter().any(|item| item.id == info.id) {
                items.insert(0, info);
            }
            let deduped = self.deduped_by_id(&items);
            self.set_chats_payload(ChatListUtils::sort(&deduped), None, None, None, cx);
            return;
        }

        self.upsert_chat(info, cx);
    }

    pub fn chat_deleted_event_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::ChatDeleted(event)) = RiseEvent::decode_payload(name::CHAT_DELETED, any) else {
            return;
        };
        let chat_id = event.chat_id;
        if chat_id.is_empty() {
            return;
        }

        let archive_needs_refresh = self.chat_list_requires_refresh_after_removing(&chat_id, true, cx);
        self.confirmed_removed_chat_ids.insert(chat_id.clone());
        cx.notify();
        ChatActionsStore::shared(cx).update(cx, |actions, _| actions.note_server_deleted_chat(&chat_id));
        self.remove_chat_local(&chat_id, cx);
        self.remove_archived_chat_local(&chat_id, cx);
        if archive_needs_refresh {
            self.refresh_chat_lists_after_archive_event(cx);
        }
        self.remove_message_request_from_realtime(&chat_id, cx);
        let draft_chat_id = chat_id.clone();
        cx.defer(move |cx| {
            ChatDraftStore::shared(cx).update(cx, |drafts, cx| drafts.clear_chat(&draft_chat_id, cx));
        });
        SaiWsResponseCache::remove(&removed_chat_messages_cache_key(&chat_id), SaiCacheCleanLayer::Everywhere, cx);
    }

    pub fn chat_updated_event_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = AnyDictionaryBridge::dictionary(any) else {
            return;
        };
        let Some(chat_id) = ws_string(&payload, &[message_key::CHAT_ID]).filter(|chat_id| !chat_id.is_empty()) else {
            return;
        };

        if self.update_archived_chat_from_realtime(&chat_id, &payload, cx) {
            return;
        }

        let title = payload.get(events_key::TITLE).and_then(Value::as_str);
        let description = payload.get(events_key::DESCRIPTION).and_then(Value::as_str);
        let logo_url = payload.get(events_key::LOGO_URL).and_then(Value::as_str);
        let wallpaper = payload.get(events_key::WALLPAPER).and_then(Value::as_object);

        let did_mutate = self.mutate_cached_chat_row(
            &chat_id,
            false,
            |row: &mut Map<String, Value>| {
                if let Some(title) = title.filter(|title| !title.is_empty()) {
                    row.insert(events_key::TITLE.to_owned(), Value::from(title));
                }
                if let Some(description) = description {
                    row.insert(events_key::DESCRIPTION.to_owned(), Value::from(description));
                }
                if let Some(logo_url) = logo_url {
                    row.insert(events_key::LOGO_URL.to_owned(), Value::from(logo_url));
                }
                if let Some(wallpaper) = wallpaper {
                    row.insert(events_key::WALLPAPER.to_owned(), Value::Object(wallpaper.clone()));
                }
            },
            cx,
        );
        if did_mutate {
            return;
        }

        self.update_chat_local(&chat_id, title, description, logo_url, cx);
        if let Some(wallpaper) = wallpaper {
            let dto = ChatWallpaperWireDTO {
                blur_percent: row_count(wallpaper, events_key::BLUR_PERCENT),
                dim_percent: row_count(wallpaper, events_key::DIM_PERCENT),
                id: ws_string(wallpaper, &[events_key::ID]).unwrap_or_default(),
                image_url: ws_string(wallpaper, &[message_key::IMAGE_URL]).unwrap_or_default(),
                preview_url: ws_string(wallpaper, &[message_key::PREVIEW_URL]).unwrap_or_default(),
                set_by_user_id: ws_string(wallpaper, &[chat_key::SET_BY_USER_ID]).unwrap_or_default(),
                updated_at_ms: row_count(wallpaper, events_key::UPDATED_AT_MS),
            };
            self.update_chat_wallpaper_local(&chat_id, dto.to_placement(), cx);
        }
    }

    pub fn member_left_event_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::MemberLeft(event)) = RiseEvent::decode_payload(name::MEMBER_LEFT, any) else {
            return;
        };
        let chat_id = event.chat_id;
        let user_id = event.user_id;
        if chat_id.is_empty() || user_id.is_empty() {
            return;
        }

        let my_id = current_user_id(cx);
        if user_id == my_id {
            let archive_needs_refresh = self.chat_list_requires_refresh_after_removing(&chat_id, true, cx);
            self.remove_chat_local(&chat_id, cx);
            self.remove_archived_chat_local(&chat_id, cx);
            if archive_needs_refresh {
                self.refresh_chat_lists_after_archive_event(cx);
            }
            SaiWsResponseCache::remove(&removed_chat_messages_cache_key(&chat_id), SaiCacheCleanLayer::Everywhere, cx);
            return;
        }

        self.remove_member_local(&chat_id, &user_id, None, false, cx);
        if self.adjust_archived_chat_member_count(&chat_id, -1, cx) {
            return;
        }
        let did_mutate = self.mutate_cached_chat_row(
            &chat_id,
            false,
            |row: &mut Map<String, Value>| {
                let count = row_count(row, events_key::MEMBER_COUNT);
                row.insert(events_key::MEMBER_COUNT.to_owned(), Value::from((count - 1).max(0)));
            },
            cx,
        );
        if !did_mutate {
            self.mutate_chat(&chat_id, |chat| chat.member_count = (chat.member_count - 1).max(0), cx);
        }
    }

    pub fn member_joined_event_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::MemberJoined(event)) = RiseEvent::decode_payload(name::MEMBER_JOINED, any) else {
            return;
        };
        let chat_id = event.chat_id;
        let user_id = event.user_id;
        if chat_id.is_empty() || user_id.is_empty() {
            return;
        }

        tracing::warn!(target: "riseonly::X2107", chat_id = %chat_id, user_id = %user_id, "member joined event");
        if user_id == current_user_id(cx) {
            self.refetch_chat_as_member(&chat_id, cx);
            return;
        }

        self.refresh_loaded_member_lists(&chat_id, cx);
        if self.adjust_archived_chat_member_count(&chat_id, 1, cx) {
            return;
        }

        let did_mutate = self.mutate_cached_chat_row(
            &chat_id,
            false,
            |row: &mut Map<String, Value>| {
                let count = row_count(row, events_key::MEMBER_COUNT);
                row.insert(events_key::MEMBER_COUNT.to_owned(), Value::from(count + 1));
            },
            cx,
        );
        if !did_mutate {
            self.mutate_chat(&chat_id, |chat| chat.member_count += 1, cx);
        }
    }

    pub fn message_edited_success_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = AnyDictionaryBridge::dictionary(any) else {
            return;
        };
        let message_payload = self.realtime_edited_message_payload(&payload);

        let Some(chat_id) = ws_string(&payload, &[message_key::CHAT_ID])
            .or_else(|| ws_string(&message_payload, &[message_key::CHAT_ID]))
        else {
            return;
        };
        let Some(message_id) = ws_string(&message_payload, &[message_key::ID, message_key::MESSAGE_ID])
            .or_else(|| ws_string(&payload, &[message_key::ID, message_key::MESSAGE_ID]))
        else {
            return;
        };

        let topic_id = self.realtime_optional_topic_id(&payload, &message_payload);
        let last_message_row = self.realtime_last_message_row(&message_payload);
        self.apply_topic_message_edited(&chat_id, topic_id.as_deref(), &message_id, last_message_row.clone(), cx);

        if self.apply_message_edited_to_archived_chat(&message_payload, &chat_id, &message_id, cx) {
            return;
        }

        let did_mutate = self.mutate_cached_chat_row(
            &chat_id,
            false,
            |row: &mut Map<String, Value>| {
                if last_message_matches(row, &message_id) {
                    row.insert(events_key::LAST_MESSAGE.to_owned(), Value::Object(last_message_row.clone()));
                }
            },
            cx,
        );
        if !did_mutate {
            self.mutate_chat(
                &chat_id,
                |chat| {
                    if chat.last_message.as_ref().is_some_and(|last| last.matches_message_id(&message_id)) {
                        chat.last_message = Some(ChatListUtils::map_inbound_message(&message_payload));
                    }
                },
                cx,
            );
        }
    }

    pub fn message_deleted_success_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = AnyDictionaryBridge::dictionary(any) else {
            return;
        };
        let Some(chat_id) = ws_string(&payload, &[message_key::CHAT_ID]) else {
            return;
        };

        let message_id = ws_string(&payload, &[message_key::ID, message_key::MESSAGE_ID]);

        let mut deleted_current_topic_preview = false;
        if let Some(message_id) = message_id.as_deref() {
            let topic_id = self.realtime_optional_topic_id(&payload, &Map::new());
            deleted_current_topic_preview = self.apply_topic_message_deleted(&chat_id, topic_id.as_deref(), message_id, cx);
        }
        if deleted_current_topic_preview {
            let topics_chat_id = chat_id.clone();
            cx.defer(move |cx| {
                ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_topics_action(&topics_chat_id, true, cx));
            });
            defer_forum_structure_refresh(chat_id.clone(), cx);
            cx.defer(|cx| {
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.refresh_chats(cx);
                    interactions.refresh_archived_chats(cx);
                });
            });
        }
        if self.apply_message_deleted_to_archived_chat(&chat_id, message_id.as_deref(), cx) {
            return;
        }

        let did_mutate = self.mutate_cached_chat_row(
            &chat_id,
            false,
            |row: &mut Map<String, Value>| {
                if message_id.as_deref().is_some_and(|message_id| last_message_matches(row, message_id)) {
                    row.insert(events_key::LAST_MESSAGE.to_owned(), Value::Null);
                }
            },
            cx,
        );
        if !did_mutate {
            self.mutate_chat(
                &chat_id,
                |chat| {
                    if let Some(message_id) = message_id.as_deref()
                        && chat.last_message.as_ref().is_some_and(|last| last.matches_message_id(message_id))
                    {
                        chat.last_message = None;
                    }
                },
                cx,
            );
        }
    }

    pub fn live_voice_change(payload: &Map<String, Value>) -> Option<bool> {
        let kind = ws_string(payload, &[message_key::SYSTEM_MESSAGE_TYPE])?;
        if kind == MessageSystemKind::VoiceCallStarted.raw_value() {
            return Some(true);
        }
        if kind == MessageSystemKind::VoiceCallEnded.raw_value() {
            return Some(false);
        }
        None
    }

    fn realtime_last_message_row(&self, payload: &Map<String, Value>) -> Map<String, Value> {
        let mut row = payload.clone();
        row.remove(chat_key::CHAT);
        row.remove(chat_key::CHAT_INFO);
        row.remove(message_key::TYPE);
        if !row.contains_key(events_key::ID)
            && let Some(message_id) = row.get(events_key::MESSAGE_ID).cloned()
        {
            row.insert(events_key::ID.to_owned(), message_id);
        }
        if !row.contains_key(events_key::SENDER)
            && let Some(sender) = row.get(events_key::SENDER_INFO).cloned()
        {
            row.insert(events_key::SENDER.to_owned(), sender);
        }
        let sender_name = ChatListUtils::inbound_sender_name(&row);
        if !sender_name.is_empty() {
            row.insert(events_key::SENDER_NAME.to_owned(), Value::from(sender_name));
        }
        row
    }

    fn realtime_edited_message_payload(&self, payload: &Map<String, Value>) -> Map<String, Value> {
        let mut message =
            AnyDictionaryBridge::dictionary(payload.get(events_key::MESSAGE)).unwrap_or_else(|| payload.clone());
        for key in [message_key::CHAT_ID, message_key::TOPIC_ID, message_key::SENDER_ID, events_key::SENDER_INFO] {
            if !message.contains_key(key)
                && let Some(value) = payload.get(key)
            {
                message.insert(key.to_owned(), value.clone());
            }
        }
        if !message.contains_key(events_key::CONTENT)
            && let Some(content) = payload.get(events_key::NEW_CONTENT)
        {
            message.insert(events_key::CONTENT.to_owned(), content.clone());
        }
        if !message.contains_key(events_key::ID)
            && let Some(id) = payload.get(events_key::MESSAGE_ID).or_else(|| payload.get(events_key::ID))
        {
            message.insert(events_key::ID.to_owned(), id.clone());
        }
        message
    }

    fn realtime_optional_topic_id(
        &self,
        payload: &Map<String, Value>,
        nested_message: &Map<String, Value>,
    ) -> Option<String> {
        for source in [payload, nested_message] {
            let Some(raw) = source.get(events_key::TOPIC_ID) else {
                continue;
            };
            if raw.is_null() {
                continue;
            }
            return Some(
                ws_string(source, &[message_key::TOPIC_ID])
                    .map(|topic_id| trimming_whitespaces_and_newlines(&topic_id).to_owned())
                    .unwrap_or_default(),
            );
        }
        None
    }

    fn realtime_timestamp(&self, payload: &Map<String, Value>) -> TimeInterval {
        MessageThreadOrder::timestamp_seconds(payload).unwrap_or_else(|| RiseClock::now().time_interval_since_1970())
    }
}
