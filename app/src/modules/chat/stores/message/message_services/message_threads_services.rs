use std::collections::{BTreeSet, HashMap};

use gpui::{App, Context, Entity};
use rise_canon::rise_key::guide_screen::camel as guide_screen_camel;
use rise_canon::rise_key::{bot as bot_key, events as events_key, message as message_key};
use rise_canon::RiseEvent;

use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Value};

use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::SaiUpdaterWriteScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_payload_casing::ws_string;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiNavigationBridge;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionsStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageCommenterPreview, MessageHistoryDirection, MessageMediaItem,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    CommenterPreviewWireDTO, MessagePayloadCodec, MessageWireDTO,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

const CHAT_SCREEN_ID: &str = "Chat";
const COMMENTS_REFRESH_DELAY_MILLISECONDS: i64 = 350;
const COMMENTS_REFRESH_GROUP_KEY: &str = "rtThreadRefresh";
const COMMENTS_REFRESH_MAX_CACHE_DATA: i64 = 64;
const RECENT_COMMENTERS_LIMIT: usize = 3;
const MILLISECOND_EDIT_DATE_THRESHOLD: f64 = 1_000_000_000_000.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelPostRef {
    pub chat_id: String,
    pub message_id: String,
}

impl ChannelPostRef {
    pub fn new(chat_id: String, message_id: String) -> Self {
        Self { chat_id, message_id }
    }

    pub fn forward_of(message: Option<&GetMessageMessage>) -> Option<Self> {
        let forward = message?.forward_info.as_ref()?;
        let chat_id = forward.from_chat_id.as_deref().filter(|id| !id.is_empty())?;
        let message_id = forward.from_message_id.as_deref().filter(|id| !id.is_empty())?;
        Some(Self::new(chat_id.to_owned(), message_id.to_owned()))
    }
}

pub struct MessageThreadTotals {
    totals: HashMap<String, i64>,
    order: Vec<String>,
}

impl MessageThreadTotals {
    const MAX_ENTRIES: usize = 64;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { totals: HashMap::new(), order: Vec::new() }
    }

    pub fn totals(&self) -> &HashMap<String, i64> {
        &self.totals
    }

    pub fn set(&mut self, count: i64, root_id: &str, cx: &mut Context<Self>) {
        if root_id.is_empty() {
            return;
        }
        if self.totals.get(root_id) != Some(&count) {
            self.totals.insert(root_id.to_owned(), count.max(0));
        }
        self.order.retain(|id| id != root_id);
        self.order.push(root_id.to_owned());
        if self.order.len() > Self::MAX_ENTRIES {
            let evicted = self.order.remove(0);
            self.totals.remove(&evicted);
        }
        cx.notify();
    }

    pub fn bump(&mut self, delta: i64, root_id: &str, cx: &mut Context<Self>) {
        let Some(current) = self.totals.get(root_id).copied() else {
            return;
        };
        self.set(current + delta, root_id, cx);
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.totals.clear();
        self.order.clear();
        cx.notify();
    }
}

fn all_strings(value: Option<&Value>) -> Option<Vec<String>> {
    value?.as_array()?.iter().map(|item| item.as_str().map(str::to_owned)).collect()
}

fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
    value.map(trimming_whitespaces_and_newlines).filter(|text| !text.is_empty()).map(str::to_owned)
}

fn chat_screen_param(screen_params: &HashMap<String, String>, chat_id: &str, key: &str) -> Option<String> {
    if screen_params.get(guide_screen_camel::CHAT_ID).map(String::as_str) != Some(chat_id) {
        return None;
    }
    screen_params.get(key).cloned()
}

impl MessageServicesStore {
    pub fn apply_thread_total(&mut self, discussion_message_id: &str, count: i64, cx: &mut Context<Self>) {
        MessageThreadTotals::shared(cx).update(cx, |totals, cx| totals.set(count, discussion_message_id, cx));
    }

    pub fn thread_total(&self, root_id: &str, cx: &mut App) -> Option<i64> {
        MessageThreadTotals::shared(cx).read(cx).totals().get(root_id).copied()
    }

    pub fn realtime_scope(&self, _payload: &Map<String, Value>, chat_id: &str) -> String {
        Self::message_scope(chat_id, None, None)
    }

    pub fn loaded_realtime_secondary_scopes(
        &self,
        payload: &Map<String, Value>,
        chat_id: &str,
        message_ids: &[String],
        cx: &mut App,
    ) -> Vec<String> {
        let mut candidates: BTreeSet<String> = BTreeSet::new();
        let mut topic_ids: BTreeSet<String> = BTreeSet::new();
        if let Some(topic_id) = trimmed_non_empty(ws_string(payload, &[message_key::TOPIC_ID]).as_deref()) {
            topic_ids.insert(topic_id);
        }
        let base_scope = self.realtime_scope(payload, chat_id);
        for message_id in message_ids {
            let message = self.message(message_id, &base_scope, cx);
            if let Some(topic_id) = trimmed_non_empty(message.as_ref().and_then(|row| row.topic_id.as_deref())) {
                topic_ids.insert(topic_id);
            }
        }
        for topic_id in &topic_ids {
            candidates.insert(Self::message_scope(chat_id, Some(topic_id), None));
        }
        if let Some(thread_root) = self.thread_root_id(payload) {
            candidates.insert(Self::message_scope(chat_id, None, Some(&thread_root)));
        }
        if let Some(active_thread) = self.on_screen_thread_scope(chat_id, cx) {
            candidates.insert(active_thread);
        }
        if let Some(screen) = SaiNavigationBridge::current_screen(cx).filter(|screen| screen.id == CHAT_SCREEN_ID) {
            let topic_id = chat_screen_param(&screen.params, chat_id, guide_screen_camel::TOPIC_ID);
            if let Some(topic_id) = trimmed_non_empty(topic_id.as_deref()) {
                candidates.insert(Self::message_scope(chat_id, Some(&topic_id), None));
            }
        }
        let mut loaded = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            if candidate != base_scope && self.did_load_messages(&candidate, cx) {
                loaded.push(candidate);
            }
        }
        loaded
    }

    pub fn message_created_thread_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(mut payload) = MessagePayloadCodec::dictionary(any) else {
            return;
        };
        let Some(chat_id) = ws_string(&payload, &[message_key::CHAT_ID]).filter(|id| !id.is_empty()) else {
            return;
        };

        if !payload.contains_key(message_key::ID)
            && let Some(mid) = payload.get(message_key::MESSAGE_ID).cloned()
        {
            payload.insert(message_key::ID.to_owned(), mid);
        }
        if !payload.contains_key(message_key::SENDER)
            && let Some(info) = payload.get(events_key::SENDER_INFO).cloned()
        {
            payload.insert(message_key::SENDER.to_owned(), info);
        }
        let payload_value = Value::Object(payload);
        let Some(dto) = MessagePayloadCodec::decode::<MessageWireDTO>(Some(&payload_value)) else {
            return;
        };
        let Some(payload) = payload_value.as_object() else {
            return;
        };
        let mut new = dto.to_message();
        if let Some(sid) = payload.get(RiseLocalKey::SERVER_ID).and_then(Value::as_str).filter(|sid| !sid.is_empty()) {
            new.server_id = Some(sid.to_owned());
        }
        let scope = self.realtime_scope(payload, &chat_id);
        let mut scopes = self.loaded_realtime_secondary_scopes(payload, &chat_id, &[], cx);
        if self.has_messages_in_memory(&scope, cx) {
            scopes.insert(0, scope);
        }
        for loaded_scope in &scopes {
            let _ = self.upsert_realtime_message_row(loaded_scope, &new, payload, cx);
        }
    }

    pub fn thread_root_id(&self, payload: &Map<String, Value>) -> Option<String> {
        let explicit = ws_string(payload, &[message_key::THREAD_ROOT_ID, guide_screen_camel::THREAD_ROOT_ID]);
        if let Some(explicit) = trimmed_non_empty(explicit.as_deref()) {
            return Some(explicit);
        }
        let reply = MessagePayloadCodec::dictionary(payload.get(message_key::REPLY_TO));
        let root = reply
            .as_ref()
            .and_then(|reply| reply.get(message_key::MESSAGE_ID))
            .and_then(Value::as_str)
            .or_else(|| payload.get(message_key::REPLY_TO_ID).and_then(Value::as_str))?;
        (!root.is_empty()).then(|| root.to_owned())
    }

    pub fn on_screen_thread_scope(&self, chat_id: &str, cx: &mut App) -> Option<String> {
        let screen = SaiNavigationBridge::current_screen(cx).filter(|screen| screen.id == CHAT_SCREEN_ID)?;
        let root = chat_screen_param(&screen.params, chat_id, guide_screen_camel::THREAD_ROOT_ID)
            .filter(|root| !root.is_empty())?;
        Some(Self::message_scope(chat_id, None, Some(&root)))
    }

    pub fn message_edited_thread_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = MessagePayloadCodec::dictionary(any) else {
            return;
        };
        let Some(chat_id) = ws_string(&payload, &[message_key::CHAT_ID]) else {
            return;
        };
        let Some(mid) = ws_string(&payload, &[message_key::ID, message_key::MESSAGE_ID]) else {
            return;
        };

        let new_content = payload
            .get(message_key::NEW_CONTENT)
            .and_then(Value::as_str)
            .or_else(|| payload.get(message_key::CONTENT).and_then(Value::as_str))
            .or_else(|| payload.get(events_key::TEXT).and_then(Value::as_str));
        let edit_date = payload
            .get(message_key::EDIT_DATE)
            .and_then(Value::as_f64)
            .unwrap_or_else(|| RiseClock::now().time_interval_since_1970());
        let edited_at = if edit_date > MILLISECOND_EDIT_DATE_THRESHOLD { edit_date / 1000.0 } else { edit_date };
        let scope = self.realtime_scope(&payload, &chat_id);
        let secondary_scopes =
            self.loaded_realtime_secondary_scopes(&payload, &chat_id, std::slice::from_ref(&mid), cx);

        let edited_media: Vec<MessageMediaItem> = payload
            .get(bot_key::MESSAGE)
            .filter(|row| row.is_object())
            .and_then(|row| MessagePayloadCodec::decode::<MessageWireDTO>(Some(row)))
            .map(|wire| wire.to_message().media_items)
            .unwrap_or_default();

        let apply_edit = |messages: &mut Vec<GetMessageMessage>| {
            if let Some(message) = messages.iter_mut().find(|message| message.matches_message_id(&mid)) {
                message.apply_edited_content(new_content, edited_at);
                if !edited_media.is_empty() {
                    message.media_items = edited_media.clone();
                }
            }
        };

        self.mutate_messages_list(&scope, SaiUpdaterWriteScope::Cache, &apply_edit, cx);
        for secondary_scope in &secondary_scopes {
            self.mutate_messages_list(secondary_scope, SaiUpdaterWriteScope::Cache, &apply_edit, cx);
        }
    }

    pub fn message_deleted_thread_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(payload) = MessagePayloadCodec::dictionary(any) else {
            return;
        };
        let Some(chat_id) = ws_string(&payload, &[message_key::CHAT_ID]) else {
            return;
        };

        let single = |key: &str| payload.get(key).and_then(Value::as_str).map(|id| vec![id.to_owned()]);
        let ids = all_strings(payload.get(message_key::MESSAGE_IDS))
            .or_else(|| single(message_key::ID))
            .or_else(|| single(message_key::MESSAGE_ID))
            .unwrap_or_default();

        if ids.is_empty() {
            return;
        }
        self.record_confirmed_message_deletions(&chat_id, ids.clone(), cx);

        let scope = self.realtime_scope(&payload, &chat_id);
        let secondary_scopes = self.loaded_realtime_secondary_scopes(&payload, &chat_id, &ids, cx);
        let _ = self.apply_delete_optimistic(&ids, &scope, true, cx);
        for secondary_scope in &secondary_scopes {
            let _ = self.apply_delete_optimistic(&ids, secondary_scope, true, cx);
        }
    }

    pub fn apply_comments_bump_optimistic(
        &mut self,
        discussion_message_id: &str,
        delta: i64,
        cx: &mut Context<Self>,
    ) -> bool {
        if discussion_message_id.is_empty() {
            return false;
        }
        MessageThreadTotals::shared(cx).update(cx, |totals, cx| totals.bump(delta, discussion_message_id, cx));

        let auth = AuthServicesStore::shared(cx);
        let (my_name, my_logo) = auth.read(cx).profile().map_or_else(Default::default, |me| {
            let logo = me.avatar_url.clone().filter(|url| !url.is_empty()).unwrap_or_else(|| me.more.logo.clone());
            (me.name.clone(), logo)
        });
        let mut found = false;
        let channels = self.loaded_channels(cx);

        for channel in &channels {
            let carries_post = self
                .messages(&channel.id, cx)
                .iter()
                .any(|message| message.discussion_message_id.as_deref() == Some(discussion_message_id));
            if !carries_post {
                continue;
            }
            found = true;
            let sign = if delta > 0 { "+" } else { "" };
            Sai::log(
                "Message",
                || format!("comments bump {sign}{delta} -> channel {} post for copy {discussion_message_id}", channel.id),
                SaiLogKind::Cache,
                cx,
            );
            self.mutate_messages_list(
                &channel.id,
                SaiUpdaterWriteScope::Cache,
                |messages| {
                    let Some(message) = messages
                        .iter_mut()
                        .find(|message| message.discussion_message_id.as_deref() == Some(discussion_message_id))
                    else {
                        return;
                    };
                    message.comments_count = Some((message.comments_count.unwrap_or(0) + delta).max(0));
                    if delta > 0 && !my_name.is_empty() {
                        message.recent_commenters.retain(|commenter| commenter.name != my_name);
                        message
                            .recent_commenters
                            .insert(0, MessageCommenterPreview { logo: my_logo.clone(), name: my_name.clone() });
                        message.recent_commenters.truncate(RECENT_COMMENTERS_LIMIT);
                    } else if delta < 0
                        && message.recent_commenters.first().is_some_and(|commenter| commenter.name == my_name)
                    {
                        message.recent_commenters.remove(0);
                    }
                },
                cx,
            );
        }
        if !found {
            let channel_count = channels.len();
            Sai::log(
                "Message",
                || {
                    format!(
                        "comments bump skipped: no loaded channel post maps to copy {discussion_message_id} (channels: {channel_count})"
                    )
                },
                SaiLogKind::Cache,
                cx,
            );
        }
        found
    }

    pub fn channel_post_for_thread(
        &self,
        discussion_chat_id: &str,
        root_id: &str,
        cx: &mut App,
    ) -> Option<ChannelPostRef> {
        if discussion_chat_id.is_empty() || root_id.is_empty() {
            return None;
        }
        let thread_scope = Self::message_scope(discussion_chat_id, None, Some(root_id));
        for scope in [thread_scope.as_str(), discussion_chat_id] {
            let message = self.message(root_id, scope, cx);
            if let Some(post_ref) = ChannelPostRef::forward_of(message.as_ref()) {
                return Some(post_ref);
            }
        }
        for channel in &self.loaded_channels(cx) {
            let post = self.messages_in_memory(&channel.id, cx).into_iter().find(|post| {
                post.discussion_message_id.as_deref() == Some(root_id)
                    && post.discussion_chat_id.as_deref().unwrap_or(discussion_chat_id) == discussion_chat_id
            });
            if let Some(post) = post {
                return Some(ChannelPostRef::new(channel.id.clone(), post.sai_server_id()));
            }
        }
        None
    }

    pub fn comments_count(&self, post: &ChannelPostRef, thread_root_id: &str, cx: &mut App) -> Option<i64> {
        let message = self.message(&post.message_id, &post.chat_id, cx)?;
        if message.discussion_message_id.as_deref() != Some(thread_root_id) {
            return None;
        }
        message.comments_count
    }

    pub fn raise_channel_post_comments(
        &self,
        post: &ChannelPostRef,
        thread_root_id: &str,
        thread_scope: &str,
        cx: &mut App,
    ) {
        if thread_root_id.is_empty() {
            return;
        }
        let mut rows: Vec<GetMessageMessage> = self
            .messages_in_memory(thread_scope, cx)
            .into_iter()
            .filter(|message| !message.matches_message_id(thread_root_id) && !message.is_system())
            .collect();
        let loaded = rows.len() as i64;
        let count = self.thread_total(thread_root_id, cx).unwrap_or(0).max(loaded);
        if count <= 0 {
            return;
        }
        rows.sort_by(|left, right| right.created_at.total_cmp(&left.created_at));
        let mut commenters: Vec<CommenterPreviewWireDTO> = Vec::new();
        for row in &rows {
            if commenters.len() >= RECENT_COMMENTERS_LIMIT {
                break;
            }
            if row.sender_name.is_empty() || commenters.iter().any(|known| known.name.as_deref() == Some(row.sender_name.as_str())) {
                continue;
            }
            commenters.push(CommenterPreviewWireDTO {
                logo: row.sender_logo.clone().filter(|logo| !logo.is_empty()),
                name: Some(row.sender_name.clone()),
            });
        }
        let channel_id = post.chat_id.clone();
        let post_id = post.message_id.clone();
        cx.spawn(async move |cx| {
            let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                return;
            };
            RiseAccountRuntime::current().spawn(async move {
                if let Err(error) = domain.messages.raise_comments_total(&channel_id, &post_id, count, commenters).await {
                    tracing::warn!(target: "riseonly::X2164", %error, post = %post_id, "channel post comments raise failed");
                }
            });
        })
        .detach();
    }

    pub fn comments_count_updated_handler(&mut self, any: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::CommentsCountUpdated(event)) =
            RiseEvent::decode_payload(rise_events::name::COMMENTS_COUNT_UPDATED, any)
        else {
            return;
        };
        if event.chat_id.is_empty() || event.message_id.is_empty() {
            return;
        }
        let chat_id = event.chat_id;
        let mid = event.message_id;
        let count = event.comments_count;
        let discussion_chat = event.discussion_chat_id;
        let discussion_msg = event.discussion_message_id;
        let commenters: Vec<MessageCommenterPreview> = event
            .recent_commenters
            .into_iter()
            .map(|commenter| MessageCommenterPreview { logo: commenter.logo, name: commenter.name })
            .collect();

        Sai::log(
            "Message",
            || format!("comments_count_updated: chat {chat_id} msg {mid} -> {count}"),
            SaiLogKind::Websocket,
            cx,
        );
        if !discussion_msg.is_empty() {
            self.apply_thread_total(&discussion_msg, count, cx);
        }
        let mut missing_row_count = None;
        self.mutate_messages_list(
            &chat_id,
            SaiUpdaterWriteScope::Cache,
            |messages| {
                let row_count = messages.len();
                let Some(message) = messages.iter_mut().find(|message| message.matches_message_id(&mid)) else {
                    missing_row_count = Some(row_count);
                    return;
                };
                message.comments_count = Some(count);
                if !discussion_chat.is_empty() {
                    message.discussion_chat_id = Some(discussion_chat.clone());
                }
                if !discussion_msg.is_empty() {
                    message.discussion_message_id = Some(discussion_msg.clone());
                }
                if !commenters.is_empty() {
                    let previous = std::mem::take(&mut message.recent_commenters);
                    message.recent_commenters = commenters
                        .iter()
                        .map(|commenter| {
                            let mut kept = commenter.clone();
                            if kept.logo.is_empty()
                                && let Some(known) = previous
                                    .iter()
                                    .find(|old| old.name == kept.name && !old.logo.is_empty())
                            {
                                kept.logo = known.logo.clone();
                            }
                            kept
                        })
                        .collect();
                }
            },
            cx,
        );
        if let Some(row_count) = missing_row_count {
            Sai::log(
                "Message",
                || format!("comments_count_updated: post {mid} not in {chat_id} cache ({row_count} rows)"),
                SaiLogKind::Cache,
                cx,
            );
        }

        let on_chat_screen = SaiNavigationBridge::current_screen(cx).is_some_and(|screen| {
            screen.id == CHAT_SCREEN_ID
                && screen.params.get(guide_screen_camel::CHAT_ID).map(String::as_str) == Some(chat_id.as_str())
        });
        if on_chat_screen {
            let refresh_chat_id = chat_id.clone();
            let _ = SaiDebouncer::debounced_action_delay_milliseconds(
                format!("rt-refresh-{chat_id}"),
                Box::new(move |cx: &mut App| {
                    MessageActionsStore::shared(cx).update(cx, |actions, cx| {
                        actions.get_messages_action(
                            &refresh_chat_id,
                            None,
                            None,
                            None,
                            None,
                            MessageHistoryDirection::Older,
                            None,
                            true,
                            Some(false),
                            cx,
                        );
                    });
                }),
                COMMENTS_REFRESH_DELAY_MILLISECONDS,
                COMMENTS_REFRESH_GROUP_KEY,
                COMMENTS_REFRESH_MAX_CACHE_DATA,
                None,
                cx,
            );
        }
    }

    fn loaded_channels(&self, cx: &mut App) -> Vec<ChatInfo> {
        let chat_services = ChatServicesStore::shared(cx);
        let app: &App = cx;
        chat_services
            .read(app)
            .current_chat_infos(app)
            .into_iter()
            .filter(|chat| chat.r#type == ChatType::Channel)
            .collect()
    }
}
