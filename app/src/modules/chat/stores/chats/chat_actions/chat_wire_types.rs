use rise_canon::rise_event_payload::{ChatRoleCard, ChatTopicCard, ChatWallpaperCard, VoiceChannelConfigCard};
use rise_canon::rise_key::chat as key;
use rise_canon::rise_key::message as message_key;
use rise_canon::rise_key::payload as payload_key;
use rise_canon::rise_proto::chat::{
    ParticipantMoreData, ProtoActionUser, ProtoAnalyticsBucket, ProtoBlockedMember, ProtoChatAction,
    ProtoChatMember, ProtoChatMemberUser, ProtoChatSettings, ProtoChatUnreadMentions, ProtoChatUnreadReactions,
    ProtoChatWallpaper, ProtoChatWallpaperCatalogEntry, ProtoChatWallpaperCatalogResponse, ProtoInviteLink,
    ProtoInviteLinkStats, ProtoJoinRequestWithUser, ProtoJoinRequestsUser, ProtoJoinRequestsUserMore,
    ProtoLinkedChatInfo, ProtoOverwrite, ProtoPaginationInfo, ProtoRole, ProtoSection, ProtoVoiceChannelConfig,
    ProtoVoiceChannelState, ProtoVoiceParticipant,
};
use rise_canon::rise_reply::chat::{ChatAnalytics, CreatedChat};
use rise_canon::rise_response::chat::{
    CreateInviteLink, CreateRole, CreateSection, GetBlockedMembers, GetChatActionsHistory, GetChatMembers,
    GetChatPreviewByInviteLink, GetEffectivePermissions, GetInviteLinks, GetJoinRequests, GetRoles, GetSections,
    GetTopicOverwrites, JoinChatByLink, RequestJoinChat, SetChatWallpaper,
};
use rise_canon::rise_view::chat::{ChatInfo as CanonChatInfo, ChatUnread, ChatUnreadMessages, TopicCreated, TopicInfo};
use rise_canon::{RiseLimits, RiseWireKey, RiseWireObject, rise_decode, rise_integer};
use rise_chat::{BanType, JoinRequestStatus};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_notify::MUTE_UNTIL_MILLISECOND_FLOOR;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::chat_types::{
    BlockedMember, ChatAction, ChatAnalyticsModel, ChatAnalyticsPoint, ChatChannelKind, ChatInfo, ChatJoinPreview,
    ChatVoiceJoinPreview,
    ChatMuteScopes, ChatPermOverwrite, ChatPermissionJSON, ChatReactionsType, ChatReactionsTypeWire, ChatRole,
    ChatSection, ChatSettingsModel, ChatTopic, ChatType, ChatWallpaperCatalogEntry, ChatWallpaperPlacement,
    InviteLink, JoinRequest, LinkedChatInfo, User, VoiceChannelConfig, VoiceChannelParticipant, VoiceChannelState,
};
use super::chat_folder_types::ChatFolder;
use super::chat_wire_types_hooks::chat_wire_types_hooks;
use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageDeliveryStatus, MessageMediaItem, MessageMediaKind,
    MessageSystemInfo, MessageSystemKind,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    MessageCallInfoWireDTO, MessageMediaInfoWireDTO, MessageSenderWireDTO, MessageSystemMetadataTargetEnvelope,
};
use crate::modules::comment::stores::comment_actions::comment_types::CommentEntities;
use crate::modules::post::stores::post_actions::post_types::AuthorInfoMore;
use crate::modules::rtc::engine::rise_rtc_engine_models::{RiseRTCFlexibleWireEnum, RiseRTCParticipantRole};

const MILLISECOND_TIMESTAMP_FLOOR: f64 = 1_000_000_000_000.0;
const UNKNOWN_SYSTEM_ACTOR: &str = "System";
const JOIN_STATUS_ALREADY_MEMBER: &str = "already_member";
const JOIN_STATUS_REQUIRES_APPROVAL: &str = "requires_approval";
const JOIN_STATUS_CHAT_UNAVAILABLE: &str = "chat_unavailable";
const OVERWRITE_TARGET_ROLE: &str = "role";
const DEFAULT_MAX_REACTION_COUNT: i64 = 11;

fn encode_if_present<M: SerializeMap, T: Serialize>(
    container: &mut M,
    key: &str,
    value: &Option<T>,
) -> Result<(), M::Error> {
    match value {
        Some(value) => container.serialize_entry(key, value),
        None => Ok(()),
    }
}

fn camel(key: &str) -> String {
    <RiseWireKey as RiseWireKeyCamel>::camel(key)
}

fn non_empty_trimmed(value: Option<&str>) -> Option<String> {
    let value = trimming_whitespaces_and_newlines(value?);
    (!value.is_empty()).then(|| value.to_owned())
}

#[derive(Clone, Debug, Default)]
pub struct RiseStoredChatPreview {
    pub id: Option<String>,
    pub chat_id: Option<String>,
    pub topic_id: Option<String>,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub content: Option<String>,
    pub caption: Option<String>,
    pub content_type: Option<String>,
    pub timestamp: Option<f64>,
    pub created_at: Option<f64>,
    pub has_attachments: Option<bool>,
    pub has_reactions: Option<bool>,
    pub is_pinned: Option<bool>,
    pub is_read: Option<bool>,
    pub is_system_message: Option<bool>,
    pub system_message_type: Option<String>,
    pub metadata: Option<MessageCallInfoWireDTO>,
    pub media_items: Option<RiseStoredChatPreviewMedia>,
    pub media_info: Option<RiseStoredChatPreviewMedia>,
    pub send_failed: Option<bool>,
    pub system_target: Option<MessageSystemMetadataTargetEnvelope>,
    pub sender: Option<RiseStoredChatPreviewSender>,
    pub author_signature: Option<String>,
}

impl RiseStoredChatPreview {
    /// the author name for the row, localized when the account is deleted
    pub fn display_sender_name(&self) -> Option<String> {
        if self.sender.as_ref().and_then(|sender| sender.is_deleted) == Some(true) {
            return Some(tr("chat_deleted_account"));
        }
        self.sender_name.clone()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseStoredChatPreviewSender {
    pub is_deleted: Option<bool>,
}

impl RiseStoredChatPreviewSender {
    pub fn new(is_deleted: Option<bool>) -> Self {
        Self { is_deleted }
    }
}

impl<'de> Deserialize<'de> for RiseStoredChatPreviewSender {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self { is_deleted: container.rise::<bool>(key::IS_DELETED)? })
        })
    }
}

impl Serialize for RiseStoredChatPreviewSender {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        encode_if_present(&mut container, &camel(key::IS_DELETED), &self.is_deleted)?;
        container.end()
    }
}

impl<'de> Deserialize<'de> for RiseStoredChatPreview {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            let stored_target =
                container.rise::<MessageSystemMetadataTargetEnvelope>(RiseLocalKey::SYSTEM_TARGET).ok().flatten();
            let wire_target =
                container.rise::<MessageSystemMetadataTargetEnvelope>(message_key::METADATA).ok().flatten();
            let system_target = [stored_target, wire_target].into_iter().flatten().find(|target| !target.is_empty());
            Ok(Self {
                id: container.rise::<String>(key::ID)?,
                chat_id: container.rise::<String>(key::CHAT_ID)?,
                topic_id: container.rise::<String>(key::TOPIC_ID)?,
                sender_id: container.rise::<String>(key::SENDER_ID)?,
                sender_name: container.rise::<String>(key::SENDER_NAME)?,
                content: container.rise::<String>(key::CONTENT)?,
                caption: container.rise::<String>(key::CAPTION)?,
                content_type: container.rise::<String>(key::CONTENT_TYPE)?,
                timestamp: container.rise::<f64>(key::TIMESTAMP)?,
                created_at: container.rise::<f64>(key::CREATED_AT)?,
                has_attachments: container.rise::<bool>(key::HAS_ATTACHMENTS)?,
                has_reactions: container.rise::<bool>(key::HAS_REACTIONS)?,
                is_pinned: container.rise::<bool>(key::IS_PINNED)?,
                is_read: container.rise::<bool>(key::IS_READ)?,
                is_system_message: container.rise::<bool>(key::IS_SYSTEM_MESSAGE)?,
                system_message_type: container.rise::<String>(key::SYSTEM_MESSAGE_TYPE)?,
                metadata: container.rise::<MessageCallInfoWireDTO>(message_key::METADATA)?,
                media_items: container.rise::<RiseStoredChatPreviewMedia>(message_key::MEDIA_ITEMS)?,
                media_info: container.rise::<RiseStoredChatPreviewMedia>(key::MEDIA_INFO)?,
                send_failed: container.rise::<bool>(RiseLocalKey::SEND_FAILED)?,
                system_target,
                sender: container.rise::<RiseStoredChatPreviewSender>(key::SENDER).ok().flatten(),
                author_signature: container.rise::<String>(message_key::AUTHOR_SIGNATURE).ok().flatten(),
            })
        })
    }
}

impl Serialize for RiseStoredChatPreview {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        encode_if_present(&mut container, key::ID, &self.id)?;
        encode_if_present(&mut container, &camel(key::CHAT_ID), &self.chat_id)?;
        encode_if_present(&mut container, &camel(key::TOPIC_ID), &self.topic_id)?;
        encode_if_present(&mut container, &camel(key::SENDER_ID), &self.sender_id)?;
        encode_if_present(&mut container, &camel(key::SENDER_NAME), &self.sender_name)?;
        encode_if_present(&mut container, key::CONTENT, &self.content)?;
        encode_if_present(&mut container, key::CAPTION, &self.caption)?;
        encode_if_present(&mut container, &camel(key::CONTENT_TYPE), &self.content_type)?;
        encode_if_present(&mut container, key::TIMESTAMP, &self.timestamp)?;
        encode_if_present(&mut container, &camel(key::CREATED_AT), &self.created_at)?;
        encode_if_present(&mut container, &camel(key::HAS_ATTACHMENTS), &self.has_attachments)?;
        encode_if_present(&mut container, &camel(key::HAS_REACTIONS), &self.has_reactions)?;
        encode_if_present(&mut container, &camel(key::IS_PINNED), &self.is_pinned)?;
        encode_if_present(&mut container, &camel(key::IS_READ), &self.is_read)?;
        encode_if_present(&mut container, &camel(key::IS_SYSTEM_MESSAGE), &self.is_system_message)?;
        encode_if_present(&mut container, &camel(key::SYSTEM_MESSAGE_TYPE), &self.system_message_type)?;
        encode_if_present(&mut container, message_key::METADATA, &self.metadata)?;
        encode_if_present(&mut container, &camel(message_key::MEDIA_ITEMS), &self.media_items)?;
        encode_if_present(&mut container, &camel(key::MEDIA_INFO), &self.media_info)?;
        encode_if_present(&mut container, &camel(RiseLocalKey::SEND_FAILED), &self.send_failed)?;
        encode_if_present(&mut container, &camel(RiseLocalKey::SYSTEM_TARGET), &self.system_target)?;
        encode_if_present(&mut container, key::SENDER, &self.sender)?;
        encode_if_present(&mut container, &camel(message_key::AUTHOR_SIGNATURE), &self.author_signature)?;
        container.end()
    }
}

/// attachments of the last message of a chat row
/// realtime brings media_items or older media_info, chat list sends media_info as json string
/// decoding never fails
#[derive(Clone, Debug, Default)]
pub struct RiseStoredChatPreviewMedia {
    pub items: Vec<MessageMediaInfoWireDTO>,
}

impl RiseStoredChatPreviewMedia {
    fn max_items() -> i64 {
        RiseLimits::value("message_media")
    }

    fn filtered(items: Vec<MessageMediaInfoWireDTO>) -> Self {
        Self { items }
    }

    pub fn make(items: Vec<MessageMediaInfoWireDTO>) -> Option<RiseStoredChatPreviewMedia> {
        let drawable: Vec<_> = items.into_iter().filter(MessageMediaInfoWireDTO::has_chat_row_source).collect();
        (!drawable.is_empty()).then(|| Self::filtered(drawable))
    }

    pub fn from_summary(raw: &str) -> Self {
        Self::filtered(
            Self::summary_items(raw).into_iter().filter(MessageMediaInfoWireDTO::has_chat_row_source).collect(),
        )
    }

    /// the summary names one item and the other album slots repeat it so the count survives the
    /// row cache
    fn summary_items(raw: &str) -> Vec<MessageMediaInfoWireDTO> {
        let Ok(object) = serde_json::from_str::<serde_json::Map<String, Value>>(raw) else {
            return Vec::new();
        };
        let text = |key: &str| non_empty_trimmed(object.get(key).and_then(Value::as_str));
        if let Some(emoji) = text(message_key::STICKER_EMOJI) {
            return vec![MessageMediaInfoWireDTO::from_chat_row_sticker_emoji(emoji)];
        }
        let file_url = text(message_key::FILE_URL);
        let thumbnail_url = text(message_key::THUMBNAIL_URL);
        let file_name = text(message_key::FILE_NAME);
        if file_url.is_none() && thumbnail_url.is_none() {
            return file_name
                .map(|name| {
                    let mut item = MessageMediaInfoWireDTO::from_chat_row_media_type(
                        text(message_key::MEDIA_TYPE).or_else(|| text(message_key::TYPE)),
                        None,
                        None,
                    );
                    item.file_name = Some(name);
                    vec![item]
                })
                .unwrap_or_default();
        }
        let mut item = MessageMediaInfoWireDTO::from_chat_row_media_type(text(message_key::MEDIA_TYPE), file_url, thumbnail_url);
        item.file_name = file_name;
        let count = match object.get(payload_key::MEDIA_COUNT) {
            Some(Value::Number(number)) => {
                rise_integer(number).or_else(|| number.as_f64().map(|value| value.trunc() as i64)).unwrap_or(1)
            }
            Some(Value::Bool(flag)) => i64::from(*flag),
            _ => 1,
        };
        let count = count.max(1).min(Self::max_items()).max(0);
        vec![item; usize::try_from(count).unwrap_or(0)]
    }
}

impl<'de> Deserialize<'de> for RiseStoredChatPreviewMedia {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer).unwrap_or(Value::Null);
        let decoded = if let Ok(items) = serde_json::from_value::<Vec<MessageMediaInfoWireDTO>>(value.clone()) {
            items
        } else if let Value::String(raw) = &value {
            Self::summary_items(raw)
        } else if let Ok(item) = serde_json::from_value::<MessageMediaInfoWireDTO>(value) {
            vec![item]
        } else {
            Vec::new()
        };
        Ok(Self { items: decoded.into_iter().filter(MessageMediaInfoWireDTO::has_chat_row_source).collect() })
    }
}

impl Serialize for RiseStoredChatPreviewMedia {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.items.serialize(serializer)
    }
}

impl MessageMediaInfoWireDTO {
    /// true when the chat row has a source to draw from
    /// sticker summary has only its emoji, file or audio only its name
    /// spoiler or view limited photos show no thumbnail in the list
    fn has_chat_row_source(&self) -> bool {
        if self.has_spoiler == Some(true) {
            return false;
        }
        [&self.file_url, &self.thumbnail_url, &self.local_preview_url, &self.sticker_id, &self.file_name]
            .into_iter()
            .any(|value| value.as_deref().is_some_and(|value| !trimming_whitespaces_and_newlines(value).is_empty()))
            || self.associated_emojis.as_ref().is_some_and(|emojis| !emojis.is_empty())
    }

    fn from_chat_row_sticker_emoji(emoji: String) -> Self {
        Self {
            media_type: Some(MessageMediaKind::Sticker.raw_value().to_string()),
            file_id: None,
            file_url: None,
            thumbnail_url: None,
            duration: None,
            width: None,
            height: None,
            file_name: None,
            file_size: None,
            mime_type: None,
            bitrate: None,
            fps: None,
            codec: None,
            waveform: None,
            sticker_id: None,
            pack_id: None,
            sticker_type: None,
            associated_emojis: Some(vec![emoji]),
            is_uploading: None,
            upload_progress: None,
            uploaded_bytes: None,
            total_bytes: None,
            is_compressing: None,
            compression_progress: None,
            is_cancelled: None,
            local_preview_url: None,
            local_slot_index: None,
            has_spoiler: None,
            view_mode: None,
            view_seconds: None,
            view_expires_at: None,
            view_state: None,
        }
    }

    fn from_chat_row_media_type(
        chat_row_media_type: Option<String>,
        file_url: Option<String>,
        thumbnail_url: Option<String>,
    ) -> Self {
        Self {
            media_type: chat_row_media_type,
            file_id: None,
            file_url,
            thumbnail_url,
            duration: None,
            width: None,
            height: None,
            file_name: None,
            file_size: None,
            mime_type: None,
            bitrate: None,
            fps: None,
            codec: None,
            waveform: None,
            sticker_id: None,
            pack_id: None,
            sticker_type: None,
            associated_emojis: None,
            is_uploading: None,
            upload_progress: None,
            uploaded_bytes: None,
            total_bytes: None,
            is_compressing: None,
            compression_progress: None,
            is_cancelled: None,
            local_preview_url: None,
            local_slot_index: None,
            has_spoiler: None,
            view_mode: None,
            view_seconds: None,
            view_expires_at: None,
            view_state: None,
        }
    }

    pub fn from_chat_row_item(item: &MessageMediaItem) -> Self {
        Self {
            media_type: Some(item.kind.raw_value().to_string()),
            file_id: Some(item.id.clone()),
            file_url: (!item.url.is_empty()).then(|| item.url.clone()),
            thumbnail_url: item.thumbnail_url.clone(),
            duration: item.duration,
            width: (item.width > 0.0).then(|| item.width as i64),
            height: (item.height > 0.0).then(|| item.height as i64),
            file_name: item.file_name.clone(),
            file_size: item.file_size,
            mime_type: item.mime_type.clone(),
            bitrate: item.bitrate,
            fps: item.fps.map(|fps| fps as f64),
            codec: item.codec.clone(),
            waveform: None,
            sticker_id: (item.kind == MessageMediaKind::Sticker).then(|| item.id.clone()),
            pack_id: item.pack_id.clone(),
            sticker_type: item.sticker_kind.map(|kind| kind.raw_value().to_string()),
            associated_emojis: (!item.sticker_emojis.is_empty()).then(|| item.sticker_emojis.clone()),
            is_uploading: Some(item.is_uploading),
            upload_progress: None,
            uploaded_bytes: None,
            total_bytes: None,
            is_compressing: Some(item.is_compressing),
            compression_progress: None,
            is_cancelled: None,
            local_preview_url: item.local_preview_url.clone(),
            local_slot_index: Some(item.local_slot_index),
            has_spoiler: (item.has_spoiler || item.view_limit.is_some()).then_some(true),
            view_mode: None,
            view_seconds: None,
            view_expires_at: None,
            view_state: None,
        }
    }
}

pub type ChatParticipantMoreWireDTO = ParticipantMoreData;

#[derive(Clone, Debug, Default)]
pub struct RiseStoredChatParticipant {
    pub id: String,
    pub name: Option<String>,
    pub tag: Option<String>,
    pub logo: Option<String>,
    pub avatar_url: Option<String>,
    pub is_online: Option<bool>,
    pub last_seen: Option<i64>,
    pub is_premium: Option<bool>,
    pub is_official: Option<bool>,
    pub more: Option<ChatParticipantMoreWireDTO>,
    pub is_deleted: Option<bool>,
    pub is_bot: Option<bool>,
    pub bot_owner_id: Option<String>,
    pub bot_is_active: Option<bool>,
    pub bot_users: Option<u64>,
    pub alias_tags: Option<Vec<String>>,
    pub premium_badge_emoji: Option<String>,
}

impl RiseStoredChatParticipant {
    pub fn resolved_logo(&self) -> String {
        [self.more.as_ref().map(|more| more.logo.as_str()), self.avatar_url.as_deref(), self.logo.as_deref()]
            .into_iter()
            .find_map(non_empty_trimmed)
            .unwrap_or_default()
    }

    pub fn resolved_is_online(&self) -> bool {
        self.more.as_ref().map(|more| more.is_online).or(self.is_online).unwrap_or(false)
    }

    pub fn resolved_last_seen(&self) -> Option<i64> {
        self.more.as_ref().map(|more| more.last_seen).or(self.last_seen)
    }
}

impl<'de> Deserialize<'de> for RiseStoredChatParticipant {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                id: container.rise_required::<String>(key::ID)?,
                name: container.rise::<String>(key::NAME)?,
                tag: container.rise::<String>(key::TAG)?,
                logo: container.rise::<String>(key::LOGO)?,
                avatar_url: container.rise::<String>(key::AVATAR_URL)?,
                is_online: container.rise::<bool>(key::IS_ONLINE)?,
                last_seen: container.rise::<i64>(key::LAST_SEEN)?,
                is_premium: container.rise::<bool>(key::IS_PREMIUM)?,
                is_official: container.rise::<bool>(key::IS_OFFICIAL)?,
                more: container.rise::<ChatParticipantMoreWireDTO>(key::MORE)?,
                is_deleted: container.rise::<bool>(key::IS_DELETED)?,
                is_bot: container.rise::<bool>(key::IS_BOT)?,
                bot_owner_id: container.rise::<String>(key::BOT_OWNER_ID)?,
                bot_is_active: container.rise::<bool>(key::BOT_IS_ACTIVE)?,
                bot_users: container.rise::<u64>(key::BOT_USERS)?,
                alias_tags: container.rise::<Vec<String>>(key::ALIAS_TAGS)?,
                premium_badge_emoji: container.rise::<String>(key::PREMIUM_BADGE_EMOJI)?,
            })
        })
    }
}

impl Serialize for RiseStoredChatParticipant {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::ID, &self.id)?;
        encode_if_present(&mut container, key::NAME, &self.name)?;
        encode_if_present(&mut container, key::TAG, &self.tag)?;
        encode_if_present(&mut container, key::LOGO, &self.logo)?;
        encode_if_present(&mut container, &camel(key::AVATAR_URL), &self.avatar_url)?;
        encode_if_present(&mut container, &camel(key::IS_ONLINE), &self.is_online)?;
        encode_if_present(&mut container, &camel(key::LAST_SEEN), &self.last_seen)?;
        encode_if_present(&mut container, &camel(key::IS_PREMIUM), &self.is_premium)?;
        encode_if_present(&mut container, &camel(key::IS_OFFICIAL), &self.is_official)?;
        encode_if_present(&mut container, key::MORE, &self.more)?;
        encode_if_present(&mut container, &camel(key::IS_DELETED), &self.is_deleted)?;
        encode_if_present(&mut container, &camel(key::IS_BOT), &self.is_bot)?;
        encode_if_present(&mut container, &camel(key::BOT_OWNER_ID), &self.bot_owner_id)?;
        encode_if_present(&mut container, &camel(key::BOT_IS_ACTIVE), &self.bot_is_active)?;
        encode_if_present(&mut container, &camel(key::BOT_USERS), &self.bot_users)?;
        encode_if_present(&mut container, &camel(key::ALIAS_TAGS), &self.alias_tags)?;
        encode_if_present(&mut container, &camel(key::PREMIUM_BADGE_EMOJI), &self.premium_badge_emoji)?;
        container.end()
    }
}

pub type ChatUnreadReactionsWireDTO = ProtoChatUnreadReactions;

pub type ChatUnreadCountWireDTO = ChatUnreadMessages;

pub type ChatUnreadMentionsWireDTO = ProtoChatUnreadMentions;

pub type ChatUnreadInfoWireDTO = ChatUnread;

#[derive(Clone, Debug)]
pub enum RiseStoredChatCreator {
    Id(String),
    User(MessageSenderWireDTO),
}

impl RiseStoredChatCreator {
    pub fn as_string(&self) -> Option<String> {
        match self {
            Self::Id(s) => Some(s.clone()),
            Self::User(u) => u.id.clone(),
        }
    }

    pub fn typed(&self) -> Option<&MessageSenderWireDTO> {
        match self {
            Self::Id(_) => None,
            Self::User(u) => Some(u),
        }
    }
}

impl<'de> Deserialize<'de> for RiseStoredChatCreator {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if let Value::String(s) = value {
            return Ok(Self::Id(s));
        }
        serde_json::from_value::<MessageSenderWireDTO>(value).map(Self::User).map_err(serde::de::Error::custom)
    }
}

impl Serialize for RiseStoredChatCreator {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Id(s) => s.serialize(serializer),
            Self::User(u) => u.serialize(serializer),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RiseStoredChat {
    pub id: String,

    pub server_id: Option<String>,
    pub title: Option<String>,
    pub r#type: Option<String>,
    pub description: Option<String>,
    pub logo_url: Option<String>,
    pub banner_url: Option<String>,
    pub username: Option<String>,
    pub tag: Option<String>,
    pub member_count: Option<i64>,
    pub is_verified: Option<bool>,
    pub is_public: Option<bool>,
    pub is_pinned: Option<bool>,
    pub pin_position: Option<i64>,
    pub is_muted: Option<bool>,
    pub unread_count: Option<i64>,
    pub role: Option<String>,
    pub created_at: Option<f64>,
    pub updated_at: Option<f64>,
    pub joined_at: Option<f64>,
    pub mute_until: Option<f64>,
    pub last_activity: Option<f64>,
    pub notifications_enabled: Option<bool>,
    pub can_invite_users: Option<bool>,
    pub can_pin_messages: Option<bool>,
    pub can_manage_chat: Option<bool>,
    pub can_accept_join_requests: Option<bool>,
    pub is_anonymous: Option<bool>,
    pub custom_title: Option<String>,
    pub custom_color: Option<String>,
    pub forward_in_chat_enabled: Option<bool>,
    pub allowed_reactions: Option<Vec<String>>,

    pub topics_enabled: Option<bool>,
    pub auto_delete_messages_after: Option<i64>,
    pub mute_scopes: Option<String>,
    pub participant: Option<RiseStoredChatParticipant>,
    pub last_message: Option<RiseStoredChatPreview>,
    pub creator_info: Option<RiseStoredChatCreator>,
    pub is_self_chat: Option<bool>,
    pub chat_type_enum: Option<i64>,
    pub is_message_request: Option<bool>,
    pub message_request_state: Option<String>,
    pub join_approval_required: Option<bool>,
    pub invite_link: Option<String>,
    pub can_share_invite: Option<bool>,
    pub has_live_voice: Option<bool>,
    pub wallpaper: Option<ChatWallpaperWireDTO>,

    pub unread_info: Option<ChatUnreadInfoWireDTO>,
    pub unread_media_count: Option<i64>,
    pub unread_mentions_count: Option<i64>,
    pub unread_replies_count: Option<i64>,
    pub unread_reactions_count: Option<i64>,
}

impl RiseStoredChat {
    pub fn creator(&self) -> Option<&MessageSenderWireDTO> {
        self.creator_info.as_ref().and_then(RiseStoredChatCreator::typed)
    }
}

impl<'de> Deserialize<'de> for RiseStoredChat {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                id: container.rise_required::<String>(key::ID)?,
                server_id: container.rise::<String>(RiseLocalKey::SERVER_ID)?,
                title: container.rise::<String>(key::TITLE)?,
                r#type: container.rise::<String>(key::TYPE)?,
                description: container.rise::<String>(key::DESCRIPTION)?,
                logo_url: container.rise::<String>(key::LOGO_URL)?,
                banner_url: container.rise::<String>(key::BANNER_URL)?,
                username: container.rise::<String>(key::USERNAME)?,
                tag: container.rise::<String>(key::TAG)?,
                member_count: container.rise::<i64>(key::MEMBER_COUNT)?,
                is_verified: container.rise::<bool>(key::IS_VERIFIED)?,
                is_public: container.rise::<bool>(key::IS_PUBLIC)?,
                is_pinned: container.rise::<bool>(key::IS_PINNED)?,
                pin_position: container.rise::<i64>(key::PIN_POSITION)?,
                is_muted: container.rise::<bool>(key::IS_MUTED)?,
                unread_count: container.rise::<i64>(key::UNREAD_COUNT)?,
                role: container.rise::<String>(key::ROLE)?,
                created_at: container.rise::<f64>(key::CREATED_AT)?,
                updated_at: container.rise::<f64>(key::UPDATED_AT)?,
                joined_at: container.rise::<f64>(key::JOINED_AT)?,
                mute_until: container.rise::<f64>(key::MUTE_UNTIL)?,
                last_activity: container.rise::<f64>(key::LAST_ACTIVITY)?,
                notifications_enabled: container.rise::<bool>(key::NOTIFICATIONS_ENABLED)?,
                can_invite_users: container.rise::<bool>(key::CAN_INVITE_USERS)?,
                can_pin_messages: container.rise::<bool>(key::CAN_PIN_MESSAGES)?,
                can_manage_chat: container.rise::<bool>(key::CAN_MANAGE_CHAT)?,
                can_accept_join_requests: container.rise::<bool>(key::CAN_ACCEPT_JOIN_REQUESTS)?,
                is_anonymous: container.rise::<bool>(key::IS_ANONYMOUS)?,
                custom_title: container.rise::<String>(key::CUSTOM_TITLE)?,
                custom_color: container.rise::<String>(key::CUSTOM_COLOR)?,
                forward_in_chat_enabled: container.rise::<bool>(key::FORWARD_IN_CHAT_ENABLED)?,
                allowed_reactions: container.rise::<Vec<String>>(key::ALLOWED_REACTIONS)?,
                topics_enabled: container.rise::<bool>(key::TOPICS_ENABLED)?,
                auto_delete_messages_after: container.rise::<i64>(key::AUTO_DELETE_MESSAGES_AFTER)?,
                mute_scopes: container.rise::<String>(key::MUTE_SCOPES)?,
                participant: container.rise::<RiseStoredChatParticipant>(key::PARTICIPANT)?,
                last_message: container.rise::<RiseStoredChatPreview>(key::LAST_MESSAGE)?,
                creator_info: container.rise::<RiseStoredChatCreator>(key::CREATOR_INFO)?,
                is_self_chat: container.rise::<bool>(key::IS_SELF_CHAT)?,
                chat_type_enum: container.rise::<i64>(key::CHAT_TYPE_ENUM)?,
                is_message_request: container.rise::<bool>(key::IS_MESSAGE_REQUEST)?,
                message_request_state: container.rise::<String>(key::MESSAGE_REQUEST_STATE)?,
                join_approval_required: container.rise::<bool>(key::JOIN_APPROVAL_REQUIRED)?,
                invite_link: container.rise::<String>(key::INVITE_LINK)?,
                can_share_invite: container.rise::<bool>(key::CAN_SHARE_INVITE)?,
                has_live_voice: container.rise::<bool>(key::HAS_LIVE_VOICE)?,
                wallpaper: container.rise::<ChatWallpaperWireDTO>(key::WALLPAPER)?,
                unread_info: container.rise::<ChatUnreadInfoWireDTO>(key::UNREAD_INFO)?,
                unread_media_count: container.rise::<i64>(key::UNREAD_MEDIA_COUNT)?,
                unread_mentions_count: container.rise::<i64>(key::UNREAD_MENTIONS_COUNT)?,
                unread_replies_count: container.rise::<i64>(key::UNREAD_REPLIES_COUNT)?,
                unread_reactions_count: container.rise::<i64>(key::UNREAD_REACTIONS_COUNT)?,
            })
        })
    }
}

impl Serialize for RiseStoredChat {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(key::ID, &self.id)?;
        encode_if_present(&mut container, &camel(RiseLocalKey::SERVER_ID), &self.server_id)?;
        encode_if_present(&mut container, key::TITLE, &self.title)?;
        encode_if_present(&mut container, key::TYPE, &self.r#type)?;
        encode_if_present(&mut container, key::DESCRIPTION, &self.description)?;
        encode_if_present(&mut container, &camel(key::LOGO_URL), &self.logo_url)?;
        encode_if_present(&mut container, &camel(key::BANNER_URL), &self.banner_url)?;
        encode_if_present(&mut container, key::USERNAME, &self.username)?;
        encode_if_present(&mut container, key::TAG, &self.tag)?;
        encode_if_present(&mut container, &camel(key::MEMBER_COUNT), &self.member_count)?;
        encode_if_present(&mut container, &camel(key::IS_VERIFIED), &self.is_verified)?;
        encode_if_present(&mut container, &camel(key::IS_PUBLIC), &self.is_public)?;
        encode_if_present(&mut container, &camel(key::IS_PINNED), &self.is_pinned)?;
        encode_if_present(&mut container, &camel(key::PIN_POSITION), &self.pin_position)?;
        encode_if_present(&mut container, &camel(key::IS_MUTED), &self.is_muted)?;
        encode_if_present(&mut container, &camel(key::UNREAD_COUNT), &self.unread_count)?;
        encode_if_present(&mut container, key::ROLE, &self.role)?;
        encode_if_present(&mut container, &camel(key::CREATED_AT), &self.created_at)?;
        encode_if_present(&mut container, &camel(key::UPDATED_AT), &self.updated_at)?;
        encode_if_present(&mut container, &camel(key::JOINED_AT), &self.joined_at)?;
        encode_if_present(&mut container, &camel(key::MUTE_UNTIL), &self.mute_until)?;
        encode_if_present(&mut container, &camel(key::LAST_ACTIVITY), &self.last_activity)?;
        encode_if_present(&mut container, &camel(key::NOTIFICATIONS_ENABLED), &self.notifications_enabled)?;
        encode_if_present(&mut container, &camel(key::CAN_INVITE_USERS), &self.can_invite_users)?;
        encode_if_present(&mut container, &camel(key::CAN_PIN_MESSAGES), &self.can_pin_messages)?;
        encode_if_present(&mut container, &camel(key::CAN_MANAGE_CHAT), &self.can_manage_chat)?;
        encode_if_present(&mut container, &camel(key::CAN_ACCEPT_JOIN_REQUESTS), &self.can_accept_join_requests)?;
        encode_if_present(&mut container, &camel(key::IS_ANONYMOUS), &self.is_anonymous)?;
        encode_if_present(&mut container, &camel(key::CUSTOM_TITLE), &self.custom_title)?;
        encode_if_present(&mut container, &camel(key::CUSTOM_COLOR), &self.custom_color)?;
        encode_if_present(&mut container, &camel(key::FORWARD_IN_CHAT_ENABLED), &self.forward_in_chat_enabled)?;
        encode_if_present(&mut container, &camel(key::ALLOWED_REACTIONS), &self.allowed_reactions)?;
        encode_if_present(&mut container, &camel(key::TOPICS_ENABLED), &self.topics_enabled)?;
        encode_if_present(&mut container, &camel(key::AUTO_DELETE_MESSAGES_AFTER), &self.auto_delete_messages_after)?;
        encode_if_present(&mut container, &camel(key::MUTE_SCOPES), &self.mute_scopes)?;
        encode_if_present(&mut container, key::PARTICIPANT, &self.participant)?;
        encode_if_present(&mut container, &camel(key::LAST_MESSAGE), &self.last_message)?;
        encode_if_present(&mut container, &camel(key::CREATOR_INFO), &self.creator_info)?;
        encode_if_present(&mut container, &camel(key::IS_SELF_CHAT), &self.is_self_chat)?;
        encode_if_present(&mut container, &camel(key::CHAT_TYPE_ENUM), &self.chat_type_enum)?;
        encode_if_present(&mut container, &camel(key::IS_MESSAGE_REQUEST), &self.is_message_request)?;
        encode_if_present(&mut container, &camel(key::MESSAGE_REQUEST_STATE), &self.message_request_state)?;
        encode_if_present(&mut container, &camel(key::JOIN_APPROVAL_REQUIRED), &self.join_approval_required)?;
        encode_if_present(&mut container, &camel(key::INVITE_LINK), &self.invite_link)?;
        encode_if_present(&mut container, &camel(key::CAN_SHARE_INVITE), &self.can_share_invite)?;
        encode_if_present(&mut container, &camel(key::HAS_LIVE_VOICE), &self.has_live_voice)?;
        encode_if_present(&mut container, key::WALLPAPER, &self.wallpaper)?;
        encode_if_present(&mut container, &camel(key::UNREAD_INFO), &self.unread_info)?;
        encode_if_present(&mut container, &camel(key::UNREAD_MEDIA_COUNT), &self.unread_media_count)?;
        encode_if_present(&mut container, &camel(key::UNREAD_MENTIONS_COUNT), &self.unread_mentions_count)?;
        encode_if_present(&mut container, &camel(key::UNREAD_REPLIES_COUNT), &self.unread_replies_count)?;
        encode_if_present(&mut container, &camel(key::UNREAD_REACTIONS_COUNT), &self.unread_reactions_count)?;
        container.end()
    }
}

/// gateway spelling of the chat wide wallpaper, same on the entity the set answer and the
/// chat_updated frame
pub type ChatWallpaperWireDTO = ProtoChatWallpaper;

/// extension methods for the wallpaper wire type
pub trait ChatWallpaperWireDTOExt: Sized {
    fn from_placement(placement: &ChatWallpaperPlacement) -> Self;
    fn to_placement(&self) -> Option<ChatWallpaperPlacement>;
    fn from_canon(card: &ChatWallpaperCard) -> Self;
}

impl ChatWallpaperWireDTOExt for ProtoChatWallpaper {
    fn from_placement(placement: &ChatWallpaperPlacement) -> Self {
        Self {
            blur_percent: placement.look.blur_percent,
            dim_percent: placement.look.dim_percent,
            id: placement.id.clone(),
            image_url: placement.image_url.clone(),
            preview_url: placement.preview_url.clone(),
            set_by_user_id: placement.set_by_user_id.clone(),
            updated_at_ms: placement.updated_at_ms,
        }
    }

    fn to_placement(&self) -> Option<ChatWallpaperPlacement> {
        let id = trimming_whitespaces_and_newlines(&self.id);
        let image = trimming_whitespaces_and_newlines(&self.image_url);
        if id.is_empty() || image.is_empty() {
            return None;
        }
        let preview = trimming_whitespaces_and_newlines(&self.preview_url);
        Some(ChatWallpaperPlacement {
            id: id.to_owned(),
            image_url: image.to_owned(),
            preview_url: if preview.is_empty() { image.to_owned() } else { preview.to_owned() },
            look: ChatWallpaperLook::new(self.dim_percent, self.blur_percent).clamped(),
            set_by_user_id: self.set_by_user_id.clone(),
            updated_at_ms: self.updated_at_ms,
        })
    }

    fn from_canon(card: &ChatWallpaperCard) -> Self {
        Self {
            blur_percent: card.blur_percent.unwrap_or(0),
            dim_percent: card.dim_percent.unwrap_or(0),
            id: card.id.clone().unwrap_or_default(),
            image_url: card.image_url.clone().unwrap_or_default(),
            preview_url: card.preview_url.clone().unwrap_or_default(),
            set_by_user_id: card.set_by_user_id.clone().unwrap_or_default(),
            updated_at_ms: card.updated_at_ms.unwrap_or(0),
        }
    }
}

pub type SetChatWallpaperGatewayDTO = SetChatWallpaper;
pub type ChatWallpaperCatalogEntryWireDTO = ProtoChatWallpaperCatalogEntry;

/// extension methods for the wallpaper catalog entry wire type
pub trait ChatWallpaperCatalogEntryWireDTOExt {
    fn to_entry(&self) -> Option<ChatWallpaperCatalogEntry>;
}

impl ChatWallpaperCatalogEntryWireDTOExt for ProtoChatWallpaperCatalogEntry {
    fn to_entry(&self) -> Option<ChatWallpaperCatalogEntry> {
        let id = trimming_whitespaces_and_newlines(&self.id);
        let image = trimming_whitespaces_and_newlines(&self.image_url);
        if id.is_empty() || image.is_empty() {
            return None;
        }
        let preview = trimming_whitespaces_and_newlines(&self.preview_url);
        Some(ChatWallpaperCatalogEntry {
            id: id.to_owned(),
            chat_id: self.chat_id.clone(),
            topic_id: self.topic_id.clone(),
            image_url: image.to_owned(),
            preview_url: if preview.is_empty() { image.to_owned() } else { preview.to_owned() },
            look: ChatWallpaperLook::new(self.dim_percent, self.blur_percent).clamped(),
            added_by_user_id: self.added_by_user_id.clone(),
            created_at_ms: self.created_at_ms,
        })
    }
}

/// add, get and delete all answer with the whole catalogue
pub type ChatWallpaperCatalogGatewayDTO = ProtoChatWallpaperCatalogResponse;

#[derive(Clone, Debug, Default)]
pub struct RiseStoredChatPage {
    pub chats: Option<Vec<RiseStoredChat>>,
    pub relative_id: Option<String>,
    pub limit: Option<i64>,
    pub is_have_more: Option<bool>,
    pub success: Option<bool>,
    pub error: Option<String>,
}

impl RiseStoredChatPage {
    pub fn new(
        chats: Option<Vec<RiseStoredChat>>,
        relative_id: Option<String>,
        limit: Option<i64>,
        is_have_more: Option<bool>,
        success: Option<bool>,
        error: Option<String>,
    ) -> Self {
        Self { chats, relative_id, limit, is_have_more, success, error }
    }
}

impl<'de> Deserialize<'de> for RiseStoredChatPage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                chats: container.rise::<Vec<RiseStoredChat>>(key::CHATS)?,
                relative_id: container.rise::<String>(key::RELATIVE_ID)?,
                limit: container.rise::<i64>(key::LIMIT)?,
                is_have_more: container.rise::<bool>(key::IS_HAVE_MORE)?,
                success: container.rise::<bool>(key::SUCCESS)?,
                error: container.rise::<String>(key::ERROR)?,
            })
        })
    }
}

impl Serialize for RiseStoredChatPage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        encode_if_present(&mut container, key::CHATS, &self.chats)?;
        encode_if_present(&mut container, &camel(key::RELATIVE_ID), &self.relative_id)?;
        encode_if_present(&mut container, key::LIMIT, &self.limit)?;
        encode_if_present(&mut container, &camel(key::IS_HAVE_MORE), &self.is_have_more)?;
        encode_if_present(&mut container, key::SUCCESS, &self.success)?;
        encode_if_present(&mut container, key::ERROR, &self.error)?;
        container.end()
    }
}

pub type CreateChatResultDTO = CreatedChat;

/// extension methods for the created chat reply
pub trait CreateChatResultDTOExt {
    fn chat_info(&self) -> Option<CanonChatInfo>;
    fn full_chat(&self) -> Option<RiseStoredChat>;
}

impl CreateChatResultDTOExt for CreatedChat {
    fn chat_info(&self) -> Option<CanonChatInfo> {
        let id = self.id.clone().filter(|id| !id.is_empty())?;
        Some(CanonChatInfo {
            unread_media_count: self.unread_media_count.unwrap_or(0),
            tag: self.tag.clone(),
            unread_info: self.unread_info.clone(),
            allowed_reactions: self.allowed_reactions.clone().unwrap_or_default(),
            auto_delete_messages_after: self.auto_delete_messages_after,
            banner_url: self.banner_url.clone(),
            can_accept_join_requests: self.can_accept_join_requests.unwrap_or(false),
            can_ban_members: self.can_ban_members.unwrap_or(false),
            can_invite_users: self.can_invite_users.unwrap_or(false),
            can_kick_members: self.can_kick_members.unwrap_or(false),
            can_manage_chat: self.can_manage_chat.unwrap_or(false),
            can_pin_messages: self.can_pin_messages.unwrap_or(false),
            can_share_invite: self.can_share_invite.unwrap_or(false),
            chat_type_enum: self.chat_type_enum.unwrap_or(0),
            creator_info: self.creator_info.clone(),
            custom_color: self.custom_color.clone(),
            custom_title: self.custom_title.clone(),
            description: self.description.clone(),
            encryption_enabled: self.encryption_enabled.unwrap_or(false),
            forward_in_chat_enabled: self.forward_in_chat_enabled,
            has_live_voice: self.has_live_voice.unwrap_or(false),
            id,
            invite_link: self.invite_link.clone(),
            is_anonymous: self.is_anonymous.unwrap_or(false),
            is_message_request: self.is_message_request.unwrap_or(false),
            is_muted: self.is_muted.unwrap_or(false),
            is_pinned: self.is_pinned.unwrap_or(false),
            is_public: self.is_public,
            is_self_chat: self.is_self_chat.unwrap_or(false),
            is_verified: self.is_verified.unwrap_or(false),
            join_approval_required: self.join_approval_required.unwrap_or(false),
            joined_at: self.joined_at,
            last_activity: self.last_activity,
            last_message: self.last_message.clone(),
            logo_url: self.logo_url.clone(),
            member_count: self.member_count.unwrap_or(0),
            message_request_state: self.message_request_state.clone(),
            mute_scopes: self.mute_scopes.clone(),
            mute_until: self.mute_until,
            notifications_enabled: self.notifications_enabled.unwrap_or(false),
            participant: self.participant.clone(),
            pin_position: self.pin_position,
            role: self.role.clone(),
            title: self.title.clone(),
            topics_enabled: self.topics_enabled.unwrap_or(false),
            r#type: self.r#type.clone(),
            unread_count: self.unread_count.unwrap_or(0),
            updated_at: self.updated_at.unwrap_or(0),
            user_chat_id: self.user_chat_id.clone(),
            username: self.username.clone(),
            wallpaper: self.wallpaper.clone(),
        })
    }

    fn full_chat(&self) -> Option<RiseStoredChat> {
        let chat_info = self.chat_info()?;
        chat_wire_types_hooks().map(|hooks| (hooks.stored_chat_from_canon)(&chat_info))
    }
}

#[derive(Clone, Debug, Default)]
pub struct RiseStoredChatDetail {
    pub chat: Option<RiseStoredChat>,
    pub is_archived: Option<bool>,
    pub success: Option<bool>,
    pub error: Option<String>,
}

impl<'de> Deserialize<'de> for RiseStoredChatDetail {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                chat: container.rise::<RiseStoredChat>(key::CHAT)?,
                is_archived: container.rise::<bool>(key::IS_ARCHIVED)?,
                success: container.rise::<bool>(key::SUCCESS)?,
                error: container.rise::<String>(key::ERROR)?,
            })
        })
    }
}

impl Serialize for RiseStoredChatDetail {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        encode_if_present(&mut container, key::CHAT, &self.chat)?;
        encode_if_present(&mut container, &camel(key::IS_ARCHIVED), &self.is_archived)?;
        encode_if_present(&mut container, key::SUCCESS, &self.success)?;
        encode_if_present(&mut container, key::ERROR, &self.error)?;
        container.end()
    }
}

pub type ChatMemberUserWireDTO = ProtoChatMemberUser;
pub type ChatMemberWireDTO = ProtoChatMember;
pub type ChatMembersPaginationWireDTO = ProtoPaginationInfo;
pub type GetChatMembersGatewayDTO = GetChatMembers;

impl RiseStoredChatPreview {
    pub fn to_message(&self) -> GetMessageMessage {
        let ts = self.timestamp.or(self.created_at).unwrap_or(0.0);
        let created_sec = if ts > MILLISECOND_TIMESTAMP_FLOOR { ts / 1000.0 } else { ts };
        let mut msg = GetMessageMessage::new(self.id.clone().unwrap_or_else(|| Uuid::new_v4().uuid_string()));
        msg.chat_id = self.chat_id.clone().unwrap_or_default();
        msg.topic_id = self.topic_id.clone();
        msg.sender_id = self.sender_id.clone().unwrap_or_default();
        let display_sender_name = self.display_sender_name();
        msg.sender_name = display_sender_name.clone().unwrap_or_default();
        msg.author_signature = self.author_signature.clone();
        let preview_content = self.content.clone().unwrap_or_default();
        msg.text = if trimming_whitespaces_and_newlines(&preview_content).is_empty() {
            self.caption.clone().unwrap_or(preview_content)
        } else {
            preview_content
        };
        msg.content_type = self.content_type.as_deref().map_or(MessageContentType::Text, MessageContentType::from_raw_value);
        if msg.content_type == MessageContentType::Call {
            msg.text = String::new();
            msg.call_info = self.metadata.as_ref().and_then(MessageCallInfoWireDTO::to_model);
        }
        msg.adopt_location_payload();
        msg.adopt_poll_payload();
        msg.created_at = created_sec;
        msg.is_pinned = self.is_pinned.unwrap_or(false);
        msg.is_read = self.is_read.unwrap_or(false);
        if self.send_failed == Some(true) {
            msg.delivery_status = MessageDeliveryStatus::Failed;
        } else if GetSaiServerId::is_temp_id(self.id.as_deref()) {
            msg.delivery_status = MessageDeliveryStatus::Sending;
        }
        let has_system_type = self.system_message_type.as_deref().is_some_and(|kind| !kind.is_empty());
        if self.is_system_message == Some(true) || has_system_type {
            msg.content_type = MessageContentType::System;
            let actor_source = display_sender_name.unwrap_or_default();
            let actor = trimming_whitespaces_and_newlines(&actor_source);
            let actor_name = (!actor.is_empty() && actor != UNKNOWN_SYSTEM_ACTOR).then(|| actor.to_owned());
            let mut system = MessageSystemInfo {
                kind: self
                    .system_message_type
                    .as_deref()
                    .and_then(MessageSystemKind::from_raw)
                    .unwrap_or(MessageSystemKind::Generic),
                actor_id: self.sender_id.clone(),
                actor_name,
                ..MessageSystemInfo::default()
            };
            if let Some(target) = &self.system_target {
                system.target_id = non_empty_trimmed(target.target_user_id.as_deref());
                system.target_name = non_empty_trimmed(target.target_name.as_deref());
            }
            msg.system = Some(system);
        }
        if msg.content_type != MessageContentType::System && msg.content_type != MessageContentType::Call {
            let media = self
                .media_items
                .as_ref()
                .map(|media| &media.items)
                .filter(|items| !items.is_empty())
                .or_else(|| self.media_info.as_ref().map(|media| &media.items));
            let owner_message_id = msg.id.clone();
            let owner_content_type = msg.content_type.clone();
            msg.media_items = media
                .map(|items| {
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            item.to_media_item(
                                i64::try_from(index).unwrap_or(i64::MAX),
                                &owner_message_id,
                                Some(owner_content_type.clone()),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
        }
        msg
    }
}

impl RiseStoredChat {
    /// unread_count is authoritative whenever the wire carries it, local writes patch only that key
    /// unread_info.messages duplicates it server side and is never kept locally
    /// taking the larger of both would be a ratchet the badge never leaves
    pub fn resolved_unread_count(&self) -> i64 {
        if let Some(unread_count) = self.unread_count {
            return unread_count.max(0);
        }
        self.unread_info
            .as_ref()
            .and_then(|info| info.messages.as_ref())
            .and_then(|messages| messages.count)
            .unwrap_or(0)
            .max(0)
    }

    /// never above resolved_unread_count, a read photo cant still be unread whatever order the
    /// keys were patched in
    pub fn resolved_unread_media_count(&self) -> i64 {
        self.unread_media_count.unwrap_or(0).max(0).min(self.resolved_unread_count())
    }

    pub fn resolved_unread_mentions_count(&self) -> i64 {
        self.unread_mentions_count
            .or_else(|| self.unread_info.as_ref().and_then(|info| info.mentions.as_ref()).map(|mentions| mentions.count))
            .unwrap_or(0)
            .max(0)
    }

    pub fn resolved_unread_replies_count(&self) -> i64 {
        self.unread_replies_count
            .or_else(|| self.unread_info.as_ref().and_then(|info| info.replies.as_ref()).map(|replies| replies.count))
            .unwrap_or(0)
            .max(0)
    }

    pub fn resolved_unread_reactions_count(&self) -> i64 {
        self.unread_reactions_count
            .or_else(|| {
                self.unread_info.as_ref().and_then(|info| info.reactions.as_ref()).map(|reactions| reactions.count)
            })
            .unwrap_or(0)
            .max(0)
    }

    /// same question as would a message here have pushed
    /// no when is_muted or notifications are off or a timed mute still runs or every scope is off
    /// legacy rows carry mute_until in seconds, told apart by the canon floor
    pub fn counts_toward_unread_badge(&self) -> bool {
        if self.is_muted == Some(true) || self.notifications_enabled == Some(false) {
            return false;
        }
        if let Some(mute_until) = self.mute_until.filter(|mute_until| *mute_until > 0.0) {
            let floor = MUTE_UNTIL_MILLISECOND_FLOOR as f64;
            let seconds = if mute_until > floor { mute_until / 1_000.0 } else { mute_until };
            if seconds > Date::now().time_interval_since_1970() {
                return false;
            }
        }
        !ChatMuteScopes::parse(self.mute_scopes.as_deref()).is_some_and(|scopes| scopes.is_fully_muted())
    }

    fn channel_acting(message: Option<GetMessageMessage>, chat_type: ChatType, title: &str) -> Option<GetMessageMessage> {
        let mut message = message?;
        let acts_as_admin = chat_type == ChatType::Channel
            && message.content_type == MessageContentType::System
            && message.system.as_ref().is_some_and(|system| system.actor_id.as_deref().is_some_and(|id| !id.is_empty()));
        if !acts_as_admin {
            return Some(message);
        }
        let name = trimming_whitespaces_and_newlines(title);
        if name.is_empty() {
            return Some(message);
        }
        if let Some(system) = message.system.as_mut() {
            system.actor_id = None;
            system.actor_name = Some(name.to_owned());
        }
        message.sender_name = name.to_owned();
        Some(message)
    }

    pub fn to_chat_info(&self) -> ChatInfo {
        let normalized_type = if self.is_self_chat == Some(true) {
            ChatType::Favourites
        } else if let Some(matched) =
            self.r#type.as_deref().filter(|wire| !wire.is_empty()).and_then(ChatType::matching)
        {
            matched
        } else if self.chat_type_enum == i64::try_from(ChatType::Favourites.proto_number()).ok() {
            ChatType::Favourites
        } else {
            ChatType::Direct
        };
        let participant_user = match &self.participant {
            None => User {
                id: String::new(),
                name: String::new(),
                tag: String::new(),
                alias_tags: Vec::new(),
                more: AuthorInfoMore::default(),
                is_online: false,
                last_seen: None,
                is_deleted: false,
                is_bot: false,
                bot_owner_id: None,
                bot_is_active: false,
                bot_users: None,
            },
            Some(p) => User {
                id: p.id.clone(),
                name: p.name.clone().unwrap_or_default(),
                tag: p.tag.clone().unwrap_or_default(),
                alias_tags: p.alias_tags.clone().unwrap_or_default(),
                more: AuthorInfoMore {
                    logo: p.resolved_logo(),
                    is_premium: p.is_premium.unwrap_or(false),
                    is_official: p.is_official.unwrap_or(false),
                    premium_badge_emoji: p.premium_badge_emoji.clone(),
                    ..AuthorInfoMore::default()
                },
                is_online: p.resolved_is_online(),
                last_seen: p.resolved_last_seen(),
                is_deleted: p.is_deleted.unwrap_or(false),
                is_bot: p.is_bot.unwrap_or(false),
                bot_owner_id: p.bot_owner_id.clone(),
                bot_is_active: p.bot_is_active.unwrap_or(false),
                bot_users: p.bot_users.map(|count| i64::try_from(count).unwrap_or(i64::MAX)),
            },
        };
        let updated = self.updated_at.or(self.last_activity).or(self.created_at).unwrap_or(0.0);

        let resolved_id = match self.server_id.as_deref() {
            Some(sid) if GetSaiServerId::is_temp_id(Some(self.id.as_str())) && !sid.is_empty() => sid.to_owned(),
            _ => self.id.clone(),
        };
        let resolved_title = match self.title.as_deref() {
            Some(title) if !title.is_empty() => title.to_owned(),
            _ if normalized_type == ChatType::Direct => participant_user.name.clone(),
            _ => String::new(),
        };
        let last_message =
            Self::channel_acting(self.last_message.as_ref().map(RiseStoredChatPreview::to_message), normalized_type, &resolved_title);
        ChatInfo {
            id: resolved_id,
            title: resolved_title,
            r#type: normalized_type,
            last_message,
            unread_count: self.resolved_unread_count(),
            unread_mentions_count: self.resolved_unread_mentions_count(),
            unread_replies_count: self.resolved_unread_replies_count(),
            unread_reactions_count: self.resolved_unread_reactions_count(),
            is_pinned: self.is_pinned.unwrap_or(false),
            pin_position: self.pin_position,
            is_muted: self.is_muted.unwrap_or(false),
            updated_at: if updated > MILLISECOND_TIMESTAMP_FLOOR { updated / 1000.0 } else { updated },
            allowed_reactions: self.allowed_reactions.clone().unwrap_or_default(),
            created_at: self.created_at.unwrap_or(0.0),
            description: self.description.clone(),
            logo_url: self.logo_url.clone(),
            banner_url: self.banner_url.clone(),
            wallpaper: self.wallpaper.as_ref().and_then(ChatWallpaperWireDTOExt::to_placement),
            username: self.username.clone(),
            member_count: self.member_count.unwrap_or(0),
            is_verified: self.is_verified.unwrap_or(false),
            is_public: self.is_public.unwrap_or(false),
            role: self.role.clone(),
            joined_at: self.joined_at,
            notifications_enabled: self.notifications_enabled.unwrap_or(true),
            mute_until: self.mute_until,
            custom_title: self.custom_title.clone(),
            is_anonymous: self.is_anonymous.unwrap_or(false),
            custom_color: self.custom_color.clone(),
            last_activity: self.last_activity,
            can_invite_users: self.can_invite_users.unwrap_or(true),
            can_pin_messages: self.can_pin_messages.unwrap_or(true),
            can_manage_chat: self.can_manage_chat.unwrap_or(false),
            can_accept_join_requests: self.can_accept_join_requests.unwrap_or(false),
            creator_info: self.creator_info.as_ref().and_then(RiseStoredChatCreator::as_string),
            participant: participant_user,
            tag: self.tag.clone(),
            forward_in_chat_enabled: self.forward_in_chat_enabled.unwrap_or(true),
            typing_datas: Vec::new(),
            topics_enabled: self.topics_enabled.unwrap_or(false),
            auto_delete_messages_after: self.auto_delete_messages_after,
            mute_scopes: self.mute_scopes.clone(),
            is_message_request: self.is_message_request.unwrap_or(false),
            message_request_state: self.message_request_state.clone(),
            join_approval_required: self.join_approval_required.unwrap_or(false),
            invite_link: self.invite_link.clone(),
            can_share_invite: self.can_share_invite.unwrap_or(false),
            has_live_voice: self.has_live_voice.unwrap_or(false),
        }
    }
}

impl ChatInfo {
    pub fn to_wire_dto(&self, existing: Option<&RiseStoredChat>) -> RiseStoredChat {
        let wire_type = self.r#type.wire().to_uppercase();
        let participant = &self.participant;
        let participant_wire = if participant.id.is_empty() {
            existing.and_then(|existing| existing.participant.clone())
        } else {
            Some(RiseStoredChatParticipant {
                id: participant.id.clone(),
                name: Some(participant.name.clone()),
                tag: Some(participant.tag.clone()),
                logo: Some(participant.more.logo.clone()),
                avatar_url: Some(participant.more.logo.clone()),
                is_online: Some(participant.is_online),
                last_seen: participant.last_seen,
                is_premium: Some(participant.more.is_premium),
                is_official: Some(participant.more.is_official),
                more: Some(ChatParticipantMoreWireDTO {
                    is_online: participant.is_online,
                    last_seen: participant.last_seen.unwrap_or(0),
                    logo: participant.more.logo.clone(),
                    ..ChatParticipantMoreWireDTO::default()
                }),
                is_deleted: Some(participant.is_deleted),
                is_bot: Some(participant.is_bot),
                bot_owner_id: participant.bot_owner_id.clone(),
                bot_is_active: Some(participant.bot_is_active),
                bot_users: participant.bot_users.map(|count| u64::try_from(count.max(0)).unwrap_or(0)),
                alias_tags: Some(participant.alias_tags.clone()),
                premium_badge_emoji: participant.more.premium_badge_emoji.clone(),
            })
        };
        let last_wire = self.last_message.as_ref().map(|msg| RiseStoredChatPreview {
            id: Some(msg.id.clone()),
            chat_id: Some(msg.chat_id.clone()),
            topic_id: if self.topics_enabled {
                Some(msg.topic_id.clone().unwrap_or_default())
            } else {
                msg.topic_id.clone()
            },
            sender_id: Some(msg.sender_id.clone()),
            sender_name: Some(msg.sender_name.clone()),
            content: Some(msg.text.clone()),
            caption: None,
            content_type: Some(msg.content_type.raw_value().to_string()),
            timestamp: Some(msg.created_at),
            created_at: Some(msg.created_at),
            has_attachments: Some(!msg.media_items.is_empty()),
            has_reactions: Some(!msg.reactions.is_empty()),
            is_pinned: Some(msg.is_pinned),
            is_read: Some(msg.is_read),
            is_system_message: Some(msg.content_type == MessageContentType::System),
            system_message_type: msg.system.as_ref().map(|system| system.kind.raw_value().to_string()),
            metadata: msg.call_info.as_ref().map(MessageCallInfoWireDTO::from_model),
            media_items: RiseStoredChatPreviewMedia::make(
                msg.media_items.iter().map(MessageMediaInfoWireDTO::from_chat_row_item).collect(),
            ),
            media_info: None,
            send_failed: (msg.delivery_status == MessageDeliveryStatus::Failed).then_some(true),
            system_target: msg.system.as_ref().and_then(|system| {
                let target = MessageSystemMetadataTargetEnvelope {
                    target_user_id: system.target_id.clone(),
                    target_name: system.target_name.clone(),
                };
                (!target.is_empty()).then_some(target)
            }),
            sender: None,
            author_signature: msg.author_signature.clone(),
        });
        RiseStoredChat {
            id: self.id.clone(),

            server_id: None,
            title: Some(self.title.clone()),
            r#type: Some(wire_type),
            description: self.description.clone().or_else(|| existing.and_then(|e| e.description.clone())),
            logo_url: self.logo_url.clone().or_else(|| existing.and_then(|e| e.logo_url.clone())),
            banner_url: self.banner_url.clone().or_else(|| existing.and_then(|e| e.banner_url.clone())),
            username: self.username.clone().or_else(|| existing.and_then(|e| e.username.clone())),
            tag: self.tag.clone().or_else(|| existing.and_then(|e| e.tag.clone())),
            member_count: Some(self.member_count),
            is_verified: Some(self.is_verified),
            is_public: Some(self.is_public),
            is_pinned: Some(self.is_pinned),
            pin_position: self.pin_position,
            is_muted: Some(self.is_muted),
            unread_count: Some(self.unread_count),
            role: self.role.clone().or_else(|| existing.and_then(|e| e.role.clone())),
            created_at: if self.created_at > 0.0 { Some(self.created_at) } else { existing.and_then(|e| e.created_at) },
            updated_at: if self.updated_at > 0.0 { Some(self.updated_at) } else { existing.and_then(|e| e.updated_at) },
            joined_at: self.joined_at.or_else(|| existing.and_then(|e| e.joined_at)),
            mute_until: self.mute_until.or_else(|| existing.and_then(|e| e.mute_until)),
            last_activity: self.last_activity.or_else(|| existing.and_then(|e| e.last_activity)),
            notifications_enabled: Some(self.notifications_enabled),
            can_invite_users: Some(self.can_invite_users),
            can_pin_messages: Some(self.can_pin_messages),
            can_manage_chat: Some(self.can_manage_chat),
            can_accept_join_requests: None,
            is_anonymous: Some(self.is_anonymous),
            custom_title: self.custom_title.clone().or_else(|| existing.and_then(|e| e.custom_title.clone())),
            custom_color: self.custom_color.clone().or_else(|| existing.and_then(|e| e.custom_color.clone())),
            forward_in_chat_enabled: Some(self.forward_in_chat_enabled),
            allowed_reactions: if self.allowed_reactions.is_empty() {
                existing.and_then(|e| e.allowed_reactions.clone())
            } else {
                Some(self.allowed_reactions.clone())
            },
            topics_enabled: Some(self.topics_enabled),
            auto_delete_messages_after: self.auto_delete_messages_after,
            mute_scopes: self.mute_scopes.clone().or_else(|| existing.and_then(|e| e.mute_scopes.clone())),
            participant: participant_wire,
            last_message: last_wire.or_else(|| existing.and_then(|e| e.last_message.clone())),
            creator_info: self
                .creator_info
                .clone()
                .map(RiseStoredChatCreator::Id)
                .or_else(|| existing.and_then(|e| e.creator_info.clone())),
            is_self_chat: Some(self.r#type.is_self_chat()),
            chat_type_enum: if self.r#type.is_self_chat() {
                i64::try_from(ChatType::Favourites.proto_number()).ok()
            } else {
                existing.and_then(|e| e.chat_type_enum)
            },
            is_message_request: Some(self.is_message_request),
            message_request_state: self.message_request_state.clone(),
            join_approval_required: Some(self.join_approval_required),
            invite_link: self.invite_link.clone(),
            can_share_invite: Some(self.can_share_invite),
            has_live_voice: Some(self.has_live_voice),
            wallpaper: self
                .wallpaper
                .as_ref()
                .map(ChatWallpaperWireDTO::from_placement)
                .or_else(|| existing.and_then(|e| e.wallpaper.clone())),
            unread_info: None,
            unread_media_count: None,
            unread_mentions_count: Some(self.unread_mentions_count),
            unread_replies_count: Some(self.unread_replies_count),
            unread_reactions_count: Some(self.unread_reactions_count),
        }
    }
}

pub type VoiceChannelConfigWireDTO = ProtoVoiceChannelConfig;
pub type VoiceChannelParticipantWireDTO = ProtoVoiceParticipant;
pub type VoiceChannelStateWireDTO = ProtoVoiceChannelState;

pub trait VoiceChannelConfigWireDTOExt {
    fn to_voice_channel_config(&self, kind: ChatChannelKind) -> Option<VoiceChannelConfig>;
    fn from_canon(card: &VoiceChannelConfigCard) -> Self;
}

impl VoiceChannelConfigWireDTOExt for ProtoVoiceChannelConfig {
    fn to_voice_channel_config(&self, kind: ChatChannelKind) -> Option<VoiceChannelConfig> {
        let mut resolved = VoiceChannelConfig::defaults(kind)?;
        resolved.user_limit = self.user_limit;
        resolved.speaker_limit = self.speaker_limit;
        resolved.audio_bitrate_kbps = self.audio_bitrate_kbps;
        resolved.video_enabled = self.video_enabled;
        resolved.screen_share_enabled = self.screen_share_enabled;
        resolved.default_muted = self.default_muted;
        if !self.region.is_empty() {
            resolved.region = self.region.clone();
        }
        resolved.normalized(kind).ok()
    }

    fn from_canon(card: &VoiceChannelConfigCard) -> Self {
        Self {
            audio_bitrate_kbps: card.audio_bitrate_kbps,
            default_muted: card.default_muted,
            region: card.region.clone(),
            screen_share_enabled: card.screen_share_enabled,
            speaker_limit: card.speaker_limit,
            user_limit: card.user_limit,
            video_enabled: card.video_enabled,
        }
    }
}

/// extension methods for the voice participant wire type
pub trait VoiceChannelParticipantWireDTOExt {
    fn to_participant(&self) -> VoiceChannelParticipant;
}

impl VoiceChannelParticipantWireDTOExt for ProtoVoiceParticipant {
    fn to_participant(&self) -> VoiceChannelParticipant {
        VoiceChannelParticipant {
            user_id: self.user_id.clone(),
            name: self.name.clone(),
            tag: self.tag.clone(),
            avatar_url: self.avatar_url.clone(),
            is_premium: self.is_premium,
            is_official: self.is_official,
            role_name: self.role_name.clone(),
            role_color: self.role_color,
            microphone_enabled: self.microphone_enabled,
            camera_enabled: self.camera_enabled,
            screen_share_enabled: self.screen_share_enabled,
            deafened: self.deafened,
            server_muted: self.server_muted,
            server_deafened: self.server_deafened,
            connected: self.connected,
            joined_at_ms: self.joined_at_ms,
            order_sequence: self.order_sequence,
            role: RiseRTCParticipantRole::parse(&self.role),
            hand_raised: self.hand_raised,
            hand_expires_at_ms: (self.hand_expires_at_ms > 0).then_some(self.hand_expires_at_ms),
        }
    }
}

/// extension methods for the voice channel state wire type
pub trait VoiceChannelStateWireDTOExt {
    fn to_voice_channel_state(&self) -> VoiceChannelState;
}

impl VoiceChannelStateWireDTOExt for ProtoVoiceChannelState {
    fn to_voice_channel_state(&self) -> VoiceChannelState {
        let mut resolved: Vec<VoiceChannelParticipant> =
            self.participants.iter().map(VoiceChannelParticipantWireDTOExt::to_participant).collect();
        resolved.sort_by_key(|participant| participant.order_sequence);
        let count = i64::try_from(resolved.len()).unwrap_or(i64::MAX).max(self.participant_count);
        let session_started_at_ms = VoiceChannelState::resolve_session_start(None, &self.room_id, &resolved, count);
        VoiceChannelState {
            room_id: self.room_id.clone(),
            participant_count: count,
            user_limit: self.user_limit.max(0),
            participants: resolved,
            has_more: self.has_more,
            generated_at_ms: self.generated_at_ms,
            session_started_at_ms,
            visible_limit: VoiceChannelState::COLLAPSED_ROWS,
        }
    }
}

fn decoded_channel_kind(value: &Value) -> ChatChannelKind {
    match value {
        Value::String(raw) => {
            let raw = trimming_whitespaces_and_newlines(raw).to_lowercase();
            ChatChannelKind::ALL.into_iter().find(|kind| kind.wire() == raw).unwrap_or(ChatChannelKind::Text)
        }
        Value::Number(number) => rise_integer(number)
            .and_then(|proto_number| u64::try_from(proto_number).ok())
            .and_then(|proto_number| ChatChannelKind::ALL.into_iter().find(|kind| kind.proto_number() == proto_number))
            .unwrap_or(ChatChannelKind::Text),
        _ => ChatChannelKind::Text,
    }
}

fn channel_kind_from_raw(raw: &str) -> Option<ChatChannelKind> {
    ChatChannelKind::ALL.into_iter().find(|kind| kind.wire() == raw)
}

/// a topic as the forum keeps it, the canon topic whose last message row is the stored chat
/// preview
/// patched locally by sends and reads with keys only this device knows
#[derive(Clone, Debug, Default)]
pub struct RiseStoredChatTopic {
    pub topic_id: String,
    pub chat_id: Option<String>,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub created_by: Option<String>,
    pub created_at: Option<i64>,
    pub position: Option<i64>,
    pub is_general: Option<bool>,
    pub is_closed: Option<bool>,
    pub section_id: Option<String>,
    pub is_private: Option<bool>,
    pub notification_messages: Option<bool>,
    pub notification_mentions: Option<bool>,
    pub notification_reactions: Option<bool>,
    pub notification_events: Option<bool>,
    pub notification_replies: Option<bool>,
    pub notification_inherited: Option<bool>,
    pub last_message: Option<RiseStoredChatPreview>,
    pub unread_count: Option<i64>,
    pub unread_mentions_count: Option<i64>,
    pub unread_replies_count: Option<i64>,
    pub unread_reactions_count: Option<i64>,
    pub is_muted: Option<bool>,
    pub updated_at: Option<f64>,
    pub channel_kind: Option<ChatChannelKind>,
    pub voice_config: Option<VoiceChannelConfigWireDTO>,
    pub voice_state: Option<VoiceChannelStateWireDTO>,
    pub wallpaper: Option<ChatWallpaperWireDTO>,
}

impl<'de> Deserialize<'de> for RiseStoredChatTopic {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                topic_id: container.rise_required::<String>(key::TOPIC_ID)?,
                chat_id: container.rise::<String>(key::CHAT_ID)?,
                title: container.rise::<String>(key::TITLE)?,
                icon: container.rise::<String>(key::ICON)?,
                created_by: container.rise::<String>(key::CREATED_BY)?,
                created_at: container.rise::<i64>(key::CREATED_AT)?,
                position: container.rise::<i64>(key::POSITION)?,
                is_general: container.rise::<bool>(key::IS_GENERAL)?,
                is_closed: container.rise::<bool>(key::IS_CLOSED)?,
                section_id: container.rise::<String>(key::SECTION_ID)?,
                is_private: container.rise::<bool>(key::IS_PRIVATE)?,
                notification_messages: container.rise::<bool>(key::NOTIFICATION_MESSAGES)?,
                notification_mentions: container.rise::<bool>(key::NOTIFICATION_MENTIONS)?,
                notification_reactions: container.rise::<bool>(key::NOTIFICATION_REACTIONS)?,
                notification_events: container.rise::<bool>(key::NOTIFICATION_EVENTS)?,
                notification_replies: container.rise::<bool>(key::NOTIFICATION_REPLIES)?,
                notification_inherited: container.rise::<bool>(key::NOTIFICATION_INHERITED)?,
                last_message: container.rise::<RiseStoredChatPreview>(key::LAST_MESSAGE)?,
                unread_count: container.rise::<i64>(key::UNREAD_COUNT)?,
                unread_mentions_count: container.rise::<i64>(key::UNREAD_MENTIONS_COUNT)?,
                unread_replies_count: container.rise::<i64>(key::UNREAD_REPLIES_COUNT)?,
                unread_reactions_count: container.rise::<i64>(key::UNREAD_REACTIONS_COUNT)?,
                is_muted: container.rise::<bool>(key::IS_MUTED)?,
                updated_at: container.rise::<f64>(key::UPDATED_AT)?,
                channel_kind: container.rise::<Value>(key::CHANNEL_KIND)?.map(|value| decoded_channel_kind(&value)),
                voice_config: container.rise::<VoiceChannelConfigWireDTO>(key::VOICE_CONFIG)?,
                voice_state: container.rise::<VoiceChannelStateWireDTO>(key::VOICE_STATE)?,
                wallpaper: container.rise::<ChatWallpaperWireDTO>(key::WALLPAPER)?,
            })
        })
    }
}

impl Serialize for RiseStoredChatTopic {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(&camel(key::TOPIC_ID), &self.topic_id)?;
        encode_if_present(&mut container, &camel(key::CHAT_ID), &self.chat_id)?;
        encode_if_present(&mut container, key::TITLE, &self.title)?;
        encode_if_present(&mut container, key::ICON, &self.icon)?;
        encode_if_present(&mut container, &camel(key::CREATED_BY), &self.created_by)?;
        encode_if_present(&mut container, &camel(key::CREATED_AT), &self.created_at)?;
        encode_if_present(&mut container, key::POSITION, &self.position)?;
        encode_if_present(&mut container, &camel(key::IS_GENERAL), &self.is_general)?;
        encode_if_present(&mut container, &camel(key::IS_CLOSED), &self.is_closed)?;
        encode_if_present(&mut container, &camel(key::SECTION_ID), &self.section_id)?;
        encode_if_present(&mut container, &camel(key::IS_PRIVATE), &self.is_private)?;
        encode_if_present(&mut container, &camel(key::NOTIFICATION_MESSAGES), &self.notification_messages)?;
        encode_if_present(&mut container, &camel(key::NOTIFICATION_MENTIONS), &self.notification_mentions)?;
        encode_if_present(&mut container, &camel(key::NOTIFICATION_REACTIONS), &self.notification_reactions)?;
        encode_if_present(&mut container, &camel(key::NOTIFICATION_EVENTS), &self.notification_events)?;
        encode_if_present(&mut container, &camel(key::NOTIFICATION_REPLIES), &self.notification_replies)?;
        encode_if_present(&mut container, &camel(key::NOTIFICATION_INHERITED), &self.notification_inherited)?;
        encode_if_present(&mut container, &camel(key::LAST_MESSAGE), &self.last_message)?;
        encode_if_present(&mut container, &camel(key::UNREAD_COUNT), &self.unread_count)?;
        encode_if_present(&mut container, &camel(key::UNREAD_MENTIONS_COUNT), &self.unread_mentions_count)?;
        encode_if_present(&mut container, &camel(key::UNREAD_REPLIES_COUNT), &self.unread_replies_count)?;
        encode_if_present(&mut container, &camel(key::UNREAD_REACTIONS_COUNT), &self.unread_reactions_count)?;
        encode_if_present(&mut container, &camel(key::IS_MUTED), &self.is_muted)?;
        encode_if_present(&mut container, &camel(key::UPDATED_AT), &self.updated_at)?;
        encode_if_present(&mut container, &camel(key::CHANNEL_KIND), &self.channel_kind.map(|kind| kind.wire()))?;
        encode_if_present(&mut container, &camel(key::VOICE_CONFIG), &self.voice_config)?;
        encode_if_present(&mut container, &camel(key::VOICE_STATE), &self.voice_state)?;
        encode_if_present(&mut container, key::WALLPAPER, &self.wallpaper)?;
        container.end()
    }
}

impl RiseStoredChatTopic {
    pub fn from_canon(topic: &TopicInfo) -> Self {
        Self {
            topic_id: topic.topic_id.clone(),
            chat_id: Some(topic.chat_id.clone()),
            title: Some(topic.title.clone()),
            icon: Some(topic.icon.clone()),
            created_by: Some(topic.created_by.clone()),
            created_at: Some(topic.created_at),
            position: Some(topic.position),
            is_general: Some(topic.is_general),
            is_closed: Some(topic.is_closed),
            section_id: Some(topic.section_id.clone()),
            is_private: Some(topic.is_private),
            notification_messages: Some(topic.notification_messages),
            notification_mentions: Some(topic.notification_mentions),
            notification_reactions: Some(topic.notification_reactions),
            notification_events: Some(topic.notification_events),
            notification_replies: Some(topic.notification_replies),
            notification_inherited: Some(topic.notification_inherited),
            last_message: topic.last_message.as_ref().and_then(|preview| {
                chat_wire_types_hooks().map(|hooks| (hooks.stored_chat_preview_from_canon)(preview, &topic.chat_id))
            }),
            unread_count: Some(topic.unread_count),
            unread_mentions_count: Some(topic.unread_mentions_count),
            unread_replies_count: Some(topic.unread_replies_count),
            unread_reactions_count: Some(topic.unread_reactions_count),
            is_muted: Some(topic.is_muted),
            updated_at: Some(topic.updated_at as f64),
            channel_kind: channel_kind_from_raw(&topic.channel_kind),
            voice_config: topic.voice_config.clone(),
            voice_state: topic.voice_state.clone(),
            wallpaper: topic.wallpaper.clone(),
        }
    }

    pub fn from_canon_card(card: &ChatTopicCard) -> Self {
        Self {
            topic_id: card.topic_id.clone(),
            chat_id: Some(card.chat_id.clone()),
            title: Some(card.title.clone()),
            icon: Some(card.icon.clone()),
            created_by: Some(card.created_by.clone()),
            created_at: Some(card.created_at),
            position: Some(card.position),
            is_general: Some(card.is_general),
            is_closed: Some(card.is_closed),
            section_id: Some(card.section_id.clone()),
            is_private: Some(card.is_private),
            notification_messages: None,
            notification_mentions: None,
            notification_reactions: None,
            notification_events: None,
            notification_replies: None,
            notification_inherited: None,
            last_message: None,
            unread_count: None,
            unread_mentions_count: None,
            unread_replies_count: None,
            unread_reactions_count: None,
            is_muted: None,
            updated_at: Some(card.updated_at as f64),
            channel_kind: channel_kind_from_raw(&card.channel_kind),
            voice_config: card.voice_config.as_ref().map(VoiceChannelConfigWireDTO::from_canon),
            voice_state: None,
            wallpaper: card.wallpaper.as_ref().map(ChatWallpaperWireDTO::from_canon),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RiseStoredChatTopicList {
    pub topics: Option<Vec<RiseStoredChatTopic>>,
    pub general_topic: Option<RiseStoredChatTopic>,
    pub hide_all_topics: Option<bool>,
}

impl<'de> Deserialize<'de> for RiseStoredChatTopicList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                topics: container.rise::<Vec<RiseStoredChatTopic>>(key::TOPICS)?,
                general_topic: container.rise::<RiseStoredChatTopic>(key::GENERAL_TOPIC)?,
                hide_all_topics: container.rise::<bool>(key::HIDE_ALL_TOPICS)?,
            })
        })
    }
}

impl Serialize for RiseStoredChatTopicList {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        encode_if_present(&mut container, key::TOPICS, &self.topics)?;
        encode_if_present(&mut container, &camel(key::GENERAL_TOPIC), &self.general_topic)?;
        encode_if_present(&mut container, &camel(key::HIDE_ALL_TOPICS), &self.hide_all_topics)?;
        container.end()
    }
}

pub type CreateTopicResultDTO = TopicCreated;

impl RiseStoredChatTopic {
    pub fn to_topic(&self) -> ChatTopic {
        let resolved_icon = self
            .icon
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|icon| !icon.is_empty())
            .map_or_else(|| ChatTopic::DEFAULT_ICON.to_owned(), str::to_owned);
        let message = self.last_message.as_ref().map(RiseStoredChatPreview::to_message);
        let raw_updated_at = self.updated_at.or_else(|| message.as_ref().map(|message| message.created_at)).unwrap_or(0.0);
        let normalized_updated_at =
            if raw_updated_at > MILLISECOND_TIMESTAMP_FLOOR { raw_updated_at / 1_000.0 } else { raw_updated_at };
        let resolved_channel_kind = self.channel_kind.unwrap_or(ChatChannelKind::Text);
        let resolved_voice_config = if resolved_channel_kind.is_rtc() {
            self.voice_config
                .as_ref()
                .and_then(|config| config.to_voice_channel_config(resolved_channel_kind))
                .or_else(|| VoiceChannelConfig::defaults(resolved_channel_kind))
        } else {
            None
        };

        ChatTopic {
            id: self.topic_id.clone(),
            title: self.title.clone().unwrap_or_default(),
            icon: resolved_icon,
            position: self.position.unwrap_or(0),
            is_general: self.is_general.unwrap_or(false),
            is_closed: self.is_closed.unwrap_or(false),
            is_closed_known: self.is_closed.is_some(),
            section_id: self.section_id.clone().unwrap_or_default(),
            is_private: self.is_private.unwrap_or(false),
            notification_scopes: ChatMuteScopes {
                messages: self.notification_messages.unwrap_or(true),
                mentions: self.notification_mentions.unwrap_or(true),
                replies: self.notification_replies.or(self.notification_mentions).unwrap_or(true),
                reactions: self.notification_reactions.unwrap_or(true),
                events: self.notification_events.unwrap_or(true),
            },
            notification_inherited: self.notification_inherited.unwrap_or(true),
            last_message: message,
            unread_count: self.unread_count.unwrap_or(0).max(0),
            unread_mentions_count: self.unread_mentions_count.unwrap_or(0).max(0),
            unread_replies_count: self.unread_replies_count.unwrap_or(0).max(0),
            unread_reactions_count: self.unread_reactions_count.unwrap_or(0).max(0),
            is_muted: self.is_muted.unwrap_or(false),
            updated_at: normalized_updated_at,
            channel_kind: resolved_channel_kind,
            voice_config: resolved_voice_config,
            voice_state: if resolved_channel_kind.is_rtc() {
                self.voice_state.as_ref().map(VoiceChannelStateWireDTOExt::to_voice_channel_state)
            } else {
                None
            },
            wallpaper: self.wallpaper.as_ref().and_then(ChatWallpaperWireDTOExt::to_placement),
        }
    }
}

pub type InviteLinkWireDTO = ProtoInviteLink;
pub type InviteLinkStatsWireDTO = ProtoInviteLinkStats;
pub type GetInviteLinksGatewayDTO = GetInviteLinks;
pub type CreateInviteLinkResultDTO = CreateInviteLink;
pub type GetVoiceInviteLinkResultDTO = rise_canon::rise_response::chat::GetVoiceInviteLink;

pub enum InviteLinkTime {}

impl InviteLinkTime {
    pub fn normalize(raw: Option<i64>) -> f64 {
        let Some(raw) = raw.filter(|raw| *raw > 0) else {
            return 0.0;
        };
        let v = raw as f64;
        if v > MILLISECOND_TIMESTAMP_FLOOR { v / 1000.0 } else { v }
    }
}

pub trait InviteLinkWireDTOExt {
    fn to_invite_link(&self) -> InviteLink;
}

impl InviteLinkWireDTOExt for ProtoInviteLink {
    fn to_invite_link(&self) -> InviteLink {
        InviteLink {
            id: self.id.clone(),
            link: self.link.clone(),
            name: self.name.clone(),
            created_at: InviteLinkTime::normalize(Some(self.created_at)),
            expires_at: (self.expires_at.unwrap_or(0) > 0).then(|| InviteLinkTime::normalize(self.expires_at)),
            usage_limit: self.usage_limit.filter(|limit| *limit > 0),
            used_count: self.used_count,
            is_revoked: self.is_revoked,
            creates_join_request: self.creates_join_request,
            is_primary: self.is_primary,
        }
    }
}

pub type JoinRequestUserMoreWireDTO = ProtoJoinRequestsUserMore;
pub type JoinRequestUserWireDTO = ProtoJoinRequestsUser;
pub type JoinRequestWireDTO = ProtoJoinRequestWithUser;
pub type GetJoinRequestsGatewayDTO = GetJoinRequests;

pub trait JoinRequestWireDTOExt {
    fn to_join_request(&self) -> JoinRequest;
}

impl JoinRequestWireDTOExt for ProtoJoinRequestWithUser {
    fn to_join_request(&self) -> JoinRequest {
        let user = self.user.as_ref();
        JoinRequest {
            id: self.id.clone(),
            chat_id: self.chat_id.clone(),
            user_id: if self.user_id.is_empty() {
                user.map(|user| user.id.clone()).unwrap_or_default()
            } else {
                self.user_id.clone()
            },
            requested_at: InviteLinkTime::normalize(Some(self.requested_at)),
            status: if self.status.is_empty() {
                JoinRequestStatus::Pending.wire().to_owned()
            } else {
                self.status.clone()
            },
            message: self.message.clone().filter(|message| !message.is_empty()),
            user_name: user.map(|user| user.name.clone()).unwrap_or_default(),
            user_tag: user.map(|user| user.tag.clone()).unwrap_or_default(),
            user_logo: user.and_then(|user| user.more.as_ref()).map(|more| more.logo.clone()).unwrap_or_default(),
            is_premium: user.is_some_and(|user| user.is_premium),
            is_official: user.is_some_and(|user| user.is_official),
            premium_badge_emoji: user.and_then(|user| user.premium_badge_emoji.clone()),
            name_color: user.and_then(|user| user.name_color.clone()),
        }
    }
}

pub type GetChatPreviewGatewayDTO = GetChatPreviewByInviteLink;

/// extension methods for the chat preview by invite link reply
pub trait GetChatPreviewGatewayDTOExt {
    fn to_chat_join_preview(&self) -> Option<ChatJoinPreview>;
}

impl GetChatPreviewGatewayDTOExt for GetChatPreviewByInviteLink {
    fn to_chat_join_preview(&self) -> Option<ChatJoinPreview> {
        let hooks = chat_wire_types_hooks();
        let Some(chat) = self.chat.as_ref() else {
            return self.to_private_voice_join_preview();
        };
        let chat = hooks.map(|hooks| (hooks.stored_chat_from_canon)(chat))?.to_chat_info();
        let raw_status = self
            .join_status
            .as_deref()
            .map(|status| trimming_whitespaces_and_newlines(status).to_lowercase())
            .unwrap_or_default();
        let member = self.is_member || raw_status == JOIN_STATUS_ALREADY_MEMBER;
        let requires_approval = raw_status == JOIN_STATUS_REQUIRES_APPROVAL
            || (self.invite_link_info.as_ref().is_some_and(|info| info.creates_join_request) && raw_status.is_empty());
        let joinable = self.can_join;
        let resolved_status = if !raw_status.is_empty() {
            raw_status
        } else if member {
            JOIN_STATUS_ALREADY_MEMBER.to_owned()
        } else if requires_approval {
            JOIN_STATUS_REQUIRES_APPROVAL.to_owned()
        } else if self.can_join {
            key::CAN_JOIN.to_owned()
        } else {
            JOIN_STATUS_CHAT_UNAVAILABLE.to_owned()
        };
        let voice = self.is_voice_invite.then(|| voice_join_preview(self, &chat.id));
        Some(ChatJoinPreview {
            chat_id: chat.id,
            title: chat.title,
            logo_url: chat.logo_url,
            description: chat.description,
            member_count: chat.member_count,
            r#type: chat.r#type,
            is_verified: chat.is_verified,
            is_member: member,
            can_join: joinable,
            requires_approval,
            join_status: resolved_status,
            voice,
        })
    }
}

fn voice_join_preview(dto: &GetChatPreviewByInviteLink, chat_id: &str) -> ChatVoiceJoinPreview {
    let topic_id = dto
        .voice
        .as_ref()
        .and_then(|voice| voice.topic_id.clone())
        .or_else(|| dto.invite_link_info.as_ref().and_then(|info| info.voice.as_ref()).and_then(|voice| voice.topic_id.clone()));
    let target = match topic_id.filter(|topic_id| !topic_id.is_empty()) {
        Some(topic_id) => RiseRTCRoomTarget::Topic { chat_id: chat_id.to_owned(), topic_id },
        None => RiseRTCRoomTarget::Group { chat_id: chat_id.to_owned() },
    };
    ChatVoiceJoinPreview {
        target,
        is_text: dto.invite_link_info.as_ref().and_then(|info| info.voice.as_ref()).is_some_and(|voice| voice.text),
        topic_title: dto
            .voice
            .as_ref()
            .and_then(|voice| voice.topic_title.clone())
            .or_else(|| dto.invite_link_info.as_ref().and_then(|info| info.topic_title.clone())),
        live: dto.voice.as_ref().is_some_and(|voice| voice.live),
        participant_count: dto.voice.as_ref().map_or(0, |voice| voice.participant_count),
        can_join_call: dto.voice.as_ref().is_some_and(|voice| voice.can_join_call),
        details_hidden: dto.voice.is_none(),
    }
}

trait PrivateVoiceJoinPreview {
    fn to_private_voice_join_preview(&self) -> Option<ChatJoinPreview>;
}

impl PrivateVoiceJoinPreview for GetChatPreviewByInviteLink {
    fn to_private_voice_join_preview(&self) -> Option<ChatJoinPreview> {
        let chat_id = self.invite_link_info.as_ref().map(|info| info.chat_id.clone()).filter(|id| !id.is_empty());
        let chat_id = chat_id.filter(|_| self.is_voice_invite)?;
        let raw_status = self
            .join_status
            .as_deref()
            .map(|status| trimming_whitespaces_and_newlines(status).to_lowercase())
            .unwrap_or_default();
        let requires_approval = raw_status == JOIN_STATUS_REQUIRES_APPROVAL;
        let is_text = self.invite_link_info.as_ref().and_then(|info| info.voice.as_ref()).is_some_and(|voice| voice.text);
        let join_status = if !raw_status.is_empty() {
            raw_status.clone()
        } else if self.can_join {
            key::CAN_JOIN.to_owned()
        } else {
            JOIN_STATUS_CHAT_UNAVAILABLE.to_owned()
        };
        Some(ChatJoinPreview {
            voice: Some(voice_join_preview(self, &chat_id)),
            chat_id,
            title: tr(if is_text { "topic_invite_preview_title" } else { "voice_invite_preview_title" }),
            logo_url: None,
            description: Some(tr(if is_text { "topic_invite_preview_private" } else { "voice_invite_preview_private" })),
            member_count: 0,
            r#type: ChatType::Group,
            is_verified: false,
            is_member: self.is_member || raw_status == JOIN_STATUS_ALREADY_MEMBER,
            can_join: self.can_join,
            requires_approval,
            join_status,
        })
    }
}

pub type JoinByLinkGatewayDTO = JoinChatByLink;
pub type RequestJoinGatewayDTO = RequestJoinChat;

pub type BlockedMemberWireDTO = ProtoBlockedMember;
pub type GetBlockedMembersGatewayDTO = GetBlockedMembers;

pub trait BlockedMemberWireDTOExt {
    fn to_blocked_member(&self) -> BlockedMember;
}

impl BlockedMemberWireDTOExt for ProtoBlockedMember {
    fn to_blocked_member(&self) -> BlockedMember {
        let user = self.user.as_ref();
        BlockedMember {
            user_id: self.user_id.clone(),
            name: user.map_or_else(|| tr("chat_deleted_account"), |user| user.name.clone()),
            tag: user.map(|user| user.tag.clone()).unwrap_or_default(),
            avatar_url: user.and_then(|user| user.avatar_url.clone()).unwrap_or_default(),
            is_premium: user.is_some_and(|user| user.is_premium),
            is_official: user.is_some_and(|user| user.is_official),
            banned_by_name: self.banned_by_user.as_ref().map(|user| user.name.clone()).unwrap_or_default(),
            banned_at: InviteLinkTime::normalize(Some(self.banned_at)),
            reason: self.reason.clone(),
            ban_type: if self.ban_type.is_empty() { BanType::Ban.wire().to_owned() } else { self.ban_type.clone() },
        }
    }
}

pub type ChatActionUserWireDTO = ProtoActionUser;
pub type ChatActionWireDTO = ProtoChatAction;
pub type GetChatActionsGatewayDTO = GetChatActionsHistory;

pub trait ChatActionWireDTOExt {
    fn to_chat_action(&self) -> ChatAction;
}

impl ChatActionWireDTOExt for ProtoChatAction {
    fn to_chat_action(&self) -> ChatAction {
        ChatAction {
            id: self.action_id.clone(),
            action_type: self.action_type.clone(),
            actor_name: self.actor.as_ref().map(|actor| actor.name.clone()).unwrap_or_default(),
            target_name: self.target.as_ref().map(|target| target.name.clone()).unwrap_or_default(),
            created_at: InviteLinkTime::normalize(Some(self.created_at)),
            details: self.action_details.clone(),
        }
    }
}

pub type ChatSettingsGatewayDTO = ProtoChatSettings;

/// extension methods for the chat settings wire type
pub trait ChatSettingsGatewayDTOExt {
    fn to_model(&self, chat_id: &str) -> ChatSettingsModel;
}

impl ChatSettingsGatewayDTOExt for ProtoChatSettings {
    fn to_model(&self, chat_id: &str) -> ChatSettingsModel {
        ChatSettingsModel {
            chat_id: if self.chat_id.is_empty() { chat_id.to_owned() } else { self.chat_id.clone() },
            reactions_type: ChatReactionsType::with_wire(self.allowed_reactions_type.as_deref()),
            allowed_reactions: self.allowed_reactions.clone(),
            max_reaction_count: self.max_reaction_count.unwrap_or(DEFAULT_MAX_REACTION_COUNT),
            slow_mode: self.slow_mode.unwrap_or(0),
            members_can_add_members: self.members_can_add_members.unwrap_or(true),
            members_can_change_pfp: self.members_can_change_pfp.unwrap_or(false),
            message_allow: ChatPermissionJSON::parse(self.message_allow.as_deref()),
            media_allow: ChatPermissionJSON::parse(self.media_allow.as_deref()),
            linked_chat_id: self.linked_chat_id.clone(),
            paid_reactions_allowed: self.paid_reactions_allowed,
        }
    }
}

pub type LinkedChatGatewayDTO = ProtoLinkedChatInfo;

/// extension methods for the linked chat wire type
pub trait LinkedChatGatewayDTOExt {
    fn to_info(&self) -> Option<LinkedChatInfo>;
}

impl LinkedChatGatewayDTOExt for ProtoLinkedChatInfo {
    fn to_info(&self) -> Option<LinkedChatInfo> {
        if self.chat_id.is_empty() {
            return None;
        }
        Some(LinkedChatInfo {
            id: self.chat_id.clone(),
            title: self.title.clone(),
            logo_url: self.logo_url.clone(),
            r#type: self.r#type.clone(),
        })
    }
}

pub type ChatAnalyticsBucketDTO = ProtoAnalyticsBucket;
pub type ChatAnalyticsGatewayDTO = ChatAnalytics;

/// extension methods for the chat analytics reply
pub trait ChatAnalyticsGatewayDTOExt {
    fn to_model(&self) -> ChatAnalyticsModel;
}

impl ChatAnalyticsGatewayDTOExt for ChatAnalytics {
    fn to_model(&self) -> ChatAnalyticsModel {
        ChatAnalyticsModel {
            member_count: self.member_count,
            new_members: self.new_members,
            buckets: self
                .joined_buckets
                .iter()
                .map(|bucket| ChatAnalyticsPoint { day_start_ms: bucket.day_start_ms, count: bucket.count })
                .collect(),
            posts_count: self.posts_count,
            views_total: self.views_total,
            reactions_total: self.reactions_total,
            avg_views_per_post: self.avg_views_per_post,
            stats_available: self.stats_available,
        }
    }
}

pub type RoleWireDTO = ProtoRole;
pub type GetRolesGatewayDTO = GetRoles;
pub type RoleResultDTO = CreateRole;

pub type SectionWireDTO = ProtoSection;
pub type GetSectionsGatewayDTO = GetSections;
pub type SectionResultDTO = CreateSection;

pub type OverwriteWireDTO = ProtoOverwrite;
pub type GetOverwritesGatewayDTO = GetTopicOverwrites;

pub type EffectivePermissionsDTO = GetEffectivePermissions;

/// chat.get_forum_structure as the forum keeps it, its topics are stored topics
#[derive(Clone, Debug, Default)]
pub struct RiseStoredForumStructure {
    pub sections: Option<Vec<SectionWireDTO>>,
    pub topics: Option<Vec<RiseStoredChatTopic>>,
    pub general_topic: Option<RiseStoredChatTopic>,
    pub roles: Option<Vec<RoleWireDTO>>,
    pub hide_all_topics: Option<bool>,
}

impl<'de> Deserialize<'de> for RiseStoredForumStructure {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container: &RiseWireObject| {
            Ok(Self {
                sections: container.rise::<Vec<SectionWireDTO>>(key::SECTIONS)?,
                topics: container.rise::<Vec<RiseStoredChatTopic>>(key::TOPICS)?,
                general_topic: container.rise::<RiseStoredChatTopic>(key::GENERAL_TOPIC)?,
                roles: container.rise::<Vec<RoleWireDTO>>(key::ROLES)?,
                hide_all_topics: container.rise::<bool>(key::HIDE_ALL_TOPICS)?,
            })
        })
    }
}

impl Serialize for RiseStoredForumStructure {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        encode_if_present(&mut container, key::SECTIONS, &self.sections)?;
        encode_if_present(&mut container, key::TOPICS, &self.topics)?;
        encode_if_present(&mut container, &camel(key::GENERAL_TOPIC), &self.general_topic)?;
        encode_if_present(&mut container, key::ROLES, &self.roles)?;
        encode_if_present(&mut container, &camel(key::HIDE_ALL_TOPICS), &self.hide_all_topics)?;
        container.end()
    }
}

/// extension methods for the role wire type
pub trait RoleWireDTOExt {
    fn from_canon(card: &ChatRoleCard) -> Self;
    fn to_role(&self) -> ChatRole;
}

impl RoleWireDTOExt for ProtoRole {
    fn from_canon(card: &ChatRoleCard) -> Self {
        Self {
            chat_id: card.chat_id.clone(),
            color: card.color,
            created_at: card.created_at,
            hoist: card.hoist,
            is_default: card.is_default,
            mentionable: card.mentionable,
            name: card.name.clone(),
            permissions: card.permissions,
            position: card.position,
            role_id: card.role_id.clone(),
        }
    }

    fn to_role(&self) -> ChatRole {
        ChatRole {
            id: self.role_id.clone(),
            name: self.name.clone(),
            color: self.color,
            position: self.position,
            permissions: self.permissions,
            mentionable: self.mentionable,
            hoist: self.hoist,
            is_default: self.is_default,
        }
    }
}

/// extension methods for the section wire type
pub trait SectionWireDTOExt {
    fn to_section(&self) -> ChatSection;
}

impl SectionWireDTOExt for ProtoSection {
    fn to_section(&self) -> ChatSection {
        let (name, name_entities) =
            ChatFolder::dropping_orphan_object_marks(&self.name, CommentEntities::normalized(self.name_entities.as_deref()));
        ChatSection { id: self.section_id.clone(), name, position: self.position, name_entities }
    }
}

/// extension methods for the overwrite wire type
pub trait OverwriteWireDTOExt {
    fn to_overwrite(&self) -> ChatPermOverwrite;
}

impl OverwriteWireDTOExt for ProtoOverwrite {
    fn to_overwrite(&self) -> ChatPermOverwrite {
        ChatPermOverwrite {
            target_type: if self.target_type.is_empty() {
                OVERWRITE_TARGET_ROLE.to_owned()
            } else {
                self.target_type.clone()
            },
            target_id: self.target_id.clone(),
            allow: self.allow,
            deny: self.deny,
        }
    }
}

/// extension methods for the effective permissions reply
pub trait EffectivePermissionsDTOExt {
    /// the mask the way the permission helpers take it
    fn mask(&self) -> i64;
}

impl EffectivePermissionsDTOExt for GetEffectivePermissions {
    fn mask(&self) -> i64 {
        self.permissions
    }
}
