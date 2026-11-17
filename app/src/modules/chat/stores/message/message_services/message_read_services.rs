use std::collections::{BTreeSet, HashSet};

use gpui::Context;
use rise_bot_api::ChatAction;
use rise_canon::rise_key::{events as events_key, message as message_key, push as push_key};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::modules::sai_updater::sai_updater::SaiUpdaterWriteScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_payload_casing::ws_string;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::messages::rise_message_ephemeral_state::RiseMessageTypingPeer;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{ChatTypingPeer, MessageThreadOrder};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessagePayloadCodec;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn non_empty_string_set(value: Option<&Value>) -> HashSet<String> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).filter(|id| !id.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn all_strings(value: Option<&Value>) -> Option<Vec<String>> {
    value?.as_array()?.iter().map(|item| item.as_str().map(str::to_owned)).collect()
}

impl MessageServicesStore {
    pub fn chat_list_typing_text(peers: &[RiseMessageTypingPeer], shows_names: bool) -> Option<String> {
        if peers.is_empty() {
            return None;
        }
        if let Some(status) = peers.iter().filter_map(|peer| peer.status_text.as_deref()).find(|text| !text.is_empty())
        {
            return Some(status.to_owned());
        }
        let names: Vec<&str> = peers
            .iter()
            .map(|peer| trimming_whitespaces_and_newlines(&peer.name))
            .filter(|name| !name.is_empty())
            .collect();
        let actions: Vec<Option<&str>> = peers.iter().map(|peer| peer.action.as_deref()).collect();
        let action = Self::shared_typing_action(&actions);
        if !shows_names || names.is_empty() {
            return Some(Self::typing_verb(action, false));
        }
        let label = Self::typing_verb(action, peers.len() > 1);
        Some(format!("{} {label}", names[..names.len().min(2)].join(", ")))
    }

    /// what a member is doing like telegram, sends a file or chooses a sticker
    /// an action canon has no text for or none reads as typing
    pub fn typing_verb(action: Option<&str>, plural: bool) -> String {
        let number = if plural { "plural" } else { "single" };
        match action.and_then(ChatAction::parse) {
            Some(
                ChatAction::UploadPhoto
                | ChatAction::UploadVideo
                | ChatAction::UploadDocument
                | ChatAction::ChooseSticker
                | ChatAction::ChooseEmoji
                | ChatAction::ChooseMedia,
            ) => tr(&format!("chat_status_{}_{number}", action.unwrap_or_default())),
            _ => tr(&format!("chat_status_typing_{number}")),
        }
    }

    /// several members doing different things read as typing
    pub fn shared_typing_action<'a>(actions: &[Option<&'a str>]) -> Option<&'a str> {
        let first = *actions.first()?;
        if actions.iter().all(|action| *action == first) { first } else { None }
    }

    pub fn typing_action(&self, chat_id: &str) -> Option<String> {
        let now = RiseClock::now().time_interval_since_1970();
        let peers = self.typing_peers_by_chat.get(chat_id)?;
        let actions: Vec<Option<&str>> = peers
            .iter()
            .filter(|peer| now - peer.at < Self::TYPING_TTL_SECONDS && !peer.name.is_empty())
            .map(|peer| peer.action.as_deref())
            .collect();
        Self::shared_typing_action(&actions).map(str::to_owned)
    }

    pub fn typists(&self, chat_id: &str) -> Vec<String> {
        let now = RiseClock::now().time_interval_since_1970();
        self.typing_peers_by_chat
            .get(chat_id)
            .map(|peers| {
                peers
                    .iter()
                    .filter(|peer| now - peer.at < Self::TYPING_TTL_SECONDS)
                    .map(|peer| peer.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn bot_activity_status(&self, chat_id: &str) -> Option<String> {
        let now = RiseClock::now().time_interval_since_1970();
        self.typing_peers_by_chat
            .get(chat_id)?
            .iter()
            .filter(|peer| now - peer.at < Self::TYPING_TTL_SECONDS)
            .filter_map(|peer| peer.status_text.clone())
            .next_back()
    }

    pub fn chat_read_receipt_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = AnyDictionaryBridge::dictionary(any) else {
            return;
        };
        let Some(raw_chat_id) = ws_string(&payload, &[message_key::CHAT_ID]).filter(|id| !id.is_empty()) else {
            return;
        };

        let reader_id = ws_string(&payload, &[message_key::USER_ID]).unwrap_or_default();
        let auth = AuthServicesStore::shared(cx);
        let my_id = {
            let auth = auth.read(cx);
            auth.effective_user_id_for_wire()
                .or_else(|| auth.profile().map(|profile| profile.id.clone()))
                .unwrap_or_default()
        };
        if reader_id.is_empty() || reader_id == my_id {
            return;
        }

        let chat_services = ChatServicesStore::shared(cx);
        let chat_id = chat_services.read(cx).canonical_chat_id(&raw_chat_id);
        let last_read_id =
            payload.get(events_key::LAST_READ_MESSAGE_ID).and_then(Value::as_str).unwrap_or_default().to_owned();
        let has_topic_scope = payload.get(events_key::TOPIC_ID).is_some_and(|topic| !topic.is_null());
        let topic_id =
            has_topic_scope.then(|| ws_string(&payload, &[message_key::TOPIC_ID]).unwrap_or_default());
        let read_message_ids = non_empty_string_set(payload.get(events_key::READ_MESSAGE_IDS));
        let scope_message_ids: Vec<String> = if read_message_ids.is_empty() {
            vec![last_read_id.clone()]
        } else {
            read_message_ids.iter().cloned().collect()
        };
        let exact_read_message_ids = (!read_message_ids.is_empty()).then_some(read_message_ids);

        let mut secondary_scopes = self.loaded_realtime_secondary_scopes(&payload, &chat_id, &scope_message_ids, cx);
        if topic_id.is_none() {
            let mut topics = chat_services.update(cx, |chats, cx| chats.topics(&chat_id, cx));
            topics.extend(chat_services.update(cx, |chats, cx| chats.forum_topics(&chat_id, cx)));
            let mut merged: BTreeSet<String> = secondary_scopes.into_iter().collect();
            for topic in topics.iter().filter(|topic| !topic.is_general && !topic.id.is_empty()) {
                let scope = Self::message_scope(&chat_id, Some(&topic.id), None);
                if self.did_load_messages(&scope, cx) {
                    merged.insert(scope);
                }
            }
            secondary_scopes = merged.into_iter().collect();
        }
        let mut scopes = Vec::with_capacity(secondary_scopes.len() + 1);
        if self.did_load_messages(&chat_id, cx) {
            scopes.push(chat_id.clone());
        }
        scopes.extend(secondary_scopes);

        let cached_cutoff =
            scopes.iter().find_map(|scope| self.in_memory_message_timestamp(scope, &last_read_id, cx));
        let mut payload_cutoff_row = Map::new();
        payload_cutoff_row.insert(
            events_key::CREATED_AT.to_owned(),
            payload.get(events_key::LAST_READ_CREATED_AT).cloned().unwrap_or_else(|| Value::from(0)),
        );
        let payload_cutoff = MessageThreadOrder::timestamp_seconds(&payload_cutoff_row);
        let Some(cutoff) =
            cached_cutoff.or(payload_cutoff).or_else(|| exact_read_message_ids.as_ref().map(|_| 0.0))
        else {
            return;
        };
        let same_millisecond_message_ids =
            non_empty_string_set(payload.get(events_key::LAST_READ_SAME_MILLISECOND_MESSAGE_IDS));
        let exact_message_ids = (!same_millisecond_message_ids.is_empty()).then_some(same_millisecond_message_ids);
        for scope in &scopes {
            self.mark_outgoing_messages_read(
                scope,
                &my_id,
                topic_id.as_deref(),
                cutoff,
                exact_message_ids.as_ref(),
                exact_read_message_ids.as_ref(),
                cx,
            );
        }
        chat_services.update(cx, |chats, cx| {
            chats.mark_last_message_previews_read(
                &chat_id,
                &my_id,
                topic_id.as_deref(),
                cutoff,
                exact_message_ids.as_ref(),
                exact_read_message_ids.as_ref(),
                cx,
            );
        });
    }

    pub fn apply_media_played_optimistic(
        &mut self,
        chat_id: &str,
        message_id: &str,
        user_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.mutate_messages_list(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |messages| {
                let Some(message) = messages.iter_mut().find(|message| message.matches_message_id(message_id)) else {
                    return;
                };
                if !message.played_by.iter().any(|played| played == user_id) {
                    message.played_by.push(user_id.to_owned());
                }
            },
            cx,
        );
    }

    pub fn media_played_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = AnyDictionaryBridge::dictionary(any) else {
            return;
        };
        let Some(raw_chat_id) = ws_string(&payload, &[message_key::CHAT_ID]).filter(|id| !id.is_empty()) else {
            return;
        };
        let Some(mid) = ws_string(&payload, &[message_key::MESSAGE_ID]).filter(|id| !id.is_empty()) else {
            return;
        };

        let played_by = all_strings(payload.get(events_key::PLAYED_BY)).unwrap_or_default();
        let chat_id = ChatServicesStore::shared(cx).read(cx).canonical_chat_id(&raw_chat_id);
        let secondary_scopes =
            self.loaded_realtime_secondary_scopes(&payload, &chat_id, std::slice::from_ref(&mid), cx);
        let mut scopes = Vec::with_capacity(secondary_scopes.len() + 1);
        if self.did_load_messages(&chat_id, cx) {
            scopes.push(chat_id.clone());
        }
        scopes.extend(secondary_scopes);
        for scope in &scopes {
            self.mutate_messages_list(
                scope,
                SaiUpdaterWriteScope::Cache,
                |messages| {
                    let Some(message) = messages.iter_mut().find(|message| message.matches_message_id(&mid)) else {
                        return;
                    };
                    for uid in &played_by {
                        if !message.played_by.contains(uid) {
                            message.played_by.push(uid.clone());
                        }
                    }
                },
                cx,
            );
        }
    }

    pub fn typing_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = MessagePayloadCodec::dictionary(any) else {
            return;
        };
        let Some(raw_chat_id) = ws_string(&payload, &[message_key::CHAT_ID]) else {
            return;
        };
        let chat_id = ChatServicesStore::shared(cx).read(cx).canonical_chat_id(&raw_chat_id);

        let user_info = payload.get(events_key::USER_INFO).and_then(Value::as_object);
        let user_id = ws_string(&payload, &[message_key::USER_ID])
            .or_else(|| {
                user_info.and_then(|info| info.get(events_key::ID)).and_then(Value::as_str).map(str::to_owned)
            })
            .unwrap_or_default();
        let auth = AuthServicesStore::shared(cx);
        let my_id = auth.read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
        if user_id.is_empty() || user_id == my_id {
            return;
        }

        let display_name = user_info
            .and_then(|info| info.get(events_key::NAME))
            .or_else(|| payload.get(events_key::SENDER_NAME).filter(|name| name.is_string()))
            .or_else(|| payload.get(push_key::USER_NAME).filter(|name| name.is_string()))
            .or_else(|| payload.get(events_key::NAME))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let is_typing = payload
            .get(events_key::TYPING)
            .and_then(Value::as_bool)
            .or_else(|| payload.get(message_key::IS_TYPING).and_then(Value::as_bool))
            .unwrap_or(false);
        let action = ws_string(&payload, &[message_key::ACTION]);
        let status_text = ws_string(&payload, &[message_key::STATUS_TEXT])
            .map(|text| trimming_whitespaces_and_newlines(&text).to_owned());

        let now = RiseClock::now().time_interval_since_1970();
        let status_text = status_text.filter(|text| !text.is_empty());
        let topic_id = ws_string(&payload, &[message_key::TOPIC_ID]).unwrap_or_default();
        let topic_key = Self::message_scope(&chat_id, Some(&topic_id), None);
        for key in [chat_id, topic_key] {
            let mut peers: Vec<ChatTypingPeer> = self
                .typing_peers_by_chat
                .get(&key)
                .map(|peers| peers.iter().filter(|peer| now - peer.at < Self::TYPING_TTL_SECONDS).cloned().collect())
                .unwrap_or_default();
            let existing = peers.iter().position(|peer| peer.user_id == user_id);
            if is_typing && !display_name.is_empty() {
                let peer = ChatTypingPeer {
                    user_id: user_id.clone(),
                    name: display_name.clone(),
                    action: action.clone(),
                    status_text: status_text.clone(),
                    at: now,
                };
                match existing {
                    Some(index) => peers[index] = peer,
                    None => peers.push(peer),
                }
            } else if let Some(index) = existing {
                peers.remove(index);
            }
            if peers.is_empty() {
                self.typing_peers_by_chat.remove(&key);
            } else {
                self.typing_peers_by_chat.insert(key, peers);
            }
        }
        cx.notify();
    }
}
