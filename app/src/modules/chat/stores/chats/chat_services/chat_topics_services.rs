use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context, Global};
use rise_canon::rise_key::{chat as key, events as events_key, message as message_key, support as support_key};
use rise_canon::{ChatTopicState, RiseEvent};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_events::name;
use rise_i18n::tr;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_payload_casing::ws_string;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::chat::shared::chat_permission_resolution::ChatPermissionResolution;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_user_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatChannelManagementPolicy, ChatMuteScopes, ChatPerm, ChatPermExt, ChatTopic,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    ChatWallpaperWireDTO, ChatWallpaperWireDTOExt, EffectivePermissionsDTOExt, RiseStoredChatTopic,
    RiseStoredChatTopicList, RiseStoredForumStructure,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageThreadOrder};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessagePayloadCodec;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChatRemovedTopicNoticeReason {
    #[default]
    Deleted,
    AccessLost,
    DeletedByMe,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatRemovedTopicNotice {
    pub id: Uuid,
    pub chat_id: String,
    pub topic_ids: Vec<String>,
    pub reason: ChatRemovedTopicNoticeReason,
}

#[derive(Default)]
struct ChatRemovedTopicNoticeAnnouncedId(Option<Uuid>);

impl Global for ChatRemovedTopicNoticeAnnouncedId {}

impl ChatRemovedTopicNotice {
    pub fn new(chat_id: String, topic_ids: Vec<String>, reason: ChatRemovedTopicNoticeReason) -> Self {
        Self { id: Uuid::new_v4(), chat_id, topic_ids, reason }
    }

    pub fn announce_once(&self, cx: &mut App) {
        let announced = cx.default_global::<ChatRemovedTopicNoticeAnnouncedId>();
        if announced.0 == Some(self.id) || self.reason == ChatRemovedTopicNoticeReason::DeletedByMe {
            return;
        }
        announced.0 = Some(self.id);
        let title = if self.reason == ChatRemovedTopicNoticeReason::AccessLost {
            tr("chat_topic_access_lost_notice")
        } else {
            tr("chat_topic_deleted_notice")
        };
        Notifier::send(
            title,
            NotifierSendOptions {
                r#type: NotifierType::Info,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }
}

struct ChatTopicsProjection {
    dto: Option<Rc<RiseStoredChatTopicList>>,
    topics: Vec<ChatTopic>,
    general: Option<ChatTopic>,
}

impl ChatTopicsProjection {
    fn empty() -> Rc<Self> {
        Rc::new(Self { dto: None, topics: Vec::new(), general: None })
    }
}

fn json_int(value: Option<&Value>) -> i64 {
    value.and_then(|value| value.as_i64().or_else(|| value.as_f64().map(|number| number as i64))).unwrap_or(0)
}

#[derive(Clone)]
struct FailedTopicPreview {
    topic_id: String,
    local_id: String,
    row: Map<String, Value>,
    updated_at: f64,
}

thread_local! {
    static FAILED_TOPIC_PREVIEWS: RefCell<HashMap<String, Vec<FailedTopicPreview>>> = RefCell::new(HashMap::new());
}

fn forget_failed_topic_previews(chat_id: &str, keep: impl Fn(&FailedTopicPreview) -> bool) {
    FAILED_TOPIC_PREVIEWS.with(|cell| {
        let mut map = cell.borrow_mut();
        if let Some(list) = map.get_mut(chat_id) {
            list.retain(|entry| keep(entry));
            if list.is_empty() {
                map.remove(chat_id);
            }
        }
    });
}

fn row_message_id(row: &Map<String, Value>) -> String {
    [key::ID, "server_id", events_key::MESSAGE_ID]
        .iter()
        .find_map(|name| row.get(*name).and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

impl ChatServicesStore {
    pub fn topics_cache_key(chat_id: &str) -> String {
        SaiCache::new(["getTopics", chat_id]).key
    }

    fn topics_projection(&self, chat_id: &str, cx: &mut App) -> Rc<ChatTopicsProjection> {
        SaiWsResponseCache::observed_projection(
            &Self::topics_cache_key(chat_id),
            "chat-topics-projection-v2",
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            |payload: Option<&Value>| {
                let Some(dto) = SaiJSONCodec::decode::<RiseStoredChatTopicList>(payload) else {
                    return ChatTopicsProjection::empty();
                };
                let mut topics: Vec<ChatTopic> =
                    dto.topics.iter().flatten().map(RiseStoredChatTopic::to_topic).collect();
                topics.sort_by_key(|topic| (!topic.is_general, topic.position));
                let general = dto.general_topic.as_ref().map(RiseStoredChatTopic::to_topic);
                Rc::new(ChatTopicsProjection { dto: Some(Rc::new(dto)), topics, general })
            },
            cx,
        )
    }

    fn topics_dto(&self, chat_id: &str, cx: &mut App) -> Option<Rc<RiseStoredChatTopicList>> {
        self.topics_projection(chat_id, cx).dto.clone()
    }

    fn forum_topics_dto(&self, chat_id: &str, cx: &mut App) -> Option<Rc<RiseStoredForumStructure>> {
        SaiWsResponseCache::observed_projection(
            &Self::forum_structure_cache_key(chat_id),
            "chat-forum-dto-v2",
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            |payload: Option<&Value>| SaiJSONCodec::decode::<RiseStoredForumStructure>(payload).map(Rc::new),
            cx,
        )
    }

    pub fn topics(&self, chat_id: &str, cx: &mut App) -> Vec<ChatTopic> {
        self.topics_projection(chat_id, cx).topics.clone()
    }

    pub fn topic(&self, chat_id: &str, topic_id: &str, cx: &mut App) -> Option<ChatTopic> {
        self.topics_projection(chat_id, cx).topics.iter().find(|topic| topic.id == topic_id).cloned()
    }

    pub fn in_memory_topics_general_topic(&self, chat_id: &str, cx: &mut App) -> Option<ChatTopic> {
        SaiWsResponseCache::observed_memory_only(&Self::topics_cache_key(chat_id), cx)?;
        self.topics_projection(chat_id, cx).general.clone()
    }

    pub fn general_topic(&self, chat_id: &str, cx: &mut App) -> Option<ChatTopic> {
        self.topics_projection(chat_id, cx).general.clone().or_else(|| {
            self.forum_topics_dto(chat_id, cx)?.general_topic.as_ref().map(RiseStoredChatTopic::to_topic)
        })
    }

    pub fn unread_count_topic_id(&self, chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> i64 {
        let Some(topic_id) = topic_id else {
            return self.unread_count(chat_id, cx);
        };
        if topic_id.is_empty() {
            return self.general_topic(chat_id, cx).map_or(0, |topic| topic.unread_count).max(0);
        }
        self.topic_from_any_cache(chat_id, topic_id, cx).map_or(0, |topic| topic.unread_count).max(0)
    }

    pub fn all_topics_view_hidden(&self, chat_id: &str, cx: &mut App) -> bool {
        if let Some(hidden) = self.hide_all_topics_override.get(chat_id) {
            return *hidden;
        }
        self.forum_topics_dto(chat_id, cx)
            .and_then(|dto| dto.hide_all_topics)
            .or_else(|| self.topics_dto(chat_id, cx).and_then(|dto| dto.hide_all_topics))
            .unwrap_or(false)
    }

    pub fn set_all_topics_view_hidden_override(
        &mut self,
        chat_id: &str,
        hidden: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if self.hide_all_topics_override.get(chat_id).copied() == hidden {
            return;
        }
        match hidden {
            Some(hidden) => {
                self.hide_all_topics_override.insert(chat_id.to_owned(), hidden);
            }
            None => {
                self.hide_all_topics_override.remove(chat_id);
            }
        }
        cx.notify();
    }

    pub fn apply_all_topics_view_hidden(&mut self, chat_id: &str, hidden: bool, cx: &mut Context<Self>) {
        self.mutate_topic_caches(
            chat_id,
            SaiUpdaterWriteScope::All,
            |root| {
                root.insert(key::HIDE_ALL_TOPICS.to_owned(), Value::Bool(hidden));
            },
            cx,
        );
        self.set_all_topics_view_hidden_override(chat_id, None, cx);
    }

    pub fn forced_unread_topic_key(chat_id: &str, topic_id: &str) -> String {
        format!("{chat_id}|{topic_id}")
    }

    pub fn is_topic_forced_unread(&self, chat_id: &str, topic_id: &str) -> bool {
        self.forced_unread_topic_keys.contains(&Self::forced_unread_topic_key(chat_id, topic_id))
    }

    pub fn displayed_unread_count(&self, chat_id: &str, topic: &ChatTopic) -> i64 {
        let unread = topic.unread_count.max(0);
        if unread != 0 || !self.is_topic_forced_unread(chat_id, &topic.id) {
            return unread;
        }
        1
    }

    pub fn set_topic_forced_unread(&mut self, chat_id: &str, topic_id: &str, forced: bool, cx: &mut Context<Self>) {
        let key = Self::forced_unread_topic_key(chat_id, topic_id);
        let changed = if forced {
            self.forced_unread_topic_keys.insert(key)
        } else {
            self.forced_unread_topic_keys.remove(&key)
        };
        if changed {
            cx.notify();
        }
    }

    pub fn apply_topic_read_local(&mut self, chat_id: &str, topic_id: &str, cx: &mut Context<Self>) {
        self.set_topic_forced_unread(chat_id, topic_id, false, cx);
        self.mutate_topic_scope_caches(
            chat_id,
            topic_id,
            SaiUpdaterWriteScope::Cache,
            |row| {
                if json_int(row.get(key::UNREAD_COUNT)) == 0 {
                    return;
                }
                row.insert(key::UNREAD_COUNT.to_owned(), Value::from(0));
            },
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mark_last_message_previews_read(
        &mut self,
        chat_id: &str,
        sender_id: &str,
        topic_id: Option<&str>,
        cutoff: f64,
        same_millisecond_message_ids: Option<&HashSet<String>>,
        read_message_ids: Option<&HashSet<String>>,
        cx: &mut Context<Self>,
    ) {
        let did_mutate = self.mutate_cached_chat_row(
            chat_id,
            false,
            |row: &mut Map<String, Value>| {
                Self::mark_raw_last_message_read(
                    row,
                    sender_id,
                    topic_id,
                    cutoff,
                    same_millisecond_message_ids,
                    read_message_ids,
                );
            },
            cx,
        );
        if !did_mutate
            && !self.mark_archived_last_message_read_local(
                chat_id,
                sender_id,
                topic_id,
                cutoff,
                same_millisecond_message_ids,
                read_message_ids,
                cx,
            )
        {
            self.mutate_chat(
                chat_id,
                |chat| {
                    let Some(message) = chat.last_message.as_mut() else {
                        return;
                    };
                    if !Self::message_matches_read_receipt(
                        message,
                        sender_id,
                        topic_id,
                        cutoff,
                        same_millisecond_message_ids,
                        read_message_ids,
                    ) {
                        return;
                    }
                    message.is_read = true;
                },
                cx,
            );
        }

        self.mutate_topic_caches(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |root| {
                if topic_id.is_none_or(str::is_empty)
                    && let Some(mut general) = AnyDictionaryBridge::dictionary(root.get(key::GENERAL_TOPIC))
                {
                    Self::mark_raw_last_message_read(
                        &mut general,
                        sender_id,
                        None,
                        cutoff,
                        same_millisecond_message_ids,
                        read_message_ids,
                    );
                    root.insert(key::GENERAL_TOPIC.to_owned(), Value::Object(general));
                }

                let mut rows = MessagePayloadCodec::array_of_dictionaries(root.get(key::TOPICS));
                for row in &mut rows {
                    let row_topic_id = row.get(key::TOPIC_ID).and_then(Value::as_str).unwrap_or_default();
                    if topic_id.is_some_and(|topic_id| topic_id != row_topic_id) {
                        continue;
                    }
                    Self::mark_raw_last_message_read(
                        row,
                        sender_id,
                        None,
                        cutoff,
                        same_millisecond_message_ids,
                        read_message_ids,
                    );
                }
                if root.contains_key(key::TOPICS) {
                    root.insert(key::TOPICS.to_owned(), Self::topic_rows_value(rows));
                }
            },
            cx,
        );
    }

    /// marks read the topic previews whose last message is one of the receipt own sends
    pub fn apply_topic_receipt(&mut self, chat_id: &str, server_ids: &[String], cx: &mut Context<Self>) {
        if server_ids.is_empty() {
            return;
        }
        let read_ids: HashSet<String> = server_ids.iter().cloned().collect();
        let mark = |row: &mut Map<String, Value>| {
            let Some(mut last_message) = AnyDictionaryBridge::dictionary(row.get(key::LAST_MESSAGE)) else {
                return;
            };
            let is_receipted = ["sai_server_id", "server_id", "id"]
                .iter()
                .filter_map(|id_key| last_message.get(*id_key).and_then(Value::as_str))
                .any(|message_id| read_ids.contains(message_id));
            if !is_receipted || last_message.get(key::IS_READ).and_then(Value::as_bool) == Some(true) {
                return;
            }
            last_message.insert(key::IS_READ.to_owned(), Value::Bool(true));
            last_message.insert(support_key::DELIVERY_STATUS.to_owned(), Value::from("read"));
            row.insert(key::LAST_MESSAGE.to_owned(), Value::Object(last_message));
        };
        self.mutate_topic_caches(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |root| {
                if let Some(mut general) = AnyDictionaryBridge::dictionary(root.get(key::GENERAL_TOPIC)) {
                    mark(&mut general);
                    root.insert(key::GENERAL_TOPIC.to_owned(), Value::Object(general));
                }
                if root.contains_key(key::TOPICS) {
                    let mut rows = MessagePayloadCodec::array_of_dictionaries(root.get(key::TOPICS));
                    for row in &mut rows {
                        mark(row);
                    }
                    root.insert(key::TOPICS.to_owned(), Self::topic_rows_value(rows));
                }
            },
            cx,
        );
    }

    pub fn message_matches_read_receipt(
        message: &GetMessageMessage,
        sender_id: &str,
        topic_id: Option<&str>,
        cutoff: f64,
        same_millisecond_message_ids: Option<&HashSet<String>>,
        read_message_ids: Option<&HashSet<String>>,
    ) -> bool {
        let server_id = message.sai_server_id();
        let message_id = if server_id.is_empty() { message.id.as_str() } else { server_id.as_str() };
        if message.sender_id != sender_id
            || !MessageServicesStore::is_within_read_boundary(
                message_id,
                message.created_at,
                cutoff,
                same_millisecond_message_ids,
                read_message_ids,
            )
        {
            return false;
        }
        let Some(topic_id) = topic_id else {
            return true;
        };
        message.topic_id.as_deref().unwrap_or_default() == topic_id
    }

    pub fn mark_raw_last_message_read(
        container: &mut Map<String, Value>,
        sender_id: &str,
        topic_id: Option<&str>,
        cutoff: f64,
        same_millisecond_message_ids: Option<&HashSet<String>>,
        read_message_ids: Option<&HashSet<String>>,
    ) {
        let Some(mut last_message) = AnyDictionaryBridge::dictionary(container.get(key::LAST_MESSAGE)) else {
            return;
        };
        let last_sender_id = ws_string(&last_message, &[message_key::SENDER_ID]).unwrap_or_default();
        let timestamp = MessageThreadOrder::timestamp_seconds(&last_message).unwrap_or(0.0);
        let message_id = ws_string(&last_message, &["sai_server_id", "server_id", "id"]).unwrap_or_default();
        if last_sender_id != sender_id
            || !MessageServicesStore::is_within_read_boundary(
                &message_id,
                timestamp,
                cutoff,
                same_millisecond_message_ids,
                read_message_ids,
            )
        {
            return;
        }
        if let Some(topic_id) = topic_id
            && ws_string(&last_message, &[message_key::TOPIC_ID]).unwrap_or_default() != topic_id
        {
            return;
        }
        last_message.insert(key::IS_READ.to_owned(), Value::Bool(true));
        last_message.insert(support_key::DELIVERY_STATUS.to_owned(), Value::from("read"));
        container.insert(key::LAST_MESSAGE.to_owned(), Value::Object(last_message));
    }

    pub fn apply_topic_unread_marks_delta(
        &self,
        chat_id: &str,
        topic_id: &str,
        mentions: i64,
        replies: i64,
        reactions: i64,
        cx: &mut App,
    ) -> bool {
        if mentions == 0 && replies == 0 && reactions == 0 {
            return false;
        }
        self.mutate_topic_scope_caches(
            chat_id,
            topic_id,
            SaiUpdaterWriteScope::Cache,
            |row| {
                if mentions != 0 {
                    let count = (json_int(row.get(key::UNREAD_MENTIONS_COUNT)) + mentions).max(0);
                    row.insert(key::UNREAD_MENTIONS_COUNT.to_owned(), Value::from(count));
                }
                if replies != 0 {
                    let count = (json_int(row.get(key::UNREAD_REPLIES_COUNT)) + replies).max(0);
                    row.insert(key::UNREAD_REPLIES_COUNT.to_owned(), Value::from(count));
                }
                if reactions != 0 {
                    let count = (json_int(row.get(key::UNREAD_REACTIONS_COUNT)) + reactions).max(0);
                    row.insert(key::UNREAD_REACTIONS_COUNT.to_owned(), Value::from(count));
                }
            },
            cx,
        )
    }

    pub fn apply_topic_message_created(
        &self,
        chat_id: &str,
        topic_id: &str,
        last_message: Map<String, Value>,
        updated_at: f64,
        increment_unread: bool,
        cx: &mut App,
    ) -> bool {
        forget_failed_topic_previews(chat_id, |entry| entry.topic_id != topic_id);
        self.write_topic_last_message(chat_id, topic_id, last_message, updated_at, increment_unread, cx)
    }

    fn write_topic_last_message(
        &self,
        chat_id: &str,
        topic_id: &str,
        last_message: Map<String, Value>,
        updated_at: f64,
        increment_unread: bool,
        cx: &mut App,
    ) -> bool {
        self.mutate_topic_scope_caches(
            chat_id,
            topic_id,
            SaiUpdaterWriteScope::Cache,
            |row| {
                row.insert(key::LAST_MESSAGE.to_owned(), Value::Object(last_message.clone()));
                row.insert(key::UPDATED_AT.to_owned(), Value::from(updated_at));
                if increment_unread {
                    let count = (json_int(row.get(key::UNREAD_COUNT)) + 1).max(1);
                    row.insert(key::UNREAD_COUNT.to_owned(), Value::from(count));
                }
            },
            cx,
        )
    }

    pub fn apply_topic_message_edited(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        message_id: &str,
        last_message: Map<String, Value>,
        cx: &mut App,
    ) {
        if let Some(topic_id) = topic_id {
            self.mutate_topic_scope_caches(
                chat_id,
                topic_id,
                SaiUpdaterWriteScope::Cache,
                |row| {
                    let Some(current) = AnyDictionaryBridge::dictionary(row.get(key::LAST_MESSAGE)) else {
                        return;
                    };
                    if row_message_id(&current) != message_id {
                        return;
                    }
                    row.insert(key::LAST_MESSAGE.to_owned(), Value::Object(last_message.clone()));
                },
                cx,
            );
            return;
        }
        self.mutate_matching_topic_previews(
            chat_id,
            message_id,
            |row| {
                row.insert(key::LAST_MESSAGE.to_owned(), Value::Object(last_message.clone()));
            },
            cx,
        );
    }

    pub fn apply_topic_message_deleted(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        message_id: &str,
        cx: &mut App,
    ) -> bool {
        forget_failed_topic_previews(chat_id, |entry| entry.local_id != message_id);
        if let Some(topic_id) = topic_id {
            let mut cleared_preview = false;
            self.mutate_topic_scope_caches(
                chat_id,
                topic_id,
                SaiUpdaterWriteScope::Cache,
                |row| {
                    let Some(current) = AnyDictionaryBridge::dictionary(row.get(key::LAST_MESSAGE)) else {
                        return;
                    };
                    if row_message_id(&current) != message_id {
                        return;
                    }
                    row.insert(key::LAST_MESSAGE.to_owned(), Value::Null);
                    cleared_preview = true;
                },
                cx,
            );
            return cleared_preview;
        }
        self.mutate_matching_topic_previews(
            chat_id,
            message_id,
            |row| {
                row.insert(key::LAST_MESSAGE.to_owned(), Value::Null);
            },
            cx,
        )
    }

    pub fn apply_topic_message_send_failed(
        &self,
        chat_id: &str,
        local_id: &str,
        failed_row: Option<(String, Map<String, Value>, f64)>,
        cx: &mut App,
    ) -> bool {
        if let Some((topic_id, row, updated_at)) = failed_row {
            self.write_topic_last_message(chat_id, &topic_id, row.clone(), updated_at, false, cx);
            FAILED_TOPIC_PREVIEWS.with(|cell| {
                let mut map = cell.borrow_mut();
                let list = map.entry(chat_id.to_owned()).or_default();
                list.retain(|entry| entry.topic_id != topic_id);
                list.push(FailedTopicPreview { topic_id, local_id: local_id.to_owned(), row, updated_at });
            });
        }
        self.mark_topic_preview_send_failed(chat_id, local_id, cx)
    }

    /// puts refused send previews back on topic rows after a topics or forum structure refetch
    pub fn reapply_failed_topic_previews(&self, chat_id: &str, cx: &mut App) {
        let entries = FAILED_TOPIC_PREVIEWS.with(|cell| cell.borrow().get(chat_id).cloned().unwrap_or_default());
        for entry in entries {
            self.write_topic_last_message(chat_id, &entry.topic_id, entry.row, entry.updated_at, false, cx);
        }
    }

    fn mark_topic_preview_send_failed(&self, chat_id: &str, local_id: &str, cx: &mut App) -> bool {
        self.mutate_matching_topic_previews(
            chat_id,
            local_id,
            |row| {
                let Some(mut current) = AnyDictionaryBridge::dictionary(row.get(key::LAST_MESSAGE)) else {
                    return;
                };
                current.insert(RiseLocalKey::SEND_FAILED.to_owned(), Value::Bool(true));
                row.insert(key::LAST_MESSAGE.to_owned(), Value::Object(current));
            },
            cx,
        )
    }

    fn mutate_matching_topic_previews(
        &self,
        chat_id: &str,
        message_id: &str,
        mut transform: impl FnMut(&mut Map<String, Value>),
        cx: &mut App,
    ) -> bool {
        let mut matched_preview = false;
        self.mutate_topic_caches(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |root| {
                if let Some(mut general) = AnyDictionaryBridge::dictionary(root.get(key::GENERAL_TOPIC))
                    && let Some(current) = AnyDictionaryBridge::dictionary(general.get(key::LAST_MESSAGE))
                    && row_message_id(&current) == message_id
                {
                    transform(&mut general);
                    root.insert(key::GENERAL_TOPIC.to_owned(), Value::Object(general));
                    matched_preview = true;
                }

                let mut rows = MessagePayloadCodec::array_of_dictionaries(root.get(key::TOPICS));
                let mut changed = false;
                for row in &mut rows {
                    let Some(current) = AnyDictionaryBridge::dictionary(row.get(key::LAST_MESSAGE)) else {
                        continue;
                    };
                    if row_message_id(&current) != message_id {
                        continue;
                    }
                    transform(row);
                    changed = true;
                    matched_preview = true;
                }
                if changed {
                    root.insert(key::TOPICS.to_owned(), Self::topic_rows_value(rows));
                }
            },
            cx,
        );
        matched_preview
    }

    pub fn topic_from_any_cache(&self, chat_id: &str, topic_id: &str, cx: &mut App) -> Option<ChatTopic> {
        if topic_id == ChatTopic::GENERAL_WALLPAPER_SCOPE_ID {
            return self.general_topic(chat_id, cx);
        }
        self.topic(chat_id, topic_id, cx)
            .or_else(|| self.forum_topics(chat_id, cx).into_iter().find(|topic| topic.id == topic_id))
    }

    pub fn topics_enabled(&self, chat_id: &str, cx: &mut App) -> bool {
        if self.topics_disabled_override.contains(chat_id) {
            return false;
        }
        if self.topics_enabled_override.contains(chat_id) {
            return true;
        }
        if let Some(chat) = self.chat_info(chat_id, cx) {
            return chat.topics_enabled;
        }
        !self.topics_projection(chat_id, cx).topics.is_empty()
    }

    pub fn can_post_in_topic(&self, chat_id: &str, topic_id: &str, cx: &mut App) -> bool {
        let state = self
            .topic_from_any_cache(chat_id, topic_id, cx)
            .map_or(ChatTopicState::Missing, |topic| topic.posting_state());
        let cx = RefCell::new(cx);
        ChatPermissionResolution::can_post(
            state,
            || ChatPerm::is_owner_role(self.chat_info(chat_id, &cx.borrow()).and_then(|chat| chat.role).as_deref()),
            || self.closed_topic_bypass_permissions(chat_id, topic_id, &mut cx.borrow_mut()),
        )
    }

    fn closed_topic_bypass_permissions(&self, chat_id: &str, topic_id: &str, cx: &mut App) -> i64 {
        if let Some(scoped) = self.memory_only_permissions(chat_id, Some(topic_id), cx) {
            return scoped;
        }
        let base = self.memory_only_permissions(chat_id, None, cx).unwrap_or(0);
        let role = self.chat_info(chat_id, cx).and_then(|chat| chat.role);
        if ChatPerm::is_owner_or_legacy_admin_role(role.as_deref()) {
            return base;
        }
        base & ChatPerm::BYPASS_BIT
    }

    pub fn memory_only_permissions(&self, chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> Option<i64> {
        SaiWsResponseCache::observed_memory_only(&Self::effective_perms_cache_key(chat_id, topic_id), cx)?;
        self.effective_permissions(chat_id, topic_id, cx).map(|permissions| permissions.mask())
    }

    pub fn can_manage_topics(&self, chat_id: &str, cx: &mut App) -> bool {
        self.can(chat_id, ChatPerm::MANAGE_TOPICS, cx)
    }

    pub fn can_manage_voice_channels(&self, chat_id: &str, cx: &mut App) -> bool {
        self.can(chat_id, ChatPerm::MANAGE_VOICE_CHANNEL, cx)
    }

    pub fn can_create_channel(&self, chat_id: &str, cx: &mut App) -> bool {
        self.can_manage_topics(chat_id, cx) || self.can_manage_voice_channels(chat_id, cx)
    }

    pub fn can_manage_channel(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        kind: ChatChannelKind,
        cx: &mut App,
    ) -> bool {
        let scoped_permissions = self.effective_permissions(chat_id, topic_id, cx).map(|permissions| permissions.mask());
        let permissions = scoped_permissions
            .or_else(|| {
                topic_id?;
                self.effective_permissions(chat_id, None, cx).map(|permissions| permissions.mask())
            })
            .unwrap_or(0);
        ChatChannelManagementPolicy::can_manage(kind, permissions)
    }

    pub fn resolved_topic_notification_scopes(
        topic_scopes: ChatMuteScopes,
        inherited: bool,
        inherited_scopes: ChatMuteScopes,
    ) -> ChatMuteScopes {
        if inherited { inherited_scopes } else { topic_scopes }
    }

    pub fn effective_topic_notification_scopes(&self, chat_id: &str, topic: &ChatTopic, cx: &App) -> ChatMuteScopes {
        if let Some(hooks) = chat_stores_user_hooks()
            && (hooks.mute_all_chats_enabled)(cx)
            && !(hooks.chat_mute_override_ids)(cx).contains(chat_id)
        {
            return ChatMuteScopes::MUTED;
        }

        let inherited_scopes = match self.chat_info(chat_id, cx) {
            Some(chat) => self.effective_notification_scopes_for_chat(chat_id, chat.notification_scopes(), cx),
            None => topic.notification_scopes,
        };

        Self::resolved_topic_notification_scopes(
            topic.notification_scopes,
            topic.notification_inherited,
            inherited_scopes,
        )
    }

    pub fn topic_raw_rows(
        &self,
        chat_id: &str,
        topic_id: &str,
        cx: &mut App,
    ) -> (Option<Map<String, Value>>, Option<Map<String, Value>>) {
        let topics_row = self.topics_dto(chat_id, cx).and_then(|dto| {
            dto.topics.iter().flatten().find(|topic| topic.topic_id == topic_id).and_then(SaiJSONCodec::encode)
        });
        let forum_row = self.forum_topics_dto(chat_id, cx).and_then(|dto| {
            dto.topics.iter().flatten().find(|topic| topic.topic_id == topic_id).and_then(SaiJSONCodec::encode)
        });

        (topics_row, forum_row)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_optimistic_topic(
        &self,
        chat_id: &str,
        topic_id: &str,
        title: &str,
        icon: Option<&str>,
        section_id: Option<&str>,
        created_by: &str,
        cx: &mut App,
    ) {
        let last_position = |topics: Vec<ChatTopic>| topics.iter().map(|topic| topic.position).max().unwrap_or(-1);
        let position =
            last_position(self.topics(chat_id, cx)).max(last_position(self.forum_topics(chat_id, cx))) + 1;
        let inherited_scopes = match self.chat_info(chat_id, cx) {
            Some(chat) => self.effective_notification_scopes_for_chat(chat_id, chat.notification_scopes(), cx),
            None => ChatMuteScopes::ALL_ENABLED,
        };
        let resolved_icon = icon
            .map(trimming_whitespaces_and_newlines)
            .filter(|icon| !icon.is_empty())
            .unwrap_or(ChatTopic::DEFAULT_ICON);
        let now = (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64;
        let row: Map<String, Value> = [
            (key::TOPIC_ID, Value::from(topic_id)),
            (key::CHAT_ID, Value::from(chat_id)),
            (key::TITLE, Value::from(title)),
            (key::ICON, Value::from(resolved_icon)),
            (key::CREATED_BY, Value::from(created_by)),
            (key::CREATED_AT, Value::from(now)),
            (key::POSITION, Value::from(position)),
            (key::IS_GENERAL, Value::Bool(false)),
            (key::IS_CLOSED, Value::Bool(false)),
            (key::SECTION_ID, Value::from(section_id.unwrap_or_default())),
            (key::IS_PRIVATE, Value::Bool(false)),
            (key::NOTIFICATION_MESSAGES, Value::Bool(inherited_scopes.messages)),
            (key::NOTIFICATION_MENTIONS, Value::Bool(inherited_scopes.mentions)),
            (key::NOTIFICATION_REPLIES, Value::Bool(inherited_scopes.replies)),
            (key::NOTIFICATION_REACTIONS, Value::Bool(inherited_scopes.reactions)),
            (key::NOTIFICATION_EVENTS, Value::Bool(inherited_scopes.events)),
            (key::NOTIFICATION_INHERITED, Value::Bool(true)),
            (key::UNREAD_COUNT, Value::from(0)),
            (key::IS_MUTED, Value::Bool(inherited_scopes.is_fully_muted())),
            (key::UPDATED_AT, Value::from(now)),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();

        self.upsert_topic_row_in_caches(&row, chat_id, SaiUpdaterWriteScope::Cache, cx);
    }

    pub fn insert_topic(&self, wire: &RiseStoredChatTopic, chat_id: &str, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(wire) else {
            return;
        };

        self.upsert_topic_row_in_caches(&row, chat_id, SaiUpdaterWriteScope::All, cx);
    }

    pub fn replace_topic(&self, wire: &RiseStoredChatTopic, chat_id: &str, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(wire) else {
            return;
        };

        self.upsert_topic_row_in_caches(&row, chat_id, SaiUpdaterWriteScope::All, cx);
    }

    pub fn patch_topic(&self, chat_id: &str, topic_id: &str, fields: Map<String, Value>, cx: &mut App) {
        if topic_id == ChatTopic::GENERAL_WALLPAPER_SCOPE_ID {
            self.mutate_topic_scope_caches(
                chat_id,
                "",
                SaiUpdaterWriteScope::Cache,
                |row| {
                    for (name, value) in &fields {
                        row.insert(name.clone(), value.clone());
                    }
                },
                cx,
            );
            return;
        }
        self.patch_topic_row(
            &Self::topics_cache_key(chat_id),
            topic_id,
            &fields,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        if self.has_forum_cache(chat_id, cx) {
            self.patch_topic_row(
                &Self::forum_structure_cache_key(chat_id),
                topic_id,
                &fields,
                SaiUpdaterWriteScope::Cache,
                cx,
            );
        }
    }

    fn mutate_topic_scope_caches(
        &self,
        chat_id: &str,
        topic_id: &str,
        write_to: SaiUpdaterWriteScope,
        mut transform: impl FnMut(&mut Map<String, Value>),
        cx: &mut App,
    ) -> bool {
        let mut did_mutate = false;
        self.mutate_topic_caches(
            chat_id,
            write_to,
            |root| {
                if topic_id.is_empty() {
                    let Some(mut general) = AnyDictionaryBridge::dictionary(root.get(key::GENERAL_TOPIC)) else {
                        return;
                    };
                    transform(&mut general);
                    root.insert(key::GENERAL_TOPIC.to_owned(), Value::Object(general));
                    did_mutate = true;
                    return;
                }

                let mut rows = MessagePayloadCodec::array_of_dictionaries(root.get(key::TOPICS));
                let Some(row) = rows
                    .iter_mut()
                    .find(|row| row.get(key::TOPIC_ID).and_then(Value::as_str) == Some(topic_id))
                else {
                    return;
                };
                transform(row);
                root.insert(key::TOPICS.to_owned(), Self::topic_rows_value(rows));
                did_mutate = true;
            },
            cx,
        );
        did_mutate
    }

    fn mutate_topic_caches(
        &self,
        chat_id: &str,
        write_to: SaiUpdaterWriteScope,
        mut transform: impl FnMut(&mut Map<String, Value>),
        cx: &mut App,
    ) {
        let cache_keys = [Self::topics_cache_key(chat_id), Self::forum_structure_cache_key(chat_id)];
        for cache_key in cache_keys {
            if SaiWsResponseCache::observed_payload(
                &cache_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
            .is_none()
            {
                continue;
            }
            let _ = SaiUpdater::update_json_value(
                &cache_key,
                |context| {
                    let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                    transform(&mut root);
                    Ok(Value::Object(root))
                },
                write_to,
                cx,
            );
            if write_to == SaiUpdaterWriteScope::Cache {
                self.schedule_topic_cache_disk_persistence(cache_key, cx);
            }
        }
        if write_to == SaiUpdaterWriteScope::All {
            self.mirror_topic_caches_to_rise(chat_id, cx);
        }
    }

    fn schedule_topic_cache_disk_persistence(&self, cache_key: String, cx: &mut App) {
        let _ = SaiDebouncer::debounced_action_delay_milliseconds(
            cache_key.clone(),
            Box::new(move |cx: &mut App| {
                let Some(payload) = SaiWsResponseCache::observed_memory_only(&cache_key, cx) else {
                    return;
                };
                SaiWsDiskCache::save_asynchronously(cache_key, payload, |_, _| {}, cx);
            }),
            1_000,
            "chatTopicDiskPersistence",
            64,
            None,
            cx,
        );
    }

    fn topic_rows_value(rows: Vec<Map<String, Value>>) -> Value {
        Value::Array(rows.into_iter().map(Value::Object).collect())
    }

    pub fn remove_topic(&self, chat_id: &str, topic_id: &str, write_to: SaiUpdaterWriteScope, cx: &mut App) {
        self.remove_topic_row(&Self::topics_cache_key(chat_id), topic_id, write_to, cx);
        if self.has_forum_cache(chat_id, cx) {
            self.remove_topic_row(&Self::forum_structure_cache_key(chat_id), topic_id, write_to, cx);
        }
        if write_to == SaiUpdaterWriteScope::All {
            self.mirror_topic_caches_to_rise(chat_id, cx);
        }
    }

    pub fn restore_topic_rows(
        &self,
        rows: (Option<Map<String, Value>>, Option<Map<String, Value>>),
        chat_id: &str,
        topic_id: &str,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) {
        let (topics_row, forum_row) = rows;
        if let Some(topics_row) = topics_row {
            self.replace_topic_row(&Self::topics_cache_key(chat_id), topic_id, &topics_row, write_to, cx);
        }
        if let Some(forum_row) = forum_row
            && self.has_forum_cache(chat_id, cx)
        {
            self.replace_topic_row(&Self::forum_structure_cache_key(chat_id), topic_id, &forum_row, write_to, cx);
        }
        if write_to == SaiUpdaterWriteScope::All {
            self.mirror_topic_caches_to_rise(chat_id, cx);
        }
    }

    pub fn apply_topic_event_upsert(&self, any: Option<&Value>, name: &str, cx: &mut App) {
        let (chat_id, card) = match RiseEvent::decode_payload(name, any) {
            Some(RiseEvent::TopicCreated(event)) => (event.chat_id, event.topic),
            Some(RiseEvent::TopicUpdated(event)) => (event.chat_id, event.topic),
            _ => return,
        };
        if chat_id.is_empty() {
            return;
        }
        if card.is_general {
            let wallpaper = card
                .wallpaper
                .as_ref()
                .map(ChatWallpaperWireDTO::from_canon)
                .and_then(|wallpaper| SaiJSONCodec::encode(&wallpaper))
                .map_or(Value::Null, Value::Object);
            let mut fields = Map::new();
            fields.insert(key::WALLPAPER.to_owned(), wallpaper);
            self.patch_topic(&chat_id, ChatTopic::GENERAL_WALLPAPER_SCOPE_ID, fields, cx);
            return;
        }
        if card.topic_id.is_empty() {
            return;
        }
        let Some(topic) = SaiJSONCodec::encode(&RiseStoredChatTopic::from_canon_card(&card)) else {
            return;
        };

        self.upsert_topic_row_in_caches(&topic, &chat_id, SaiUpdaterWriteScope::All, cx);
    }

    /// returns the topic unread count read before its rows left the caches
    pub fn apply_topic_deleted_event(&mut self, any: Option<&Value>, cx: &mut Context<Self>) -> i64 {
        let Some(RiseEvent::TopicDeleted(event)) = RiseEvent::decode_payload(name::TOPIC_DELETED, any) else {
            return 0;
        };
        let chat_id = event.chat_id;
        let topic_id = event.topic_id;
        if chat_id.is_empty() || topic_id.is_empty() {
            return 0;
        }

        let key = (chat_id.clone(), topic_id.clone());
        let unread = match self.topic_from_any_cache(&chat_id, &topic_id, cx) {
            Some(topic) => {
                let unread = topic.unread_count.max(0);
                if unread > 0 {
                    self.deleted_topic_unread.insert(key, unread);
                }
                unread
            }
            None => self.deleted_topic_unread.remove(&key).unwrap_or(0),
        };
        tracing::warn!(target: "riseonly::X1677", chat = chat_id.as_str(), topic = topic_id.as_str(), unread, "topic deleted unread");
        self.remove_topic(&chat_id, &topic_id, SaiUpdaterWriteScope::All, cx);

        let auth = AuthServicesStore::shared(cx);
        let my_id = {
            let auth = auth.read(cx);
            auth.effective_user_id_for_wire().or_else(|| auth.profile().map(|profile| profile.id.clone()))
        };
        let reason = if my_id.as_deref() == Some(event.deleted_by.as_str()) {
            ChatRemovedTopicNoticeReason::DeletedByMe
        } else {
            ChatRemovedTopicNoticeReason::Deleted
        };
        self.removed_topic_notice = Some(ChatRemovedTopicNotice::new(chat_id, vec![topic_id], reason));
        cx.notify();
        unread
    }

    pub fn note_own_deleted_topic(&mut self, chat_id: &str, topic_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() || topic_id.is_empty() {
            return;
        }
        self.removed_topic_notice = Some(ChatRemovedTopicNotice::new(
            chat_id.to_owned(),
            vec![topic_id.to_owned()],
            ChatRemovedTopicNoticeReason::DeletedByMe,
        ));
        cx.notify();
    }

    pub fn note_lost_forum_topics(
        &mut self,
        chat_id: &str,
        before: impl Borrow<HashSet<String>>,
        after: impl Borrow<HashSet<String>>,
        cx: &mut Context<Self>,
    ) {
        let before: &HashSet<String> = before.borrow();
        let after: &HashSet<String> = after.borrow();
        let mut lost: Vec<String> = before.difference(after).cloned().collect();
        lost.sort();
        if chat_id.is_empty() || lost.is_empty() {
            return;
        }
        let forgotten: Vec<String> = lost.iter().filter(|topic_id| !topic_id.is_empty()).cloned().collect();
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.forget_lost_topics(chat_id, &forgotten, cx));
        tracing::warn!(target: "riseonly::X1197", chat = chat_id, lost = ?lost, "forum topics lost");
        self.removed_topic_notice = Some(ChatRemovedTopicNotice::new(
            chat_id.to_owned(),
            lost,
            ChatRemovedTopicNoticeReason::AccessLost,
        ));
        cx.notify();
    }

    fn upsert_topic_row_in_caches(
        &self,
        row: &Map<String, Value>,
        chat_id: &str,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) {
        self.upsert_topic_row(&Self::topics_cache_key(chat_id), row, write_to, cx);
        let has_forum_cache = self.has_forum_cache(chat_id, cx);
        if has_forum_cache {
            self.upsert_topic_row(&Self::forum_structure_cache_key(chat_id), row, write_to, cx);
        }
        tracing::warn!(
            target: "riseonly::X1046",
            chat = chat_id,
            topic = ?row.get(key::TOPIC_ID),
            section = ?row.get(key::SECTION_ID),
            has_forum_cache,
            forum_topics = self.in_memory_forum_topics(chat_id, cx).len(),
            "topic upsert"
        );
        if write_to == SaiUpdaterWriteScope::All {
            self.mirror_topic_caches_to_rise(chat_id, cx);
        }
    }

    fn mirror_topic_caches_to_rise(&self, chat_id: &str, cx: &mut App) {
        let owned_chat_id = chat_id.to_owned();
        let _ = SaiDebouncer::debounced_action_delay_milliseconds(
            format!("rise-topic-mirror-{chat_id}"),
            Box::new(move |cx: &mut App| {
                Self::shared(cx).update(cx, |store, cx| store.persist_topic_caches_to_rise(&owned_chat_id, cx));
            }),
            50,
            "riseTopicCacheMirror",
            64,
            None,
            cx,
        );
    }

    fn persist_topic_caches_to_rise(&self, chat_id: &str, cx: &mut App) {
        let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() else {
            return;
        };
        let resources = [
            (rise_canon::rise_wire_methods::chat::GET_TOPICS, Self::topics_cache_key(chat_id)),
            (rise_canon::rise_wire_methods::chat::GET_FORUM_STRUCTURE, Self::forum_structure_cache_key(chat_id)),
        ];
        for (method, cache_key) in resources {
            let Some(payload) = SaiWsResponseCache::observed_payload(
                &cache_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::Persistent,
                cx,
            ) else {
                continue;
            };
            let Ok(encoded) = serde_json::to_vec(&payload) else {
                continue;
            };
            let domain = domain.clone();
            drop(RiseAccountRuntime::current().spawn(async move {
                let _ = domain
                    .auxiliary
                    .replace_compatibility_cached_response(
                        rise_canon::rise_wire_methods::chat::SERVICE,
                        method,
                        &cache_key,
                        &encoded,
                    )
                    .await;
            }));
        }
    }

    fn upsert_topic_row(
        &self,
        cache_key: &str,
        row: &Map<String, Value>,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) {
        let Some(topic_id) = row.get(key::TOPIC_ID).and_then(Value::as_str).filter(|id| !id.is_empty()) else {
            return;
        };

        let _ = SaiUpdater::insert_row_if_missing(
            cache_key,
            key::TOPICS,
            message_key::TOPIC_ID,
            row.clone(),
            false,
            write_to,
            cx,
        );
        self.patch_topic_row(cache_key, topic_id, row, write_to, cx);
    }

    fn patch_topic_row(
        &self,
        cache_key: &str,
        topic_id: &str,
        fields: &Map<String, Value>,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) {
        let _ = SaiUpdater::patch_row(cache_key, key::TOPICS, message_key::TOPIC_ID, topic_id, fields, write_to, cx);
    }

    fn remove_topic_row(&self, cache_key: &str, topic_id: &str, write_to: SaiUpdaterWriteScope, cx: &mut App) {
        let _ = SaiUpdater::remove_row(cache_key, key::TOPICS, message_key::TOPIC_ID, topic_id, write_to, cx);
    }

    fn replace_topic_row(
        &self,
        cache_key: &str,
        topic_id: &str,
        row: &Map<String, Value>,
        write_to: SaiUpdaterWriteScope,
        cx: &mut App,
    ) {
        self.remove_topic_row(cache_key, topic_id, write_to, cx);
        self.upsert_topic_row(cache_key, row, write_to, cx);
    }
}
