use std::any::Any;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, Global, Task, Window};
use rise_canon::RisePushType;
use rise_canon::rise_event_payload::{
    MessageChatInfo, MessageCreatedEvent, MessageMediaInfo, MessageSenderInfo, MessageSenderMore,
    NotificationCreatedData, ReactedByEntryEvent, ReactionEventPayload, ReactionSender,
};
use rise_canon::rise_key::{
    message as message_key, notify as notify_key, payload as payload_key, push, push_route, story_reply,
};
use rise_chat::{ChatType, MessageContentType, MessageEntityField, MessageEntityType, SystemKind};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::language_store::LanguageStore;
use rise_limits::service_limit::{MAX_MENTION_TAG_BYTES, NOTIFY_CHAT_PREVIEW_CHARS};
use rise_notification_texts::{entry, fill, keys};
use rise_notify::{
    MUTE_UNTIL_MILLISECOND_FLOOR, NotificationScope, NotificationSettings, settings_flag_for_type, type_enabled,
};
use rise_engine::RiseJSONValue;
use rise_platform::host_os::HostOs;
use rise_platform::notifications::{NotificationAuthorizationStatus, NotificationTag, authorization_status};
use rise_platform::system_activity::{SystemActivityEvent, SystemActivityWatches, watch};
use rise_platform::window_minimise::hide_window;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::app::dev_drive::DevDrive;
use crate::app::shell::window_presenter::WindowPresenter;
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::desktop_platform::rise_app_quit_drain::request_quit;
use crate::core::desktop_platform::tray_controller::TrayController;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{SAI_WS_DID_CONNECT, SAI_WS_DID_DISCONNECT};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_event_registry::SaiWsEventRegistry;
use crate::core::push_notifications::desktop_notification_delivery::{
    DesktopNotificationDelivery, conversation_tag,
};
use crate::core::push_notifications::notification_service_extension::notification_service::{
    NotificationRequest, NotificationService,
};
use crate::core::push_notifications::push_notification_manager::PushNotificationManager;
use crate::modules::auth::services::auth_local_storage::AuthLocalStorage;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::message_system_pill::MessageSystemFormatter;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, User};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType as RowContentType, MessageReaction, MessageSystemInfo, MessageSystemKind,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::notify::engine::rise_notify_domain::RiseNotifyActionExecutor;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_actions::user_wire_types::{
    UserGetProfileByTagGatewayDTO, UserGetProfileByTagGatewayDTOExt,
};
use crate::modules::notify::stores::notify_actions::notify_types::NotificationItem;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;

const SESSION_DEDUP_CAPACITY: usize = 512;
const FALLBACK_LANGUAGE: &str = "en";
const PREVIEW_ELLIPSIS: &str = "...";
const TITLE_SUFFIX: &str = ".title";
const BODY_SUFFIX: &str = ".body";
const ACTOR_PARAM: &str = "actor";
const CHAT_TITLE_PARAM: &str = "chat_title";
const MESSAGE_PARAM: &str = "message";
const EMOJI_PARAM: &str = "emoji";
const MESSAGE_KEY_PREFIX: &str = "message:";
const REACTION_KEY_PREFIX: &str = "reaction:";
const NOTIFICATION_KEY_PREFIX: &str = "notification:";
const MISSED_LEAD_MS: i64 = 120_000;
const CREATED_ROW_RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(300),
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(8),
];
const CATCH_UP_DELAYS: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(30),
];

/// one socket event as a push, the alert and its group tag and the dictionary
#[derive(Clone, Debug)]
pub struct SocketPush {
    pub title: String,
    pub body: String,
    pub tag: NotificationTag,
    pub dedup_key: String,
    pub created_at_ms: i64,
    pub user_info: Map<String, Value>,
}

/// a reconnect replays what the socket already delivered so one toast per key for the session
#[derive(Debug, Default)]
pub struct SocketNotificationDedup {
    seen: HashSet<String>,
    order: VecDeque<String>,
}

impl SocketNotificationDedup {
    pub fn admit(&mut self, key: &str) -> bool {
        if !self.seen.insert(key.to_owned()) {
            return false;
        }
        self.order.push_back(key.to_owned());
        if self.order.len() > SESSION_DEDUP_CAPACITY
            && let Some(oldest) = self.order.pop_front()
        {
            self.seen.remove(&oldest);
        }
        true
    }
}

/// what closing the last window leaves running
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastWindowClose {
    StayInDock,
    HideToTray,
    CloseToTray,
    Quit,
}

pub fn last_window_close(host: HostOs, keeps_running_in_tray: bool) -> LastWindowClose {
    match (host, keeps_running_in_tray) {
        (HostOs::MacOs, _) => LastWindowClose::StayInDock,
        (HostOs::Windows, true) => LastWindowClose::HideToTray,
        (_, true) => LastWindowClose::CloseToTray,
        (_, false) => LastWindowClose::Quit,
    }
}

fn replies_to_recipient(event: &MessageCreatedEvent, chat_type: ChatType, recipient: &str) -> bool {
    chat_type != ChatType::Channel
        && event.sender_id != recipient
        && event.reply_to.as_ref().is_some_and(|reply| reply.sender_id == recipient)
}

fn names_recipient(event: &MessageCreatedEvent, chat_type: ChatType, recipient: &str, recipient_tag: Option<&str>) -> bool {
    if !matches!(chat_type, ChatType::Group | ChatType::Direct) || event.sender_id == recipient {
        return false;
    }
    entity_names(event.entities.as_ref(), recipient)
        || recipient_tag.is_some_and(|tag| content_tags(&event.content).any(|found| found.eq_ignore_ascii_case(tag)))
}

fn entity_names(entities: Option<&Value>, recipient: &str) -> bool {
    let Some(items) = entities.and_then(Value::as_array) else {
        return false;
    };
    items.iter().any(|entity| {
        entity
            .get(MessageEntityField::EntityType.wire())
            .and_then(Value::as_str)
            .and_then(MessageEntityType::from_wire)
            .filter(|kind| kind.is_user_mention())
            .and_then(MessageEntityType::field)
            .and_then(|field| entity.get(field.wire()))
            .and_then(Value::as_str)
            == Some(recipient)
    })
}

fn tag_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn content_tags(content: &str) -> impl Iterator<Item = &str> {
    let widest = usize::try_from(MAX_MENTION_TAG_BYTES).unwrap_or(usize::MAX);
    let bytes = content.as_bytes();
    let command = content
        .split_whitespace()
        .next()
        .and_then(|token| token.strip_prefix('/'))
        .and_then(|token| token.split_once('@'))
        .filter(|(command, _)| !command.is_empty())
        .map(|(_, rest)| &rest[..rest.bytes().position(|byte| !tag_byte(byte)).unwrap_or(rest.len())]);
    let typed = bytes.iter().enumerate().filter_map(move |(index, byte)| {
        if *byte != b'@' || (index > 0 && tag_byte(bytes[index - 1])) {
            return None;
        }
        let start = index + 1;
        let end = bytes[start..].iter().position(|byte| !tag_byte(*byte)).map_or(bytes.len(), |width| start + width);
        Some(&content[start..end])
    });
    command.into_iter().chain(typed).filter(move |tag| !tag.is_empty() && tag.len() <= widest)
}

fn is_channel_or_group(chat_type: ChatType) -> bool {
    matches!(chat_type, ChatType::Group | ChatType::Channel)
}

/// services spell the type in upper case and clients in lower, both reach the socket
fn chat_type_of(raw: &str) -> ChatType {
    let raw = trimming_whitespaces_and_newlines(raw);
    [ChatType::Group, ChatType::Channel]
        .into_iter()
        .find(|kind| {
            raw.eq_ignore_ascii_case(kind.wire()) || raw.eq_ignore_ascii_case(kind.internal_wire())
        })
        .unwrap_or(ChatType::Direct)
}

fn text(language: &str, key: &str) -> Option<&'static str> {
    entry(language, key).or_else(|| entry(FALLBACK_LANGUAGE, key))
}

fn fragment(language: &str, key: &str) -> String {
    text(language, key).unwrap_or_default().to_owned()
}

fn rendered_part(language: &str, key: &str, suffix: &str, params: &[(&str, &str)], fallback: &str) -> String {
    match text(language, &format!("{key}{suffix}")) {
        Some(template) => {
            let filled = fill(template, params);
            if trimming_whitespaces_and_newlines(&filled).is_empty() {
                fallback.to_owned()
            } else {
                filled
            }
        }
        None => fallback.to_owned(),
    }
}

fn truncated_preview(content: &str) -> String {
    let limit = usize::try_from(NOTIFY_CHAT_PREVIEW_CHARS).unwrap_or(usize::MAX);
    match content.char_indices().nth(limit) {
        Some((index, _)) => format!("{}{PREVIEW_ELLIPSIS}", &content[..index]),
        None => content.to_owned(),
    }
}

fn preview_key(content_type: &str, first_media_type: Option<&str>) -> Option<&'static str> {
    let kind = trimming_whitespaces_and_newlines(content_type).to_ascii_lowercase();
    let kind = if kind.is_empty() || kind == MessageContentType::Text.wire() {
        first_media_type.map(|value| trimming_whitespaces_and_newlines(value).to_ascii_lowercase()).unwrap_or_default()
    } else {
        kind
    };
    Some(match kind.as_str() {
        "" | "text" => return None,
        "image" | "photo" => keys::CHAT_PREVIEW_PHOTO,
        "video" => keys::CHAT_PREVIEW_VIDEO,
        "video_note" => keys::CHAT_PREVIEW_VIDEO_NOTE,
        "voice" => keys::CHAT_PREVIEW_VOICE,
        "audio" | "music" => keys::CHAT_PREVIEW_AUDIO,
        "sticker" => keys::CHAT_PREVIEW_STICKER,
        "animation" | "gif" => keys::CHAT_PREVIEW_GIF,
        "document" | "file" => keys::CHAT_PREVIEW_FILE,
        "contact" => keys::CHAT_PREVIEW_CONTACT,
        "location" => keys::CHAT_PREVIEW_LOCATION,
        "poll" => keys::CHAT_PREVIEW_POLL,
        "post" | "post_short" => keys::CHAT_PREVIEW_POST,
        _ => keys::CHAT_PREVIEW_NEW_MESSAGE,
    })
}

fn message_preview(language: &str, event: &MessageCreatedEvent) -> String {
    let content_type = trimming_whitespaces_and_newlines(&event.content_type);
    let keeps_json = [MessageContentType::Location, MessageContentType::Poll]
        .iter()
        .any(|kind| content_type.eq_ignore_ascii_case(kind.wire()));
    let preview = if keeps_json {
        String::new()
    } else {
        truncated_preview(trimming_whitespaces_and_newlines(&event.content))
    };
    if !preview.is_empty() {
        return preview;
    }
    let first_media = event.media_items.first().map(|media| media.media_type.as_str());
    let key = preview_key(content_type, first_media).unwrap_or(keys::CHAT_PREVIEW_NEW_MESSAGE);
    let label = fragment(language, key);
    if label.is_empty() {
        fragment(language, keys::CHAT_PREVIEW_NEW_MESSAGE)
    } else {
        label
    }
}

fn system_key(is_system: bool, kind: Option<&str>) -> Option<&'static str> {
    if !is_system {
        return None;
    }
    let raw = trimming_whitespaces_and_newlines(kind?);
    let kind = SystemKind::ALL.iter().find(|candidate| candidate.wire() == raw)?;
    Some(match kind {
        SystemKind::MemberJoined => keys::CHAT_SYSTEM_MEMBER_JOINED,
        SystemKind::MemberLeft => keys::CHAT_SYSTEM_MEMBER_LEFT,
        SystemKind::MemberKicked | SystemKind::MemberBanned => keys::CHAT_SYSTEM_MEMBER_KICKED,
        SystemKind::ChatPhotoChanged => keys::CHAT_SYSTEM_CHAT_PHOTO_CHANGED,
        SystemKind::ChatPhotoRemoved => keys::CHAT_SYSTEM_CHAT_PHOTO_REMOVED,
        SystemKind::ChatRenamed => keys::CHAT_SYSTEM_CHAT_RENAMED,
        SystemKind::MessagePinned => keys::CHAT_SYSTEM_MESSAGE_PINNED,
        SystemKind::MessageUnpinned => keys::CHAT_SYSTEM_MESSAGE_UNPINNED,
        _ => return None,
    })
}

/// canon models type these fields as their own json value, the flags are plain json keys
fn json_of<T: Serialize>(value: Option<&T>) -> Option<Value> {
    value.and_then(|value| serde_json::to_value(value).ok())
}

fn metadata_flag(event: &MessageCreatedEvent, key: &str) -> bool {
    json_of(event.metadata.as_ref())
        .and_then(|metadata| metadata.get(key).and_then(Value::as_bool))
        .unwrap_or(false)
}

fn story_reply_key(event: &MessageCreatedEvent, recipient: &str, group: bool, has_text: bool) -> Option<&'static str> {
    if group {
        return None;
    }
    let story = json_of(event.story_reply.as_ref())?;
    let author = story.get(story_reply::AUTHOR)?.get(story_reply::ID)?.as_str()?;
    if author.is_empty() || author != recipient {
        return None;
    }
    Some(if has_text {
        keys::CHAT_MESSAGE_STORY_REPLY
    } else {
        keys::CHAT_MESSAGE_STORY_REPLY_NO_TEXT
    })
}

fn message_template(system: Option<&'static str>, story: Option<&'static str>, chat_type: ChatType) -> &'static str {
    if let Some(key) = system.or(story) {
        return key;
    }
    match chat_type {
        ChatType::Group => keys::CHAT_MESSAGE_GROUP,
        ChatType::Channel => keys::CHAT_MESSAGE_CHANNEL,
        _ => keys::CHAT_MESSAGE_PRIVATE,
    }
}

fn sender_display_name(sender: Option<&MessageSenderInfo>) -> Option<String> {
    let sender = sender?;
    if !trimming_whitespaces_and_newlines(&sender.name).is_empty() {
        Some(sender.name.clone())
    } else if !trimming_whitespaces_and_newlines(&sender.tag).is_empty() {
        Some(format!("@{}", sender.tag))
    } else {
        None
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(trimming_whitespaces_and_newlines).filter(|value| !value.is_empty())
}

fn localize_system_body(event: &MessageCreatedEvent, socket_push: &mut SocketPush, cx: &App) {
    let kind = non_blank(event.system_message_type.as_deref());
    if !event.is_system_message.unwrap_or(false) && kind.is_none() {
        return;
    }
    let mut message = GetMessageMessage::new(event.message_id.clone());
    message.chat_id = event.chat_id.clone();
    message.sender_id = event.sender_id.clone();
    message.content_type = RowContentType::System;
    message.text = event.content.clone();
    if let Some(sender) = event.sender_info.as_ref() {
        message.sender_name = sender.name.clone();
        message.sender_tag = sender.tag.clone();
    }
    message.system = Some(MessageSystemInfo {
        kind: kind.and_then(MessageSystemKind::from_raw).unwrap_or(MessageSystemKind::Generic),
        actor_id: non_blank(Some(event.sender_id.as_str())).map(str::to_owned),
        actor_name: non_blank(event.sender_info.as_ref().map(|sender| sender.name.as_str())).map(str::to_owned),
        ..MessageSystemInfo::default()
    });
    let rendered = MessageSystemFormatter::render_current(&message, cx);
    let text = trimming_whitespaces_and_newlines(&rendered);
    if text.is_empty() {
        return;
    }
    socket_push.body = text.to_owned();
    insert_text(&mut socket_push.user_info, push::MESSAGE_PREVIEW, text);
}

fn insert_text(map: &mut Map<String, Value>, key: &str, value: impl Into<String>) {
    map.insert(key.to_owned(), Value::String(value.into()));
}

fn normalized_ms(value: i64) -> i64 {
    if value > MUTE_UNTIL_MILLISECOND_FLOOR {
        value
    } else {
        value.saturating_mul(1_000)
    }
}

/// push the server would send for a new message, same compose and skip rules
/// none when the server would push nothing for this recipient
pub fn message_created_push(event: &MessageCreatedEvent, recipient: &str, language: &str) -> Option<SocketPush> {
    message_created_push_for(event, recipient, None, language)
}

/// message_created_push that also knows the recipient tag so a typed @tag is a mention
pub fn message_created_push_for(
    event: &MessageCreatedEvent,
    recipient: &str,
    recipient_tag: Option<&str>,
    language: &str,
) -> Option<SocketPush> {
    let is_system = event.is_system_message.unwrap_or(false);
    if event.content_type.eq_ignore_ascii_case(MessageContentType::Call.wire())
        || (is_system && metadata_flag(event, message_key::SILENT))
        || metadata_flag(event, payload_key::OWN_AUTO_FORWARD)
        || event.sender_id == recipient
    {
        return None;
    }

    let chat_type = chat_type_of(event.chat.as_ref().map_or("", |chat| chat.r#type.as_str()));
    let group = is_channel_or_group(chat_type);
    let sender_name = sender_display_name(event.sender_info.as_ref())
        .unwrap_or_else(|| fragment(language, keys::COMMON_ACTOR_SOMEONE));
    let chat_title = non_blank(event.chat.as_ref().map(|chat| chat.title.as_str()));
    let resolved_title = chat_title
        .map(str::to_owned)
        .unwrap_or_else(|| fragment(language, keys::CHAT_TITLE_FALLBACK));
    let preview = message_preview(language, event);
    let has_text = !trimming_whitespaces_and_newlines(&event.content).is_empty();
    let system = system_key(is_system, event.system_message_type.as_deref());
    let story = story_reply_key(event, recipient, group, has_text);
    let template = message_template(system, story, chat_type);

    let notice_title = if system.is_some() && !group {
        sender_name.as_str()
    } else {
        resolved_title.as_str()
    };
    let system_title = trimming_whitespaces_and_newlines(&event.content);
    let mut params = vec![
        (ACTOR_PARAM, sender_name.as_str()),
        (CHAT_TITLE_PARAM, notice_title),
        (MESSAGE_PARAM, preview.as_str()),
    ];
    if system.is_some() {
        params.push((push::TITLE, system_title));
    }
    let title = rendered_part(language, template, TITLE_SUFFIX, &params, &resolved_title);
    let body = rendered_part(language, template, BODY_SUFFIX, &params, &preview);

    let scope = if is_system {
        NotificationScope::Events
    } else if event.is_mentioned_you.unwrap_or(false) || names_recipient(event, chat_type, recipient, recipient_tag) {
        NotificationScope::Mentions
    } else if replies_to_recipient(event, chat_type, recipient) {
        NotificationScope::Replies
    } else {
        NotificationScope::Messages
    };
    let topic_id = non_blank(event.topic_id.as_deref());

    let mut user_info = Map::new();
    insert_text(&mut user_info, push_route::TYPE, RisePushType::NEW_MESSAGE);
    insert_text(&mut user_info, push::NOTIFICATION_SCOPE, scope.as_str());
    insert_text(&mut user_info, push::RECIPIENT_USER_ID, recipient);
    insert_text(&mut user_info, push::CHAT_ID, event.chat_id.as_str());
    insert_text(&mut user_info, push::MESSAGE_ID, event.message_id.as_str());
    insert_text(&mut user_info, push::SENDER_ID, event.sender_id.as_str());
    insert_text(&mut user_info, push::CHAT_TYPE, chat_type.internal_wire());
    insert_text(&mut user_info, push::SENDER_NAME, sender_name.as_str());
    insert_text(
        &mut user_info,
        push::MESSAGE_PREVIEW,
        if system.is_some() { body.as_str() } else { preview.as_str() },
    );
    if let Some(topic_id) = topic_id {
        insert_text(&mut user_info, push::TOPIC_ID, topic_id);
    }
    if group {
        insert_text(&mut user_info, push::CHAT_TITLE, resolved_title.as_str());
    } else if let Some(chat_title) = chat_title {
        insert_text(&mut user_info, push::CHAT_TITLE, chat_title);
    }
    let avatar = if group {
        event.chat.as_ref().and_then(|chat| chat.logo_url.clone())
    } else {
        event.sender_info.as_ref().map(|sender| sender.more.logo.clone())
    };
    if let Some(avatar) = non_blank(avatar.as_deref()) {
        insert_text(&mut user_info, push::AVATAR_URL, avatar);
    }
    if let Some(media) = event.media_items.first()
        && let Some(url) = media.thumbnail_url.as_ref().or(media.file_url.as_ref())
    {
        insert_text(&mut user_info, push::MEDIA_URL, url.as_str());
        insert_text(&mut user_info, push::MEDIA_TYPE, media.media_type.as_str());
    }

    Some(SocketPush {
        title,
        body,
        tag: conversation_tag(&event.chat_id, topic_id),
        dedup_key: format!("{MESSAGE_KEY_PREFIX}{}", event.message_id),
        created_at_ms: normalized_ms(event.created_at),
        user_info,
    })
}

fn message_created_ms(created_at: f64) -> i64 {
    if created_at > MUTE_UNTIL_MILLISECOND_FLOOR as f64 {
        created_at as i64
    } else {
        (created_at * 1_000.0) as i64
    }
}

/// last message of a chat row as the message.created frame
/// the dead socket missed, same compose and skip rules
/// none for a read chat or own or system message or one older than since_ms
pub fn missed_message_push(
    chat: &ChatInfo,
    recipient: &str,
    recipient_tag: Option<&str>,
    since_ms: i64,
    language: &str,
) -> Option<SocketPush> {
    let message = chat.last_message.as_ref()?;
    let created_at_ms = message_created_ms(message.created_at);
    if chat.unread_count <= 0 || message.is_temp || message.is_system() || created_at_ms < since_ms {
        return None;
    }
    let message_id = message.sai_server_id();
    if message_id.is_empty() {
        return None;
    }
    let direct = !is_channel_or_group(chat.r#type);
    let peer = direct.then_some(&chat.participant).filter(|peer| peer.id == message.sender_id);
    let sender_name = non_blank(Some(message.sender_name.as_str()))
        .or_else(|| peer.and_then(|peer| non_blank(Some(peer.name.as_str()))))
        .unwrap_or_default()
        .to_owned();
    let sender_tag = non_blank(Some(message.sender_tag.as_str()))
        .or_else(|| peer.and_then(|peer| non_blank(Some(peer.tag.as_str()))))
        .unwrap_or_default()
        .trim_start_matches('@')
        .to_owned();
    let sender_logo = message
        .sender_logo
        .clone()
        .or_else(|| peer.map(|peer| peer.more.logo.clone()))
        .unwrap_or_default();
    let event = MessageCreatedEvent {
        message_id,
        chat_id: chat.id.clone(),
        topic_id: message.topic_id.clone(),
        sender_id: message.sender_id.clone(),
        content: message.text.clone(),
        content_type: message.content_type.raw_value().to_owned(),
        created_at: created_at_ms,
        timestamp: created_at_ms,
        media_items: message
            .media_items
            .first()
            .map(|item| MessageMediaInfo {
                media_type: item.kind.raw_value().to_owned(),
                file_url: non_blank(Some(item.url.as_str())).map(str::to_owned),
                thumbnail_url: item.thumbnail_url.clone(),
                ..MessageMediaInfo::default()
            })
            .into_iter()
            .collect(),
        sender_info: Some(MessageSenderInfo {
            id: message.sender_id.clone(),
            name: sender_name,
            tag: sender_tag,
            more: MessageSenderMore { logo: sender_logo, ..MessageSenderMore::default() },
            ..MessageSenderInfo::default()
        }),
        chat: Some(MessageChatInfo {
            id: chat.id.clone(),
            title: if direct { String::new() } else { chat.title.clone() },
            r#type: chat.r#type.wire().to_owned(),
            logo_url: chat.logo_url.clone(),
            ..MessageChatInfo::default()
        }),
        is_mentioned_you: Some(message.is_mentioned_you),
        ..MessageCreatedEvent::default()
    };
    message_created_push_for(&event, recipient, recipient_tag, language)
}

fn reaction_actor(payload: &ReactionEventPayload) -> Option<&ReactedByEntryEvent> {
    let actor_id = non_blank(payload.actor_id.as_deref());
    let emoji = non_blank(payload.reaction.as_deref());
    if let Some(actor_id) = actor_id {
        let mut by_actor = payload.reacted_by.iter().filter(|entry| entry.sender.id == actor_id);
        if let Some(exact) = emoji.and_then(|emoji| by_actor.clone().find(|entry| entry.reaction == emoji)) {
            return Some(exact);
        }
        if let Some(any) = by_actor.next() {
            return Some(any);
        }
    }
    payload.reacted_by.first()
}

fn reaction_display_name(sender: &ReactionSender) -> String {
    if !trimming_whitespaces_and_newlines(&sender.name).is_empty() {
        sender.name.clone()
    } else if !trimming_whitespaces_and_newlines(&sender.tag).is_empty() {
        format!("@{}", sender.tag)
    } else {
        String::new()
    }
}

fn reaction_avatar(sender: &ReactionSender) -> Option<String> {
    if !trimming_whitespaces_and_newlines(&sender.logo).is_empty() {
        return Some(sender.logo.clone());
    }
    sender.more.as_ref().map(|more| more.logo.clone()).filter(|logo| !trimming_whitespaces_and_newlines(logo).is_empty())
}

/// the loaded peer of the chat when it is the reactor
/// message service sends an empty reacted_by and notify
/// service fills name and photo from the actor profile
fn known_actor<'a>(peer: Option<&'a User>, actor_id: &str) -> Option<&'a User> {
    peer.filter(|peer| !actor_id.is_empty() && peer.id == actor_id)
}

pub fn unnamed_reaction_actor(payload: &ReactionEventPayload, peer: Option<&User>) -> Option<String> {
    let actor = reaction_actor(payload);
    if actor.is_some_and(|entry| !reaction_display_name(&entry.sender).is_empty()) {
        return None;
    }
    let actor_id = actor
        .map(|entry| entry.sender.id.clone())
        .or_else(|| payload.actor_id.clone())
        .filter(|id| !trimming_whitespaces_and_newlines(id).is_empty())?;
    if known_actor(peer, &actor_id).is_some_and(|user| !user_display_name(user).is_empty()) {
        return None;
    }
    Some(actor_id)
}

fn user_display_name(user: &User) -> String {
    let name = trimming_whitespaces_and_newlines(&user.name);
    if !name.is_empty() {
        return name.to_owned();
    }
    let tag = trimming_whitespaces_and_newlines(&user.tag);
    if tag.is_empty() {
        String::new()
    } else if tag.starts_with('@') {
        tag.to_owned()
    } else {
        format!("@{tag}")
    }
}

fn user_avatar(user: &User) -> Option<String> {
    non_blank(Some(user.more.logo.as_str())).map(str::to_owned)
}

fn reaction_emoji(reaction: &str) -> String {
    let reaction = trimming_whitespaces_and_newlines(reaction);
    if reaction.is_empty() || reaction.starts_with(MessageReaction::CUSTOM_EMOJI_PREFIX) {
        return String::new();
    }
    character_prefix(reaction, 1).to_owned()
}

fn collapse_spaces(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// push for an added reaction, only the message author gets it
/// not in a channel or for own reaction, author comes from the chat or message
/// peer is the loaded participant, fills reactor name and photo if frame has none
pub fn reaction_added_push(
    payload: &ReactionEventPayload,
    message_author: Option<&str>,
    peer: Option<&User>,
    recipient: &str,
    language: &str,
) -> Option<SocketPush> {
    let chat_type = chat_type_of(payload.chat.as_ref().map_or("", |chat| chat.chat_type.as_str()));
    if chat_type == ChatType::Channel || message_author != Some(recipient) {
        return None;
    }
    let actor = reaction_actor(payload);
    let actor_id = actor
        .map(|entry| entry.sender.id.clone())
        .or_else(|| payload.actor_id.clone())
        .unwrap_or_default();
    if actor_id == recipient {
        return None;
    }

    let group = is_channel_or_group(chat_type);
    let profile = known_actor(peer, &actor_id);
    let actor_name = actor
        .map(|entry| reaction_display_name(&entry.sender))
        .filter(|name| !name.is_empty())
        .or_else(|| profile.map(user_display_name).filter(|name| !name.is_empty()))
        .unwrap_or_else(|| fragment(language, keys::COMMON_ACTOR_SOMEONE));
    let raw_reaction = payload
        .reaction
        .clone()
        .or_else(|| actor.map(|entry| entry.reaction.clone()))
        .unwrap_or_default();
    let chat_title = non_blank(payload.chat.as_ref().map(|chat| chat.title.as_str()))
        .map(str::to_owned)
        .unwrap_or_else(|| fragment(language, keys::CHAT_TITLE_FALLBACK));
    let emoji = reaction_emoji(&raw_reaction);
    let params = [
        (ACTOR_PARAM, actor_name.as_str()),
        (EMOJI_PARAM, emoji.as_str()),
        (CHAT_TITLE_PARAM, chat_title.as_str()),
    ];
    let template = if group { keys::CHAT_REACTION_GROUP } else { keys::CHAT_REACTION_PRIVATE };
    let fallback_title = if group { &chat_title } else { &actor_name };
    let title = collapse_spaces(&rendered_part(language, template, TITLE_SUFFIX, &params, fallback_title));
    let body = collapse_spaces(&rendered_part(language, template, BODY_SUFFIX, &params, ""));
    let topic_id = non_blank(payload.topic_id.as_deref());

    let mut user_info = Map::new();
    insert_text(&mut user_info, push_route::TYPE, RisePushType::REACTION_ADDED);
    insert_text(&mut user_info, push::NOTIFICATION_SCOPE, NotificationScope::Reactions.as_str());
    insert_text(&mut user_info, push::RECIPIENT_USER_ID, recipient);
    insert_text(&mut user_info, push::CHAT_ID, payload.chat_id.as_str());
    insert_text(&mut user_info, push::MESSAGE_ID, payload.message_id.as_str());
    insert_text(&mut user_info, push::REACTION, raw_reaction.as_str());
    insert_text(&mut user_info, push::CHAT_TYPE, chat_type.internal_wire());
    insert_text(&mut user_info, push::CHAT_TITLE, chat_title.as_str());
    insert_text(&mut user_info, push::MESSAGE_PREVIEW, body.as_str());
    if let Some(topic_id) = topic_id {
        insert_text(&mut user_info, push::TOPIC_ID, topic_id);
    }
    if !actor_id.is_empty() {
        insert_text(&mut user_info, push::SENDER_ID, actor_id.as_str());
    }
    let avatar = if group {
        payload.chat.as_ref().map(|chat| chat.logo_url.clone())
    } else {
        actor.and_then(|entry| reaction_avatar(&entry.sender)).or_else(|| profile.and_then(user_avatar))
    };
    if let Some(avatar) = non_blank(avatar.as_deref()) {
        insert_text(&mut user_info, push::AVATAR_URL, avatar);
    }

    Some(SocketPush {
        title,
        body,
        tag: conversation_tag(&payload.chat_id, topic_id),
        dedup_key: format!(
            "{REACTION_KEY_PREFIX}{}:{actor_id}:{raw_reaction}",
            payload.message_id
        ),
        created_at_ms: if payload.timestamp > 0 { normalized_ms(payload.timestamp) } else { now_ms() },
        user_info,
    })
}

fn music_sub_flag(event_type: &str) -> Option<&'static str> {
    Some(match event_type {
        RisePushType::PLAYLIST_SESSION_JOINED => notify_key::MUSIC_SESSION_JOINED_ENABLED,
        RisePushType::PLAYLIST_SESSION_PARTICIPANT_LEFT => notify_key::MUSIC_SESSION_LEFT_ENABLED,
        RisePushType::PLAYLIST_SESSION_ENDED => notify_key::MUSIC_SESSION_ENDED_ENABLED,
        RisePushType::PLAYLIST_PERMISSION_REQUEST => notify_key::MUSIC_SESSION_PERMISSION_REQUESTS_ENABLED,
        RisePushType::PLAYLIST_SESSION_PERMISSIONS_UPDATED => notify_key::MUSIC_SESSION_PERMISSION_UPDATES_ENABLED,
        RisePushType::PLAYLIST_TRACK_ADDED => notify_key::MUSIC_PLAYLIST_TRACK_ADDED_ENABLED,
        RisePushType::PLAYLIST_TRACK_REMOVED => notify_key::MUSIC_PLAYLIST_TRACK_REMOVED_ENABLED,
        RisePushType::PLAYLIST_LISTENING_STARTED => notify_key::MUSIC_PLAYLIST_LISTENING_STARTED_ENABLED,
        RisePushType::ARTIST_TRACK_RELEASED | RisePushType::ARTIST_ALBUM_RELEASED => {
            notify_key::MUSIC_ARTIST_RELEASES_ENABLED
        }
        RisePushType::PLAYLIST_JOIN_REQUEST
        | RisePushType::PLAYLIST_JOIN_APPROVED
        | RisePushType::PLAYLIST_JOIN_REJECTED
        | RisePushType::PLAYLIST_MEMBER_LEFT
        | RisePushType::PLAYLIST_MEMBER_REMOVED
        | RisePushType::PLAYLIST_ACCESS_SUSPENDED => notify_key::MUSIC_PLAYLIST_ACCESS_ENABLED,
        _ => return None,
    })
}

fn created_row_enabled(settings: &NotificationSettings, notification_type: &str, event_type: &str) -> bool {
    if !type_enabled(settings, notification_type) {
        return false;
    }
    let is_music = settings_flag_for_type(notification_type).is_some_and(|flag| flag.wire == notify_key::MUSIC_ENABLED);
    match music_sub_flag(event_type).filter(|_| is_music) {
        Some(wire) => settings.flag(wire).unwrap_or(true),
        None => true,
    }
}

/// the frame names a feed row without text so the row
/// holds what the server rendered and the data map
/// none when theres nothing to show
pub fn notification_created_push(
    created: &NotificationCreatedData,
    item: &NotificationItem,
    recipient: &str,
) -> Option<SocketPush> {
    let title = trimming_whitespaces_and_newlines(&item.title);
    let body = trimming_whitespaces_and_newlines(&item.body);
    if title.is_empty() && body.is_empty() {
        return None;
    }
    let data: &BTreeMap<String, String> = &item.data;
    let mut user_info = Map::new();
    for (key, value) in data {
        insert_text(&mut user_info, key, value.as_str());
    }
    if !user_info.contains_key(push_route::TYPE) {
        insert_text(&mut user_info, push_route::TYPE, created.notification_type.as_str());
    }
    insert_text(&mut user_info, push::NOTIFICATION_ID, created.notification_id.to_string());
    insert_text(&mut user_info, push::RECIPIENT_USER_ID, recipient);
    insert_text(&mut user_info, push::TITLE, title);
    insert_text(&mut user_info, push::MESSAGE_PREVIEW, body);
    if !user_info.contains_key(push_route::ACTOR_ID)
        && let Some(actor_id) = item.source_user_id()
    {
        insert_text(&mut user_info, push_route::ACTOR_ID, actor_id);
    }
    if let Some(actor) = item.primary_actor() {
        let actor_name = trimming_whitespaces_and_newlines(&actor.actor_name);
        if !user_info.contains_key(push::ACTOR_NAME) && !actor_name.is_empty() {
            insert_text(&mut user_info, push::ACTOR_NAME, actor_name);
        }
        if !user_info.contains_key(push::ACTOR_AVATAR)
            && let Some(avatar) = actor
                .actor_avatar
                .as_deref()
                .map(trimming_whitespaces_and_newlines)
                .filter(|avatar| !avatar.is_empty())
        {
            insert_text(&mut user_info, push::ACTOR_AVATAR, avatar);
        }
    }

    let key = format!(
        "{NOTIFICATION_KEY_PREFIX}{}:{}:{}",
        created.notification_id,
        item.total_count(),
        item.created_at_ms
    );
    Some(SocketPush {
        title: title.to_owned(),
        body: body.to_owned(),
        tag: NotificationTag::unique(&key),
        dedup_key: key,
        created_at_ms: item.created_at_ms,
        user_info,
    })
}

/// account switches the server checks before it pushes
/// the socket reaches the app either way so the same check runs here
pub fn scope_enabled(settings: &NotificationSettings, scope: Option<&str>) -> bool {
    if !settings.all_enabled {
        return false;
    }
    match scope.and_then(NotificationScope::parse) {
        Some(NotificationScope::Messages | NotificationScope::Reactions) => settings.messages_enabled,
        Some(NotificationScope::Mentions | NotificationScope::Replies) => settings.mentions_enabled,
        Some(NotificationScope::Events) | None => true,
    }
}

fn now_ms() -> i64 {
    (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64
}

pub struct SocketNotificationSource {
    dedup: SocketNotificationDedup,
    floor_ms: i64,
    offline_since_ms: Option<i64>,
    catch_up_since_ms: Option<i64>,
    _catch_up: Option<Task<()>>,
    _activity: SystemActivityWatches,
    _wake: Task<()>,
    _link: Task<()>,
    _link_watches: [RiseSubscription; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SocketLink {
    Down,
    Up,
}

impl Global for SocketNotificationSource {}

impl SocketNotificationSource {
    pub fn install(cx: &mut App) {
        if cx.has_global::<Self>() {
            return;
        }
        let (signals, mut inbox) = mpsc::unbounded::<()>();
        let activity = watch(move |event| {
            if event == SystemActivityEvent::DidWake {
                let _ = signals.unbounded_send(());
            }
        });
        let wake = cx.spawn(async move |cx: &mut AsyncApp| {
            while inbox.next().await.is_some() {
                cx.update(|cx| cx.global_mut::<Self>().floor_ms = now_ms());
            }
        });
        let (links, mut link_inbox) = mpsc::unbounded::<SocketLink>();
        let down = links.clone();
        let link_watches = [
            RiseAppEvents::default_center().observe(SAI_WS_DID_DISCONNECT, move |_| {
                let _ = down.unbounded_send(SocketLink::Down);
            }),
            RiseAppEvents::default_center().observe(SAI_WS_DID_CONNECT, move |_| {
                let _ = links.unbounded_send(SocketLink::Up);
            }),
        ];
        let link = cx.spawn(async move |cx: &mut AsyncApp| {
            while let Some(link) = link_inbox.next().await {
                cx.update(|cx| Self::link_changed(link, cx));
            }
        });
        cx.set_global(Self {
            dedup: SocketNotificationDedup::default(),
            floor_ms: now_ms(),
            offline_since_ms: None,
            catch_up_since_ms: None,
            _catch_up: None,
            _activity: activity,
            _wake: wake,
            _link: link,
            _link_watches: link_watches,
        });
        SaiWsEventRegistry::set_frame_observer(Rc::new(Self::observe_frame), cx);
    }

    fn link_changed(link: SocketLink, cx: &mut App) {
        let source = cx.global_mut::<Self>();
        match link {
            SocketLink::Down => {
                source.offline_since_ms.get_or_insert_with(now_ms);
            }
            SocketLink::Up => {
                let Some(since) = source.offline_since_ms.take() else {
                    return;
                };
                let since = source.catch_up_since_ms.map_or(since, |running| running.min(since));
                source.catch_up_since_ms = Some(since);
                let task = cx.spawn(async move |cx: &mut AsyncApp| {
                    for delay in CATCH_UP_DELAYS {
                        cx.background_executor().timer(delay).await;
                        cx.update(|cx| Self::catch_up(since, cx));
                    }
                    cx.update(|cx| cx.global_mut::<Self>().catch_up_since_ms = None);
                });
                cx.global_mut::<Self>()._catch_up = Some(task);
            }
        }
    }

    /// toasts what the chat list gained while the socket was down
    /// dedup keeps one toast per message across steps and live frames after reconnect
    fn catch_up(since_ms: i64, cx: &mut App) {
        let Some(recipient) = Self::recipient(cx) else {
            return;
        };
        let language = LanguageStore::shared(cx).code().to_owned();
        let tag = Self::recipient_tag(&recipient, cx);
        let floor_ms = (since_ms - MISSED_LEAD_MS).max(cx.global::<Self>().floor_ms);
        let mut pushes: Vec<SocketPush> = ChatServicesStore::shared(cx)
            .read(cx)
            .current_chat_infos(cx)
            .iter()
            .filter_map(|chat| missed_message_push(chat, &recipient, tag.as_deref(), floor_ms, &language))
            .collect();
        pushes.sort_by_key(|push| push.created_at_ms);
        tracing::warn!(target: "riseonly::X2081", "catch up since={since_ms} missed={}", pushes.len());
        for push in pushes {
            Self::route(push, cx);
        }
    }

    fn recipient_tag(recipient: &str, cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .filter(|profile| profile.id == recipient)
            .map(|profile| profile.tag.trim_start_matches('@').to_owned())
            .filter(|tag| !tag.is_empty())
    }

    pub fn install_last_window_policy(cx: &mut App) {
        cx.on_window_closed(|cx, _| {
            if !cx.windows().is_empty() {
                return;
            }
            let keeps = TrayController::closing_keeps_running_in_tray(cx);
            if last_window_close(HostOs::current(), keeps) == LastWindowClose::Quit {
                request_quit(cx);
            }
        })
        .detach();
    }

    /// on_window_should_close handler, false keeps the window for the tray icon
    pub fn close_or_keep(window: &mut Window, cx: &mut App) -> bool {
        if cx.windows().len() > 1 {
            return true;
        }
        let keeps = TrayController::closing_keeps_running_in_tray(cx);
        if last_window_close(HostOs::current(), keeps) == LastWindowClose::HideToTray {
            if !hide_window(&*window).is_supported() {
                window.minimize_window();
            }
            return false;
        }
        true
    }

    /// the painted close button removes the window without asking on_window_should_close
    pub fn close_button(window: &mut Window, cx: &mut App) {
        if Self::close_or_keep(window, cx) {
            window.remove_window();
        }
    }

    pub fn did_reopen(cx: &mut App) {
        if WindowPresenter::window_count(cx) == 0 {
            WindowPresenter::open_shell(cx);
        }
    }

    fn recipient(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .active_account_id()
            .map(str::to_owned)
            .or_else(AuthLocalStorage::active_account_id)
    }

    fn observe_frame(event_type: &str, json: &Map<String, Value>, cx: &mut App) {
        let names = [
            rise_events::name::MESSAGE_CREATED,
            rise_events::name::REACTION_ADDED,
            rise_events::name::NOTIFICATION_CREATED,
        ];
        if !names.contains(&event_type) {
            return;
        }
        let Some(payload) = SaiWsEventRegistry::event_payload(json) else {
            return;
        };
        let Some(recipient) = Self::recipient(cx) else {
            return;
        };
        let language = LanguageStore::shared(cx).code().to_owned();
        let push = if event_type == rise_events::name::MESSAGE_CREATED {
            let Ok(event) = serde_json::from_value::<MessageCreatedEvent>(payload) else {
                return;
            };
            let tag = Self::recipient_tag(&recipient, cx);
            let mut created = message_created_push_for(&event, &recipient, tag.as_deref(), &language);
            if let Some(created) = created.as_mut() {
                localize_system_body(&event, created, cx);
            }
            created
        } else if event_type == rise_events::name::REACTION_ADDED {
            let event = match serde_json::from_value::<ReactionEventPayload>(payload) {
                Ok(event) => event,
                Err(error) => {
                    tracing::warn!(target: "riseonly::X752", "reaction frame unreadable: {error}");
                    return;
                }
            };
            tracing::warn!(
                target: "riseonly::X752",
                "reaction frame message={} chat={} actor={:?} reacted_by={} ts={}",
                event.message_id,
                event.chat_id,
                event.actor_id,
                event.reacted_by.len(),
                event.timestamp
            );
            let author = MessageServicesStore::shared(cx)
                .update(cx, |store, cx| store.message(&event.message_id, &event.chat_id, cx))
                .map(|message| message.sender_id);
            let Some(author) = author else {
                tracing::warn!(target: "riseonly::X752", "reaction message={} not loaded, reading author", event.message_id);
                Self::read_reaction_author(event, recipient, language, cx);
                return;
            };
            Self::compose_reaction(event, author, recipient, language, cx);
            return;
        } else {
            let created = match serde_json::from_value::<NotificationCreatedData>(payload) {
                Ok(created) => created,
                Err(error) => {
                    tracing::warn!(target: "riseonly::X753", "created frame unreadable: {error}");
                    return;
                }
            };
            tracing::warn!(target: "riseonly::X753", "created frame id={}", created.notification_id);
            Self::read_created_row(created, recipient, cx);
            return;
        };
        if let Some(push) = push {
            Self::route(push, cx);
        }
    }

    /// a group frame has an empty reacted_by and no loaded peer names the reactor
    /// so its taken from loaded chat members or else one profile read
    fn compose_reaction(event: ReactionEventPayload, author: String, recipient: String, language: String, cx: &mut App) {
        if author != recipient {
            Self::reaction_push(&event, &author, None, &recipient, &language);
            return;
        }
        let chats = ChatServicesStore::shared(cx);
        let participant = chats.read(cx).chat_info(&event.chat_id, cx).map(|chat| chat.participant);
        let Some(actor_id) = unnamed_reaction_actor(&event, participant.as_ref()) else {
            if let Some(push) = Self::reaction_push(&event, &author, participant.as_ref(), &recipient, &language) {
                Self::route(push, cx);
            }
            return;
        };
        let member = chats
            .update(cx, |store, cx| store.member(&event.chat_id, &actor_id, cx))
            .map(|member| member.to_user())
            .filter(|user| !user_display_name(user).is_empty());
        if let Some(member) = member {
            if let Some(push) = Self::reaction_push(&event, &author, Some(&member), &recipient, &language) {
                Self::route(push, cx);
            }
            return;
        }
        tracing::warn!(target: "riseonly::X759", "reaction message={} reading actor profile", event.message_id);
        let event = Rc::new(event);
        let (success_event, success_recipient, success_language) = (event.clone(), recipient.clone(), language.clone());
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let profile = payload
                .as_deref()
                .and_then(|payload| {
                    payload
                        .downcast_ref::<UserGetProfileByTagGatewayDTO>()
                        .cloned()
                        .or_else(|| SaiJSONCodec::decode::<UserGetProfileByTagGatewayDTO>(payload.downcast_ref::<Value>()))
                })
                .and_then(|response| response.as_profile_model());
            let actor = profile.map(|profile| User {
                id: profile.id,
                name: profile.name,
                tag: profile.tag,
                more: profile.more,
                ..User::default()
            });
            tracing::warn!(target: "riseonly::X759", "reaction actor profile read={}", actor.is_some());
            if Self::recipient(cx).as_deref() != Some(success_recipient.as_str()) {
                return;
            }
            if let Some(push) =
                Self::reaction_push(&success_event, &success_recipient, actor.as_ref(), &success_recipient, &success_language)
            {
                Self::route(push, cx);
            }
        });
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            tracing::warn!(target: "riseonly::X759", "reaction actor profile failed: {error}");
            if Self::recipient(cx).as_deref() != Some(recipient.as_str()) {
                return;
            }
            if let Some(push) = Self::reaction_push(&event, &recipient, None, &recipient, &language) {
                Self::route(push, cx);
            }
        });
        UserActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_profile_by_id_action(&actor_id, Some(on_success), Some(on_error), cx);
        });
    }

    fn reaction_push(
        event: &ReactionEventPayload,
        author: &str,
        peer: Option<&User>,
        recipient: &str,
        language: &str,
    ) -> Option<SocketPush> {
        let push = reaction_added_push(event, Some(author), peer, recipient, language);
        tracing::warn!(
            target: "riseonly::X752",
            "reaction compose message={} author_is_me={} peer={} push={}",
            event.message_id,
            author == recipient,
            peer.is_some(),
            push.as_ref().map_or("none".to_owned(), |push| format!("ts={} title_len={}", push.created_at_ms, push.title.len()))
        );
        push
    }

    fn read_reaction_author(event: ReactionEventPayload, recipient: String, language: String, cx: &mut App) {
        let chat_type = chat_type_of(event.chat.as_ref().map_or("", |chat| chat.chat_type.as_str()));
        if chat_type == ChatType::Channel || event.message_id.is_empty() {
            return;
        }
        cx.spawn(async move |cx: &mut AsyncApp| {
            let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                return;
            };
            if domain.account_id != recipient {
                tracing::warn!(target: "riseonly::X752", "reaction read skipped: domain account differs");
                return;
            }
            let transport_owner = format!("push.reaction_read.{}", Uuid::new_v4().uuid_string());
            cx.update(|cx| SaiWs::set_transport_hold(&transport_owner, true, cx));
            let response = domain.messages.remote.message_by_id_surviving_suspension(event.message_id.clone()).await;
            cx.update(|cx| SaiWs::set_transport_hold(&transport_owner, false, cx));
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    tracing::warn!(target: "riseonly::X752", "reaction message={} read failed: {error}", event.message_id);
                    return;
                }
            };
            let author = response
                .object_value()
                .and_then(|root| root.get(message_key::MESSAGE))
                .and_then(RiseJSONValue::object_value)
                .and_then(|message| message.get(message_key::SENDER_ID))
                .and_then(RiseJSONValue::string_value)
                .map(str::to_owned);
            let Some(author) = author else {
                tracing::warn!(target: "riseonly::X752", "reaction message={} read has no author", event.message_id);
                return;
            };
            tracing::warn!(target: "riseonly::X752", "reaction message={} author read", event.message_id);
            cx.update(|cx| {
                if Self::recipient(cx).as_deref() != Some(recipient.as_str()) {
                    return;
                }
                Self::compose_reaction(event, author, recipient, language, cx);
            });
        })
        .detach();
    }

    fn read_created_row(created: NotificationCreatedData, recipient: String, cx: &mut App) {
        let received_ms = now_ms();
        let domain = RiseNotifyActionExecutor::require_domain(cx);
        cx.spawn(async move |cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                tracing::warn!(target: "riseonly::X753", "created id={} no notify domain", created.notification_id);
                return;
            };
            let status = authorization_status().await;
            tracing::warn!(target: "riseonly::X753", "created id={} type={} status={status:?}", created.notification_id, created.notification_type);
            if matches!(
                status,
                NotificationAuthorizationStatus::Denied | NotificationAuthorizationStatus::NotDetermined
            ) {
                return;
            }
            let transport_owner = format!("push.created_read.{}", Uuid::new_v4().uuid_string());
            cx.update(|cx| SaiWs::set_transport_hold(&transport_owner, true, cx));
            let mut item = domain.created_notification(created.notification_id).await;
            for delay in CREATED_ROW_RETRY_DELAYS {
                if item.is_some() {
                    break;
                }
                tracing::warn!(target: "riseonly::X753", "created id={} row not on the first page, retry in {delay:?}", created.notification_id);
                cx.background_executor().timer(delay).await;
                item = domain.created_notification(created.notification_id).await;
            }
            cx.update(|cx| SaiWs::set_transport_hold(&transport_owner, false, cx));
            let Some(item) = item else {
                tracing::warn!(target: "riseonly::X753", "created id={} row not on the first page", created.notification_id);
                return;
            };
            cx.update(|cx| {
                let settings = NotifyServicesStore::shared(cx).read(cx).notification_settings(cx);
                if !created_row_enabled(&settings, &created.notification_type, &item.event_type()) {
                    tracing::warn!(target: "riseonly::X2138", "created id={} type={} event={} disabled", created.notification_id, created.notification_type, item.event_type());
                    return;
                }
                let push = notification_created_push(&created, &item, &recipient);
                tracing::warn!(target: "riseonly::X753", "created id={} push={} row_ms={} count={}", created.notification_id, push.is_some(), item.created_at_ms, item.total_count());
                if let Some(mut push) = push {
                    push.created_at_ms = push.created_at_ms.max(received_ms);
                    Self::route(push, cx);
                }
            });
        })
        .detach();
    }

    fn admits(push: &SocketPush, cx: &mut App) -> bool {
        let source = cx.global_mut::<Self>();
        push.created_at_ms >= source.floor_ms && source.dedup.admit(&push.dedup_key)
    }

    fn route(push: SocketPush, cx: &mut App) {
        let is_reaction = push.dedup_key.starts_with(REACTION_KEY_PREFIX);
        if !Self::admits(&push, cx) {
            if is_reaction {
                let floor_ms = cx.global::<Self>().floor_ms;
                tracing::warn!(target: "riseonly::X752", "reaction route refused: ts={} floor={floor_ms} or dup", push.created_at_ms);
            }
            return;
        }
        let SocketPush { title, body, tag, user_info, .. } = push;
        if !PushNotificationManager::should_handle_notification(&user_info, cx) {
            if is_reaction {
                tracing::warn!(target: "riseonly::X752", "reaction route refused: should_handle_notification");
            }
            return;
        }
        let settings = NotifyServicesStore::shared(cx).read(cx).notification_settings(cx);
        let scope = user_info.get(push::NOTIFICATION_SCOPE).and_then(Value::as_str);
        if !scope_enabled(&settings, scope) {
            if is_reaction {
                tracing::warn!(target: "riseonly::X752", "reaction route refused: scope {scope:?} off");
            }
            return;
        }
        if is_reaction {
            tracing::warn!(target: "riseonly::X752", "reaction route presents");
        }
        let stealth = DevDrive::is_stealth(cx);
        if stealth || RiseAppLifecycle::scene_phase(cx) == ScenePhase::Active {
            PushNotificationManager::shared(cx).update(cx, |manager, cx| {
                manager.will_present(&user_info, &title, &body, cx);
            });
            if !stealth {
                return;
            }
        }
        if !PushNotificationManager::should_present_for_chat_notification_settings(&user_info, cx) {
            return;
        }
        NotificationService::did_receive(
            NotificationRequest { tag, title, body, user_info },
            |content, cx| {
                let _ = DesktopNotificationDelivery::deliver(
                    content.notification,
                    content.category.as_deref(),
                    content.user_info,
                    cx,
                );
            },
            cx,
        );
    }
}

#[cfg(test)]
#[path = "socket_notification_source_tests.rs"]
mod tests;
