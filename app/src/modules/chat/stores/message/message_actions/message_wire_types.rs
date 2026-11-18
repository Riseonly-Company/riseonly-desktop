use std::any::type_name;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;

use gpui::App;
use rise_bot_api::reply_markup as reply_markup_key;
use rise_canon::rise_blob::call_sheet::Sheet as CallSheet;
use rise_canon::rise_key::bot as bot_key;
use rise_canon::rise_key::chat_action as chat_action_key;
use rise_canon::rise_key::events as events_key;
use rise_canon::rise_key::message as key;
use rise_canon::rise_key::payload as payload_key;
use rise_canon::rise_key::recent_commenter as recent_commenter_key;
use rise_canon::rise_key::shared_post as shared_post_key;
use rise_canon::rise_key::stickers as stickers_key;
use rise_canon::rise_key::story_reply as story_reply_key;
use rise_canon::rise_proto::message as proto;
use rise_canon::rise_request::message;
use rise_canon::rise_view::message as view;
use rise_canon::{RiseUploadFormat, RiseWireKey, RiseWireObject, rise_decode};
use rise_chat::{
    LinkPreviewSize as ChatLinkPreviewSize, MessageEntityField, MessageEntityType, StickerKind as ChatStickerKind,
};
use rise_core::foundation::foundation_character::{double_from_string, int_from_string};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::RiseJSONValue;
use rise_errors::ErrorParam;
use rise_i18n::tr;
use serde::de::{DeserializeOwned, Error as _};
use serde::ser::{Error as _, SerializeMap};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_errors::RiseFailure;
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::chat::components::chat::bubble::message_sticker_view::{
    ChatAnimatedMediaPlaybackKind, ChatAnimatedMediaSourcePolicy,
};
use crate::modules::chat::components::chat::composer::chat_link_preview_draft::ChatLinkPreviewURL;
use crate::modules::chat::components::chat::message_system_content_parser::MessageSystemContentParser;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageBotKeyboardButton, MessageBotReplyMarkup, MessageBotWebAppData, MessageCallInfo,
    MessageCallMediaMode, MessageCallTerminalState, MessageCommenterPreview, MessageContentType,
    MessageDeliveryStatus, MessageForwardInfo, MessageLinkPreview, MessageLinkPreviewSize, MessageMediaItem,
    MessageMediaKind, MessageMediaViewLimit, MessageNormalizedEntities, MessageReaction, MessageReplyTo, MessageSharedPost,
    MessageSharedPostAuthor, MessageStickerKind, MessageStoryReply, MessageStoryReplyAuthor, MessageSystemGiftItem,
    MessageSystemInfo, MessageSystemKind, MessageSystemWallpaperPreview, ReactionReactor, RemoteMediaURL,
};
use crate::modules::post::stores::post_actions::post_types::AuthorInfoMore;
use crate::modules::post::stores::post_actions::post_wire_types::WireID;

pub type GetMessagesBody = message::GetMessages;

pub type GetMessagesAroundBody = message::GetMessagesAround;

pub type GetChatCalendarBody = message::GetChatCalendar;

#[derive(Clone, Debug, Default)]
pub struct RiseStoredOutgoingMedia {
    pub media_type: String,
    pub file_id: Option<String>,
    pub file_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub duration: Option<i64>,
    pub bitrate: Option<i64>,
    pub fps: Option<f32>,
    pub codec: Option<String>,
    pub waveform: Option<String>,
    pub sticker_id: Option<String>,
    pub sticker_type: Option<String>,
    pub pack_id: Option<String>,
    pub associated_emojis: Option<Vec<String>>,
    pub has_spoiler: Option<bool>,
    pub view_mode: Option<String>,
    pub view_seconds: Option<i64>,
}

impl<'de> Deserialize<'de> for RiseStoredOutgoingMedia {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                media_type: c.rise_required::<String>(key::MEDIA_TYPE)?,
                file_id: c.rise::<String>(key::FILE_ID)?,
                file_url: c.rise::<String>(key::FILE_URL)?,
                thumbnail_url: c.rise::<String>(key::THUMBNAIL_URL)?,
                width: c.rise::<i64>(key::WIDTH)?,
                height: c.rise::<i64>(key::HEIGHT)?,
                file_name: c.rise::<String>(key::FILE_NAME)?,
                file_size: c.rise::<i64>(key::FILE_SIZE)?,
                mime_type: c.rise::<String>(key::MIME_TYPE)?,
                duration: c.rise::<i64>(key::DURATION)?,
                bitrate: c.rise::<i64>(key::BITRATE)?,
                fps: c.rise::<f32>(key::FPS)?,
                codec: c.rise::<String>(key::CODEC)?,
                waveform: c.rise::<String>(key::WAVEFORM)?,
                sticker_id: c.rise::<String>(key::STICKER_ID)?,
                sticker_type: c.rise::<String>(key::STICKER_TYPE)?,
                pack_id: c.rise::<String>(key::PACK_ID)?,
                associated_emojis: c.rise::<Vec<String>>(key::ASSOCIATED_EMOJIS)?,
                has_spoiler: c.rise::<bool>(key::HAS_SPOILER)?,
                view_mode: c.rise::<String>(key::VIEW_MODE)?,
                view_seconds: c.rise::<i64>(key::VIEW_SECONDS)?,
            })
        })
    }
}

impl Serialize for RiseStoredOutgoingMedia {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::MEDIA_TYPE, &self.media_type)?;
        if let Some(value) = &self.file_id {
            container.serialize_entry(key::FILE_ID, value)?;
        }
        if let Some(value) = &self.file_url {
            container.serialize_entry(key::FILE_URL, value)?;
        }
        if let Some(value) = &self.thumbnail_url {
            container.serialize_entry(key::THUMBNAIL_URL, value)?;
        }
        if let Some(value) = &self.width {
            container.serialize_entry(key::WIDTH, value)?;
        }
        if let Some(value) = &self.height {
            container.serialize_entry(key::HEIGHT, value)?;
        }
        if let Some(value) = &self.file_name {
            container.serialize_entry(key::FILE_NAME, value)?;
        }
        if let Some(value) = &self.file_size {
            container.serialize_entry(key::FILE_SIZE, value)?;
        }
        if let Some(value) = &self.mime_type {
            container.serialize_entry(key::MIME_TYPE, value)?;
        }
        if let Some(value) = &self.duration {
            container.serialize_entry(key::DURATION, value)?;
        }
        if let Some(value) = &self.bitrate {
            container.serialize_entry(key::BITRATE, value)?;
        }
        if let Some(value) = &self.fps {
            container.serialize_entry(key::FPS, value)?;
        }
        if let Some(value) = &self.codec {
            container.serialize_entry(key::CODEC, value)?;
        }
        if let Some(value) = &self.waveform {
            container.serialize_entry(key::WAVEFORM, value)?;
        }
        if let Some(value) = &self.sticker_id {
            container.serialize_entry(key::STICKER_ID, value)?;
        }
        if let Some(value) = &self.sticker_type {
            container.serialize_entry(key::STICKER_TYPE, value)?;
        }
        if let Some(value) = &self.pack_id {
            container.serialize_entry(key::PACK_ID, value)?;
        }
        if let Some(value) = &self.associated_emojis {
            container.serialize_entry(key::ASSOCIATED_EMOJIS, value)?;
        }
        if self.has_spoiler == Some(true) {
            container.serialize_entry(key::HAS_SPOILER, &true)?;
        }
        if let Some(value) = &self.view_mode {
            container.serialize_entry(key::VIEW_MODE, value)?;
        }
        if let Some(value) = &self.view_seconds {
            container.serialize_entry(key::VIEW_SECONDS, value)?;
        }
        container.end()
    }
}

impl RiseStoredOutgoingMedia {
    pub fn new(media_type: String) -> Self {
        Self { media_type, ..Self::default() }
    }

    pub fn waveform_wire_string(peaks: &[f32]) -> Option<String> {
        if peaks.is_empty() {
            return None;
        }
        Some(peaks.iter().map(|peak| format!("{:.4}", peak.max(0.0).min(1.0))).collect::<Vec<_>>().join(","))
    }
}

#[derive(Clone, Debug, Default)]
pub struct RiseStoredOutgoingMessage {
    pub chat_id: Option<String>,
    pub topic_id: Option<String>,
    pub user_chat_id: Option<String>,
    pub content: String,
    pub original_content: String,
    pub content_type: String,
    pub reply_to_id: Option<String>,
    pub forward_from_message_id: Option<String>,
    pub forward_from_chat_id: Option<String>,
    pub media_group_id: Option<String>,
    pub entities: Option<String>,
    pub caption: Option<String>,
    pub media_items: Option<Vec<RiseStoredOutgoingMedia>>,
    pub publish_to_shorts_feed: Option<bool>,
    pub post_id: Option<i64>,
    pub story_id: Option<i64>,
    pub shared_post_info: Option<String>,
    pub story_reply_info: Option<String>,
    pub reply_markup: Option<String>,
    pub web_app_data: Option<String>,
    pub document: Option<String>,
    pub temp_id: Option<String>,
    pub link_preview_type: Option<String>,
    pub link_preview: Option<MessageLinkMetadataWireDTO>,
}

impl<'de> Deserialize<'de> for RiseStoredOutgoingMessage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                chat_id: c.rise::<String>(key::CHAT_ID)?,
                topic_id: c.rise::<String>(key::TOPIC_ID)?,
                user_chat_id: c.rise::<String>(key::USER_CHAT_ID)?,
                content: c.rise_required::<String>(key::CONTENT)?,
                original_content: c.rise_required::<String>(key::ORIGINAL_CONTENT)?,
                content_type: c.rise_required::<String>(key::CONTENT_TYPE)?,
                reply_to_id: c.rise::<String>(key::REPLY_TO_ID)?,
                forward_from_message_id: c.rise::<String>(key::FORWARD_FROM_MESSAGE_ID)?,
                forward_from_chat_id: c.rise::<String>(key::FORWARD_FROM_CHAT_ID)?,
                media_group_id: c.rise::<String>(key::MEDIA_GROUP_ID)?,
                entities: c.rise::<String>(key::ENTITIES)?,
                caption: c.rise::<String>(key::CAPTION)?,
                media_items: c.rise::<Vec<RiseStoredOutgoingMedia>>(key::MEDIA_ITEMS)?,
                publish_to_shorts_feed: c.rise::<bool>(key::PUBLISH_TO_SHORTS_FEED)?,
                post_id: c.rise::<i64>(key::POST_ID)?,
                story_id: c.rise::<i64>(key::STORY_ID)?,
                shared_post_info: c.rise::<String>(key::SHARED_POST_INFO)?,
                story_reply_info: c.rise::<String>(key::STORY_REPLY_INFO)?,
                reply_markup: c.rise::<String>(key::REPLY_MARKUP)?,
                web_app_data: c.rise::<String>(key::WEB_APP_DATA)?,
                document: c.rise::<String>(key::DOCUMENT)?,
                temp_id: c.rise::<String>(key::TEMP_ID)?,
                link_preview_type: c.rise::<String>(key::LINK_PREVIEW_TYPE)?,
                link_preview: c.rise::<MessageLinkMetadataWireDTO>(key::LINK_PREVIEW)?,
            })
        })
    }
}

impl Serialize for RiseStoredOutgoingMessage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.chat_id {
            container.serialize_entry(key::CHAT_ID, value)?;
        }
        if let Some(value) = &self.topic_id {
            container.serialize_entry(key::TOPIC_ID, value)?;
        }
        if let Some(value) = &self.user_chat_id {
            container.serialize_entry(key::USER_CHAT_ID, value)?;
        }
        container.serialize_entry(key::CONTENT, &self.content)?;
        container.serialize_entry(key::ORIGINAL_CONTENT, &self.original_content)?;
        container.serialize_entry(key::CONTENT_TYPE, &self.content_type)?;
        if let Some(value) = &self.reply_to_id {
            container.serialize_entry(key::REPLY_TO_ID, value)?;
        }
        if let Some(value) = &self.forward_from_message_id {
            container.serialize_entry(key::FORWARD_FROM_MESSAGE_ID, value)?;
        }
        if let Some(value) = &self.forward_from_chat_id {
            container.serialize_entry(key::FORWARD_FROM_CHAT_ID, value)?;
        }
        if let Some(value) = &self.media_group_id {
            container.serialize_entry(key::MEDIA_GROUP_ID, value)?;
        }
        if let Some(value) = &self.entities {
            container.serialize_entry(key::ENTITIES, value)?;
        }
        if let Some(value) = &self.caption {
            container.serialize_entry(key::CAPTION, value)?;
        }
        if let Some(value) = &self.media_items {
            container.serialize_entry(key::MEDIA_ITEMS, value)?;
        }
        if let Some(value) = &self.publish_to_shorts_feed {
            container.serialize_entry(key::PUBLISH_TO_SHORTS_FEED, value)?;
        }
        if let Some(value) = &self.post_id {
            container.serialize_entry(key::POST_ID, value)?;
        }
        if let Some(value) = &self.story_id {
            container.serialize_entry(key::STORY_ID, value)?;
        }
        if let Some(value) = &self.shared_post_info {
            container.serialize_entry(key::SHARED_POST_INFO, value)?;
        }
        if let Some(value) = &self.story_reply_info {
            container.serialize_entry(key::STORY_REPLY_INFO, value)?;
        }
        if let Some(value) = &self.reply_markup {
            container.serialize_entry(key::REPLY_MARKUP, value)?;
        }
        if let Some(value) = &self.web_app_data {
            container.serialize_entry(key::WEB_APP_DATA, value)?;
        }
        if let Some(value) = &self.document {
            container.serialize_entry(key::DOCUMENT, value)?;
        }
        if let Some(value) = &self.temp_id {
            container.serialize_entry(key::TEMP_ID, value)?;
        }
        if let Some(value) = &self.link_preview_type {
            container.serialize_entry(key::LINK_PREVIEW_TYPE, value)?;
        }
        if let Some(value) = &self.link_preview {
            container.serialize_entry(key::LINK_PREVIEW, value)?;
        }
        container.end()
    }
}

pub type EditMessageBody = message::EditMessage;

pub type DeleteMessagesBody = message::DeleteMessages;

pub type ForwardMessageDestinationBody = message::forward_messages::DestinationsElement;

pub type ForwardMessagesBody = message::ForwardMessages;

pub type MarkChatAsReadBody = message::MarkChatAsRead;

pub type MarkMediaPlayedBody = message::MarkMediaAsPlayed;

pub type MarkMessageViewedBody = message::MarkMessageAsViewed;

pub type MessageTypingBody = message::MessageTyping;

pub type PinMessageBody = message::PinMessage;

pub type UnpinAllMessagesBody = message::UnpinAllMessages;

pub type DeleteAllMessagesFromUserBody = message::DeleteAllMessagesFromUser;

#[derive(Clone, Debug)]
pub struct MessageMediaInfoWireDTO {
    pub media_type: Option<String>,
    pub file_id: Option<String>,
    pub file_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub bitrate: Option<i64>,
    pub fps: Option<f64>,
    pub codec: Option<String>,
    pub waveform: Option<String>,
    pub sticker_id: Option<String>,
    pub pack_id: Option<String>,
    pub sticker_type: Option<String>,
    pub associated_emojis: Option<Vec<String>>,
    pub is_uploading: Option<bool>,
    pub upload_progress: Option<f64>,
    pub uploaded_bytes: Option<i64>,
    pub total_bytes: Option<i64>,
    pub is_compressing: Option<bool>,
    pub compression_progress: Option<f64>,
    pub is_cancelled: Option<bool>,
    pub local_preview_url: Option<String>,
    pub local_slot_index: Option<i64>,
    pub has_spoiler: Option<bool>,
    pub view_mode: Option<String>,
    pub view_seconds: Option<i64>,
    pub view_expires_at: Option<i64>,
    pub view_state: Option<String>,
}

impl<'de> Deserialize<'de> for MessageMediaInfoWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            let mut bad = Vec::<&'static str>::new();
            let item = Self {
                media_type: lenient_media_field::<String>(c, key::MEDIA_TYPE, &mut bad),
                file_id: lenient_media_field::<String>(c, key::FILE_ID, &mut bad),
                file_url: lenient_media_field::<String>(c, key::FILE_URL, &mut bad),
                thumbnail_url: lenient_media_field::<String>(c, key::THUMBNAIL_URL, &mut bad),
                duration: lenient_media_field::<f64>(c, key::DURATION, &mut bad),
                width: lenient_media_int(c, key::WIDTH, &mut bad),
                height: lenient_media_int(c, key::HEIGHT, &mut bad),
                file_name: lenient_media_field::<String>(c, key::FILE_NAME, &mut bad),
                file_size: lenient_media_int(c, key::FILE_SIZE, &mut bad),
                mime_type: lenient_media_field::<String>(c, key::MIME_TYPE, &mut bad),
                bitrate: lenient_media_int(c, key::BITRATE, &mut bad),
                fps: lenient_media_field::<f64>(c, key::FPS, &mut bad),
                codec: lenient_media_field::<String>(c, key::CODEC, &mut bad),
                waveform: lenient_media_field::<String>(c, key::WAVEFORM, &mut bad),
                sticker_id: lenient_media_field::<String>(c, key::STICKER_ID, &mut bad),
                pack_id: lenient_media_field::<String>(c, key::PACK_ID, &mut bad),
                sticker_type: lenient_media_field::<String>(c, key::STICKER_TYPE, &mut bad),
                associated_emojis: lenient_media_field::<Vec<String>>(c, key::ASSOCIATED_EMOJIS, &mut bad),
                is_uploading: lenient_media_field::<bool>(c, RiseLocalKey::IS_UPLOADING, &mut bad),
                upload_progress: lenient_media_field::<f64>(c, RiseLocalKey::UPLOAD_PROGRESS, &mut bad),
                uploaded_bytes: lenient_media_int(c, RiseLocalKey::UPLOADED_BYTES, &mut bad),
                total_bytes: lenient_media_int(c, RiseLocalKey::TOTAL_BYTES, &mut bad),
                is_compressing: lenient_media_field::<bool>(c, RiseLocalKey::IS_COMPRESSING, &mut bad),
                compression_progress: lenient_media_field::<f64>(c, RiseLocalKey::COMPRESSION_PROGRESS, &mut bad),
                is_cancelled: lenient_media_field::<bool>(c, RiseLocalKey::IS_CANCELLED, &mut bad),
                local_preview_url: lenient_media_field::<String>(c, RiseLocalKey::LOCAL_PREVIEW_URL, &mut bad),
                local_slot_index: lenient_media_int(c, RiseLocalKey::LOCAL_SLOT_INDEX, &mut bad),
                has_spoiler: lenient_media_field::<bool>(c, key::HAS_SPOILER, &mut bad),
                view_mode: lenient_media_field::<String>(c, key::VIEW_MODE, &mut bad),
                view_seconds: lenient_media_int(c, key::VIEW_SECONDS, &mut bad),
                view_expires_at: lenient_media_int(c, key::VIEW_EXPIRES_AT, &mut bad),
                view_state: lenient_media_field::<String>(c, key::VIEW_STATE, &mut bad),
            };
            if !bad.is_empty() {
                tracing::warn!(target: "riseonly::X1946", keys = ?bad, "media item fields dropped on decode");
            }
            Ok(item)
        })
    }
}

fn lenient_media_field<T: DeserializeOwned>(c: &RiseWireObject, key: &'static str, bad: &mut Vec<&'static str>) -> Option<T> {
    c.rise::<T>(key).unwrap_or_else(|_| {
        bad.push(key);
        None
    })
}

fn lenient_media_int(c: &RiseWireObject, key: &'static str, bad: &mut Vec<&'static str>) -> Option<i64> {
    match c.rise::<i64>(key) {
        Ok(value) => value,
        Err(_) => match c.rise::<f64>(key) {
            Ok(value) => value.filter(|value| value.is_finite()).map(|value| value.round() as i64),
            Err(_) => {
                bad.push(key);
                None
            }
        },
    }
}

impl Serialize for MessageMediaInfoWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.media_type {
            container.serialize_entry(&RiseWireKey::camel(key::MEDIA_TYPE), value)?;
        }
        if let Some(value) = &self.file_id {
            container.serialize_entry(&RiseWireKey::camel(key::FILE_ID), value)?;
        }
        if let Some(value) = &self.file_url {
            container.serialize_entry(&RiseWireKey::camel(key::FILE_URL), value)?;
        }
        if let Some(value) = &self.thumbnail_url {
            container.serialize_entry(&RiseWireKey::camel(key::THUMBNAIL_URL), value)?;
        }
        if let Some(value) = &self.duration {
            container.serialize_entry(key::DURATION, value)?;
        }
        if let Some(value) = &self.width {
            container.serialize_entry(key::WIDTH, value)?;
        }
        if let Some(value) = &self.height {
            container.serialize_entry(key::HEIGHT, value)?;
        }
        if let Some(value) = &self.file_name {
            container.serialize_entry(&RiseWireKey::camel(key::FILE_NAME), value)?;
        }
        if let Some(value) = &self.file_size {
            container.serialize_entry(&RiseWireKey::camel(key::FILE_SIZE), value)?;
        }
        if let Some(value) = &self.mime_type {
            container.serialize_entry(&RiseWireKey::camel(key::MIME_TYPE), value)?;
        }
        if let Some(value) = &self.bitrate {
            container.serialize_entry(key::BITRATE, value)?;
        }
        if let Some(value) = &self.fps {
            container.serialize_entry(key::FPS, value)?;
        }
        if let Some(value) = &self.codec {
            container.serialize_entry(key::CODEC, value)?;
        }
        if let Some(value) = &self.waveform {
            container.serialize_entry(key::WAVEFORM, value)?;
        }
        if let Some(value) = &self.sticker_id {
            container.serialize_entry(&RiseWireKey::camel(key::STICKER_ID), value)?;
        }
        if let Some(value) = &self.pack_id {
            container.serialize_entry(&RiseWireKey::camel(key::PACK_ID), value)?;
        }
        if let Some(value) = &self.sticker_type {
            container.serialize_entry(&RiseWireKey::camel(key::STICKER_TYPE), value)?;
        }
        if let Some(value) = &self.associated_emojis {
            container.serialize_entry(&RiseWireKey::camel(key::ASSOCIATED_EMOJIS), value)?;
        }
        if let Some(value) = &self.is_uploading {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::IS_UPLOADING), value)?;
        }
        if let Some(value) = &self.upload_progress {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::UPLOAD_PROGRESS), value)?;
        }
        if let Some(value) = &self.uploaded_bytes {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::UPLOADED_BYTES), value)?;
        }
        if let Some(value) = &self.total_bytes {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::TOTAL_BYTES), value)?;
        }
        if let Some(value) = &self.is_compressing {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::IS_COMPRESSING), value)?;
        }
        if let Some(value) = &self.compression_progress {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::COMPRESSION_PROGRESS), value)?;
        }
        if let Some(value) = &self.is_cancelled {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::IS_CANCELLED), value)?;
        }
        if let Some(value) = &self.local_preview_url {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::LOCAL_PREVIEW_URL), value)?;
        }
        if let Some(value) = &self.local_slot_index {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::LOCAL_SLOT_INDEX), value)?;
        }
        if self.has_spoiler == Some(true) {
            container.serialize_entry(key::HAS_SPOILER, &true)?;
        }
        if let Some(value) = &self.view_mode {
            container.serialize_entry(key::VIEW_MODE, value)?;
        }
        if let Some(value) = &self.view_seconds {
            container.serialize_entry(key::VIEW_SECONDS, value)?;
        }
        if let Some(value) = &self.view_expires_at {
            container.serialize_entry(key::VIEW_EXPIRES_AT, value)?;
        }
        if let Some(value) = &self.view_state {
            container.serialize_entry(key::VIEW_STATE, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageReplyInfoWireDTO {
    pub message_id: Option<String>,
    pub sender_id: Option<String>,
    pub content: Option<String>,
    pub content_type: Option<String>,
    pub created_at: Option<f64>,
    pub caption: Option<String>,
    pub media_items: Option<Vec<MessageMediaInfoWireDTO>>,
    pub sender: Option<MessageSenderWireDTO>,
}

impl<'de> Deserialize<'de> for MessageReplyInfoWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                message_id: c.rise::<String>(key::MESSAGE_ID)?,
                sender_id: c.rise::<String>(key::SENDER_ID)?,
                content: c.rise::<String>(key::CONTENT)?,
                content_type: c.rise::<String>(key::CONTENT_TYPE)?,
                created_at: c.rise::<f64>(key::CREATED_AT)?,
                caption: c.rise::<String>(key::CAPTION)?,
                media_items: c.rise::<Vec<MessageMediaInfoWireDTO>>(key::MEDIA_ITEMS)?,
                sender: c.rise::<MessageSenderWireDTO>(key::SENDER)?,
            })
        })
    }
}

impl Serialize for MessageReplyInfoWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.message_id {
            container.serialize_entry(&RiseWireKey::camel(key::MESSAGE_ID), value)?;
        }
        if let Some(value) = &self.sender_id {
            container.serialize_entry(&RiseWireKey::camel(key::SENDER_ID), value)?;
        }
        if let Some(value) = &self.content {
            container.serialize_entry(key::CONTENT, value)?;
        }
        if let Some(value) = &self.content_type {
            container.serialize_entry(&RiseWireKey::camel(key::CONTENT_TYPE), value)?;
        }
        if let Some(value) = &self.created_at {
            container.serialize_entry(&RiseWireKey::camel(key::CREATED_AT), value)?;
        }
        if let Some(value) = &self.caption {
            container.serialize_entry(key::CAPTION, value)?;
        }
        if let Some(value) = &self.media_items {
            container.serialize_entry(&RiseWireKey::camel(key::MEDIA_ITEMS), value)?;
        }
        if let Some(value) = &self.sender {
            container.serialize_entry(key::SENDER, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageForwardInfoWireDTO {
    pub from_chat_id: Option<String>,
    pub from_message_id: Option<String>,
    pub original_message_id: Option<String>,
    pub sender_name: Option<String>,
    pub sender_logo: Option<String>,
    pub sender_id: Option<String>,
    pub sender: Option<MessageSenderWireDTO>,
    pub date: Option<f64>,
    pub forward_chain_count: Option<i64>,
}

impl<'de> Deserialize<'de> for MessageForwardInfoWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                from_chat_id: c.rise::<String>(key::FROM_CHAT_ID)?,
                from_message_id: c.rise::<String>(key::FROM_MESSAGE_ID)?,
                original_message_id: c.rise::<String>(key::ORIGINAL_MESSAGE_ID)?,
                sender_name: c.rise::<String>(key::SENDER_NAME)?,
                sender_logo: c.rise::<String>(key::SENDER_LOGO)?,
                sender_id: c.rise::<String>(key::SENDER_ID)?,
                sender: c.rise::<MessageSenderWireDTO>(key::SENDER)?,
                date: c.rise::<f64>(key::DATE)?,
                forward_chain_count: c.rise::<i64>(key::FORWARD_CHAIN_COUNT)?,
            })
        })
    }
}

impl Serialize for MessageForwardInfoWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.from_chat_id {
            container.serialize_entry(&RiseWireKey::camel(key::FROM_CHAT_ID), value)?;
        }
        if let Some(value) = &self.from_message_id {
            container.serialize_entry(&RiseWireKey::camel(key::FROM_MESSAGE_ID), value)?;
        }
        if let Some(value) = &self.original_message_id {
            container.serialize_entry(&RiseWireKey::camel(key::ORIGINAL_MESSAGE_ID), value)?;
        }
        if let Some(value) = &self.sender_name {
            container.serialize_entry(&RiseWireKey::camel(key::SENDER_NAME), value)?;
        }
        if let Some(value) = &self.sender_logo {
            container.serialize_entry(&RiseWireKey::camel(key::SENDER_LOGO), value)?;
        }
        if let Some(value) = &self.sender_id {
            container.serialize_entry(&RiseWireKey::camel(key::SENDER_ID), value)?;
        }
        if let Some(value) = &self.sender {
            container.serialize_entry(key::SENDER, value)?;
        }
        if let Some(value) = &self.date {
            container.serialize_entry(key::DATE, value)?;
        }
        if let Some(value) = &self.forward_chain_count {
            container.serialize_entry(&RiseWireKey::camel(key::FORWARD_CHAIN_COUNT), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageLinkMetadataWireDTO {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub link_preview_type: Option<String>,
}

impl<'de> Deserialize<'de> for MessageLinkMetadataWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                title: c.rise::<String>(key::TITLE)?,
                description: c.rise::<String>(key::DESCRIPTION)?,
                image_url: c.rise::<String>(key::IMAGE_URL)?,
                link_preview_type: c.rise::<String>(key::LINK_PREVIEW_TYPE)?,
            })
        })
    }
}

impl Serialize for MessageLinkMetadataWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.title {
            container.serialize_entry(key::TITLE, value)?;
        }
        if let Some(value) = &self.description {
            container.serialize_entry(key::DESCRIPTION, value)?;
        }
        if let Some(value) = &self.image_url {
            container.serialize_entry(&RiseWireKey::camel(key::IMAGE_URL), value)?;
        }
        if let Some(value) = &self.link_preview_type {
            container.serialize_entry(&RiseWireKey::camel(key::LINK_PREVIEW_TYPE), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageReactionSummaryWireDTO {
    pub reaction: String,
    pub count: i64,
    pub reacted_by_you: Option<bool>,
}

impl<'de> Deserialize<'de> for MessageReactionSummaryWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                reaction: c.rise_required::<String>(key::REACTION)?,
                count: c.rise_required::<i64>(key::COUNT)?,
                reacted_by_you: c.rise::<bool>(key::REACTED_BY_YOU)?,
            })
        })
    }
}

impl Serialize for MessageReactionSummaryWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::REACTION, &self.reaction)?;
        container.serialize_entry(key::COUNT, &self.count)?;
        if let Some(value) = &self.reacted_by_you {
            container.serialize_entry(&RiseWireKey::camel(key::REACTED_BY_YOU), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageReactedByWireDTO {
    pub reaction: Option<String>,
    pub created_at: Option<f64>,
    pub sender: Option<MessageSenderWireDTO>,
    pub chat_id: Option<String>,
}

impl<'de> Deserialize<'de> for MessageReactedByWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                reaction: c.rise::<String>(key::REACTION)?,
                created_at: c.rise::<f64>(key::CREATED_AT)?,
                sender: c.rise::<MessageSenderWireDTO>(key::SENDER)?,
                chat_id: c.rise::<String>(key::CHAT_ID)?,
            })
        })
    }
}

impl Serialize for MessageReactedByWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.reaction {
            container.serialize_entry(key::REACTION, value)?;
        }
        if let Some(value) = &self.created_at {
            container.serialize_entry(&RiseWireKey::camel(key::CREATED_AT), value)?;
        }
        if let Some(value) = &self.sender {
            container.serialize_entry(key::SENDER, value)?;
        }
        if let Some(value) = &self.chat_id {
            container.serialize_entry(&RiseWireKey::camel(key::CHAT_ID), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageSenderMoreWireDTO {
    pub logo: Option<String>,
    pub banner: Option<String>,
    pub description: Option<String>,
    pub level: Option<i64>,
    pub is_online: Option<bool>,
    pub last_seen: Option<f64>,
}

impl<'de> Deserialize<'de> for MessageSenderMoreWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                logo: c.rise::<String>(key::LOGO)?,
                banner: c.rise::<String>(key::BANNER)?,
                description: c.rise::<String>(key::DESCRIPTION)?,
                level: c.rise::<i64>(key::LEVEL)?,
                is_online: c.rise::<bool>(key::IS_ONLINE)?,
                last_seen: c.rise::<f64>(key::LAST_SEEN)?,
            })
        })
    }
}

impl Serialize for MessageSenderMoreWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.logo {
            container.serialize_entry(key::LOGO, value)?;
        }
        if let Some(value) = &self.banner {
            container.serialize_entry(key::BANNER, value)?;
        }
        if let Some(value) = &self.description {
            container.serialize_entry(key::DESCRIPTION, value)?;
        }
        if let Some(value) = &self.level {
            container.serialize_entry(key::LEVEL, value)?;
        }
        if let Some(value) = &self.is_online {
            container.serialize_entry(&RiseWireKey::camel(key::IS_ONLINE), value)?;
        }
        if let Some(value) = &self.last_seen {
            container.serialize_entry(&RiseWireKey::camel(key::LAST_SEEN), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageSenderWireDTO {
    pub id: Option<String>,
    pub name: Option<String>,
    pub tag: Option<String>,
    pub role: Option<String>,
    pub user_chat_id: Option<String>,
    pub is_premium: Option<bool>,
    pub is_official: Option<bool>,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
    pub more: Option<MessageSenderMoreWireDTO>,
    pub is_deleted: Option<bool>,
}

impl<'de> Deserialize<'de> for MessageSenderWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                id: c.rise::<String>(key::ID)?,
                name: c.rise::<String>(key::NAME)?,
                tag: c.rise::<String>(key::TAG)?,
                role: c.rise::<String>(key::ROLE)?,
                user_chat_id: c.rise::<String>(key::USER_CHAT_ID)?,
                is_premium: c.rise::<bool>(key::IS_PREMIUM)?,
                is_official: c.rise::<bool>(key::IS_OFFICIAL)?,
                premium_badge_emoji: c.rise::<String>(key::PREMIUM_BADGE_EMOJI)?,
                name_color: c.rise::<String>(key::NAME_COLOR)?,
                more: c.rise::<MessageSenderMoreWireDTO>(key::MORE)?,
                is_deleted: c.rise::<bool>(key::IS_DELETED)?,
            })
        })
    }
}

impl Serialize for MessageSenderWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.id {
            container.serialize_entry(key::ID, value)?;
        }
        if let Some(value) = &self.name {
            container.serialize_entry(key::NAME, value)?;
        }
        if let Some(value) = &self.tag {
            container.serialize_entry(key::TAG, value)?;
        }
        if let Some(value) = &self.role {
            container.serialize_entry(key::ROLE, value)?;
        }
        if let Some(value) = &self.user_chat_id {
            container.serialize_entry(&RiseWireKey::camel(key::USER_CHAT_ID), value)?;
        }
        if let Some(value) = &self.is_premium {
            container.serialize_entry(&RiseWireKey::camel(key::IS_PREMIUM), value)?;
        }
        if let Some(value) = &self.is_official {
            container.serialize_entry(&RiseWireKey::camel(key::IS_OFFICIAL), value)?;
        }
        if let Some(value) = &self.premium_badge_emoji {
            container.serialize_entry(&RiseWireKey::camel(key::PREMIUM_BADGE_EMOJI), value)?;
        }
        if let Some(value) = &self.name_color {
            container.serialize_entry(&RiseWireKey::camel(key::NAME_COLOR), value)?;
        }
        if let Some(value) = &self.more {
            container.serialize_entry(key::MORE, value)?;
        }
        if let Some(value) = &self.is_deleted {
            container.serialize_entry(&RiseWireKey::camel(key::IS_DELETED), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageSystemActorWireDTO {
    pub id: Option<String>,
    pub name: Option<String>,
}

impl<'de> Deserialize<'de> for MessageSystemActorWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                id: c.rise::<String>(key::ID)?,
                name: c.rise::<String>(key::NAME)?,
            })
        })
    }
}

impl Serialize for MessageSystemActorWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.id {
            container.serialize_entry(key::ID, value)?;
        }
        if let Some(value) = &self.name {
            container.serialize_entry(key::NAME, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageSharedPostAuthorWireDTO {
    pub id: Option<String>,
    pub tag: Option<String>,
    pub name: Option<String>,
    pub logo: Option<String>,
    pub is_premium: Option<bool>,
    pub is_official: Option<bool>,
}

impl<'de> Deserialize<'de> for MessageSharedPostAuthorWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                id: c.rise::<String>(shared_post_key::ID)?,
                tag: c.rise::<String>(shared_post_key::TAG)?,
                name: c.rise::<String>(shared_post_key::NAME)?,
                logo: c.rise::<String>(shared_post_key::LOGO)?,
                is_premium: c.rise::<bool>(shared_post_key::IS_PREMIUM)?,
                is_official: c.rise::<bool>(shared_post_key::IS_OFFICIAL)?,
            })
        })
    }
}

impl Serialize for MessageSharedPostAuthorWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.id {
            container.serialize_entry(shared_post_key::ID, value)?;
        }
        if let Some(value) = &self.tag {
            container.serialize_entry(shared_post_key::TAG, value)?;
        }
        if let Some(value) = &self.name {
            container.serialize_entry(shared_post_key::NAME, value)?;
        }
        if let Some(value) = &self.logo {
            container.serialize_entry(shared_post_key::LOGO, value)?;
        }
        if let Some(value) = &self.is_premium {
            container.serialize_entry(&RiseWireKey::camel(shared_post_key::IS_PREMIUM), value)?;
        }
        if let Some(value) = &self.is_official {
            container.serialize_entry(&RiseWireKey::camel(shared_post_key::IS_OFFICIAL), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageSharedPostWireDTO {
    pub post_id: Option<WireID>,
    pub is_short: Option<bool>,
    pub is_riff: Option<bool>,
    pub show_in_publications: Option<bool>,
    pub title: Option<String>,
    pub snippet: Option<String>,
    pub images: Option<Vec<String>>,
    pub video: Option<String>,
    pub video_thumbnail: Option<String>,
    pub likes_count: Option<i64>,
    pub comments_count: Option<i64>,
    pub author: Option<MessageSharedPostAuthorWireDTO>,
}

impl<'de> Deserialize<'de> for MessageSharedPostWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                post_id: c.rise::<WireID>(shared_post_key::POST_ID)?,
                is_short: c.rise::<bool>(shared_post_key::IS_SHORT)?,
                is_riff: c.rise::<bool>(shared_post_key::IS_RIFF)?,
                show_in_publications: c.rise::<bool>(shared_post_key::SHOW_IN_PUBLICATIONS)?,
                title: c.rise::<String>(shared_post_key::TITLE)?,
                snippet: c.rise::<String>(shared_post_key::SNIPPET)?,
                images: c.rise::<Vec<String>>(shared_post_key::IMAGES)?,
                video: c.rise::<String>(shared_post_key::VIDEO)?,
                video_thumbnail: c.rise::<String>(shared_post_key::VIDEO_THUMBNAIL)?,
                likes_count: c.rise::<i64>(shared_post_key::LIKES_COUNT)?,
                comments_count: c.rise::<i64>(shared_post_key::COMMENTS_COUNT)?,
                author: c.rise::<MessageSharedPostAuthorWireDTO>(shared_post_key::AUTHOR)?,
            })
        })
    }
}

impl Serialize for MessageSharedPostWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.post_id {
            container.serialize_entry(&RiseWireKey::camel(shared_post_key::POST_ID), value)?;
        }
        if let Some(value) = &self.is_short {
            container.serialize_entry(&RiseWireKey::camel(shared_post_key::IS_SHORT), value)?;
        }
        if let Some(value) = &self.is_riff {
            container.serialize_entry(&RiseWireKey::camel(shared_post_key::IS_RIFF), value)?;
        }
        if let Some(value) = &self.show_in_publications {
            container.serialize_entry(&RiseWireKey::camel(shared_post_key::SHOW_IN_PUBLICATIONS), value)?;
        }
        if let Some(value) = &self.title {
            container.serialize_entry(shared_post_key::TITLE, value)?;
        }
        if let Some(value) = &self.snippet {
            container.serialize_entry(shared_post_key::SNIPPET, value)?;
        }
        if let Some(value) = &self.images {
            container.serialize_entry(shared_post_key::IMAGES, value)?;
        }
        if let Some(value) = &self.video {
            container.serialize_entry(shared_post_key::VIDEO, value)?;
        }
        if let Some(value) = &self.video_thumbnail {
            container.serialize_entry(&RiseWireKey::camel(shared_post_key::VIDEO_THUMBNAIL), value)?;
        }
        if let Some(value) = &self.likes_count {
            container.serialize_entry(&RiseWireKey::camel(shared_post_key::LIKES_COUNT), value)?;
        }
        if let Some(value) = &self.comments_count {
            container.serialize_entry(&RiseWireKey::camel(shared_post_key::COMMENTS_COUNT), value)?;
        }
        if let Some(value) = &self.author {
            container.serialize_entry(shared_post_key::AUTHOR, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageStoryReplyAuthorWireDTO {
    pub id: Option<String>,
    pub tag: Option<String>,
    pub name: Option<String>,
    pub logo: Option<String>,
    pub is_premium: Option<bool>,
}

impl<'de> Deserialize<'de> for MessageStoryReplyAuthorWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                id: c.rise::<String>(story_reply_key::ID)?,
                tag: c.rise::<String>(story_reply_key::TAG)?,
                name: c.rise::<String>(story_reply_key::NAME)?,
                logo: c.rise::<String>(story_reply_key::LOGO)?,
                is_premium: c.rise::<bool>(story_reply_key::IS_PREMIUM)?,
            })
        })
    }
}

impl Serialize for MessageStoryReplyAuthorWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.id {
            container.serialize_entry(story_reply_key::ID, value)?;
        }
        if let Some(value) = &self.tag {
            container.serialize_entry(story_reply_key::TAG, value)?;
        }
        if let Some(value) = &self.name {
            container.serialize_entry(story_reply_key::NAME, value)?;
        }
        if let Some(value) = &self.logo {
            container.serialize_entry(story_reply_key::LOGO, value)?;
        }
        if let Some(value) = &self.is_premium {
            container.serialize_entry(&RiseWireKey::camel(story_reply_key::IS_PREMIUM), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageStoryReplyWireDTO {
    pub story_id: Option<WireID>,
    pub media_url: Option<String>,
    pub is_video: Option<bool>,
    pub caption: Option<String>,
    pub created_at_ms: Option<i64>,
    pub expires_at_ms: Option<i64>,
    pub post_to_profile: Option<bool>,
    pub author: Option<MessageStoryReplyAuthorWireDTO>,
}

impl<'de> Deserialize<'de> for MessageStoryReplyWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                story_id: c.rise::<WireID>(story_reply_key::STORY_ID)?,
                media_url: c.rise::<String>(story_reply_key::MEDIA_URL)?,
                is_video: c.rise::<bool>(story_reply_key::IS_VIDEO)?,
                caption: c.rise::<String>(story_reply_key::CAPTION)?,
                created_at_ms: c.rise::<i64>(story_reply_key::CREATED_AT_MS)?,
                expires_at_ms: c.rise::<i64>(story_reply_key::EXPIRES_AT_MS)?,
                post_to_profile: c.rise::<bool>(story_reply_key::POST_TO_PROFILE)?,
                author: c.rise::<MessageStoryReplyAuthorWireDTO>(story_reply_key::AUTHOR)?,
            })
        })
    }
}

impl Serialize for MessageStoryReplyWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.story_id {
            container.serialize_entry(&RiseWireKey::camel(story_reply_key::STORY_ID), value)?;
        }
        if let Some(value) = &self.media_url {
            container.serialize_entry(&RiseWireKey::camel(story_reply_key::MEDIA_URL), value)?;
        }
        if let Some(value) = &self.is_video {
            container.serialize_entry(&RiseWireKey::camel(story_reply_key::IS_VIDEO), value)?;
        }
        if let Some(value) = &self.caption {
            container.serialize_entry(story_reply_key::CAPTION, value)?;
        }
        if let Some(value) = &self.created_at_ms {
            container.serialize_entry(&RiseWireKey::camel(story_reply_key::CREATED_AT_MS), value)?;
        }
        if let Some(value) = &self.expires_at_ms {
            container.serialize_entry(&RiseWireKey::camel(story_reply_key::EXPIRES_AT_MS), value)?;
        }
        if let Some(value) = &self.post_to_profile {
            container.serialize_entry(&RiseWireKey::camel(story_reply_key::POST_TO_PROFILE), value)?;
        }
        if let Some(value) = &self.author {
            container.serialize_entry(story_reply_key::AUTHOR, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageBotWebAppWireDTO {
    pub url: String,
    pub bot_id: Option<String>,
    pub mini_app_id: Option<String>,
}

impl<'de> Deserialize<'de> for MessageBotWebAppWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                url: c.rise_required::<String>(reply_markup_key::URL)?,
                bot_id: c.rise::<String>(reply_markup_key::BOT_ID)?,
                mini_app_id: c.rise::<String>(reply_markup_key::MINI_APP_ID)?,
            })
        })
    }
}

impl Serialize for MessageBotWebAppWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(reply_markup_key::URL, &self.url)?;
        if let Some(value) = &self.bot_id {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::BOT_ID), value)?;
        }
        if let Some(value) = &self.mini_app_id {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::MINI_APP_ID), value)?;
        }
        container.end()
    }
}

impl MessageBotWebAppWireDTO {
    pub fn new(url: String, bot_id: Option<String>, mini_app_id: Option<String>) -> Self {
        Self { url, bot_id, mini_app_id }
    }
}

#[derive(Clone, Debug)]
pub struct MessageBotKeyboardButtonWireDTO {
    pub text: String,
    pub callback_data: Option<String>,
    pub url: Option<String>,
    pub web_app: Option<MessageBotWebAppWireDTO>,
    pub switch_inline_query: Option<String>,
    pub switch_inline_query_current_chat: Option<String>,
    pub pay: Option<bool>,
    pub has_callback_game: bool,
}

impl<'de> Deserialize<'de> for MessageBotKeyboardButtonWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let serde_json::Value::String(text) = value {
            return Ok(Self {
                text,
                callback_data: None,
                url: None,
                web_app: None,
                switch_inline_query: None,
                switch_inline_query_current_chat: None,
                pay: None,
                has_callback_game: false,
            });
        }
        rise_decode(value, |c| {
            let web_app = if let Ok(value) = c.rise_required::<MessageBotWebAppWireDTO>(reply_markup_key::WEB_APP) {
                Some(value)
            } else if let Ok(value) = c.rise_required::<String>(reply_markup_key::WEB_APP) {
                Some(MessageBotWebAppWireDTO::new(value, None, None))
            } else {
                None
            };
            Ok(Self {
                text: c.rise_required::<String>(reply_markup_key::TEXT)?,
                callback_data: c.rise::<String>(reply_markup_key::CALLBACK_DATA)?,
                url: c.rise::<String>(reply_markup_key::URL)?,
                web_app,
                switch_inline_query: c.rise::<String>(reply_markup_key::SWITCH_INLINE_QUERY)?,
                switch_inline_query_current_chat: c
                    .rise::<String>(reply_markup_key::SWITCH_INLINE_QUERY_CURRENT_CHAT)?,
                pay: c.rise::<bool>(reply_markup_key::PAY)?,
                has_callback_game: c.rise_contains(reply_markup_key::CALLBACK_GAME),
            })
        })
        .map_err(D::Error::custom)
    }
}

impl Serialize for MessageBotKeyboardButtonWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(reply_markup_key::TEXT, &self.text)?;
        if let Some(value) = &self.callback_data {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::CALLBACK_DATA), value)?;
        }
        if let Some(value) = &self.url {
            container.serialize_entry(reply_markup_key::URL, value)?;
        }
        if let Some(value) = &self.web_app {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::WEB_APP), value)?;
        }
        if let Some(value) = &self.switch_inline_query {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::SWITCH_INLINE_QUERY), value)?;
        }
        if let Some(value) = &self.switch_inline_query_current_chat {
            container
                .serialize_entry(&RiseWireKey::camel(reply_markup_key::SWITCH_INLINE_QUERY_CURRENT_CHAT), value)?;
        }
        if let Some(value) = &self.pay {
            container.serialize_entry(reply_markup_key::PAY, value)?;
        }
        if self.has_callback_game {
            container.serialize_entry(reply_markup_key::CALLBACK_GAME, &HashMap::<String, String>::new())?;
        }
        container.end()
    }
}

impl MessageBotKeyboardButtonWireDTO {
    pub fn to_model(&self) -> MessageBotKeyboardButton {
        MessageBotKeyboardButton {
            text: self.text.clone(),
            callback_data: self.callback_data.clone(),
            url: self.url.clone(),
            web_app_url: self.web_app.as_ref().map(|web_app| web_app.url.clone()),
            bot_id: self.web_app.as_ref().and_then(|web_app| web_app.bot_id.clone()),
            mini_app_id: self.web_app.as_ref().and_then(|web_app| web_app.mini_app_id.clone()),
            switch_inline_query: self.switch_inline_query.clone(),
            switch_inline_query_current_chat: self.switch_inline_query_current_chat.clone(),
            is_pay: self.pay.unwrap_or(false),
            is_game: self.has_callback_game,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MessageBotReplyMarkupWireDTO {
    pub inline_keyboard: Option<Vec<Vec<MessageBotKeyboardButtonWireDTO>>>,
    pub keyboard: Option<Vec<Vec<MessageBotKeyboardButtonWireDTO>>>,
    pub resize_keyboard: Option<bool>,
    pub one_time_keyboard: Option<bool>,
    pub selective: Option<bool>,
    pub input_field_placeholder: Option<String>,
    pub remove_keyboard: Option<bool>,
    pub force_reply: Option<bool>,
}

impl<'de> Deserialize<'de> for MessageBotReplyMarkupWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let serde_json::Value::String(raw) = value {
            return serde_json::from_str::<Self>(&raw).map_err(D::Error::custom);
        }
        rise_decode(value, |c| {
            Ok(Self {
                inline_keyboard: c.rise::<Vec<Vec<MessageBotKeyboardButtonWireDTO>>>(reply_markup_key::INLINE_KEYBOARD)?,
                keyboard: c.rise::<Vec<Vec<MessageBotKeyboardButtonWireDTO>>>(reply_markup_key::KEYBOARD)?,
                resize_keyboard: c.rise::<bool>(reply_markup_key::RESIZE_KEYBOARD)?,
                one_time_keyboard: c.rise::<bool>(reply_markup_key::ONE_TIME_KEYBOARD)?,
                selective: c.rise::<bool>(reply_markup_key::SELECTIVE)?,
                input_field_placeholder: c.rise::<String>(reply_markup_key::INPUT_FIELD_PLACEHOLDER)?,
                remove_keyboard: c.rise::<bool>(reply_markup_key::REMOVE_KEYBOARD)?,
                force_reply: c.rise::<bool>(reply_markup_key::FORCE_REPLY)?,
            })
        })
        .map_err(D::Error::custom)
    }
}

impl Serialize for MessageBotReplyMarkupWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.inline_keyboard {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::INLINE_KEYBOARD), value)?;
        }
        if let Some(value) = &self.keyboard {
            container.serialize_entry(reply_markup_key::KEYBOARD, value)?;
        }
        if let Some(value) = &self.resize_keyboard {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::RESIZE_KEYBOARD), value)?;
        }
        if let Some(value) = &self.one_time_keyboard {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::ONE_TIME_KEYBOARD), value)?;
        }
        if let Some(value) = &self.selective {
            container.serialize_entry(reply_markup_key::SELECTIVE, value)?;
        }
        if let Some(value) = &self.input_field_placeholder {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::INPUT_FIELD_PLACEHOLDER), value)?;
        }
        if let Some(value) = &self.remove_keyboard {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::REMOVE_KEYBOARD), value)?;
        }
        if let Some(value) = &self.force_reply {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::FORCE_REPLY), value)?;
        }
        container.end()
    }
}

impl MessageBotReplyMarkupWireDTO {
    pub fn to_model(&self) -> MessageBotReplyMarkup {
        MessageBotReplyMarkup {
            inline_keyboard: Self::drawable(self.inline_keyboard.as_deref()),
            keyboard: Self::drawable(self.keyboard.as_deref()),
            resize_keyboard: self.resize_keyboard.unwrap_or(false),
            one_time_keyboard: self.one_time_keyboard.unwrap_or(false),
            selective: self.selective.unwrap_or(false),
            input_field_placeholder: self.input_field_placeholder.clone(),
            remove_keyboard: self.remove_keyboard.unwrap_or(false),
            force_reply: self.force_reply.unwrap_or(false),
        }
    }

    /// payment and game buttons are not drawn, the app has neither and a button that only said
    /// not supported looked unfinished
    /// rows left with no buttons go too
    fn drawable(rows: Option<&[Vec<MessageBotKeyboardButtonWireDTO>]>) -> Vec<Vec<MessageBotKeyboardButton>> {
        rows.unwrap_or_default()
            .iter()
            .filter_map(|row| {
                let buttons: Vec<MessageBotKeyboardButton> = row
                    .iter()
                    .map(MessageBotKeyboardButtonWireDTO::to_model)
                    .filter(|button| !button.is_pay && !button.is_game)
                    .collect();
                (!buttons.is_empty()).then_some(buttons)
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct MessageBotWebAppDataWireDTO {
    pub data: String,
    pub button_text: Option<String>,
}

impl<'de> Deserialize<'de> for MessageBotWebAppDataWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if let serde_json::Value::String(raw) = value {
            return serde_json::from_str::<Self>(&raw).map_err(D::Error::custom);
        }
        rise_decode(value, |c| {
            Ok(Self {
                data: c.rise_required::<String>(reply_markup_key::DATA)?,
                button_text: c.rise::<String>(reply_markup_key::BUTTON_TEXT)?,
            })
        })
        .map_err(D::Error::custom)
    }
}

impl Serialize for MessageBotWebAppDataWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(reply_markup_key::DATA, &self.data)?;
        if let Some(value) = &self.button_text {
            container.serialize_entry(&RiseWireKey::camel(reply_markup_key::BUTTON_TEXT), value)?;
        }
        container.end()
    }
}

impl MessageBotWebAppDataWireDTO {
    pub fn to_model(&self) -> MessageBotWebAppData {
        MessageBotWebAppData { data: self.data.clone(), button_text: self.button_text.clone() }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MessageSystemMetadataWallpaperEnvelope {
    pub wallpaper: Option<MessageSystemWallpaperWireDTO>,
}

impl<'de> Deserialize<'de> for MessageSystemMetadataWallpaperEnvelope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self { wallpaper: c.rise::<MessageSystemWallpaperWireDTO>(chat_action_key::WALLPAPER)? })
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct MessageSystemMetadataGiftEnvelope {
    pub gift: Option<MessageSystemGiftWireDTO>,
}

impl<'de> Deserialize<'de> for MessageSystemMetadataGiftEnvelope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| Ok(Self { gift: c.rise::<MessageSystemGiftWireDTO>(chat_action_key::GIFT)? }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageSystemMetadataTargetEnvelope {
    pub target_user_id: Option<String>,
    pub target_name: Option<String>,
}

impl<'de> Deserialize<'de> for MessageSystemMetadataTargetEnvelope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                target_user_id: c.rise::<String>(chat_action_key::TARGET_USER_ID)?,
                target_name: c.rise::<String>(chat_action_key::TARGET_NAME)?,
            })
        })
    }
}

impl Serialize for MessageSystemMetadataTargetEnvelope {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.target_user_id {
            container.serialize_entry(chat_action_key::TARGET_USER_ID, value)?;
        }
        if let Some(value) = &self.target_name {
            container.serialize_entry(chat_action_key::TARGET_NAME, value)?;
        }
        container.end()
    }
}

impl MessageSystemMetadataTargetEnvelope {
    pub fn is_empty(&self) -> bool {
        self.target_user_id.as_deref().unwrap_or_default().is_empty()
            && self.target_name.as_deref().unwrap_or_default().is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub struct MessageSystemMetadataGiftItemEnvelope {
    pub gift_item: Option<MessageSystemGiftItemWireDTO>,
}

impl<'de> Deserialize<'de> for MessageSystemMetadataGiftItemEnvelope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self { gift_item: c.rise::<MessageSystemGiftItemWireDTO>(chat_action_key::GIFT_ITEM)? })
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageSystemGiftItemWireDTO {
    pub instance_id: Option<String>,
    pub gift_id: Option<i64>,
    pub gift_slug: Option<String>,
    pub name: Option<String>,
    pub poster_url: Option<String>,
    pub animation_url: Option<String>,
    pub recipient_user_id: Option<String>,
    pub caption: Option<String>,
    pub anonymous: Option<bool>,
    pub serial: Option<i64>,
    pub supply_total: Option<i64>,
    pub rarity: Option<String>,
    pub value_sparks: Option<i64>,
    pub backdrop: Option<HashMap<String, String>>,
    pub item_kind: Option<String>,
}

impl<'de> Deserialize<'de> for MessageSystemGiftItemWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                instance_id: c.rise::<String>(chat_action_key::INSTANCE_ID)?,
                gift_id: c.rise::<i64>(chat_action_key::GIFT_ID)?,
                gift_slug: c.rise::<String>(chat_action_key::GIFT_SLUG)?,
                name: c.rise::<String>(chat_action_key::NAME)?,
                poster_url: c.rise::<String>(chat_action_key::POSTER_URL)?,
                animation_url: c.rise::<String>(chat_action_key::ANIMATION_URL)?,
                recipient_user_id: c.rise::<String>(chat_action_key::RECIPIENT_USER_ID)?,
                caption: c.rise::<String>(chat_action_key::CAPTION)?,
                anonymous: c.rise::<bool>(chat_action_key::ANONYMOUS)?,
                serial: c.rise::<i64>(chat_action_key::SERIAL)?,
                supply_total: c.rise::<i64>(chat_action_key::SUPPLY_TOTAL)?,
                rarity: c.rise::<String>(chat_action_key::RARITY)?,
                value_sparks: c.rise::<i64>(chat_action_key::VALUE_SPARKS)?,
                backdrop: c.rise::<HashMap<String, String>>(chat_action_key::BACKDROP)?,
                item_kind: c.rise::<String>(chat_action_key::ITEM_KIND)?,
            })
        })
    }
}

impl Serialize for MessageSystemGiftItemWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.instance_id {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::INSTANCE_ID), value)?;
        }
        if let Some(value) = &self.gift_id {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::GIFT_ID), value)?;
        }
        if let Some(value) = &self.gift_slug {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::GIFT_SLUG), value)?;
        }
        if let Some(value) = &self.name {
            container.serialize_entry(chat_action_key::NAME, value)?;
        }
        if let Some(value) = &self.poster_url {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::POSTER_URL), value)?;
        }
        if let Some(value) = &self.animation_url {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::ANIMATION_URL), value)?;
        }
        if let Some(value) = &self.recipient_user_id {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::RECIPIENT_USER_ID), value)?;
        }
        if let Some(value) = &self.caption {
            container.serialize_entry(chat_action_key::CAPTION, value)?;
        }
        if let Some(value) = &self.anonymous {
            container.serialize_entry(chat_action_key::ANONYMOUS, value)?;
        }
        if let Some(value) = &self.serial {
            container.serialize_entry(chat_action_key::SERIAL, value)?;
        }
        if let Some(value) = &self.supply_total {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::SUPPLY_TOTAL), value)?;
        }
        if let Some(value) = &self.rarity {
            container.serialize_entry(chat_action_key::RARITY, value)?;
        }
        if let Some(value) = &self.value_sparks {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::VALUE_SPARKS), value)?;
        }
        if let Some(value) = &self.backdrop {
            container.serialize_entry(chat_action_key::BACKDROP, value)?;
        }
        if let Some(value) = &self.item_kind {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::ITEM_KIND), value)?;
        }
        container.end()
    }
}

impl MessageSystemGiftItemWireDTO {
    pub fn to_model(&self) -> Option<MessageSystemGiftItem> {
        let instance = trimming_whitespaces_and_newlines(self.instance_id.as_deref().unwrap_or_default());
        if instance.is_empty() {
            return None;
        }
        Some(MessageSystemGiftItem {
            instance_id: instance.to_owned(),
            name: self.name.clone().unwrap_or_default(),
            poster_url: self.poster_url.clone().unwrap_or_default(),
            animation_url: self.animation_url.clone().unwrap_or_default(),
            recipient_id: trimming_whitespaces_and_newlines(self.recipient_user_id.as_deref().unwrap_or_default())
                .to_owned(),
            caption: self.caption.clone().unwrap_or_default(),
            anonymous: self.anonymous.unwrap_or(false),
            serial: self.serial.unwrap_or(0),
            supply_total: self.supply_total.unwrap_or(0),
            rarity: self.rarity.clone().unwrap_or_default(),
            backdrop: self.backdrop.clone().unwrap_or_default().into_iter().collect(),
            item_kind: trimming_whitespaces_and_newlines(self.item_kind.as_deref().unwrap_or_default()).to_owned(),
            gift_id: self.gift_id.unwrap_or(0),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageSystemGiftWireDTO {
    pub months: Option<i64>,
    pub product_id: Option<String>,
    pub recipient_user_id: Option<String>,
}

impl<'de> Deserialize<'de> for MessageSystemGiftWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                months: c.rise::<i64>(chat_action_key::MONTHS)?,
                product_id: c.rise::<String>(chat_action_key::PRODUCT_ID)?,
                recipient_user_id: c.rise::<String>(chat_action_key::RECIPIENT_USER_ID)?,
            })
        })
    }
}

impl Serialize for MessageSystemGiftWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.months {
            container.serialize_entry(chat_action_key::MONTHS, value)?;
        }
        if let Some(value) = &self.product_id {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::PRODUCT_ID), value)?;
        }
        if let Some(value) = &self.recipient_user_id {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::RECIPIENT_USER_ID), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageSystemWallpaperWireDTO {
    pub preview_url: Option<String>,
    pub image_url: Option<String>,
    pub dim_percent: Option<i64>,
    pub blur_percent: Option<i64>,
}

impl<'de> Deserialize<'de> for MessageSystemWallpaperWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                preview_url: c.rise::<String>(chat_action_key::PREVIEW_URL)?,
                image_url: c.rise::<String>(chat_action_key::IMAGE_URL)?,
                dim_percent: c.rise::<i64>(chat_action_key::DIM_PERCENT)?,
                blur_percent: c.rise::<i64>(chat_action_key::BLUR_PERCENT)?,
            })
        })
    }
}

impl Serialize for MessageSystemWallpaperWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.preview_url {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::PREVIEW_URL), value)?;
        }
        if let Some(value) = &self.image_url {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::IMAGE_URL), value)?;
        }
        if let Some(value) = &self.dim_percent {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::DIM_PERCENT), value)?;
        }
        if let Some(value) = &self.blur_percent {
            container.serialize_entry(&RiseWireKey::camel(chat_action_key::BLUR_PERCENT), value)?;
        }
        container.end()
    }
}

impl MessageSystemWallpaperWireDTO {
    pub fn to_preview(&self) -> Option<MessageSystemWallpaperPreview> {
        let preview = trimming_whitespaces_and_newlines(self.preview_url.as_deref().unwrap_or_default());
        let image = trimming_whitespaces_and_newlines(self.image_url.as_deref().unwrap_or_default());
        let source = if preview.is_empty() { image } else { preview };
        if source.is_empty() {
            return None;
        }
        Some(MessageSystemWallpaperPreview {
            image_url: source.to_owned(),
            look: ChatWallpaperLook::new(
                self.dim_percent.unwrap_or(ChatWallpaperLook::DEFAULT_DIM_PERCENT),
                self.blur_percent.unwrap_or(0),
            )
            .clamped(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct MessageCallInfoWireDTO {
    pub schema_version: Option<i64>,
    pub call_id: Option<String>,
    pub state: Option<String>,
    pub media_mode: Option<String>,
    pub caller_user_id: Option<String>,
    pub callee_user_id: Option<String>,
    pub started_at_ms: Option<i64>,
    pub answered_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub ended_by_user_id: Option<String>,
    pub end_reason: Option<String>,
}

impl<'de> Deserialize<'de> for MessageCallInfoWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let sheet = match value {
            serde_json::Value::String(raw) => serde_json::from_str::<CallSheet>(&raw),
            other => serde_json::from_value::<CallSheet>(other),
        }
        .map_err(D::Error::custom)?;
        Ok(Self::from_sheet(&sheet))
    }
}

impl Serialize for MessageCallInfoWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.sheet().serialize(serializer)
    }
}

impl MessageCallInfoWireDTO {
    pub fn from_sheet(sheet: &CallSheet) -> Self {
        Self {
            schema_version: sheet.schema_version,
            call_id: sheet.call_id.clone(),
            state: sheet.state.clone(),
            media_mode: sheet.media_mode.clone(),
            caller_user_id: sheet.caller_user_id.clone(),
            callee_user_id: sheet.callee_user_id.clone(),
            started_at_ms: sheet.started_at_ms,
            answered_at_ms: sheet.answered_at_ms,
            ended_at_ms: sheet.ended_at_ms,
            duration_ms: sheet.duration_ms,
            ended_by_user_id: sheet.ended_by_user_id.clone(),
            end_reason: sheet.end_reason.clone(),
        }
    }

    pub fn sheet(&self) -> CallSheet {
        CallSheet {
            schema_version: self.schema_version,
            call_id: self.call_id.clone(),
            state: self.state.clone(),
            media_mode: self.media_mode.clone(),
            caller_user_id: self.caller_user_id.clone(),
            callee_user_id: self.callee_user_id.clone(),
            started_at_ms: self.started_at_ms,
            ended_at_ms: self.ended_at_ms,
            duration_ms: self.duration_ms,
            answered_at_ms: self.answered_at_ms,
            ended_by_user_id: self.ended_by_user_id.clone(),
            end_reason: self.end_reason.clone(),
        }
    }

    pub fn from_model(model: &MessageCallInfo) -> Self {
        Self {
            schema_version: Some(model.schema_version),
            call_id: Some(model.call_id.clone()),
            state: Some(model.terminal_state.raw_value().to_string()),
            media_mode: Some(model.media_mode.raw_value().to_string()),
            caller_user_id: Some(model.caller_user_id.clone()),
            callee_user_id: Some(model.callee_user_id.clone()),
            started_at_ms: Some(model.created_at_ms),
            answered_at_ms: model.answered_at_ms,
            ended_at_ms: Some(model.ended_at_ms),
            duration_ms: Some(model.duration_ms),
            ended_by_user_id: model.ended_by_user_id.clone(),
            end_reason: model.end_reason.clone(),
        }
    }

    pub fn to_model(&self) -> Option<MessageCallInfo> {
        fn normalized(value: Option<&str>) -> Option<String> {
            let value = trimming_whitespaces_and_newlines(value?);
            if value.is_empty() { None } else { Some(value.to_owned()) }
        }

        let call_id = normalized(self.call_id.as_deref())?;
        let caller_user_id = normalized(self.caller_user_id.as_deref())?;
        let callee_user_id = normalized(self.callee_user_id.as_deref())?;
        let media_mode = MessageCallMediaMode::from_raw_value(&normalized(self.media_mode.as_deref())?);
        if !matches!(media_mode, MessageCallMediaMode::Audio | MessageCallMediaMode::Video) {
            return None;
        }
        let terminal_state = MessageCallTerminalState::from_raw_value(&normalized(self.state.as_deref())?);
        if !terminal_state.is_terminal() {
            return None;
        }
        let created_at_ms = self.started_at_ms?;
        let ended_at_ms = self.ended_at_ms?;
        if created_at_ms <= 0 || ended_at_ms <= 0 {
            return None;
        }

        let answered_at_ms = self.answered_at_ms.filter(|value| *value > 0);
        let resolved_duration_ms = self
            .duration_ms
            .or_else(|| answered_at_ms.map(|answered| ended_at_ms.saturating_sub(answered).max(0)))
            .unwrap_or(0)
            .max(0);
        Some(MessageCallInfo {
            schema_version: self.schema_version.unwrap_or(1).max(1),
            call_id,
            media_mode,
            terminal_state,
            caller_user_id,
            callee_user_id,
            ended_by_user_id: normalized(self.ended_by_user_id.as_deref()),
            created_at_ms,
            answered_at_ms,
            ended_at_ms,
            duration_ms: resolved_duration_ms,
            end_reason: normalized(self.end_reason.as_deref()),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct MessageWireDTO {
    pub id: Option<String>,
    pub server_id: Option<String>,
    pub chat_id: Option<String>,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub content: Option<String>,
    pub original_content: Option<String>,
    pub content_type: Option<String>,
    pub caption: Option<String>,
    pub created_at: Option<f64>,
    pub timestamp: Option<f64>,
    pub is_pinned: Option<bool>,
    pub is_read: Option<bool>,
    pub played_by: Option<Vec<String>>,
    pub is_mentioned_you: Option<bool>,
    pub is_replied_you: Option<bool>,
    pub is_system_message: Option<bool>,
    pub system_message_type: Option<String>,
    pub system_actor_info: Option<MessageSystemActorWireDTO>,
    pub system_wallpaper: Option<MessageSystemWallpaperWireDTO>,
    pub system_gift: Option<MessageSystemGiftWireDTO>,
    pub system_gift_item: Option<MessageSystemGiftItemWireDTO>,
    pub system_target: Option<MessageSystemMetadataTargetEnvelope>,
    pub is_forwarded: Option<bool>,
    pub is_reply: Option<bool>,
    pub is_edited: Option<bool>,
    pub edit_date: Option<f64>,
    pub has_attachments: Option<bool>,
    pub has_protected_content: Option<bool>,
    pub has_reactions: Option<bool>,
    pub reactions_unreaded: Option<bool>,
    pub media_group_id: Option<String>,
    pub view_count: Option<i64>,
    pub entities: Option<String>,
    pub media_info: Option<MessageMediaInfoWireDTO>,
    pub media_items: Option<Vec<MessageMediaInfoWireDTO>>,
    pub reply_to: Option<MessageReplyInfoWireDTO>,
    pub forward_info: Option<MessageForwardInfoWireDTO>,
    pub link_preview: Option<MessageLinkMetadataWireDTO>,
    pub reactions: Option<Vec<MessageReactionSummaryWireDTO>>,
    pub reacted_by: Option<Vec<MessageReactedByWireDTO>>,
    pub sender: Option<MessageSenderWireDTO>,
    pub shared_post: Option<MessageSharedPostWireDTO>,
    pub story_reply: Option<MessageStoryReplyWireDTO>,
    pub document_raw: Option<String>,
    pub moderation_status: Option<String>,
    pub temp_id: Option<String>,
    pub is_temp: Option<bool>,
    pub reply_markup: Option<MessageBotReplyMarkupWireDTO>,
    pub is_bot_message: Option<bool>,
    pub via_bot: Option<String>,
    pub via_bot_tag: Option<String>,
    pub web_app_data: Option<MessageBotWebAppDataWireDTO>,
    pub metadata: Option<MessageCallInfoWireDTO>,
    pub topic_id: Option<String>,
    pub discussion_chat_id: Option<String>,
    pub discussion_message_id: Option<String>,
    pub comments_count: Option<i64>,
    pub recent_commenters: Option<Vec<CommenterPreviewWireDTO>>,
    pub sparks_total: Option<i64>,
    pub my_sparks: Option<i64>,
    pub author_signature: Option<String>,
}

impl<'de> Deserialize<'de> for MessageWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                id: c.wire_string(bot_key::ID),
                server_id: c.wire_string(RiseLocalKey::SERVER_ID),
                chat_id: c.wire_string(bot_key::CHAT_ID),
                sender_id: c.wire_string(bot_key::SENDER_ID),
                sender_name: c.wire_string(bot_key::SENDER_NAME),
                content: c.wire_string(bot_key::CONTENT),
                original_content: c.wire_string(bot_key::ORIGINAL_CONTENT),
                content_type: c.wire_string(bot_key::CONTENT_TYPE),
                caption: c.wire_string(bot_key::CAPTION),
                created_at: c.wire_double(bot_key::CREATED_AT),
                timestamp: c.wire_double(bot_key::TIMESTAMP),
                is_pinned: c.wire_bool(bot_key::IS_PINNED),
                is_read: c.wire_bool(bot_key::IS_READ),
                played_by: c.wire_value::<Vec<String>>(bot_key::PLAYED_BY),
                is_mentioned_you: c.wire_bool(bot_key::IS_MENTIONED_YOU),
                is_replied_you: c.wire_bool(key::IS_REPLIED_YOU),
                is_system_message: c.wire_bool(bot_key::IS_SYSTEM_MESSAGE),
                system_message_type: c.wire_string(bot_key::SYSTEM_MESSAGE_TYPE),
                system_actor_info: c.wire_value::<MessageSystemActorWireDTO>(bot_key::SYSTEM_ACTOR_INFO),
                system_wallpaper: c.wire_value::<MessageSystemMetadataWallpaperEnvelope>(bot_key::METADATA).and_then(|envelope| envelope.wallpaper),
                system_gift: c.wire_value::<MessageSystemMetadataGiftEnvelope>(bot_key::METADATA).and_then(|envelope| envelope.gift),
                system_gift_item: c.wire_value::<MessageSystemMetadataGiftItemEnvelope>(bot_key::METADATA).and_then(|envelope| envelope.gift_item),
                system_target: c
                    .wire_value::<MessageSystemMetadataTargetEnvelope>(bot_key::METADATA)
                    .filter(|target| !target.is_empty()),
                is_forwarded: c.wire_bool(bot_key::IS_FORWARDED),
                is_reply: c.wire_bool(bot_key::IS_REPLY),
                is_edited: c.wire_bool(bot_key::IS_EDITED),
                edit_date: c.wire_double(bot_key::EDIT_DATE),
                has_attachments: c.wire_bool(bot_key::HAS_ATTACHMENTS),
                has_protected_content: c.wire_bool(bot_key::HAS_PROTECTED_CONTENT),
                has_reactions: c.wire_bool(bot_key::HAS_REACTIONS),
                reactions_unreaded: c.wire_bool(bot_key::REACTIONS_UNREADED),
                media_group_id: c.wire_string(bot_key::MEDIA_GROUP_ID),
                view_count: c.wire_int(bot_key::VIEW_COUNT),
                entities: c.wire_string(bot_key::ENTITIES),
                media_info: c.wire_value::<MessageMediaInfoWireDTO>(bot_key::MEDIA_INFO),
                media_items: c.wire_value::<Vec<MessageMediaInfoWireDTO>>(bot_key::MEDIA_ITEMS),
                reply_to: c.wire_value::<MessageReplyInfoWireDTO>(bot_key::REPLY_TO),
                forward_info: c.wire_value::<MessageForwardInfoWireDTO>(bot_key::FORWARD_INFO),
                link_preview: c.wire_value::<MessageLinkMetadataWireDTO>(bot_key::LINK_PREVIEW),
                reactions: c.wire_value::<Vec<MessageReactionSummaryWireDTO>>(bot_key::REACTIONS),
                reacted_by: c.wire_value::<Vec<MessageReactedByWireDTO>>(bot_key::REACTED_BY),
                sender: c.wire_value::<MessageSenderWireDTO>(bot_key::SENDER),
                shared_post: c.wire_shared_post(),
                story_reply: c.wire_story_reply(),
                document_raw: c.wire_document(),
                moderation_status: c.wire_string(bot_key::MODERATION_STATUS),
                temp_id: c.wire_string(bot_key::TEMP_ID),
                is_temp: c.wire_bool(RiseLocalKey::IS_TEMP),
                reply_markup: c.wire_value::<MessageBotReplyMarkupWireDTO>(bot_key::REPLY_MARKUP),
                is_bot_message: c.wire_bool(bot_key::IS_BOT_MESSAGE),
                via_bot: c.wire_string(bot_key::VIA_BOT),
                via_bot_tag: c.wire_string(bot_key::VIA_BOT_TAG),
                web_app_data: c.wire_value::<MessageBotWebAppDataWireDTO>(bot_key::WEB_APP_DATA),
                metadata: c.wire_value::<MessageCallInfoWireDTO>(bot_key::METADATA),
                topic_id: c.wire_string(bot_key::TOPIC_ID),
                discussion_chat_id: c.wire_string(bot_key::DISCUSSION_CHAT_ID),
                discussion_message_id: c.wire_string(bot_key::DISCUSSION_MESSAGE_ID),
                comments_count: c.wire_int(bot_key::COMMENTS_COUNT),
                recent_commenters: c.wire_recent_commenters(),
                sparks_total: c.wire_value::<i64>(bot_key::SPARKS_TOTAL),
                my_sparks: c.wire_value::<i64>(bot_key::MY_SPARKS),
                author_signature: c.wire_string(key::AUTHOR_SIGNATURE),
            })
        })
    }
}

impl Serialize for MessageWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.id {
            container.serialize_entry(bot_key::ID, value)?;
        }
        if let Some(value) = &self.server_id {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::SERVER_ID), value)?;
        }
        if let Some(value) = &self.chat_id {
            container.serialize_entry(&RiseWireKey::camel(bot_key::CHAT_ID), value)?;
        }
        if let Some(value) = &self.sender_id {
            container.serialize_entry(&RiseWireKey::camel(bot_key::SENDER_ID), value)?;
        }
        if let Some(value) = &self.sender_name {
            container.serialize_entry(&RiseWireKey::camel(bot_key::SENDER_NAME), value)?;
        }
        if let Some(value) = &self.content {
            container.serialize_entry(bot_key::CONTENT, value)?;
        }
        if let Some(value) = &self.original_content {
            container.serialize_entry(&RiseWireKey::camel(bot_key::ORIGINAL_CONTENT), value)?;
        }
        if let Some(value) = &self.content_type {
            container.serialize_entry(&RiseWireKey::camel(bot_key::CONTENT_TYPE), value)?;
        }
        if let Some(value) = &self.caption {
            container.serialize_entry(bot_key::CAPTION, value)?;
        }
        if let Some(value) = &self.created_at {
            container.serialize_entry(&RiseWireKey::camel(bot_key::CREATED_AT), value)?;
        }
        if let Some(value) = &self.timestamp {
            container.serialize_entry(bot_key::TIMESTAMP, value)?;
        }
        if let Some(value) = &self.is_pinned {
            container.serialize_entry(&RiseWireKey::camel(bot_key::IS_PINNED), value)?;
        }
        if let Some(value) = &self.is_read {
            container.serialize_entry(&RiseWireKey::camel(bot_key::IS_READ), value)?;
        }
        if let Some(value) = &self.played_by {
            container.serialize_entry(&RiseWireKey::camel(bot_key::PLAYED_BY), value)?;
        }
        if let Some(value) = &self.is_mentioned_you {
            container.serialize_entry(&RiseWireKey::camel(bot_key::IS_MENTIONED_YOU), value)?;
        }
        if let Some(value) = &self.is_replied_you {
            container.serialize_entry(&RiseWireKey::camel(key::IS_REPLIED_YOU), value)?;
        }
        if let Some(value) = &self.is_system_message {
            container.serialize_entry(&RiseWireKey::camel(bot_key::IS_SYSTEM_MESSAGE), value)?;
        }
        if let Some(value) = &self.system_message_type {
            container.serialize_entry(&RiseWireKey::camel(bot_key::SYSTEM_MESSAGE_TYPE), value)?;
        }
        if let Some(value) = &self.system_actor_info {
            container.serialize_entry(&RiseWireKey::camel(bot_key::SYSTEM_ACTOR_INFO), value)?;
        }
        if let Some(value) = &self.is_forwarded {
            container.serialize_entry(&RiseWireKey::camel(bot_key::IS_FORWARDED), value)?;
        }
        if let Some(value) = &self.is_reply {
            container.serialize_entry(&RiseWireKey::camel(bot_key::IS_REPLY), value)?;
        }
        if let Some(value) = &self.is_edited {
            container.serialize_entry(&RiseWireKey::camel(bot_key::IS_EDITED), value)?;
        }
        if let Some(value) = &self.edit_date {
            container.serialize_entry(&RiseWireKey::camel(bot_key::EDIT_DATE), value)?;
        }
        if let Some(value) = &self.has_attachments {
            container.serialize_entry(&RiseWireKey::camel(bot_key::HAS_ATTACHMENTS), value)?;
        }
        if let Some(value) = &self.has_protected_content {
            container.serialize_entry(&RiseWireKey::camel(bot_key::HAS_PROTECTED_CONTENT), value)?;
        }
        if let Some(value) = &self.has_reactions {
            container.serialize_entry(&RiseWireKey::camel(bot_key::HAS_REACTIONS), value)?;
        }
        if let Some(value) = &self.reactions_unreaded {
            container.serialize_entry(&RiseWireKey::camel(bot_key::REACTIONS_UNREADED), value)?;
        }
        if let Some(value) = &self.media_group_id {
            container.serialize_entry(&RiseWireKey::camel(bot_key::MEDIA_GROUP_ID), value)?;
        }
        if let Some(value) = &self.view_count {
            container.serialize_entry(&RiseWireKey::camel(bot_key::VIEW_COUNT), value)?;
        }
        if let Some(value) = &self.entities {
            container.serialize_entry(bot_key::ENTITIES, value)?;
        }
        if let Some(value) = &self.media_info {
            container.serialize_entry(&RiseWireKey::camel(bot_key::MEDIA_INFO), value)?;
        }
        if let Some(value) = &self.media_items {
            container.serialize_entry(&RiseWireKey::camel(bot_key::MEDIA_ITEMS), value)?;
        }
        if let Some(value) = &self.reply_to {
            container.serialize_entry(&RiseWireKey::camel(bot_key::REPLY_TO), value)?;
        }
        if let Some(value) = &self.forward_info {
            container.serialize_entry(&RiseWireKey::camel(bot_key::FORWARD_INFO), value)?;
        }
        if let Some(value) = &self.link_preview {
            container.serialize_entry(&RiseWireKey::camel(bot_key::LINK_PREVIEW), value)?;
        }
        if let Some(value) = &self.reactions {
            container.serialize_entry(bot_key::REACTIONS, value)?;
        }
        if let Some(value) = &self.reacted_by {
            container.serialize_entry(&RiseWireKey::camel(bot_key::REACTED_BY), value)?;
        }
        if let Some(value) = &self.sender {
            container.serialize_entry(bot_key::SENDER, value)?;
        }
        if let Some(value) = &self.shared_post {
            container.serialize_entry(&RiseWireKey::camel(payload_key::SHARED_POST), value)?;
        }
        if let Some(value) = &self.story_reply {
            container.serialize_entry(&RiseWireKey::camel(events_key::STORY_REPLY), value)?;
        }
        if let Some(value) = &self.document_raw {
            container.serialize_entry(&RiseWireKey::camel(bot_key::DOCUMENT), value)?;
        }
        if let Some(value) = &self.moderation_status {
            container.serialize_entry(&RiseWireKey::camel(bot_key::MODERATION_STATUS), value)?;
        }
        if let Some(value) = &self.temp_id {
            container.serialize_entry(&RiseWireKey::camel(bot_key::TEMP_ID), value)?;
        }
        if let Some(value) = &self.is_temp {
            container.serialize_entry(&RiseWireKey::camel(RiseLocalKey::IS_TEMP), value)?;
        }
        if let Some(value) = &self.reply_markup {
            container.serialize_entry(&RiseWireKey::camel(bot_key::REPLY_MARKUP), value)?;
        }
        if let Some(value) = &self.is_bot_message {
            container.serialize_entry(&RiseWireKey::camel(bot_key::IS_BOT_MESSAGE), value)?;
        }
        if let Some(value) = &self.via_bot {
            container.serialize_entry(&RiseWireKey::camel(bot_key::VIA_BOT), value)?;
        }
        if let Some(value) = &self.via_bot_tag {
            container.serialize_entry(&RiseWireKey::camel(bot_key::VIA_BOT_TAG), value)?;
        }
        if let Some(value) = &self.web_app_data {
            container.serialize_entry(&RiseWireKey::camel(bot_key::WEB_APP_DATA), value)?;
        }
        if let Some(value) = &self.topic_id {
            container.serialize_entry(&RiseWireKey::camel(bot_key::TOPIC_ID), value)?;
        }
        if let Some(value) = &self.discussion_chat_id {
            container.serialize_entry(&RiseWireKey::camel(bot_key::DISCUSSION_CHAT_ID), value)?;
        }
        if let Some(value) = &self.discussion_message_id {
            container.serialize_entry(&RiseWireKey::camel(bot_key::DISCUSSION_MESSAGE_ID), value)?;
        }
        if let Some(value) = &self.comments_count {
            container.serialize_entry(&RiseWireKey::camel(bot_key::COMMENTS_COUNT), value)?;
        }
        if let Some(value) = &self.recent_commenters {
            container.serialize_entry(&RiseWireKey::camel(bot_key::RECENT_COMMENTERS), value)?;
        }
        if let Some(value) = &self.sparks_total {
            container.serialize_entry(&RiseWireKey::camel(bot_key::SPARKS_TOTAL), value)?;
        }
        if let Some(value) = &self.my_sparks {
            container.serialize_entry(&RiseWireKey::camel(bot_key::MY_SPARKS), value)?;
        }
        let stored = MessageStoredMetadata {
            call: self.metadata.as_ref(),
            wallpaper: self.system_wallpaper.as_ref(),
            gift: self.system_gift.as_ref(),
            gift_item: self.system_gift_item.as_ref(),
            target: self.system_target.as_ref(),
        };
        if !stored.is_empty() {
            container.serialize_entry(bot_key::METADATA, &stored)?;
        }
        container.end()
    }
}

struct MessageStoredMetadata<'a> {
    call: Option<&'a MessageCallInfoWireDTO>,
    wallpaper: Option<&'a MessageSystemWallpaperWireDTO>,
    gift: Option<&'a MessageSystemGiftWireDTO>,
    gift_item: Option<&'a MessageSystemGiftItemWireDTO>,
    target: Option<&'a MessageSystemMetadataTargetEnvelope>,
}

impl MessageStoredMetadata<'_> {
    fn is_empty(&self) -> bool {
        self.call.is_none()
            && self.wallpaper.is_none()
            && self.gift.is_none()
            && self.gift_item.is_none()
            && self.target.is_none()
    }

    fn merge_object<S: Serializer>(
        object: &mut serde_json::Map<String, serde_json::Value>,
        value: &impl Serialize,
    ) -> Result<(), S::Error> {
        if let serde_json::Value::Object(members) = serde_json::to_value(value).map_err(S::Error::custom)? {
            object.extend(members);
        }
        Ok(())
    }

    fn insert_member<S: Serializer>(
        object: &mut serde_json::Map<String, serde_json::Value>,
        member: &str,
        value: Option<&impl Serialize>,
    ) -> Result<(), S::Error> {
        if let Some(value) = value {
            object.insert(member.to_owned(), serde_json::to_value(value).map_err(S::Error::custom)?);
        }
        Ok(())
    }
}

impl Serialize for MessageStoredMetadata<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut object = serde_json::Map::new();
        if let Some(call) = self.call {
            Self::merge_object::<S>(&mut object, call)?;
        }
        if let Some(target) = self.target {
            Self::merge_object::<S>(&mut object, target)?;
        }
        Self::insert_member::<S>(&mut object, chat_action_key::WALLPAPER, self.wallpaper)?;
        Self::insert_member::<S>(&mut object, chat_action_key::GIFT, self.gift)?;
        Self::insert_member::<S>(&mut object, chat_action_key::GIFT_ITEM, self.gift_item)?;
        object.serialize(serializer)
    }
}

trait MessageWireObjectReads {
    fn wire_value<T: DeserializeOwned>(&self, key: &str) -> Option<T>;
    fn wire_string(&self, key: &str) -> Option<String>;
    fn wire_double(&self, key: &str) -> Option<f64>;
    fn wire_int(&self, key: &str) -> Option<i64>;
    fn wire_bool(&self, key: &str) -> Option<bool>;
    fn wire_blob<T: DeserializeOwned>(&self, keys: &[&str]) -> Option<T>;
    fn wire_shared_post(&self) -> Option<MessageSharedPostWireDTO>;
    fn wire_document(&self) -> Option<String>;
    fn wire_story_reply(&self) -> Option<MessageStoryReplyWireDTO>;
    fn wire_recent_commenters(&self) -> Option<Vec<CommenterPreviewWireDTO>>;
}

impl MessageWireObjectReads for RiseWireObject {
    fn wire_value<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.rise::<T>(key).ok().flatten()
    }

    fn wire_string(&self, key: &str) -> Option<String> {
        if let Some(value) = self.wire_value::<String>(key) {
            return Some(value);
        }
        if let Some(value) = self.wire_value::<i64>(key) {
            return Some(value.to_string());
        }
        if let Some(value) = self.wire_value::<u64>(key) {
            return Some(value.to_string());
        }
        None
    }

    fn wire_double(&self, key: &str) -> Option<f64> {
        if let Some(value) = self.wire_value::<f64>(key) {
            return Some(value);
        }
        if let Some(value) = self.wire_value::<String>(key) {
            return double_from_string(&value);
        }
        None
    }

    fn wire_int(&self, key: &str) -> Option<i64> {
        if let Some(value) = self.wire_value::<i64>(key) {
            return Some(value);
        }
        if let Some(value) = self.wire_value::<String>(key) {
            return int_from_string(&value);
        }
        None
    }

    fn wire_bool(&self, key: &str) -> Option<bool> {
        if let Some(value) = self.wire_value::<bool>(key) {
            return Some(value);
        }
        if let Some(value) = self.wire_value::<i64>(key) {
            return Some(value != 0);
        }
        if let Some(value) = self.wire_value::<String>(key) {
            return match value.to_lowercase().as_str() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                _ => None,
            };
        }
        None
    }

    fn wire_blob<T: DeserializeOwned>(&self, keys: &[&str]) -> Option<T> {
        for key in keys {
            if let Some(direct) = self.wire_value::<T>(key) {
                return Some(direct);
            }
            if let Some(raw) = self.wire_value::<String>(key)
                && let Ok(decoded) = serde_json::from_str::<T>(&raw)
            {
                return Some(decoded);
            }
        }
        None
    }

    fn wire_shared_post(&self) -> Option<MessageSharedPostWireDTO> {
        self.wire_blob::<MessageSharedPostWireDTO>(&[payload_key::SHARED_POST, key::SHARED_POST_INFO])
    }

    fn wire_document(&self) -> Option<String> {
        let document_key = bot_key::DOCUMENT;
        if let Some(raw) = self.wire_value::<String>(document_key) {
            if raw.is_empty() {
                return None;
            }
            if let Ok(object) = RiseJSONValue::decode(raw.as_bytes())
                && matches!(object, RiseJSONValue::Object(_))
                && let Ok(canonical) = object.data(true)
            {
                return String::from_utf8(canonical).ok();
            }
            return Some(raw);
        }
        let object = self.wire_value::<RiseJSONValue>(document_key)?;
        if !matches!(object, RiseJSONValue::Object(_)) {
            return None;
        }
        let data = object.data(true).ok()?;
        String::from_utf8(data).ok()
    }

    fn wire_story_reply(&self) -> Option<MessageStoryReplyWireDTO> {
        self.wire_blob::<MessageStoryReplyWireDTO>(&[events_key::STORY_REPLY, key::STORY_REPLY_INFO])
    }

    fn wire_recent_commenters(&self) -> Option<Vec<CommenterPreviewWireDTO>> {
        self.wire_blob::<Vec<CommenterPreviewWireDTO>>(&[bot_key::RECENT_COMMENTERS])
    }
}

#[derive(Clone, Debug)]
pub struct CommenterPreviewWireDTO {
    pub logo: Option<String>,
    pub name: Option<String>,
}

impl<'de> Deserialize<'de> for CommenterPreviewWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                logo: c.rise::<String>(recent_commenter_key::LOGO)?,
                name: c.rise::<String>(recent_commenter_key::NAME)?,
            })
        })
    }
}

impl Serialize for CommenterPreviewWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.logo {
            container.serialize_entry(recent_commenter_key::LOGO, value)?;
        }
        if let Some(value) = &self.name {
            container.serialize_entry(recent_commenter_key::NAME, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct GetMessagesGatewayDTO {
    pub chat_id: Option<String>,
    pub limit: Option<i64>,
    pub relative_id: Option<String>,
    pub is_have_more: Option<bool>,
    pub messages: Option<Vec<MessageWireDTO>>,
    pub success: Option<bool>,
    pub error: Option<String>,
}

impl<'de> Deserialize<'de> for GetMessagesGatewayDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                chat_id: c.rise::<String>(key::CHAT_ID)?,
                limit: c.rise::<i64>(key::LIMIT)?,
                relative_id: c.rise::<String>(key::RELATIVE_ID)?,
                is_have_more: c.rise::<bool>(key::IS_HAVE_MORE)?,
                messages: c.rise::<Vec<MessageWireDTO>>(key::MESSAGES)?,
                success: c.rise::<bool>(key::SUCCESS)?,
                error: c.rise::<String>(key::ERROR)?,
            })
        })
    }
}

impl Serialize for GetMessagesGatewayDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.chat_id {
            container.serialize_entry(&RiseWireKey::camel(key::CHAT_ID), value)?;
        }
        if let Some(value) = &self.limit {
            container.serialize_entry(key::LIMIT, value)?;
        }
        if let Some(value) = &self.relative_id {
            container.serialize_entry(&RiseWireKey::camel(key::RELATIVE_ID), value)?;
        }
        if let Some(value) = &self.is_have_more {
            container.serialize_entry(&RiseWireKey::camel(key::IS_HAVE_MORE), value)?;
        }
        if let Some(value) = &self.messages {
            container.serialize_entry(key::MESSAGES, value)?;
        }
        if let Some(value) = &self.success {
            container.serialize_entry(key::SUCCESS, value)?;
        }
        if let Some(value) = &self.error {
            container.serialize_entry(key::ERROR, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct GetMessagesAroundGatewayDTO {
    pub success: Option<bool>,
    pub error: Option<String>,
    pub chat_id: Option<String>,
    pub anchor_message_id: Option<String>,
    pub anchor_relative_id: Option<String>,
    pub before_limit: Option<i64>,
    pub after_limit: Option<i64>,
    pub older_relative_id: Option<String>,
    pub has_older: Option<bool>,
    pub newer_relative_id: Option<String>,
    pub has_newer: Option<bool>,
    pub messages: Option<Vec<MessageWireDTO>>,
}

impl<'de> Deserialize<'de> for GetMessagesAroundGatewayDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                success: c.rise::<bool>(key::SUCCESS)?,
                error: c.rise::<String>(key::ERROR)?,
                chat_id: c.rise::<String>(key::CHAT_ID)?,
                anchor_message_id: c.rise::<String>(key::ANCHOR_MESSAGE_ID)?,
                anchor_relative_id: c.rise::<String>(key::ANCHOR_RELATIVE_ID)?,
                before_limit: c.rise::<i64>(key::BEFORE_LIMIT)?,
                after_limit: c.rise::<i64>(key::AFTER_LIMIT)?,
                older_relative_id: c.rise::<String>(key::OLDER_RELATIVE_ID)?,
                has_older: c.rise::<bool>(key::HAS_OLDER)?,
                newer_relative_id: c.rise::<String>(key::NEWER_RELATIVE_ID)?,
                has_newer: c.rise::<bool>(key::HAS_NEWER)?,
                messages: c.rise::<Vec<MessageWireDTO>>(key::MESSAGES)?,
            })
        })
    }
}

impl Serialize for GetMessagesAroundGatewayDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.success {
            container.serialize_entry(key::SUCCESS, value)?;
        }
        if let Some(value) = &self.error {
            container.serialize_entry(key::ERROR, value)?;
        }
        if let Some(value) = &self.chat_id {
            container.serialize_entry(&RiseWireKey::camel(key::CHAT_ID), value)?;
        }
        if let Some(value) = &self.anchor_message_id {
            container.serialize_entry(&RiseWireKey::camel(key::ANCHOR_MESSAGE_ID), value)?;
        }
        if let Some(value) = &self.anchor_relative_id {
            container.serialize_entry(&RiseWireKey::camel(key::ANCHOR_RELATIVE_ID), value)?;
        }
        if let Some(value) = &self.before_limit {
            container.serialize_entry(&RiseWireKey::camel(key::BEFORE_LIMIT), value)?;
        }
        if let Some(value) = &self.after_limit {
            container.serialize_entry(&RiseWireKey::camel(key::AFTER_LIMIT), value)?;
        }
        if let Some(value) = &self.older_relative_id {
            container.serialize_entry(&RiseWireKey::camel(key::OLDER_RELATIVE_ID), value)?;
        }
        if let Some(value) = &self.has_older {
            container.serialize_entry(&RiseWireKey::camel(key::HAS_OLDER), value)?;
        }
        if let Some(value) = &self.newer_relative_id {
            container.serialize_entry(&RiseWireKey::camel(key::NEWER_RELATIVE_ID), value)?;
        }
        if let Some(value) = &self.has_newer {
            container.serialize_entry(&RiseWireKey::camel(key::HAS_NEWER), value)?;
        }
        if let Some(value) = &self.messages {
            container.serialize_entry(key::MESSAGES, value)?;
        }
        container.end()
    }
}

pub type ChatCalendarDayWireDTO = view::CalendarDay;

pub type GetChatCalendarGatewayDTO = view::Calendar;

#[derive(Clone, Debug)]
pub struct CreateMessageGatewayDTO {
    pub message_id: Option<String>,
    pub id: Option<String>,
    pub chat_id: Option<String>,
    pub sender_id: Option<String>,
    pub created_at: Option<f64>,
    pub timestamp: Option<f64>,
    pub temp_id: Option<String>,
    pub message: Option<MessageWireDTO>,
    pub messages: Option<Vec<MessageWireDTO>>,
    pub success: Option<bool>,
    pub error: Option<String>,
}

impl<'de> Deserialize<'de> for CreateMessageGatewayDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                message_id: c.rise::<String>(key::MESSAGE_ID)?,
                id: c.rise::<String>(key::ID)?,
                chat_id: c.rise::<String>(key::CHAT_ID)?,
                sender_id: c.rise::<String>(key::SENDER_ID)?,
                created_at: c.rise::<f64>(key::CREATED_AT)?,
                timestamp: c.rise::<f64>(key::TIMESTAMP)?,
                temp_id: c.rise::<String>(key::TEMP_ID)?,
                message: c.rise::<MessageWireDTO>(key::MESSAGE)?,
                messages: c.rise::<Vec<MessageWireDTO>>(key::MESSAGES)?,
                success: c.rise::<bool>(key::SUCCESS)?,
                error: c.rise::<String>(key::ERROR)?,
            })
        })
    }
}

impl Serialize for CreateMessageGatewayDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.message_id {
            container.serialize_entry(&RiseWireKey::camel(key::MESSAGE_ID), value)?;
        }
        if let Some(value) = &self.id {
            container.serialize_entry(key::ID, value)?;
        }
        if let Some(value) = &self.chat_id {
            container.serialize_entry(&RiseWireKey::camel(key::CHAT_ID), value)?;
        }
        if let Some(value) = &self.sender_id {
            container.serialize_entry(&RiseWireKey::camel(key::SENDER_ID), value)?;
        }
        if let Some(value) = &self.created_at {
            container.serialize_entry(&RiseWireKey::camel(key::CREATED_AT), value)?;
        }
        if let Some(value) = &self.timestamp {
            container.serialize_entry(key::TIMESTAMP, value)?;
        }
        if let Some(value) = &self.temp_id {
            container.serialize_entry(&RiseWireKey::camel(key::TEMP_ID), value)?;
        }
        if let Some(value) = &self.message {
            container.serialize_entry(key::MESSAGE, value)?;
        }
        if let Some(value) = &self.messages {
            container.serialize_entry(key::MESSAGES, value)?;
        }
        if let Some(value) = &self.success {
            container.serialize_entry(key::SUCCESS, value)?;
        }
        if let Some(value) = &self.error {
            container.serialize_entry(key::ERROR, value)?;
        }
        container.end()
    }
}

pub type DeleteMessagesGatewayDTO = proto::ProtoDeleteMessagesResponse;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ForwardMessageDestinationWireDTO {
    pub chat_id: String,
    pub topic_id: Option<String>,
}

impl<'de> Deserialize<'de> for ForwardMessageDestinationWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                chat_id: c.rise_required::<String>(key::CHAT_ID)?,
                topic_id: c.rise::<String>(key::TOPIC_ID)?,
            })
        })
    }
}

impl Serialize for ForwardMessageDestinationWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(&RiseWireKey::camel(key::CHAT_ID), &self.chat_id)?;
        if let Some(value) = &self.topic_id {
            container.serialize_entry(&RiseWireKey::camel(key::TOPIC_ID), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct ForwardMessageResultWireDTO {
    pub source_message_id: String,
    pub destination: Option<ForwardMessageDestinationWireDTO>,
    pub success: bool,
    pub error: Option<String>,
    pub error_code: Option<String>,
    pub error_details: Option<FanoutRefusalDetailsWireDTO>,
    pub retryable: bool,
    pub message_id: Option<String>,
    pub message: Option<MessageWireDTO>,
}

impl<'de> Deserialize<'de> for ForwardMessageResultWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                source_message_id: c.rise_required::<String>(key::SOURCE_MESSAGE_ID)?,
                destination: c.rise::<ForwardMessageDestinationWireDTO>(key::DESTINATION)?,
                success: c.rise_required::<bool>(key::SUCCESS)?,
                error: c.rise::<String>(key::ERROR)?,
                error_code: c.rise::<String>(key::ERROR_CODE)?,
                error_details: c.rise::<FanoutRefusalDetailsWireDTO>(key::ERROR_DETAILS)?,
                retryable: c.rise_required::<bool>(key::RETRYABLE)?,
                message_id: c.rise::<String>(key::MESSAGE_ID)?,
                message: c.rise::<MessageWireDTO>(key::MESSAGE)?,
            })
        })
    }
}

impl Serialize for ForwardMessageResultWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(&RiseWireKey::camel(key::SOURCE_MESSAGE_ID), &self.source_message_id)?;
        if let Some(value) = &self.destination {
            container.serialize_entry(key::DESTINATION, value)?;
        }
        container.serialize_entry(key::SUCCESS, &self.success)?;
        if let Some(value) = &self.error {
            container.serialize_entry(key::ERROR, value)?;
        }
        if let Some(value) = &self.error_code {
            container.serialize_entry(&RiseWireKey::camel(key::ERROR_CODE), value)?;
        }
        if let Some(value) = &self.error_details {
            container.serialize_entry(&RiseWireKey::camel(key::ERROR_DETAILS), value)?;
        }
        container.serialize_entry(key::RETRYABLE, &self.retryable)?;
        if let Some(value) = &self.message_id {
            container.serialize_entry(&RiseWireKey::camel(key::MESSAGE_ID), value)?;
        }
        if let Some(value) = &self.message {
            container.serialize_entry(key::MESSAGE, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct FanoutRefusalDetailsWireDTO {
    pub retry_after_s: Option<String>,
}

impl<'de> Deserialize<'de> for FanoutRefusalDetailsWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                retry_after_s: c.rise::<String>(payload_key::RETRY_AFTER_S)?,
            })
        })
    }
}

impl Serialize for FanoutRefusalDetailsWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.retry_after_s {
            container.serialize_entry(&RiseWireKey::camel(payload_key::RETRY_AFTER_S), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct ForwardMessagesGatewayDTO {
    pub success: Option<bool>,
    pub error: Option<String>,
    pub forwarded_messages: Option<HashMap<String, String>>,
    pub idempotency_key: Option<String>,
    pub all_succeeded: Option<bool>,
    pub results: Option<Vec<ForwardMessageResultWireDTO>>,
}

impl<'de> Deserialize<'de> for ForwardMessagesGatewayDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                success: c.rise::<bool>(key::SUCCESS)?,
                error: c.rise::<String>(key::ERROR)?,
                forwarded_messages: c.rise::<HashMap<String, String>>(key::FORWARDED_MESSAGES)?,
                idempotency_key: c.rise::<String>(key::IDEMPOTENCY_KEY)?,
                all_succeeded: c.rise::<bool>(key::ALL_SUCCEEDED)?,
                results: c.rise::<Vec<ForwardMessageResultWireDTO>>(key::RESULTS)?,
            })
        })
    }
}

impl Serialize for ForwardMessagesGatewayDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.success {
            container.serialize_entry(key::SUCCESS, value)?;
        }
        if let Some(value) = &self.error {
            container.serialize_entry(key::ERROR, value)?;
        }
        if let Some(value) = &self.forwarded_messages {
            container.serialize_entry(&RiseWireKey::camel(key::FORWARDED_MESSAGES), value)?;
        }
        if let Some(value) = &self.idempotency_key {
            container.serialize_entry(&RiseWireKey::camel(key::IDEMPOTENCY_KEY), value)?;
        }
        if let Some(value) = &self.all_succeeded {
            container.serialize_entry(&RiseWireKey::camel(key::ALL_SUCCEEDED), value)?;
        }
        if let Some(value) = &self.results {
            container.serialize_entry(key::RESULTS, value)?;
        }
        container.end()
    }
}

impl FanoutRefusalDetailsWireDTO {
    pub fn canonical(&self) -> HashMap<String, String> {
        let Some(retry_after_s) = &self.retry_after_s else {
            return HashMap::new();
        };
        HashMap::from([(ErrorParam::RetryAfterS.wire().to_owned(), retry_after_s.clone())])
    }
}

pub trait RiseFailureFanout: Sized {
    fn from_fanout_code(
        code: Option<&str>,
        message: Option<&str>,
        details: Option<&FanoutRefusalDetailsWireDTO>,
    ) -> Option<Self>;
}

impl RiseFailureFanout for RiseFailure {
    fn from_fanout_code(
        code: Option<&str>,
        message: Option<&str>,
        details: Option<&FanoutRefusalDetailsWireDTO>,
    ) -> Option<Self> {
        let code = code?;
        if rise_errors::is_absent_code(code) {
            return None;
        }
        Some(Self::new(
            Some(code.to_owned()),
            message.unwrap_or_default().to_owned(),
            details.map(FanoutRefusalDetailsWireDTO::canonical).unwrap_or_default(),
        ))
    }
}

impl ForwardMessageResultWireDTO {
    pub fn refusal(&self) -> Option<RiseFailure> {
        RiseFailure::from_fanout_code(self.error_code.as_deref(), self.error.as_deref(), self.error_details.as_ref())
    }
}

pub type MarkChatAsReadGatewayDTO = view::ChatRead;

pub type EditMessageGatewayDTO = view::Edited;

pub type UnpinAllMessagesGatewayDTO = proto::ProtoUnpinAllMessagesResponse;

pub type DeleteAllMessagesFromUserGatewayDTO = proto::ProtoDeleteAllMessagesFromUserResponse;

#[derive(Clone, Debug)]
pub struct MediaItemWireDTO {
    pub id: Option<String>,
    pub message_id: Option<String>,
    pub chat_id: Option<String>,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub sender: Option<MessageSenderWireDTO>,
    pub caption: Option<String>,
    pub content_type: Option<String>,
    pub created_at: Option<f64>,
    pub timestamp: Option<f64>,
    pub media_info: Option<MessageMediaInfoWireDTO>,
    pub media_index: Option<i64>,
    pub total_media_in_message: Option<i64>,
    pub has_protected_content: Option<bool>,
    pub topic_id: Option<String>,
}

impl<'de> Deserialize<'de> for MediaItemWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                id: c.rise::<String>(key::ID)?,
                message_id: c.rise::<String>(key::MESSAGE_ID)?,
                chat_id: c.rise::<String>(key::CHAT_ID)?,
                sender_id: c.rise::<String>(key::SENDER_ID)?,
                sender_name: c.rise::<String>(key::SENDER_NAME)?,
                sender: c.rise::<MessageSenderWireDTO>(key::SENDER)?,
                caption: c.rise::<String>(key::CAPTION)?,
                content_type: c.rise::<String>(key::CONTENT_TYPE)?,
                created_at: c.rise::<f64>(key::CREATED_AT)?,
                timestamp: c.rise::<f64>(key::TIMESTAMP)?,
                media_info: c.rise::<MessageMediaInfoWireDTO>(key::MEDIA_INFO)?,
                media_index: c.rise::<i64>(key::MEDIA_INDEX)?,
                total_media_in_message: c.rise::<i64>(key::TOTAL_MEDIA_IN_MESSAGE)?,
                has_protected_content: c.rise::<bool>(key::HAS_PROTECTED_CONTENT)?,
                topic_id: c.rise::<String>(key::TOPIC_ID)?,
            })
        })
    }
}

impl Serialize for MediaItemWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.id {
            container.serialize_entry(key::ID, value)?;
        }
        if let Some(value) = &self.message_id {
            container.serialize_entry(&RiseWireKey::camel(key::MESSAGE_ID), value)?;
        }
        if let Some(value) = &self.chat_id {
            container.serialize_entry(&RiseWireKey::camel(key::CHAT_ID), value)?;
        }
        if let Some(value) = &self.sender_id {
            container.serialize_entry(&RiseWireKey::camel(key::SENDER_ID), value)?;
        }
        if let Some(value) = &self.sender_name {
            container.serialize_entry(&RiseWireKey::camel(key::SENDER_NAME), value)?;
        }
        if let Some(value) = &self.sender {
            container.serialize_entry(key::SENDER, value)?;
        }
        if let Some(value) = &self.caption {
            container.serialize_entry(key::CAPTION, value)?;
        }
        if let Some(value) = &self.content_type {
            container.serialize_entry(&RiseWireKey::camel(key::CONTENT_TYPE), value)?;
        }
        if let Some(value) = &self.created_at {
            container.serialize_entry(&RiseWireKey::camel(key::CREATED_AT), value)?;
        }
        if let Some(value) = &self.timestamp {
            container.serialize_entry(key::TIMESTAMP, value)?;
        }
        if let Some(value) = &self.media_info {
            container.serialize_entry(&RiseWireKey::camel(key::MEDIA_INFO), value)?;
        }
        if let Some(value) = &self.media_index {
            container.serialize_entry(&RiseWireKey::camel(key::MEDIA_INDEX), value)?;
        }
        if let Some(value) = &self.total_media_in_message {
            container.serialize_entry(&RiseWireKey::camel(key::TOTAL_MEDIA_IN_MESSAGE), value)?;
        }
        if let Some(value) = &self.has_protected_content {
            container.serialize_entry(&RiseWireKey::camel(key::HAS_PROTECTED_CONTENT), value)?;
        }
        if let Some(value) = &self.topic_id {
            container.serialize_entry(key::TOPIC_ID, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug, Default)]
pub struct GetMediaMessagesGatewayDTO {
    pub success: Option<bool>,
    pub error: Option<String>,
    pub media_items: Option<Vec<MediaItemWireDTO>>,
    pub is_have_more: Option<bool>,
    pub next_cursor: Option<String>,
}

impl<'de> Deserialize<'de> for GetMediaMessagesGatewayDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                success: c.rise::<bool>(key::SUCCESS)?,
                error: c.rise::<String>(key::ERROR)?,
                media_items: c.rise::<Vec<MediaItemWireDTO>>(key::MEDIA_ITEMS)?,
                is_have_more: c.rise::<bool>(key::IS_HAVE_MORE)?,
                next_cursor: c.rise::<String>(key::NEXT_CURSOR)?,
            })
        })
    }
}

impl Serialize for GetMediaMessagesGatewayDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.success {
            container.serialize_entry(key::SUCCESS, value)?;
        }
        if let Some(value) = &self.error {
            container.serialize_entry(key::ERROR, value)?;
        }
        if let Some(value) = &self.media_items {
            container.serialize_entry(&RiseWireKey::camel(key::MEDIA_ITEMS), value)?;
        }
        if let Some(value) = &self.is_have_more {
            container.serialize_entry(&RiseWireKey::camel(key::IS_HAVE_MORE), value)?;
        }
        if let Some(value) = &self.next_cursor {
            container.serialize_entry(&RiseWireKey::camel(key::NEXT_CURSOR), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageSearchRangeWireDTO {
    pub start: i64,
    pub end: i64,
}

impl<'de> Deserialize<'de> for MessageSearchRangeWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                start: c.rise_required::<i64>(key::START)?,
                end: c.rise_required::<i64>(key::END)?,
            })
        })
    }
}

impl Serialize for MessageSearchRangeWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::START, &self.start)?;
        container.serialize_entry(key::END, &self.end)?;
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageSearchMatchWireDTO {
    pub message_id: Option<String>,
    pub ranges: Option<Vec<MessageSearchRangeWireDTO>>,
    pub snippet: Option<String>,
    pub snippet_ranges: Option<Vec<MessageSearchRangeWireDTO>>,
}

impl<'de> Deserialize<'de> for MessageSearchMatchWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                message_id: c.rise::<String>(key::MESSAGE_ID)?,
                ranges: c.rise::<Vec<MessageSearchRangeWireDTO>>(key::RANGES)?,
                snippet: c.rise::<String>(key::SNIPPET)?,
                snippet_ranges: c.rise::<Vec<MessageSearchRangeWireDTO>>(key::SNIPPET_RANGES)?,
            })
        })
    }
}

impl Serialize for MessageSearchMatchWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.message_id {
            container.serialize_entry(&RiseWireKey::camel(key::MESSAGE_ID), value)?;
        }
        if let Some(value) = &self.ranges {
            container.serialize_entry(key::RANGES, value)?;
        }
        if let Some(value) = &self.snippet {
            container.serialize_entry(key::SNIPPET, value)?;
        }
        if let Some(value) = &self.snippet_ranges {
            container.serialize_entry(&RiseWireKey::camel(key::SNIPPET_RANGES), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct MessageSearchChatWireDTO {
    pub id: Option<String>,
    pub title: Option<String>,
    pub r#type: Option<String>,
    pub logo_url: Option<String>,
    pub username: Option<String>,
    pub is_public: Option<bool>,
    pub is_verified: Option<bool>,
}

impl<'de> Deserialize<'de> for MessageSearchChatWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                id: c.rise::<String>(key::ID)?,
                title: c.rise::<String>(key::TITLE)?,
                r#type: c.rise::<String>(key::TYPE)?,
                logo_url: c.rise::<String>(key::LOGO_URL)?,
                username: c.rise::<String>(key::USERNAME)?,
                is_public: c.rise::<bool>(key::IS_PUBLIC)?,
                is_verified: c.rise::<bool>(key::IS_VERIFIED)?,
            })
        })
    }
}

impl Serialize for MessageSearchChatWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.id {
            container.serialize_entry(key::ID, value)?;
        }
        if let Some(value) = &self.title {
            container.serialize_entry(key::TITLE, value)?;
        }
        if let Some(value) = &self.r#type {
            container.serialize_entry(key::TYPE, value)?;
        }
        if let Some(value) = &self.logo_url {
            container.serialize_entry(&RiseWireKey::camel(key::LOGO_URL), value)?;
        }
        if let Some(value) = &self.username {
            container.serialize_entry(key::USERNAME, value)?;
        }
        if let Some(value) = &self.is_public {
            container.serialize_entry(&RiseWireKey::camel(key::IS_PUBLIC), value)?;
        }
        if let Some(value) = &self.is_verified {
            container.serialize_entry(&RiseWireKey::camel(key::IS_VERIFIED), value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug, Default)]
pub struct GetMessagesListGatewayDTO {
    pub success: Option<bool>,
    pub error: Option<String>,
    pub messages: Option<Vec<MessageWireDTO>>,
    pub is_have_more: Option<bool>,
    pub relative_id: Option<String>,
    pub matches: Option<Vec<MessageSearchMatchWireDTO>>,
    pub total_count: Option<i64>,
    pub chats: Option<Vec<MessageSearchChatWireDTO>>,
}

impl<'de> Deserialize<'de> for GetMessagesListGatewayDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                success: c.rise::<bool>(key::SUCCESS)?,
                error: c.rise::<String>(key::ERROR)?,
                messages: c.rise::<Vec<MessageWireDTO>>(key::MESSAGES)?,
                is_have_more: c.rise::<bool>(key::IS_HAVE_MORE)?,
                relative_id: c.rise::<String>(key::RELATIVE_ID)?,
                matches: c.rise::<Vec<MessageSearchMatchWireDTO>>(key::MATCHES)?,
                total_count: c.rise::<i64>(key::TOTAL_COUNT)?,
                chats: c.rise::<Vec<MessageSearchChatWireDTO>>(key::CHATS)?,
            })
        })
    }
}

impl Serialize for GetMessagesListGatewayDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.success {
            container.serialize_entry(key::SUCCESS, value)?;
        }
        if let Some(value) = &self.error {
            container.serialize_entry(key::ERROR, value)?;
        }
        if let Some(value) = &self.messages {
            container.serialize_entry(key::MESSAGES, value)?;
        }
        if let Some(value) = &self.is_have_more {
            container.serialize_entry(&RiseWireKey::camel(key::IS_HAVE_MORE), value)?;
        }
        if let Some(value) = &self.relative_id {
            container.serialize_entry(&RiseWireKey::camel(key::RELATIVE_ID), value)?;
        }
        if let Some(value) = &self.matches {
            container.serialize_entry(key::MATCHES, value)?;
        }
        if let Some(value) = &self.total_count {
            container.serialize_entry(&RiseWireKey::camel(key::TOTAL_COUNT), value)?;
        }
        if let Some(value) = &self.chats {
            container.serialize_entry(key::CHATS, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug)]
pub struct LinkMessageWireDTO {
    pub created_at: Option<f64>,
    pub link: Option<String>,
    pub message_id: Option<String>,
    pub link_metadata: Option<MessageLinkMetadataWireDTO>,
    pub sender: Option<MessageSenderWireDTO>,
}

impl<'de> Deserialize<'de> for LinkMessageWireDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                created_at: c.rise::<f64>(key::CREATED_AT)?,
                link: c.rise::<String>(key::LINK)?,
                message_id: c.rise::<String>(key::MESSAGE_ID)?,
                link_metadata: c.rise::<MessageLinkMetadataWireDTO>(key::LINK_METADATA)?,
                sender: c.rise::<MessageSenderWireDTO>(key::SENDER)?,
            })
        })
    }
}

impl Serialize for LinkMessageWireDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.created_at {
            container.serialize_entry(&RiseWireKey::camel(key::CREATED_AT), value)?;
        }
        if let Some(value) = &self.link {
            container.serialize_entry(key::LINK, value)?;
        }
        if let Some(value) = &self.message_id {
            container.serialize_entry(&RiseWireKey::camel(key::MESSAGE_ID), value)?;
        }
        if let Some(value) = &self.link_metadata {
            container.serialize_entry(&RiseWireKey::camel(key::LINK_METADATA), value)?;
        }
        if let Some(value) = &self.sender {
            container.serialize_entry(key::SENDER, value)?;
        }
        container.end()
    }
}

#[derive(Clone, Debug, Default)]
pub struct GetLinkMessagesGatewayDTO {
    pub success: Option<bool>,
    pub error: Option<String>,
    pub link_messages: Option<Vec<LinkMessageWireDTO>>,
    pub is_have_more: Option<bool>,
}

impl<'de> Deserialize<'de> for GetLinkMessagesGatewayDTO {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                success: c.rise::<bool>(key::SUCCESS)?,
                error: c.rise::<String>(key::ERROR)?,
                link_messages: c.rise::<Vec<LinkMessageWireDTO>>(key::LINK_MESSAGES)?,
                is_have_more: c.rise::<bool>(key::IS_HAVE_MORE)?,
            })
        })
    }
}

impl Serialize for GetLinkMessagesGatewayDTO {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(value) = &self.success {
            container.serialize_entry(key::SUCCESS, value)?;
        }
        if let Some(value) = &self.error {
            container.serialize_entry(key::ERROR, value)?;
        }
        if let Some(value) = &self.link_messages {
            container.serialize_entry(&RiseWireKey::camel(key::LINK_MESSAGES), value)?;
        }
        if let Some(value) = &self.is_have_more {
            container.serialize_entry(&RiseWireKey::camel(key::IS_HAVE_MORE), value)?;
        }
        container.end()
    }
}

const MILLISECOND_TIMESTAMP_THRESHOLD: f64 = 1_000_000_000_000.0;
const MILLISECONDS_PER_SECOND: f64 = 1000.0;
const LOTTIE_JSON_MIME_TYPE: &str = "application/lottie+json";
const LOTTIE_MIME_TYPE: &str = "application/x-lottie";
const MIME_PARAMETER_SEPARATOR: char = ';';
const WAVEFORM_SEPARATORS: [char; 2] = [',', ' '];
const UNSET_SHARED_POST_ID: &str = "0";

fn seconds_from_wire_timestamp(raw: f64) -> f64 {
    if raw > MILLISECOND_TIMESTAMP_THRESHOLD { raw / MILLISECONDS_PER_SECOND } else { raw }
}

fn non_empty_trimmed(value: Option<&str>) -> Option<String> {
    let trimmed = trimming_whitespaces_and_newlines(value?);
    if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
}

fn is_blank(value: Option<&str>) -> bool {
    value.is_none_or(|value| trimming_whitespaces_and_newlines(value).is_empty())
}

fn slot_index_of(index: usize) -> i64 {
    i64::try_from(index).unwrap_or(i64::MAX)
}

pub enum MessageWaveformCodec {}

impl MessageWaveformCodec {
    pub fn parse_floats(raw: Option<&str>) -> Vec<f32> {
        let Some(raw) = raw.map(trimming_whitespaces_and_newlines).filter(|raw| !raw.is_empty()) else {
            return Vec::new();
        };
        raw.split(WAVEFORM_SEPARATORS)
            .filter(|piece| !piece.is_empty())
            .filter_map(|piece| piece.parse::<f32>().ok())
            .collect()
    }
}

impl MessageMediaInfoWireDTO {
    pub fn to_media_item(
        &self,
        slot_index: i64,
        owner_message_id: &str,
        owner_content_type: Option<MessageContentType>,
    ) -> MessageMediaItem {
        let raw_kind = trimming_whitespaces_and_newlines(self.media_type.as_deref().unwrap_or_default()).to_lowercase();
        let decoded_wire_kind = MessageMediaKind::from_raw(&raw_kind);
        let owner_kind = match owner_content_type {
            Some(MessageContentType::Image) => Some(MessageMediaKind::Image),
            Some(MessageContentType::Video) => Some(MessageMediaKind::Video),
            Some(MessageContentType::Animation) => Some(MessageMediaKind::Animation),
            Some(MessageContentType::Voice) => Some(MessageMediaKind::Voice),
            Some(MessageContentType::Audio) => Some(MessageMediaKind::Audio),
            Some(MessageContentType::Document) => Some(MessageMediaKind::Document),
            Some(MessageContentType::Sticker) => Some(MessageMediaKind::Sticker),
            Some(MessageContentType::VideoNote) => Some(MessageMediaKind::VideoNote),
            _ => None,
        };
        let wire_kind = decoded_wire_kind.or(owner_kind).unwrap_or(MessageMediaKind::Image);
        let kind = match (&owner_content_type, wire_kind) {
            (Some(MessageContentType::Sticker), _) => MessageMediaKind::Sticker,
            (Some(MessageContentType::Document), _) => MessageMediaKind::Document,
            (Some(MessageContentType::Audio), MessageMediaKind::Voice) => MessageMediaKind::Audio,
            (Some(MessageContentType::Voice), MessageMediaKind::Audio) => MessageMediaKind::Voice,
            (Some(MessageContentType::VideoNote), MessageMediaKind::Video) => MessageMediaKind::VideoNote,
            (_, MessageMediaKind::Gif) | (Some(MessageContentType::Animation), MessageMediaKind::Image) => {
                MessageMediaKind::Animation
            }
            _ => wire_kind,
        };
        let url = RemoteMediaURL::optional_normalized(self.file_url.as_deref()).unwrap_or_default();
        let waveform_values = MessageWaveformCodec::parse_floats(self.waveform.as_deref());
        let resolved_id = match self.sticker_id.as_deref() {
            Some(sticker_id) if kind == MessageMediaKind::Sticker && !sticker_id.is_empty() => sticker_id.to_owned(),
            _ => self.file_id.clone().unwrap_or_else(|| format!("{owner_message_id}-{slot_index}")),
        };
        let sticker_kind = self.resolved_sticker_kind(kind, &url);
        let mut item = MessageMediaItem::new(resolved_id, kind, url);
        item.thumbnail_url = RemoteMediaURL::optional_normalized(self.thumbnail_url.as_deref());
        item.width = self.width.unwrap_or(0) as f32;
        item.height = self.height.unwrap_or(0) as f32;
        item.duration = self.duration;
        item.file_name = self.file_name.clone();
        item.file_size = self.file_size;
        item.mime_type = self.mime_type.clone();
        item.bitrate = self.bitrate;
        item.fps = self.fps.map(|fps| fps as i64);
        item.codec = self.codec.clone();
        item.variants = Vec::new();
        item.waveform = waveform_values;
        item.sticker_kind = sticker_kind;
        item.sticker_emojis = self.associated_emojis.clone().unwrap_or_default();
        item.pack_id = self.pack_id.clone();
        item.is_uploading = self.is_uploading.unwrap_or(false);
        item.is_compressing = self.is_compressing.unwrap_or(false);
        item.upload_progress = self.upload_progress.unwrap_or(0.0);
        item.compression_progress = self.compression_progress.unwrap_or(0.0);
        item.uploaded_bytes = self.uploaded_bytes.unwrap_or(0);
        item.total_bytes = self.total_bytes.unwrap_or(0);
        item.is_cancelled = self.is_cancelled.unwrap_or(false);
        item.local_slot_index = self.local_slot_index.unwrap_or(slot_index).max(0);
        let is_photo_or_video = matches!(item.kind, MessageMediaKind::Image | MessageMediaKind::Video);
        item.has_spoiler = self.has_spoiler == Some(true) && is_photo_or_video;
        if let Some(view_mode) = self.view_mode.as_deref()
            && !view_mode.is_empty()
            && is_photo_or_video
        {
            item.view_limit = Some(MessageMediaViewLimit {
                mode: view_mode.to_owned(),
                seconds: self.view_seconds,
                expires_at_ms: self.view_expires_at.filter(|expires_at| *expires_at > 0),
                state: self.view_state.clone().filter(|state| !state.is_empty()),
            });
        }
        if let Some(preview) = &self.local_preview_url
            && MessageMediaItem::is_local_preview_url(preview)
        {
            item.local_preview_url = Some(preview.clone());
        }
        item
    }

    fn resolved_sticker_kind(&self, kind: MessageMediaKind, source: &str) -> Option<MessageStickerKind> {
        if kind != MessageMediaKind::Sticker {
            return None;
        }

        let explicit_type = self
            .sticker_type
            .as_deref()
            .map(|value| trimming_whitespaces_and_newlines(value).to_lowercase());
        if let Some(explicit_kind) = explicit_type.as_deref().and_then(MessageStickerKind::from_raw)
            && explicit_kind == MessageStickerKind::Lottie
        {
            return Some(explicit_kind);
        }

        let normalized_mime_type = self
            .mime_type
            .as_deref()
            .and_then(|mime| mime.splitn(2, MIME_PARAMETER_SEPARATOR).find(|piece| !piece.is_empty()))
            .map(|mime| trimming_whitespaces_and_newlines(mime).to_lowercase());
        if let Some(mime) = normalized_mime_type.as_deref()
            && (mime == RiseUploadFormat::APPLICATION_JSON.mime
                || mime == LOTTIE_JSON_MIME_TYPE
                || mime == LOTTIE_MIME_TYPE
                || mime == RiseUploadFormat::APPLICATION_X_ROSTICKER.mime
                || mime == RiseUploadFormat::APPLICATION_X_TGSTICKER.mime)
        {
            return Some(MessageStickerKind::Lottie);
        }

        let extension_value = match Url::from_string(source) {
            Some(parsed_url) if !parsed_url.path_extension().is_empty() => parsed_url.path_extension().to_lowercase(),
            _ => Path::new(source)
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default()
                .to_lowercase(),
        };
        if extension_value == RiseUploadFormat::APPLICATION_JSON.extension
            || extension_value == RiseUploadFormat::APPLICATION_ZIP_AS_LOTTIE.extension
            || extension_value == RiseUploadFormat::APPLICATION_X_ROSTICKER.extension
            || extension_value == RiseUploadFormat::APPLICATION_X_TGSTICKER.extension
        {
            return Some(MessageStickerKind::Lottie);
        }
        if explicit_type.as_deref() == Some(MessageStickerKind::Animated.raw_value()) {
            return Some(MessageStickerKind::Animated);
        }
        match ChatAnimatedMediaSourcePolicy::playback_kind(source, self.mime_type.as_deref(), self.codec.as_deref()) {
            ChatAnimatedMediaPlaybackKind::AnimatedImage
            | ChatAnimatedMediaPlaybackKind::AvFoundationVideo
            | ChatAnimatedMediaPlaybackKind::WebMVideo
            | ChatAnimatedMediaPlaybackKind::UnsupportedVideo => Some(MessageStickerKind::Animated),
            ChatAnimatedMediaPlaybackKind::Unknown => explicit_type.as_deref().and_then(MessageStickerKind::from_raw),
        }
    }
}

impl MessageSenderWireDTO {
    pub fn to_user(&self) -> User {
        let more = AuthorInfoMore {
            logo: self.more.as_ref().and_then(|more| more.logo.clone()).unwrap_or_default(),
            is_premium: self.is_premium.unwrap_or(false),
            is_official: self.is_official.unwrap_or(false),
            premium_badge_emoji: self.premium_badge_emoji.clone(),
            name_color: self.name_color.clone(),
            ..AuthorInfoMore::default()
        };
        User {
            id: self.id.clone().unwrap_or_default(),
            name: self.name.clone().unwrap_or_default(),
            tag: self.tag.clone().unwrap_or_default(),
            more,
            is_online: self.more.as_ref().and_then(|more| more.is_online).unwrap_or(false),
            ..User::default()
        }
    }
}

impl MessageReplyInfoWireDTO {
    pub fn to_reply(&self) -> Option<MessageReplyTo> {
        let mid = self.message_id.as_deref().filter(|mid| !mid.is_empty())?;
        let resolved_content_type =
            self.content_type.as_deref().map(MessageContentType::from_raw_value).unwrap_or(MessageContentType::Text);
        let items = self
            .media_items
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(index, media)| media.to_media_item(slot_index_of(index), mid, Some(resolved_content_type.clone())))
            .collect();
        let created_sec = seconds_from_wire_timestamp(self.created_at.unwrap_or(0.0));
        let resolved_sender_id = self
            .sender_id
            .clone()
            .or_else(|| self.sender.as_ref().and_then(|sender| sender.id.clone()))
            .unwrap_or_default();
        let wire_sender_name = self.sender.as_ref().and_then(|sender| sender.name.clone()).unwrap_or_default();
        let sender_is_deleted = self.sender.as_ref().and_then(|sender| sender.is_deleted) == Some(true)
            || (trimming_whitespaces_and_newlines(&wire_sender_name).is_empty() && !resolved_sender_id.is_empty());
        let sender_name = if sender_is_deleted { tr("chat_deleted_account") } else { wire_sender_name };
        Some(MessageReplyTo {
            message_id: mid.to_owned(),
            sender_id: resolved_sender_id,
            sender_name,
            content_type: resolved_content_type,
            content: self.content.clone().unwrap_or_default(),
            caption: self.caption.clone(),
            media_items: items,
            created_at: created_sec,
        })
    }
}

impl MessageForwardInfoWireDTO {
    pub fn to_forward(&self) -> Option<MessageForwardInfo> {
        let sender = self.sender.as_ref();
        let resolved_from_chat_id = non_empty_trimmed(self.from_chat_id.as_deref());
        let resolved_from_message_id = non_empty_trimmed(self.from_message_id.as_deref());
        let resolved_original_message_id = non_empty_trimmed(self.original_message_id.as_deref());
        let resolved_sender_id = non_empty_trimmed(self.sender_id.as_deref())
            .or_else(|| non_empty_trimmed(sender.and_then(|sender| sender.id.as_deref())));
        let resolved_sender_tag = non_empty_trimmed(sender.and_then(|sender| sender.tag.as_deref()));
        let resolved_sender_name = non_empty_trimmed(self.sender_name.as_deref())
            .or_else(|| non_empty_trimmed(sender.and_then(|sender| sender.name.as_deref())));
        let resolved_sender_logo = non_empty_trimmed(self.sender_logo.as_deref()).or_else(|| {
            non_empty_trimmed(sender.and_then(|sender| sender.more.as_ref()).and_then(|more| more.logo.as_deref()))
        });
        let has_any = [
            &resolved_from_chat_id,
            &resolved_from_message_id,
            &resolved_original_message_id,
            &resolved_sender_name,
            &resolved_sender_id,
            &resolved_sender_tag,
        ]
        .iter()
        .any(|value| value.is_some());
        if !has_any {
            return None;
        }
        let created_sec = seconds_from_wire_timestamp(self.date.unwrap_or(0.0));
        Some(MessageForwardInfo {
            from_chat_id: resolved_from_chat_id,
            from_message_id: resolved_from_message_id,
            original_message_id: resolved_original_message_id,
            sender_id: resolved_sender_id,
            sender_tag: resolved_sender_tag,
            sender_name: resolved_sender_name.unwrap_or_default(),
            sender_logo: resolved_sender_logo,
            from_chat_title: None,
            from_chat_logo: None,
            date: created_sec,
            forward_chain_count: self.forward_chain_count.unwrap_or(1).max(1),
        })
    }
}

impl MessageLinkMetadataWireDTO {
    pub fn to_link_preview(&self, source_text: &str) -> Option<MessageLinkPreview> {
        let preview_type = self.link_preview_type.as_deref().map(str::to_uppercase);
        if preview_type.as_deref() == Some(ChatLinkPreviewSize::None.wire()) {
            return None;
        }
        let url = ChatLinkPreviewURL::first(source_text)?;
        let has_content = [&self.title, &self.description, &self.image_url]
            .iter()
            .any(|value| value.as_deref().is_some_and(|value| !trimming_whitespaces_and_newlines(value).is_empty()));
        if !has_content
            || Self::only_repeats_host(
                self.title.as_deref(),
                self.description.as_deref(),
                self.image_url.as_deref(),
                &url,
            )
        {
            return None;
        }
        let above_suffix = ChatLinkPreviewSize::BigAbove
            .wire()
            .strip_prefix(ChatLinkPreviewSize::Big.wire())
            .unwrap_or_default();
        let size = if preview_type.as_deref().is_some_and(|value| value.starts_with(ChatLinkPreviewSize::Big.wire())) {
            MessageLinkPreviewSize::Big
        } else {
            MessageLinkPreviewSize::Small
        };
        let is_above_text = preview_type.as_deref().is_some_and(|value| value.ends_with(above_suffix));
        let preview = MessageLinkPreview {
            url,
            title: self.title.clone(),
            description: self.description.clone(),
            site_name: None,
            image_url: self.image_url.clone(),
            size,
            is_above_text,
            ..MessageLinkPreview::default()
        };
        Some(preview.localized_for_own_link())
    }

    fn only_repeats_host(title: Option<&str>, description: Option<&str>, image_url: Option<&str>, url: &str) -> bool {
        if !is_blank(description) || !is_blank(image_url) {
            return false;
        }
        let Some(host) = Url::from_string(url).and_then(|parsed| parsed.host()) else {
            return false;
        };
        let Some(title) = title.map(trimming_whitespaces_and_newlines) else {
            return false;
        };
        title.to_lowercase() == host.to_lowercase() || title == url
    }
}

impl MessageSharedPostAuthorWireDTO {
    pub fn to_model(&self) -> MessageSharedPostAuthor {
        MessageSharedPostAuthor {
            id: self.id.clone().unwrap_or_default(),
            tag: self.tag.clone().unwrap_or_default(),
            name: self.name.clone().unwrap_or_default(),
            logo: RemoteMediaURL::optional_normalized(self.logo.as_deref()).unwrap_or_default(),
            is_premium: self.is_premium.unwrap_or(false),
            is_official: self.is_official.unwrap_or(false),
        }
    }
}

impl MessageSharedPostWireDTO {
    pub fn to_model(&self) -> Option<MessageSharedPost> {
        let raw_id = self.post_id.as_ref()?.as_string();
        let pid = trimming_whitespaces_and_newlines(&raw_id);
        if pid.is_empty() || pid == UNSET_SHARED_POST_ID {
            return None;
        }
        let is_short = self.is_short.unwrap_or(false);
        Some(MessageSharedPost {
            post_id: pid.to_owned(),
            is_short,
            is_riff: self.is_riff.unwrap_or(is_short),
            show_in_publications: self.show_in_publications.unwrap_or(true),
            title: self.title.clone().unwrap_or_default(),
            snippet: self.snippet.clone().unwrap_or_default(),
            images: self.images.clone().unwrap_or_default(),
            video: RemoteMediaURL::optional_normalized(self.video.as_deref()).unwrap_or_default(),
            video_thumbnail: RemoteMediaURL::optional_normalized(self.video_thumbnail.as_deref()).unwrap_or_default(),
            likes_count: self.likes_count.unwrap_or(0),
            comments_count: self.comments_count.unwrap_or(0),
            author: self.author.as_ref().map(MessageSharedPostAuthorWireDTO::to_model).unwrap_or_default(),
        })
    }
}

impl MessageStoryReplyAuthorWireDTO {
    pub fn to_model(&self) -> MessageStoryReplyAuthor {
        MessageStoryReplyAuthor {
            id: self.id.clone().unwrap_or_default(),
            tag: self.tag.clone().unwrap_or_default(),
            name: self.name.clone().unwrap_or_default(),
            logo: RemoteMediaURL::optional_normalized(self.logo.as_deref()).unwrap_or_default(),
            is_premium: self.is_premium.unwrap_or(false),
        }
    }
}

impl MessageStoryReplyWireDTO {
    pub fn to_model(&self) -> Option<MessageStoryReply> {
        let raw_id = self.story_id.as_ref()?.as_string();
        let numeric_id = trimming_whitespaces_and_newlines(&raw_id).parse::<i64>().ok().filter(|id| *id > 0)?;
        Some(MessageStoryReply {
            story_id: numeric_id,
            media_url: RemoteMediaURL::optional_normalized(self.media_url.as_deref()).unwrap_or_default(),
            is_video: self.is_video.unwrap_or(false),
            caption: self.caption.clone().unwrap_or_default(),
            created_at_ms: self.created_at_ms.unwrap_or(0),
            expires_at_ms: self.expires_at_ms.unwrap_or(0),
            post_to_profile: self.post_to_profile,
            author: self.author.as_ref().map(MessageStoryReplyAuthorWireDTO::to_model).unwrap_or_default(),
            is_known_gone: false,
        })
    }
}

impl MessageWireDTO {
    pub fn to_message(&self) -> GetMessageMessage {
        let mid = self.id.clone().unwrap_or_else(|| Uuid::new_v4().uuid_string());
        let raw_ts = self.created_at.or(self.timestamp).unwrap_or(0.0);
        let mut created_sec = seconds_from_wire_timestamp(raw_ts);
        let normalized_content_type =
            self.content_type.as_deref().map(MessageContentType::from_raw_value).unwrap_or(MessageContentType::Text);
        let is_system_wire = self.is_system_message == Some(true)
            || !self.system_message_type.as_deref().is_none_or(|kind| trimming_whitespaces_and_newlines(kind).is_empty())
            || normalized_content_type == MessageContentType::System;

        if created_sec <= 0.0 && is_system_wire {
            created_sec = RiseClock::now().time_interval_since_1970();
        }

        let wire_items = self.media_items.as_deref().unwrap_or_default();
        let items: Vec<MessageMediaItem> = if !wire_items.is_empty() {
            wire_items
                .iter()
                .enumerate()
                .map(|(index, media)| {
                    media.to_media_item(slot_index_of(index), &mid, Some(normalized_content_type.clone()))
                })
                .collect()
        } else if let Some(single) = &self.media_info {
            vec![single.to_media_item(0, &mid, Some(normalized_content_type.clone()))]
        } else {
            Vec::new()
        };

        let sender_info = self.sender.as_ref();
        let wire_sender_name = self
            .sender_name
            .clone()
            .or_else(|| sender_info.and_then(|sender| sender.name.clone()))
            .unwrap_or_default();
        let sender_is_deleted = !is_system_wire && sender_info.and_then(|sender| sender.is_deleted) == Some(true);
        let sender_name = if sender_is_deleted
            || (!is_system_wire
                && sender_info.is_none()
                && trimming_whitespaces_and_newlines(&wire_sender_name).is_empty()
                && !self.sender_id.as_deref().unwrap_or_default().is_empty())
        {
            tr("chat_deleted_account")
        } else {
            wire_sender_name
        };

        let resolved_sender_id = {
            let top = trimming_whitespaces_and_newlines(self.sender_id.as_deref().unwrap_or_default());
            if top.is_empty() {
                trimming_whitespaces_and_newlines(sender_info.and_then(|sender| sender.id.as_deref()).unwrap_or_default())
                    .to_owned()
            } else {
                top.to_owned()
            }
        };
        let sender_logo = sender_info.and_then(|sender| sender.more.as_ref()).and_then(|more| more.logo.clone());

        let enriched_system_info = |info: MessageSystemInfo| -> MessageSystemInfo {
            let mut resolved = info;
            let actor = self.system_actor_info.as_ref();
            let wire_actor_id =
                trimming_whitespaces_and_newlines(actor.and_then(|actor| actor.id.as_deref()).unwrap_or_default());
            let wire_actor_name =
                trimming_whitespaces_and_newlines(actor.and_then(|actor| actor.name.as_deref()).unwrap_or_default());
            let fallback_actor_name = trimming_whitespaces_and_newlines(&sender_name);

            resolved.actor_id = if wire_actor_id.is_empty() {
                if resolved_sender_id.is_empty() { None } else { Some(resolved_sender_id.clone()) }
            } else {
                Some(wire_actor_id.to_owned())
            };
            resolved.actor_name = if wire_actor_name.is_empty() {
                if fallback_actor_name.is_empty() { None } else { Some(fallback_actor_name.to_owned()) }
            } else {
                Some(wire_actor_name.to_owned())
            };
            resolved
        };

        let mut msg = GetMessageMessage::new(mid.clone());
        if let Some(sid) = self.server_id.as_deref()
            && !sid.is_empty()
            && (GetSaiServerId::is_temp_id(Some(mid.as_str())) || sid != mid)
        {
            msg.server_id = Some(sid.to_owned());
        }
        msg.chat_id = self.chat_id.clone().unwrap_or_default();
        msg.sender_id = resolved_sender_id.clone();
        msg.sender_name = sender_name.clone();
        msg.sender_tag = sender_info.and_then(|sender| sender.tag.clone()).unwrap_or_default();
        msg.sender_logo = sender_logo;
        msg.sender_is_official = sender_info.and_then(|sender| sender.is_official).unwrap_or(false);
        msg.sender_is_deleted = sender_is_deleted;
        msg.reply_markup = self.reply_markup.as_ref().map(MessageBotReplyMarkupWireDTO::to_model);
        msg.is_bot_message = self.is_bot_message.unwrap_or(false);
        msg.via_bot = self.via_bot.clone();
        msg.via_bot_tag = self.via_bot_tag.clone();
        msg.web_app_data = self.web_app_data.as_ref().map(MessageBotWebAppDataWireDTO::to_model);
        msg.content_type = if is_system_wire { MessageContentType::System } else { normalized_content_type.clone() };
        let has_content = self.content.as_deref().is_some_and(|content| content.chars().any(|ch| !ch.is_whitespace()));
        msg.text = if normalized_content_type == MessageContentType::Call {
            String::new()
        } else if has_content {
            self.content.clone().unwrap_or_default()
        } else {
            self.caption.clone().or_else(|| self.content.clone()).unwrap_or_default()
        };
        msg.original_text = self.original_content.clone();
        msg.entities = self.entities.clone();
        msg.normalized_entities = Some(MessageNormalizedEntities::parse(self.entities.as_deref()));
        msg.edited_at = self.edit_date;
        msg.created_at = created_sec;
        msg.media_items = items;
        match msg.content_type {
            MessageContentType::Audio => {
                for item in &mut msg.media_items {
                    if item.kind == MessageMediaKind::Voice {
                        item.kind = MessageMediaKind::Audio;
                    }
                }
            }
            MessageContentType::Voice => {
                for item in &mut msg.media_items {
                    if item.kind == MessageMediaKind::Audio {
                        item.kind = MessageMediaKind::Voice;
                    }
                }
            }
            MessageContentType::VideoNote => {
                for item in &mut msg.media_items {
                    if item.kind == MessageMediaKind::Video || item.kind == MessageMediaKind::Animation {
                        item.kind = MessageMediaKind::VideoNote;
                    }
                }
            }
            MessageContentType::Document => {
                for item in &mut msg.media_items {
                    item.kind = MessageMediaKind::Document;
                }
            }
            _ => {}
        }
        msg.media_group_id = self.media_group_id.clone();
        msg.reply_to = self.reply_to.as_ref().and_then(MessageReplyInfoWireDTO::to_reply);
        msg.forward_info = self.forward_info.as_ref().and_then(MessageForwardInfoWireDTO::to_forward);
        msg.link_preview = self.link_preview.as_ref().and_then(|preview| preview.to_link_preview(&msg.text));
        if normalized_content_type == MessageContentType::Call {
            msg.call_info = self.metadata.as_ref().and_then(MessageCallInfoWireDTO::to_model);
        }
        if let Some(shared) = self.shared_post.as_ref().and_then(MessageSharedPostWireDTO::to_model) {
            msg.shared_post = Some(shared);
            msg.content_type = MessageContentType::PostShort;
        }
        msg.story_reply = self.story_reply.as_ref().and_then(MessageStoryReplyWireDTO::to_model);
        msg.document_raw = self.document_raw.clone();
        msg.adopt_location_payload();
        msg.adopt_poll_payload();
        msg.moderation_status = self
            .moderation_status
            .as_deref()
            .map(|status| trimming_whitespaces_and_newlines(status).to_owned())
            .unwrap_or_default();
        if is_system_wire {
            let base = if let Some(kind) = self.system_message_type.as_deref().and_then(MessageSystemKind::from_raw) {
                MessageSystemInfo { kind, ..MessageSystemInfo::default() }
            } else if let Some(inferred) = MessageSystemContentParser::infer(&msg.text) {
                inferred
            } else {
                MessageSystemInfo { kind: MessageSystemKind::Generic, ..MessageSystemInfo::default() }
            };
            let mut system = enriched_system_info(base);
            system.wallpaper_preview = self.system_wallpaper.as_ref().and_then(MessageSystemWallpaperWireDTO::to_preview);
            if let Some(target) = &self.system_target {
                system.target_id = non_empty_trimmed(target.target_user_id.as_deref());
                system.target_name = non_empty_trimmed(target.target_name.as_deref());
            }
            if let Some(gift) = &self.system_gift {
                system.gift_months = gift.months.filter(|months| *months > 0);
                system.gift_recipient_id = non_empty_trimmed(gift.recipient_user_id.as_deref());
            }
            if let Some(item) = self.system_gift_item.as_ref().and_then(MessageSystemGiftItemWireDTO::to_model) {
                system.gift_recipient_id = if item.recipient_id.is_empty() { None } else { Some(item.recipient_id.clone()) };
                system.gift_item = Some(item);
            }
            msg.system = Some(system);
        }

        let mut reactors_by_emoji: HashMap<String, Vec<ReactionReactor>> = HashMap::new();
        for entry in self.reacted_by.as_deref().unwrap_or_default() {
            let Some(emoji) = entry.reaction.as_deref().filter(|emoji| !emoji.is_empty()) else {
                continue;
            };
            let sender = entry.sender.as_ref();
            let reactor = ReactionReactor {
                user_id: trimming_whitespaces_and_newlines(sender.and_then(|sender| sender.id.as_deref()).unwrap_or_default())
                    .to_owned(),
                name: sender.and_then(|sender| sender.name.clone()).unwrap_or_default(),
                avatar_url: RemoteMediaURL::optional_normalized(
                    sender.and_then(|sender| sender.more.as_ref()).and_then(|more| more.logo.as_deref()),
                ),
                created_at: seconds_from_wire_timestamp(entry.created_at.unwrap_or(0.0)),
            };
            reactors_by_emoji.entry(emoji.to_owned()).or_default().push(reactor);
        }
        for reactors in reactors_by_emoji.values_mut() {
            reactors.sort_by(|lhs, rhs| rhs.created_at.partial_cmp(&lhs.created_at).unwrap_or(Ordering::Equal));
        }
        msg.reactions = self
            .reactions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|summary| MessageReaction {
                emoji: summary.reaction.clone(),
                count: summary.count,
                reacted_by_me: summary.reacted_by_you.unwrap_or(false),
                reactors: reactors_by_emoji.get(&summary.reaction).cloned().unwrap_or_default(),
            })
            .collect();
        msg.is_read = self.is_read.unwrap_or(false);
        msg.played_by = self.played_by.clone().unwrap_or_default();
        msg.is_pinned = self.is_pinned.unwrap_or(false);
        msg.is_mentioned_you = self.is_mentioned_you.unwrap_or(false);
        msg.is_replied_you = self.is_replied_you.unwrap_or(false);
        msg.has_unread_reactions = self.reactions_unreaded.unwrap_or(false);
        msg.has_protected_content = self.has_protected_content.unwrap_or(false);
        msg.view_count = self.view_count;
        msg.topic_id = self.topic_id.clone();
        msg.discussion_chat_id = self.discussion_chat_id.clone();
        msg.discussion_message_id = self.discussion_message_id.clone();
        msg.comments_count = self.comments_count;
        msg.sparks_total = self.sparks_total.unwrap_or(0).max(0);
        msg.my_sparks = self.my_sparks.unwrap_or(0).max(0);
        msg.recent_commenters = self
            .recent_commenters
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|commenter| MessageCommenterPreview {
                logo: commenter.logo.clone().unwrap_or_default(),
                name: commenter.name.clone().unwrap_or_default(),
            })
            .collect();
        if let Some(flag) = self.is_temp {
            msg.is_temp = flag;
        } else if GetSaiServerId::is_temp_id(Some(mid.as_str())) {
            msg.is_temp = msg.server_id.as_deref().is_none_or(|sid| trimming_whitespaces_and_newlines(sid).is_empty());
        }
        msg.delivery_status = if msg.is_temp {
            MessageDeliveryStatus::Sending
        } else if msg.is_read {
            MessageDeliveryStatus::Read
        } else {
            MessageDeliveryStatus::Sent
        };
        msg
    }
}

const WIRE_DECODE_LOG_NAME: &str = "WireDecode";

pub enum MessagePayloadCodec {}

impl MessagePayloadCodec {
    pub fn decode<T: DeserializeOwned>(any: Option<&Value>) -> Option<T> {
        Self::decode_value(any?).ok()
    }

    pub fn decode_reporting<T: DeserializeOwned>(
        any: Option<&Value>,
        context: impl FnOnce() -> String,
        cx: &mut App,
    ) -> Option<T> {
        let any = any?;
        if !Self::has_json_data(any) {
            Sai::log(
                WIRE_DECODE_LOG_NAME,
                move || format!("no JSON data for {} ({})", type_name::<T>(), context()),
                SaiLogKind::Cache,
                cx,
            );
            return None;
        }
        match Self::decode_value::<T>(any) {
            Ok(value) => Some(value),
            Err(error) => {
                Sai::log(
                    WIRE_DECODE_LOG_NAME,
                    move || format!("decode {} failed ({}): {error}", type_name::<T>(), context()),
                    SaiLogKind::Cache,
                    cx,
                );
                None
            }
        }
    }

    fn has_json_data(any: &Value) -> bool {
        matches!(any, Value::String(_) | Value::Object(_) | Value::Array(_))
    }

    fn decode_value<T: DeserializeOwned>(any: &Value) -> Result<T, serde_json::Error> {
        match any {
            Value::String(text) => serde_json::from_str::<T>(text),
            Value::Object(_) | Value::Array(_) => T::deserialize(any),
            _ => Err(<serde_json::Error as serde::de::Error>::custom(format!("no JSON data for {}", type_name::<T>()))),
        }
    }

    pub fn dictionary(any: Option<&Value>) -> Option<Map<String, Value>> {
        AnyDictionaryBridge::dictionary(any)
    }

    pub fn array_of_dictionaries(any: Option<&Value>) -> Vec<Map<String, Value>> {
        AnyDictionaryBridge::array_of_dictionaries(any)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CustomEmojiEntityWire {
    pub offset: i64,
    pub length: i64,
    pub custom_emoji_id: String,
    pub file_url: String,
    pub asset_type: String,
    pub width: i64,
    pub height: i64,
    pub pack_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RoleMentionEntityWire {
    pub offset: i64,
    pub length: i64,
    pub role_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TopicMentionEntityWire {
    pub offset: i64,
    pub length: i64,
    pub topic_id: String,
    pub chat_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextLinkEntityWire {
    pub offset: i64,
    pub length: i64,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageEntitiesCodecParsed {
    pub custom_emojis: Vec<CustomEmojiEntityWire>,
    pub role_mentions: Vec<RoleMentionEntityWire>,
    pub topic_mentions: Vec<TopicMentionEntityWire>,
    pub text_links: Vec<TextLinkEntityWire>,
}

impl MessageEntitiesCodecParsed {
    pub const EMPTY: MessageEntitiesCodecParsed = MessageEntitiesCodecParsed {
        custom_emojis: Vec::new(),
        role_mentions: Vec::new(),
        topic_mentions: Vec::new(),
        text_links: Vec::new(),
    };
}

pub enum MessageEntitiesCodec {}

impl MessageEntitiesCodec {
    pub const CUSTOM_EMOJI_TYPE: &'static str = MessageEntityType::CustomEmoji.wire();

    pub fn parse(entities_json: Option<&str>) -> MessageEntitiesCodecParsed {
        let Some(entries) = Self::entries(entities_json) else {
            return MessageEntitiesCodecParsed::EMPTY;
        };

        let mut custom_emojis: Vec<CustomEmojiEntityWire> = Vec::new();
        let mut role_mentions: Vec<RoleMentionEntityWire> = Vec::new();
        let mut topic_mentions: Vec<TopicMentionEntityWire> = Vec::new();
        let mut text_links: Vec<TextLinkEntityWire> = Vec::new();
        for entry in &entries {
            let Some(entity_type) = entry.get(events_key::TYPE).and_then(Value::as_str) else {
                continue;
            };
            let Some(offset) = Self::int_value(entry.get(bot_key::OFFSET)).filter(|offset| *offset >= 0) else {
                continue;
            };
            let Some(length) = Self::int_value(entry.get(MessageEntityField::Length.wire())).filter(|length| *length > 0)
            else {
                continue;
            };
            if length > i64::MAX - offset {
                continue;
            }
            if entity_type == Self::CUSTOM_EMOJI_TYPE {
                let id = Self::string_value(entry, MessageEntityField::CustomEmojiId.wire());
                if id.is_empty() {
                    continue;
                }
                let raw_file_url = Self::string_value(entry, stickers_key::FILE_URL);
                custom_emojis.push(CustomEmojiEntityWire {
                    offset,
                    length,
                    custom_emoji_id: id,
                    file_url: RemoteMediaURL::optional_normalized(Some(&raw_file_url)).unwrap_or(raw_file_url),
                    asset_type: entry
                        .get(stickers_key::ASSET_TYPE)
                        .and_then(Value::as_str)
                        .unwrap_or(ChatStickerKind::StaticImage.wire())
                        .to_owned(),
                    width: Self::int_value(entry.get(stickers_key::WIDTH)).unwrap_or(0),
                    height: Self::int_value(entry.get(stickers_key::HEIGHT)).unwrap_or(0),
                    pack_id: Self::string_value(entry, stickers_key::PACK_ID),
                });
            } else if entity_type == MessageEntityType::RoleMention.wire() {
                let role_id = Self::string_value(entry, events_key::ROLE_ID);
                if role_id.is_empty() {
                    continue;
                }
                role_mentions.push(RoleMentionEntityWire { offset, length, role_id });
            } else if entity_type == MessageEntityType::TopicMention.wire() {
                let topic_id = Self::string_value(entry, MessageEntityField::TopicId.wire());
                if topic_id.is_empty() {
                    continue;
                }
                let chat_id = Self::string_value(entry, MessageEntityField::ChatId.wire());
                topic_mentions.push(TopicMentionEntityWire { offset, length, topic_id, chat_id });
            } else if entity_type == MessageEntityType::TextLink.wire() {
                let url = Self::string_value(entry, MessageEntityField::Url.wire());
                let url = url.trim();
                if url.is_empty() {
                    continue;
                }
                text_links.push(TextLinkEntityWire { offset, length, url: url.to_owned() });
            }
        }
        custom_emojis.sort_by_key(|entity| entity.offset);
        role_mentions.sort_by_key(|entity| entity.offset);
        topic_mentions.sort_by_key(|entity| entity.offset);
        text_links.sort_by_key(|entity| entity.offset);
        MessageEntitiesCodecParsed { custom_emojis, role_mentions, topic_mentions, text_links }
    }

    pub fn parse_custom_emojis(entities_json: Option<&str>) -> Vec<CustomEmojiEntityWire> {
        Self::parse(entities_json).custom_emojis
    }

    pub fn encode(custom_emojis: &[CustomEmojiEntityWire], merging_into: Option<&str>) -> Option<String> {
        let mut entries: Vec<Value> = Self::entries(merging_into)
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.get(events_key::TYPE).and_then(Value::as_str) != Some(Self::CUSTOM_EMOJI_TYPE))
            .map(Value::Object)
            .collect();
        for custom_emoji in custom_emojis {
            let normalized_file_url = RemoteMediaURL::optional_normalized(Some(&custom_emoji.file_url))
                .unwrap_or_else(|| custom_emoji.file_url.clone());
            let mut entry = Map::new();
            entry.insert(events_key::TYPE.to_owned(), Value::from(Self::CUSTOM_EMOJI_TYPE));
            entry.insert(bot_key::OFFSET.to_owned(), Value::from(custom_emoji.offset));
            entry.insert(MessageEntityField::Length.wire().to_owned(), Value::from(custom_emoji.length));
            entry.insert(
                MessageEntityField::CustomEmojiId.wire().to_owned(),
                Value::from(custom_emoji.custom_emoji_id.clone()),
            );
            entry.insert(stickers_key::FILE_URL.to_owned(), Value::from(normalized_file_url));
            entry.insert(stickers_key::ASSET_TYPE.to_owned(), Value::from(custom_emoji.asset_type.clone()));
            entry.insert(stickers_key::WIDTH.to_owned(), Value::from(custom_emoji.width));
            entry.insert(stickers_key::HEIGHT.to_owned(), Value::from(custom_emoji.height));
            if !custom_emoji.pack_id.is_empty() {
                entry.insert(stickers_key::PACK_ID.to_owned(), Value::from(custom_emoji.pack_id.clone()));
            }
            entries.push(Value::Object(entry));
        }
        if entries.is_empty() {
            return merging_into.map(str::to_owned);
        }
        serde_json::to_string(&entries).ok().or_else(|| merging_into.map(str::to_owned))
    }

    fn entries(entities_json: Option<&str>) -> Option<Vec<Map<String, Value>>> {
        let json = trimming_whitespaces_and_newlines(entities_json?);
        if json.is_empty() {
            return None;
        }
        let Ok(Value::Array(elements)) = serde_json::from_str::<Value>(json) else {
            return None;
        };
        elements
            .into_iter()
            .map(|element| match element {
                Value::Object(object) => Some(object),
                _ => None,
            })
            .collect()
    }

    fn string_value(entry: &Map<String, Value>, key: &str) -> String {
        entry.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
    }

    fn int_value(any: Option<&Value>) -> Option<i64> {
        match any? {
            Value::Number(number) => {
                if let Some(integer) = number.as_i64() {
                    return Some(integer);
                }
                let truncated = number.as_f64()?.trunc();
                if truncated.is_finite() && truncated >= i64::MIN as f64 && truncated < i64::MAX as f64 {
                    Some(truncated as i64)
                } else {
                    None
                }
            }
            Value::Bool(flag) => Some(i64::from(*flag)),
            Value::String(text) => text.parse::<i64>().ok(),
            _ => None,
        }
    }
}

pub type AddReactionBody = message::AddReaction;

pub type RemoveReactionBody = message::RemoveReaction;

pub type AddReactionGatewayDTO = proto::ProtoAddReactionResponse;

pub type RemoveReactionGatewayDTO = proto::ProtoRemoveReactionResponse;
