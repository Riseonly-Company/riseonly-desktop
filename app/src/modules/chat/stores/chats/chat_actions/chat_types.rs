use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use gpui::{Hsla, Rgba};
use rise_canon::rise_key::{chat as key, chat_action as action_key};
use rise_canon::rise_service_text::chat_service;
use rise_canon::{ChatAllowKey, ChatTopicState, RiseLimits, RiseWireKey, rise_decode};
use rise_chat::ChatActionType;
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use super::chat_types_hooks::chat_types_hooks;
use super::chat_wire_types::ChatMemberWireDTO;
use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageCustomEmojiEntity, MessageReaction,
};
use crate::modules::comment::stores::comment_actions::comment_types::CommentEntities;
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, AuthorInfoMore};
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCParticipantRole;

pub type ChatType = rise_chat::ChatType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatVoiceConnectGateDecision {
    Allowed,
    Denied,
    RefreshRequired,
}

pub enum ChatVoiceConnectGatePolicy {}

impl ChatVoiceConnectGatePolicy {
    pub fn decision(cached_permissions: Option<i64>) -> ChatVoiceConnectGateDecision {
        let Some(cached_permissions) = cached_permissions else {
            return ChatVoiceConnectGateDecision::RefreshRequired;
        };
        if ChatPerm::has(cached_permissions, ChatPerm::CONNECT_VOICE) {
            ChatVoiceConnectGateDecision::Allowed
        } else {
            ChatVoiceConnectGateDecision::Denied
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct User {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub alias_tags: Vec<String>,
    pub more: AuthorInfoMore,
    pub is_online: bool,
    pub last_seen: Option<i64>,
    pub is_deleted: bool,
    pub is_bot: bool,
    pub bot_owner_id: Option<String>,
    pub bot_is_active: bool,
    pub bot_users: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChatCreatorInfo {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub role: Option<String>,
    pub is_premium: bool,
    pub user_chat_id: Option<String>,
    pub logo: Option<String>,
    pub description: Option<String>,
    pub is_online: bool,
}

/// snapshot of a theme row taken when the owner or a side of a private chat pinned it,
/// shared with every member
/// whether this account draws it is a preference
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatWallpaperPlacement {
    pub id: String,
    pub image_url: String,
    pub preview_url: String,
    pub look: ChatWallpaperLook,
    pub set_by_user_id: String,
    pub updated_at_ms: i64,
}

impl ChatWallpaperPlacement {
    pub fn display_url(&self) -> &str {
        if self.preview_url.is_empty() { &self.image_url } else { &self.preview_url }
    }
}

/// one remembered upload, newest first shared by everyone who may change the wallpaper
/// its id is what wallpaper.id reads once pinned
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatWallpaperCatalogEntry {
    pub id: String,
    pub chat_id: String,
    pub topic_id: String,
    pub image_url: String,
    pub preview_url: String,
    pub look: ChatWallpaperLook,
    pub added_by_user_id: String,
    pub created_at_ms: i64,
}

impl ChatWallpaperCatalogEntry {
    pub fn display_url(&self) -> &str {
        if self.preview_url.is_empty() { &self.image_url } else { &self.preview_url }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatWallpaperCatalogState {
    pub entries: Vec<ChatWallpaperCatalogEntry>,
    pub is_loading: bool,
    pub is_loaded: bool,
    pub error: Option<String>,
    pub mutating_ids: HashSet<String>,
    pub is_uploading: bool,
}

impl ChatWallpaperCatalogState {
    pub fn key(chat_id: &str, topic_id: Option<&str>) -> String {
        let topic = topic_id.unwrap_or("");
        if topic.is_empty() { chat_id.to_owned() } else { format!("{chat_id}/{topic}") }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatInfo {
    pub id: String,
    pub title: String,
    pub r#type: ChatType,
    pub last_message: Option<GetMessageMessage>,
    pub unread_count: i64,
    pub unread_mentions_count: i64,
    pub unread_replies_count: i64,
    pub unread_reactions_count: i64,
    pub is_pinned: bool,
    pub pin_position: Option<i64>,
    pub is_muted: bool,
    pub updated_at: TimeInterval,
    pub allowed_reactions: Vec<String>,
    pub created_at: TimeInterval,
    pub description: Option<String>,
    pub logo_url: Option<String>,
    pub banner_url: Option<String>,
    pub wallpaper: Option<ChatWallpaperPlacement>,
    pub username: Option<String>,
    pub member_count: i64,
    pub is_verified: bool,
    pub is_public: bool,
    pub role: Option<String>,
    pub joined_at: Option<TimeInterval>,
    pub notifications_enabled: bool,
    pub mute_until: Option<TimeInterval>,
    pub custom_title: Option<String>,
    pub is_anonymous: bool,
    pub custom_color: Option<String>,
    pub last_activity: Option<TimeInterval>,
    pub can_invite_users: bool,
    pub can_pin_messages: bool,
    pub can_manage_chat: bool,
    pub can_accept_join_requests: bool,
    pub creator_info: Option<String>,
    pub participant: User,
    pub tag: Option<String>,
    pub forward_in_chat_enabled: bool,
    pub typing_datas: Vec<String>,

    pub topics_enabled: bool,
    pub auto_delete_messages_after: Option<i64>,
    pub mute_scopes: Option<String>,
    pub is_message_request: bool,
    pub message_request_state: Option<String>,
    pub join_approval_required: bool,
    pub invite_link: Option<String>,
    pub can_share_invite: bool,
    pub has_live_voice: bool,
}

impl Default for ChatInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            r#type: ChatType::Direct,
            last_message: None,
            unread_count: 0,
            unread_mentions_count: 0,
            unread_replies_count: 0,
            unread_reactions_count: 0,
            is_pinned: false,
            pin_position: None,
            is_muted: false,
            updated_at: 0.0,
            allowed_reactions: Vec::new(),
            created_at: 0.0,
            description: None,
            logo_url: None,
            banner_url: None,
            wallpaper: None,
            username: None,
            member_count: 0,
            is_verified: false,
            is_public: false,
            role: None,
            joined_at: None,
            notifications_enabled: true,
            mute_until: None,
            custom_title: None,
            is_anonymous: false,
            custom_color: None,
            last_activity: None,
            can_invite_users: true,
            can_pin_messages: true,
            can_manage_chat: false,
            can_accept_join_requests: false,
            creator_info: None,
            participant: User::default(),
            tag: None,
            forward_in_chat_enabled: true,
            typing_datas: Vec::new(),
            topics_enabled: false,
            auto_delete_messages_after: None,
            mute_scopes: None,
            is_message_request: false,
            message_request_state: None,
            join_approval_required: false,
            invite_link: None,
            can_share_invite: false,
            has_live_voice: false,
        }
    }
}

impl ChatInfo {
    /// new chat info with only the id, every other field takes its default
    pub fn new(id: String) -> Self {
        Self { id, ..Self::default() }
    }

    pub fn is_community_preview(&self) -> bool {
        if !self.is_public || !self.r#type.is_community() {
            return false;
        }
        let is_joined = chat_types_hooks()
            .map(|hooks| (hooks.is_joined_member_role)(self.role.as_deref()))
            .unwrap_or(false);
        !is_joined
    }

    pub fn is_removed_from_community(&self) -> bool {
        self.is_community_preview() && self.role.as_deref().map(str::to_uppercase).as_deref() == Some("BANNED")
    }

    pub fn notification_scopes(&self) -> ChatMuteScopes {
        ChatMuteScopes::parse(self.mute_scopes.as_deref()).unwrap_or(ChatMuteScopes::ALL_ENABLED)
    }

    pub fn is_bot_chat(&self) -> bool {
        self.r#type == ChatType::Direct && self.participant.is_bot
    }

    pub fn is_official_bot_chat(&self) -> bool {
        if !self.is_bot_chat() || !self.participant.bot_is_active || !self.participant.more.is_official {
            return false;
        }
        let normalized_tag =
            trimming_whitespaces_and_newlines(&self.participant.tag).trim_start_matches('@').to_lowercase();
        normalized_tag == "riseonly"
    }

    pub fn is_active_bot_chat(&self) -> bool {
        self.is_bot_chat() && self.participant.bot_is_active
    }

    /// the peer own profile name for a direct chat, or their tag if the name isnt loaded
    pub fn direct_peer_profile_name(&self) -> String {
        let name = trimming_whitespaces_and_newlines(&self.participant.name);
        if !name.is_empty() {
            return name.to_owned();
        }
        let tag = trimming_whitespaces_and_newlines(&self.participant.tag);
        if tag.is_empty() {
            if self.has_no_direct_peer() {
                return trimming_whitespaces_and_newlines(&self.title).to_owned();
            }
            return String::new();
        }
        if tag.starts_with('@') { tag.to_owned() } else { format!("@{tag}") }
    }

    /// a direct chat that names no person, no id no name no tag
    pub fn has_no_direct_peer(&self) -> bool {
        self.r#type == ChatType::Direct
            && trimming_whitespaces_and_newlines(&self.participant.id).is_empty()
            && trimming_whitespaces_and_newlines(&self.participant.name).is_empty()
            && trimming_whitespaces_and_newlines(&self.participant.tag).is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChatMuteScopes {
    pub messages: bool,
    pub mentions: bool,
    pub replies: bool,
    pub reactions: bool,
    pub events: bool,
}

impl Default for ChatMuteScopes {
    fn default() -> Self {
        Self::ALL_ENABLED
    }
}

impl ChatMuteScopes {
    pub const ALL_ENABLED: ChatMuteScopes = ChatMuteScopes { messages: true, mentions: true, replies: true, reactions: true, events: true };

    pub const MUTED: ChatMuteScopes = ChatMuteScopes { messages: false, mentions: false, replies: false, reactions: false, events: false };

    pub fn is_all_enabled(&self) -> bool {
        self.messages && self.mentions && self.replies && self.reactions && self.events
    }

    pub fn is_fully_muted(&self) -> bool {
        !(self.messages || self.mentions || self.replies || self.reactions || self.events)
    }

    pub fn encoded_json(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }

    pub fn parse(raw: Option<&str>) -> Option<ChatMuteScopes> {
        let raw = raw.filter(|raw| !raw.is_empty())?;
        serde_json::from_str(raw).ok()
    }
}

impl<'de> Deserialize<'de> for ChatMuteScopes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            let mentions = c.rise_required::<bool>(key::MENTIONS)?;
            Ok(Self {
                messages: c.rise_required::<bool>(key::MESSAGES)?,
                mentions,
                replies: c.rise::<bool>(key::REPLIES)?.unwrap_or(mentions),
                reactions: c.rise_required::<bool>(key::REACTIONS)?,
                events: c.rise_required::<bool>(key::EVENTS)?,
            })
        })
    }
}

impl Serialize for ChatMuteScopes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(Some(5))?;
        container.serialize_entry(key::MESSAGES, &self.messages)?;
        container.serialize_entry(key::MENTIONS, &self.mentions)?;
        container.serialize_entry(key::REPLIES, &self.replies)?;
        container.serialize_entry(key::REACTIONS, &self.reactions)?;
        container.serialize_entry(key::EVENTS, &self.events)?;
        container.end()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GetChatsResponse {
    pub limit: i64,
    pub relative_id: Option<String>,
    pub is_have_more: bool,
    pub chats: Vec<ChatInfo>,
}

impl Default for GetChatsResponse {
    fn default() -> Self {
        Self { limit: 50, relative_id: None, is_have_more: false, chats: Vec::new() }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatMember {
    pub user_id: String,
    pub name: String,
    pub tag: String,
    pub avatar_url: String,
    pub role: String,
    pub is_online: bool,
    pub last_seen: Option<i64>,
    pub is_premium: bool,
    pub is_official: bool,
    pub premium_badge_emoji: Option<String>,
    pub premium_name_color: Option<String>,
    pub is_verified: bool,
    pub joined_at: TimeInterval,
    pub custom_title: String,
    pub custom_color: String,
    pub is_anonymous: bool,
    pub can_manage_chat: bool,
    pub can_kick_members: bool,
    pub can_ban_members: bool,
    pub can_promote_members: bool,
    pub can_manage_reactions: bool,
    pub can_change_info: bool,
    pub can_delete_messages: bool,
    pub can_pin_messages: bool,
    pub can_invite_users: bool,
    pub can_manage_stories: bool,
    pub can_manage_video_chats: bool,

    pub role_ids: Vec<String>,
    pub display_color: i64,
}

impl ChatMember {
    pub fn id(&self) -> &str {
        &self.user_id
    }

    pub fn is_admin(&self) -> bool {
        self.role.to_uppercase() == "ADMIN"
    }

    pub fn is_owner(&self) -> bool {
        ChatPerm::is_owner_role(Some(&self.role))
    }

    pub fn name_color(&self) -> Option<Hsla> {
        rgb_color(self.display_color)
    }

    pub fn to_author_info(&self) -> AuthorInfo {
        AuthorInfo {
            id: Some(self.user_id.clone()),
            name: self.name.clone(),
            tag: self.tag.clone(),
            more: AuthorInfoMore {
                logo: self.avatar_url.clone(),
                is_premium: self.is_premium,
                is_official: self.is_official,
                premium_badge_emoji: self.premium_badge_emoji.clone(),
                name_color: self.premium_name_color.clone(),
                ..AuthorInfoMore::default()
            },
            ..AuthorInfo::default()
        }
    }

    pub fn to_user(&self) -> User {
        User {
            id: self.user_id.clone(),
            name: self.name.clone(),
            tag: self.tag.clone(),
            more: AuthorInfoMore {
                logo: self.avatar_url.clone(),
                is_premium: self.is_premium,
                is_official: self.is_official,
                premium_badge_emoji: self.premium_badge_emoji.clone(),
                name_color: self.premium_name_color.clone(),
                ..AuthorInfoMore::default()
            },
            is_online: self.is_online,
            last_seen: self.last_seen,
            ..User::default()
        }
    }
}

/// color from packed 0xRRGGBB, zero means no color
fn rgb_color(packed: i64) -> Option<Hsla> {
    if packed == 0 {
        return None;
    }
    Some(
        Rgba {
            r: ((packed >> 16) & 0xFF) as f32 / 255.0,
            g: ((packed >> 8) & 0xFF) as f32 / 255.0,
            b: (packed & 0xFF) as f32 / 255.0,
            a: 1.0,
        }
        .into(),
    )
}

pub trait ChatMemberWireDTOExt {
    fn to_chat_member(&self) -> ChatMember;
}

impl ChatMemberWireDTOExt for ChatMemberWireDTO {
    fn to_chat_member(&self) -> ChatMember {
        let u = self.user.as_ref();
        ChatMember {
            user_id: self.user_id.clone(),
            name: u.map(|u| u.name.clone()).unwrap_or_default(),
            tag: u.map(|u| u.tag.clone()).unwrap_or_default(),
            avatar_url: u.and_then(|u| u.avatar_url.clone()).unwrap_or_default(),
            role: self.role.clone(),
            is_online: u.map(|u| u.is_online).unwrap_or(false),
            last_seen: u.and_then(|u| u.last_seen).filter(|seen| *seen > 0),
            is_premium: u.map(|u| u.is_premium).unwrap_or(false),
            is_official: u.map(|u| u.is_official).unwrap_or(false),
            premium_badge_emoji: u.and_then(|u| u.premium_badge_emoji.clone()),
            premium_name_color: u.and_then(|u| u.name_color.clone()),
            is_verified: u.map(|u| u.is_verified).unwrap_or(false),
            joined_at: self.joined_at as f64 / 1000.0,
            custom_title: self.custom_title.clone().unwrap_or_default(),
            custom_color: self.custom_color.clone().unwrap_or_default(),
            is_anonymous: self.is_anonymous,
            can_manage_chat: self.can_manage_chat,
            can_kick_members: self.can_kick_members,
            can_ban_members: self.can_ban_members,
            can_promote_members: self.can_promote_members,
            can_manage_reactions: self.can_manage_reactions,
            can_change_info: self.can_change_info,
            can_delete_messages: self.can_delete_messages,
            can_pin_messages: self.can_pin_messages,
            can_invite_users: self.can_invite_users,
            can_manage_stories: self.can_manage_stories,
            can_manage_video_chats: self.can_manage_video_chats,
            role_ids: self.role_ids.clone(),
            display_color: self.display_color,
        }
    }
}

impl ChatInfo {
    pub fn creator_info_decoded(&self) -> Option<ChatCreatorInfo> {
        let raw = trimming_whitespaces_and_newlines(self.creator_info.as_deref()?);
        if raw.is_empty() {
            return None;
        }
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(raw) else {
            return None;
        };
        let id = obj.get(key::ID).and_then(Value::as_str).filter(|id| !id.is_empty())?.to_owned();
        let empty = Map::new();
        let more = obj.get(key::MORE).and_then(Value::as_object).unwrap_or(&empty);
        let text = |object: &Map<String, Value>, name: &str| object.get(name).and_then(Value::as_str).map(str::to_owned);
        let flag = |object: &Map<String, Value>, name: &str| object.get(name).and_then(Value::as_bool).unwrap_or(false);
        Some(ChatCreatorInfo {
            id,
            name: text(&obj, key::NAME).unwrap_or_default(),
            tag: text(&obj, key::TAG).unwrap_or_default(),
            role: text(&obj, key::ROLE),
            is_premium: flag(&obj, key::IS_PREMIUM),
            user_chat_id: text(&obj, key::USER_CHAT_ID),
            logo: text(more, key::LOGO),
            description: text(more, key::DESCRIPTION),
            is_online: flag(more, key::IS_ONLINE),
        })
    }
}

pub type ChatChannelKind = rise_chat::ChannelKind;

pub trait ChatChannelKindCodable: Sized {
    fn from_wire_value(value: &Value) -> Self;
    fn encoded_wire_value(&self) -> &'static str;
}

impl ChatChannelKindCodable for ChatChannelKind {
    fn from_wire_value(value: &Value) -> Self {
        match value {
            Value::String(raw_value) => {
                ChatChannelKind::from_wire(&trimming_whitespaces_and_newlines(raw_value).to_lowercase())
                    .unwrap_or(ChatChannelKind::Text)
            }
            Value::Number(number) => match number.as_i64() {
                Some(proto_number) => ChatChannelKind::ALL
                    .into_iter()
                    .find(|kind| kind.proto_number() as i64 == proto_number)
                    .unwrap_or(ChatChannelKind::Text),
                None => ChatChannelKind::Text,
            },
            _ => ChatChannelKind::Text,
        }
    }

    fn encoded_wire_value(&self) -> &'static str {
        self.wire()
    }
}

pub fn deserialize_chat_channel_kind<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ChatChannelKind, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(ChatChannelKind::from_wire_value(&value))
}

pub fn serialize_chat_channel_kind<S: Serializer>(kind: &ChatChannelKind, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(kind.encoded_wire_value())
}

pub enum ChatChannelManagementPolicy {}

impl ChatChannelManagementPolicy {
    pub fn required_permission(kind: ChatChannelKind) -> i64 {
        ChatPerm::mask(kind.required_permission()).unwrap_or(ChatPerm::ADMINISTRATOR)
    }

    pub fn can_manage(kind: ChatChannelKind, permissions: i64) -> bool {
        ChatPerm::has(permissions, Self::required_permission(kind))
    }

    /// kind is picked once at creation, an existing channel has either an rtc room or a message
    /// feed so it can only stay what it is
    /// source none means creation and picks from everything the permissions allow
    pub fn can_mutate(source: Option<ChatChannelKind>, destination: ChatChannelKind, permissions: i64) -> bool {
        if let Some(source) = source {
            return source == destination && Self::can_manage(source, permissions);
        }
        Self::can_manage(destination, permissions)
    }

    pub fn allowed_destination_kinds(source: Option<ChatChannelKind>, permissions: i64) -> Vec<ChatChannelKind> {
        ChatChannelKind::ALL
            .into_iter()
            .filter(|kind| Self::can_mutate(source, *kind, permissions))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VoiceChannelConfigValidationError {
    #[error("unsupported channel kind")]
    UnsupportedChannelKind,
    #[error("invalid user limit {0}")]
    InvalidUserLimit(i64),
    #[error("invalid speaker limit {0}")]
    InvalidSpeakerLimit(i64),
    #[error("speaker limit exceeds user limit")]
    SpeakerLimitExceedsUserLimit,
    #[error("invalid region")]
    InvalidRegion,
}

static VOICE_MINIMUM_USER_LIMIT: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("voice_user_limit_min"));
static VOICE_MAXIMUM_USER_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("voice_channel_config_maximum_user_limit"));
static VOICE_MAXIMUM_SPEAKER_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("voice_speaker_limit_max"));
static VOICE_MINIMUM_AUDIO_BITRATE_KBPS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("voice_audio_bitrate_kbps_min"));
static VOICE_MAXIMUM_AUDIO_BITRATE_KBPS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("voice_channel_config_maximum_audio_bitrate_kbps"));
static VOICE_FREE_AUDIO_BITRATE_CEILING_KBPS: LazyLock<i64> = LazyLock::new(|| {
    RiseLimits::service_value("voice_audio_bitrate_kbps_tier_ceiling").min(*VOICE_MAXIMUM_AUDIO_BITRATE_KBPS)
});

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VoiceChannelConfig {
    pub user_limit: i64,
    pub speaker_limit: i64,
    pub audio_bitrate_kbps: i64,
    pub video_enabled: bool,
    pub screen_share_enabled: bool,
    pub default_muted: bool,
    pub region: String,
}

impl VoiceChannelConfig {
    pub fn minimum_user_limit() -> i64 {
        *VOICE_MINIMUM_USER_LIMIT
    }

    pub fn maximum_user_limit() -> i64 {
        *VOICE_MAXIMUM_USER_LIMIT
    }

    pub fn maximum_speaker_limit() -> i64 {
        *VOICE_MAXIMUM_SPEAKER_LIMIT
    }

    pub fn minimum_audio_bitrate_kbps() -> i64 {
        *VOICE_MINIMUM_AUDIO_BITRATE_KBPS
    }

    pub fn maximum_audio_bitrate_kbps() -> i64 {
        *VOICE_MAXIMUM_AUDIO_BITRATE_KBPS
    }

    pub fn free_audio_bitrate_ceiling_kbps() -> i64 {
        *VOICE_FREE_AUDIO_BITRATE_CEILING_KBPS
    }

    /// the server ties the bitrate ceiling to the tier of whoever saves the setting
    pub fn audio_bitrate_ceiling_kbps(is_premium: bool) -> i64 {
        if is_premium { Self::maximum_audio_bitrate_kbps() } else { Self::free_audio_bitrate_ceiling_kbps() }
    }

    pub fn capped_for_author(&self, is_premium: bool) -> VoiceChannelConfig {
        let mut capped = self.clone();
        capped.audio_bitrate_kbps = self.audio_bitrate_kbps.min(Self::audio_bitrate_ceiling_kbps(is_premium));
        capped
    }

    pub fn defaults(kind: ChatChannelKind) -> Option<VoiceChannelConfig> {
        match kind {
            ChatChannelKind::Text => None,
            ChatChannelKind::Voice => Some(VoiceChannelConfig {
                user_limit: 0,
                speaker_limit: 0,
                audio_bitrate_kbps: 32,
                video_enabled: true,
                screen_share_enabled: true,
                default_muted: false,
                region: String::new(),
            }),
            ChatChannelKind::Stage => Some(VoiceChannelConfig {
                user_limit: 0,
                speaker_limit: 25,
                audio_bitrate_kbps: 32,
                video_enabled: true,
                screen_share_enabled: true,
                default_muted: true,
                region: String::new(),
            }),
        }
    }

    pub fn normalized(&self, kind: ChatChannelKind) -> Result<VoiceChannelConfig, VoiceChannelConfigValidationError> {
        if !kind.is_rtc() {
            return Err(VoiceChannelConfigValidationError::UnsupportedChannelKind);
        }
        if !(self.user_limit == 0
            || (Self::minimum_user_limit()..=Self::maximum_user_limit()).contains(&self.user_limit))
        {
            return Err(VoiceChannelConfigValidationError::InvalidUserLimit(self.user_limit));
        }
        if !(0..=Self::maximum_speaker_limit()).contains(&self.speaker_limit) {
            return Err(VoiceChannelConfigValidationError::InvalidSpeakerLimit(self.speaker_limit));
        }
        if !(self.user_limit == 0 || self.speaker_limit == 0 || self.speaker_limit <= self.user_limit) {
            return Err(VoiceChannelConfigValidationError::SpeakerLimitExceedsUserLimit);
        }

        let normalized_region = trimming_whitespaces_and_newlines(&self.region).to_lowercase();
        if normalized_region.len() > 32 || !normalized_region.bytes().all(Self::is_allowed_region_byte) {
            return Err(VoiceChannelConfigValidationError::InvalidRegion);
        }

        let mut normalized = self.clone();
        normalized.audio_bitrate_kbps = self
            .audio_bitrate_kbps
            .max(Self::minimum_audio_bitrate_kbps())
            .min(Self::maximum_audio_bitrate_kbps());
        normalized.region = normalized_region;
        Ok(normalized)
    }

    fn is_allowed_region_byte(byte: u8) -> bool {
        matches!(byte, 48..=57 | 65..=90 | 97..=122 | 45 | 95)
    }
}

impl<'de> Deserialize<'de> for VoiceChannelConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                user_limit: c.rise_required::<i64>(key::USER_LIMIT)?,
                speaker_limit: c.rise_required::<i64>(key::SPEAKER_LIMIT)?,
                audio_bitrate_kbps: c.rise_required::<i64>(key::AUDIO_BITRATE_KBPS)?,
                video_enabled: c.rise_required::<bool>(key::VIDEO_ENABLED)?,
                screen_share_enabled: c.rise_required::<bool>(key::SCREEN_SHARE_ENABLED)?,
                default_muted: c.rise_required::<bool>(key::DEFAULT_MUTED)?,
                region: c.rise_required::<String>(key::REGION)?,
            })
        })
    }
}

impl Serialize for VoiceChannelConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(Some(7))?;
        container.serialize_entry(&RiseWireKey::camel(key::USER_LIMIT), &self.user_limit)?;
        container.serialize_entry(&RiseWireKey::camel(key::SPEAKER_LIMIT), &self.speaker_limit)?;
        container.serialize_entry(&RiseWireKey::camel(key::AUDIO_BITRATE_KBPS), &self.audio_bitrate_kbps)?;
        container.serialize_entry(&RiseWireKey::camel(key::VIDEO_ENABLED), &self.video_enabled)?;
        container.serialize_entry(&RiseWireKey::camel(key::SCREEN_SHARE_ENABLED), &self.screen_share_enabled)?;
        container.serialize_entry(&RiseWireKey::camel(key::DEFAULT_MUTED), &self.default_muted)?;
        container.serialize_entry(key::REGION, &self.region)?;
        container.end()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatTopic {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub position: i64,
    pub is_general: bool,
    pub is_closed: bool,
    pub is_closed_known: bool,
    pub section_id: String,
    pub is_private: bool,
    pub notification_scopes: ChatMuteScopes,
    pub notification_inherited: bool,
    pub last_message: Option<GetMessageMessage>,
    pub unread_count: i64,
    pub unread_mentions_count: i64,
    pub unread_replies_count: i64,
    pub unread_reactions_count: i64,
    pub is_muted: bool,
    pub updated_at: TimeInterval,
    pub channel_kind: ChatChannelKind,
    pub voice_config: Option<VoiceChannelConfig>,
    pub voice_state: Option<VoiceChannelState>,
    pub wallpaper: Option<ChatWallpaperPlacement>,
}

impl ChatTopic {
    pub const DEFAULT_ICON: &'static str = "\u{1F4AC}";

    pub const GENERAL_WALLPAPER_SCOPE_ID: &'static str = "00000000-0000-0000-0000-000000000001";

    /// the fields the constructor requires, every other field takes its default
    pub fn new(id: String, title: String, icon: String, position: i64, is_general: bool, is_closed: bool) -> Self {
        Self { id, title, icon, position, is_general, is_closed, ..Self::default() }
    }
}

impl Default for ChatTopic {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            icon: String::new(),
            position: 0,
            is_general: false,
            is_closed: false,
            is_closed_known: true,
            section_id: String::new(),
            is_private: false,
            notification_scopes: ChatMuteScopes::ALL_ENABLED,
            notification_inherited: true,
            last_message: None,
            unread_count: 0,
            unread_mentions_count: 0,
            unread_replies_count: 0,
            unread_reactions_count: 0,
            is_muted: false,
            updated_at: 0.0,
            channel_kind: ChatChannelKind::Text,
            voice_config: None,
            voice_state: None,
            wallpaper: None,
        }
    }
}

impl ChatTopic {
    /// unicode emoji stay catalog normalized, ce:id custom emoji refs pass as is
    /// the server accepts both
    pub fn selectable_icon(raw: Option<&str>) -> Option<String> {
        let raw = trimming_whitespaces_and_newlines(raw?);
        if raw.is_empty() {
            return None;
        }
        if let Some(id) = raw.strip_prefix(MessageReaction::CUSTOM_EMOJI_PREFIX) {
            return if id.is_empty() { None } else { Some(raw.to_owned()) };
        }
        EmojiPickerCatalog::normalized_selectable_emoji(Some(raw))
    }

    pub fn wallpaper_scope_id(&self) -> &str {
        if self.is_general { Self::GENERAL_WALLPAPER_SCOPE_ID } else { &self.id }
    }

    pub fn is_rtc_channel(&self) -> bool {
        self.channel_kind.is_rtc()
    }

    pub fn posting_state(&self) -> ChatTopicState {
        if !self.is_closed_known {
            return ChatTopicState::Unknown;
        }
        if self.is_closed { ChatTopicState::Closed } else { ChatTopicState::Opened }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VoiceChannelParticipant {
    pub user_id: String,
    pub name: String,
    pub tag: String,
    pub avatar_url: String,
    pub is_premium: bool,
    pub is_official: bool,
    pub role_name: String,
    pub role_color: i64,
    pub microphone_enabled: bool,
    pub camera_enabled: bool,
    pub screen_share_enabled: bool,
    pub deafened: bool,
    pub server_muted: bool,
    pub server_deafened: bool,
    pub connected: bool,
    pub joined_at_ms: i64,
    pub order_sequence: i64,
    pub role: RiseRTCParticipantRole,
    pub hand_raised: bool,
    pub hand_expires_at_ms: Option<i64>,
}

impl VoiceChannelParticipant {
    pub fn id(&self) -> &str {
        &self.user_id
    }

    /// live means screen sharing, a camera alone is just video
    pub fn is_live(&self) -> bool {
        self.screen_share_enabled
    }

    pub fn is_on_stage(&self) -> bool {
        self.role == RiseRTCParticipantRole::Speaker || self.role == RiseRTCParticipantRole::Moderator
    }

    /// same deadline everywhere so the badge leaves the topic list the instant it leaves the
    /// room screen
    pub fn is_hand_raised(&self, now_ms: i64) -> bool {
        if !self.hand_raised {
            return false;
        }
        let Some(hand_expires_at_ms) = self.hand_expires_at_ms else {
            return true;
        };
        hand_expires_at_ms > now_ms
    }

    /// a moderator deafen counts as a mute
    /// the sfu revokes publish with the subscription so a row with stale microphone_enabled must
    /// not draw a mic
    pub fn is_microphone_muted(&self) -> bool {
        !self.microphone_enabled || self.server_muted || self.server_deafened
    }

    pub fn is_deafened(&self) -> bool {
        self.deafened || self.server_deafened
    }

    pub fn role_badge_color_hex(&self) -> String {
        if self.role_color == 0 {
            return String::new();
        }
        format!("#{:06X}", self.role_color & 0xFF_FFFF)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VoiceChannelState {
    pub room_id: String,
    pub participant_count: i64,
    pub user_limit: i64,
    pub participants: Vec<VoiceChannelParticipant>,
    pub has_more: bool,
    pub generated_at_ms: i64,
    pub session_started_at_ms: i64,
    pub visible_limit: i64,
}

impl VoiceChannelState {
    pub const COLLAPSED_ROWS: i64 = 5;
    pub const EXPAND_STEP: i64 = 10;

    pub fn preview_cap() -> i64 {
        RiseLimits::service_value("voice_preview_participants")
    }

    pub fn is_empty(&self) -> bool {
        self.participant_count <= 0
    }

    pub fn is_expanded(&self) -> bool {
        self.visible_limit > Self::COLLAPSED_ROWS
    }

    /// rows the list owes right now, everyone up to the visible limit
    fn owed_rows(&self) -> i64 {
        self.participant_count.max(0).min(self.visible_limit)
    }

    pub fn missing_preview_count(&self) -> i64 {
        let held = i64::try_from(self.participants.len()).unwrap_or(i64::MAX);
        (self.owed_rows() - held.min(self.visible_limit)).max(0)
    }

    /// only members beyond the visible limit, a roster gap shows as placeholders and isnt counted
    pub fn hidden_participant_count(&self) -> i64 {
        (self.participant_count - self.owed_rows()).max(0)
    }

    pub fn display_participants(&self, is_stage: bool, now_ms: i64) -> Vec<VoiceChannelParticipant> {
        let limit = usize::try_from(self.visible_limit.max(0)).unwrap_or(usize::MAX);
        let mut ordered = self.ordered_participants(is_stage, now_ms);
        ordered.truncate(limit);
        ordered
    }

    fn ordered_participants(&self, is_stage: bool, now_ms: i64) -> Vec<VoiceChannelParticipant> {
        if !is_stage {
            return self.participants.clone();
        }
        let rank = |participant: &VoiceChannelParticipant| -> u8 {
            if participant.is_on_stage() {
                return 0;
            }
            if participant.is_hand_raised(now_ms) { 1 } else { 2 }
        };
        let mut ordered = self.participants.clone();
        ordered.sort_by_key(|participant| rank(participant));
        ordered
    }

    pub fn session_started_at(&self) -> Option<Date> {
        if self.participant_count <= 0 || self.session_started_at_ms <= 0 {
            return None;
        }
        Some(Date::from_ms(self.session_started_at_ms))
    }

    /// earliest joined_at is honest only while that member is still there, else the timer counts backwards
    /// so a known start is kept while anyone is inside
    /// only an earlier one replaces it
    pub fn resolve_session_start(
        previous: Option<&VoiceChannelState>,
        room_id: &str,
        participants: &[VoiceChannelParticipant],
        participant_count: i64,
    ) -> i64 {
        if participant_count <= 0 {
            return 0;
        }
        let earliest = participants
            .iter()
            .map(|participant| participant.joined_at_ms)
            .filter(|joined| *joined > 0)
            .min()
            .unwrap_or(0);
        let Some(previous) = previous.filter(|previous| {
            previous.participant_count > 0
                && previous.session_started_at_ms > 0
                && (previous.room_id.is_empty() || room_id.is_empty() || previous.room_id == room_id)
        }) else {
            return earliest;
        };
        if earliest <= 0 {
            return previous.session_started_at_ms;
        }
        previous.session_started_at_ms.min(earliest)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InviteLink {
    pub id: String,
    pub link: String,
    pub name: Option<String>,
    pub created_at: TimeInterval,
    pub expires_at: Option<TimeInterval>,
    pub usage_limit: Option<i64>,
    pub used_count: i64,
    pub is_revoked: bool,
    pub creates_join_request: bool,
    pub is_primary: bool,
}

impl InviteLink {
    pub fn is_exhausted(&self) -> bool {
        if let Some(limit) = self.usage_limit
            && limit > 0
            && self.used_count >= limit
        {
            return true;
        }
        if let Some(exp) = self.expires_at
            && exp > 0.0
            && exp < RiseClock::now().time_interval_since_1970()
        {
            return true;
        }
        false
    }

    pub fn is_active(&self) -> bool {
        !self.is_revoked && !self.is_exhausted()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct JoinRequest {
    pub id: String,
    pub chat_id: String,
    pub user_id: String,
    pub requested_at: TimeInterval,
    pub status: String,
    pub message: Option<String>,
    pub user_name: String,
    pub user_tag: String,
    pub user_logo: String,
    pub is_premium: bool,
    pub is_official: bool,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
}

impl JoinRequest {
    pub fn is_pending(&self) -> bool {
        self.status.is_empty() || self.status.to_uppercase() == "PENDING"
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatJoinPreview {
    pub chat_id: String,
    pub title: String,
    pub logo_url: Option<String>,
    pub description: Option<String>,
    pub member_count: i64,
    pub r#type: ChatType,
    pub is_verified: bool,
    pub is_member: bool,
    pub can_join: bool,
    pub requires_approval: bool,
    pub join_status: String,
    pub voice: Option<ChatVoiceJoinPreview>,
}

impl ChatJoinPreview {
    pub fn is_channel(&self) -> bool {
        self.r#type == ChatType::Channel
    }
}

/// the voice chat or text topic behind an invite link, as far as the viewer may see it
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatVoiceJoinPreview {
    pub target: RiseRTCRoomTarget,
    pub is_text: bool,
    pub topic_title: Option<String>,
    pub live: bool,
    pub participant_count: i64,
    pub can_join_call: bool,
    pub details_hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatVoiceInviteError {
    Refused(String),
}

impl std::fmt::Display for ChatVoiceInviteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(raw) if raw.is_empty() => f.write_str("Voice invite link refused"),
            Self::Refused(raw) => f.write_str(raw),
        }
    }
}

impl std::error::Error for ChatVoiceInviteError {}

impl ChatVoiceInviteError {
    pub fn error_description(&self) -> Option<String> {
        Some(self.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChatInviteActionError {
    #[error("Invalid chat invite request")]
    InvalidRequest,
    #[error("Invalid chat invite response")]
    InvalidResponse,
}

impl ChatInviteActionError {
    pub fn error_description(&self) -> Option<String> {
        Some(self.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatPublicJoinOutcome {
    Joined,
    Banned,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum JoinLinkSheetPhase {
    Loading,
    Preview(ChatJoinPreview),
    Empty,
    Failed(String, bool),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JoinLinkSheetState {
    pub token: String,
    pub preview_request_id: Uuid,
    pub phase: JoinLinkSheetPhase,
    pub submission_id: Option<Uuid>,
    pub submission_error: Option<String>,
}

impl JoinLinkSheetState {
    pub fn new(token: String) -> Self {
        Self {
            token,
            preview_request_id: Uuid::new_v4(),
            phase: JoinLinkSheetPhase::Loading,
            submission_id: None,
            submission_error: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.token
    }

    pub fn is_loading(&self) -> bool {
        self.phase == JoinLinkSheetPhase::Loading
    }

    pub fn is_submitting(&self) -> bool {
        self.submission_id.is_some()
    }

    pub fn preview(&self) -> Option<&ChatJoinPreview> {
        match &self.phase {
            JoinLinkSheetPhase::Preview(preview) => Some(preview),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockedMember {
    pub user_id: String,
    pub name: String,
    pub tag: String,
    pub avatar_url: String,
    pub is_premium: bool,
    pub is_official: bool,
    pub banned_by_name: String,
    pub banned_at: TimeInterval,
    pub reason: Option<String>,
    pub ban_type: String,
}

impl BlockedMember {
    pub fn id(&self) -> &str {
        &self.user_id
    }

    fn author_more(&self) -> AuthorInfoMore {
        AuthorInfoMore {
            logo: self.avatar_url.clone(),
            is_premium: self.is_premium,
            is_official: self.is_official,
            ..AuthorInfoMore::default()
        }
    }

    pub fn to_author_info(&self) -> AuthorInfo {
        AuthorInfo {
            id: Some(self.user_id.clone()),
            name: self.name.clone(),
            tag: self.tag.clone(),
            more: self.author_more(),
            ..AuthorInfo::default()
        }
    }

    pub fn to_user(&self) -> User {
        User {
            id: self.user_id.clone(),
            name: self.name.clone(),
            tag: self.tag.clone(),
            more: self.author_more(),
            ..User::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatAction {
    pub id: String,
    pub action_type: String,
    pub actor_name: String,
    pub target_name: String,
    pub created_at: TimeInterval,
    pub details: Option<String>,
}

static CHAT_ACTION_TARGET_ACTIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        ChatActionType::MemberAdded,
        ChatActionType::MemberRemoved,
        ChatActionType::MemberBanned,
        ChatActionType::MemberUnbanned,
        ChatActionType::MemberKicked,
        ChatActionType::MemberPromoted,
        ChatActionType::MemberDemoted,
        ChatActionType::MemberMuted,
        ChatActionType::MemberUnmuted,
        ChatActionType::MemberRightsChanged,
        ChatActionType::OwnershipTransferred,
        ChatActionType::MessageDeleted,
    ]
    .into_iter()
    .map(ChatActionType::wire)
    .collect()
});

static CHAT_ACTION_MEMBER_PERM_LABEL_KEYS: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        (ChatPermKey::TEXT, "chat_perm_text"),
        (ChatPermKey::POLLS, "chat_perm_polls"),
        (ChatPermKey::WEB_PREVIEWS, "chat_perm_link_previews"),
        (ChatPermKey::PHOTO, "chat_perm_media_photo"),
        (ChatPermKey::VIDEO, "chat_perm_media_video"),
        (ChatPermKey::STICKERS, "chat_perm_media_stickers"),
        (ChatPermKey::GIF, "chat_perm_media_gif"),
        (ChatPermKey::MUSIC, "chat_perm_media_music"),
        (ChatPermKey::FILES, "chat_perm_media_files"),
        (ChatPermKey::VOICE, "chat_perm_media_voice"),
        (ChatPermKey::VIDEO_MESSAGES, "chat_perm_media_video_messages"),
        (chat_service::SETTINGS_PERM_ADD_MEMBERS, "chat_perm_add_members"),
        (chat_service::SETTINGS_PERM_CHANGE_INFO, "chat_perm_change_info"),
    ])
});

static CHAT_ACTION_RULE_LABEL_KEYS: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        (key::CAN_CHANGE_INFO, "chat_admin_perm_change_info"),
        (key::CAN_MANAGE_CHAT, "perm_manage_group"),
        (key::CAN_MANAGE_REACTIONS, "perm_manage_reactions"),
        (key::CAN_DELETE_MESSAGES, "chat_admin_perm_delete_messages"),
        (key::CAN_BAN_MEMBERS, "chat_admin_perm_ban_users"),
        (key::CAN_RESTRICT_MEMBERS, "chat_admin_perm_ban_users"),
        (key::CAN_KICK_MEMBERS, "chat_admin_perm_ban_users"),
        (key::CAN_INVITE_USERS, "chat_admin_perm_add_members"),
        (key::CAN_PIN_MESSAGES, "chat_admin_perm_pin"),
        (key::CAN_PROMOTE_MEMBERS, "chat_admin_perm_promote"),
        (key::CAN_MANAGE_STORIES, "chat_admin_perm_stories"),
        (key::CAN_MANAGE_VIDEO_CHATS, "chat_admin_perm_video_chats"),
        (key::CAN_SEND_MEDIA, "chat_perm_media"),
        (key::CAN_SEND_STICKERS, "chat_perm_media_stickers"),
        (key::CAN_SEND_POLLS, "chat_perm_polls"),
        (key::CAN_ADD_WEB_PAGE_PREVIEWS, "chat_perm_link_previews"),
        (key::CAN_SEND_AUDIO, "chat_perm_media_music"),
        (key::CAN_SEND_VOICE, "chat_perm_media_voice"),
        (key::CAN_SEND_DOCUMENTS, "chat_perm_media_files"),
        (key::CAN_REACT, "perm_add_reactions"),
    ])
});

/// reads an int from json, a number or a bool and a fraction truncates
fn ns_number_int(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64().or_else(|| number.as_f64().map(|value| value as i64)),
        Value::Bool(flag) => Some(i64::from(*flag)),
        _ => None,
    }
}

/// array where every element is a string
fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    value?.as_array()?.iter().map(|item| item.as_str().map(str::to_owned)).collect()
}

fn non_empty_str<'a>(object: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    object.get(name).and_then(Value::as_str).filter(|text| !text.is_empty())
}

impl ChatAction {
    pub fn target_actions() -> &'static HashSet<&'static str> {
        &CHAT_ACTION_TARGET_ACTIONS
    }

    pub fn localization_key(r#type: &str) -> String {
        format!("chat_audit_{}", r#type.to_lowercase())
    }

    pub fn description(&self) -> String {
        let key = ChatAction::localization_key(&self.action_type);
        let resolved = tr(&key);
        let verb = if resolved == key || self.action_type.is_empty() { tr("chat_audit_unknown") } else { resolved };
        let actor = if self.actor_name.is_empty() { tr("chat_deleted_account") } else { self.actor_name.clone() };
        let upper_type = self.action_type.to_uppercase();
        if !self.target_name.is_empty() && ChatAction::target_actions().contains(upper_type.as_str()) {
            return format!("{actor} {verb} {}", self.target_name);
        }
        if upper_type == ChatActionType::MessageDeleted.wire() {
            return format!("{actor} {verb} {}", tr("chat_deleted_account"));
        }
        if let Some(detail_object) = self.detail_object() {
            for name in [action_key::ROLE, action_key::TOPIC, action_key::LINK_NAME] {
                if let Some(value) = non_empty_str(&detail_object, name) {
                    return format!("{actor} {verb} {value}");
                }
            }
        }
        format!("{actor} {verb}")
    }

    fn detail_object(&self) -> Option<Map<String, Value>> {
        let details = self.details.as_deref().filter(|details| !details.is_empty())?;
        match serde_json::from_str::<Value>(details) {
            Ok(Value::Object(object)) => Some(object),
            _ => None,
        }
    }

    pub fn detail_text(&self) -> Option<String> {
        let object = self.detail_object()?;
        let mut lines: Vec<String> = Vec::new();
        if let Some(previous) = non_empty_str(&object, action_key::PREVIOUS_ROLE)
            && let Some(role) = object.get(action_key::ROLE).and_then(Value::as_str)
        {
            lines.push(format!("{previous} \u{2192} {role}"));
        }
        if let Some(previous) = non_empty_str(&object, action_key::PREVIOUS_TOPIC)
            && let Some(topic) = object.get(action_key::TOPIC).and_then(Value::as_str)
            && previous != topic
        {
            lines.push(format!("{previous} \u{2192} {topic}"));
        }
        if let Some(roles) = string_array(object.get(action_key::ROLES))
            && !roles.is_empty()
        {
            lines.push(roles.join(", "));
        }
        lines.extend(Self::member_perm_lines(object.get(action_key::PERMS_ON), "+"));
        lines.extend(Self::member_perm_lines(object.get(action_key::PERMS_OFF), "\u{2212}"));
        if let Some(after) = ns_number_int(object.get(action_key::SLOW_MODE))
            && let Some(before) = ns_number_int(object.get(action_key::PREVIOUS_SLOW_MODE))
        {
            lines.push(format!("{} \u{2192} {}", Self::slow_mode_label(before), Self::slow_mode_label(after)));
        }
        if let Some(role) = non_empty_str(&object, action_key::ROLE_ADDED) {
            lines.push(format!("{}: + {role}", tr("roles_title")));
        }
        if let Some(role) = non_empty_str(&object, action_key::ROLE_REMOVED) {
            lines.push(format!("{}: \u{2212} {role}", tr("roles_title")));
        }
        lines.extend(Self::rule_lines(object.get(action_key::RULES_ON), "+"));
        lines.extend(Self::rule_lines(object.get(action_key::RULES_OFF), "\u{2212}"));
        if let Some(allow) = ns_number_int(object.get(key::ALLOW))
            && let Some(deny) = ns_number_int(object.get(key::DENY))
        {
            let previous_allow = ns_number_int(object.get(action_key::PREVIOUS_ALLOW)).unwrap_or(0);
            let previous_deny = ns_number_int(object.get(action_key::PREVIOUS_DENY)).unwrap_or(0);
            let state = |allow_mask: i64, deny_mask: i64, bit: i64| -> u8 {
                if allow_mask & bit != 0 {
                    1
                } else if deny_mask & bit != 0 {
                    2
                } else {
                    0
                }
            };
            for bit in ChatPerm::editor_rows() {
                let after = state(allow, deny, bit.mask);
                if after == state(previous_allow, previous_deny, bit.mask) {
                    continue;
                }
                match after {
                    1 => lines.push(format!("+ {}", tr(bit.i18n_key))),
                    2 => lines.push(format!("\u{2212} {}", tr(bit.i18n_key))),
                    _ => lines.push(format!("{}: {}", tr(bit.i18n_key), tr("member_perm_inherit"))),
                }
            }
        }
        if lines.is_empty() { None } else { Some(lines.join("\n")) }
    }

    fn rule_lines(value: Option<&Value>, sign: &str) -> Vec<String> {
        let mut seen: HashSet<&'static str> = HashSet::new();
        string_array(value)
            .unwrap_or_default()
            .iter()
            .filter_map(|rule| {
                let key = *CHAT_ACTION_RULE_LABEL_KEYS.get(rule.as_str())?;
                if !seen.insert(key) {
                    return None;
                }
                Some(format!("{sign} {}", tr(key)))
            })
            .collect()
    }

    fn member_perm_lines(value: Option<&Value>, sign: &str) -> Vec<String> {
        string_array(value)
            .unwrap_or_default()
            .iter()
            .filter_map(|key| CHAT_ACTION_MEMBER_PERM_LABEL_KEYS.get(key.as_str()).map(|label| format!("{sign} {}", tr(label))))
            .collect()
    }

    fn slow_mode_label(seconds: i64) -> String {
        if seconds <= 0 {
            return tr("chat_perm_slowmode_off");
        }
        if seconds < 60 {
            return tr_args("slowmode_seconds_abbr", &[FormatArg::Int(seconds)]);
        }
        if seconds < 3600 {
            return tr_args("slowmode_minutes_abbr", &[FormatArg::Int(seconds / 60)]);
        }
        tr_args("slowmode_hours_abbr", &[FormatArg::Int(seconds / 3600)])
    }

    pub fn icon_system_name(&self) -> &'static str {
        let upper_type = self.action_type.to_uppercase();
        let Some(action_type) = ChatActionType::ALL.into_iter().find(|action_type| action_type.wire() == upper_type)
        else {
            return "clock.arrow.circlepath";
        };
        match action_type {
            ChatActionType::MemberAdded | ChatActionType::MemberUnbanned => "person.badge.plus",
            ChatActionType::MemberRemoved | ChatActionType::MemberKicked | ChatActionType::MemberBanned => {
                "person.badge.minus"
            }
            ChatActionType::MemberPromoted => "checkmark.shield",
            ChatActionType::MemberDemoted => "shield.slash",
            ChatActionType::MemberMuted => "bell.slash",
            ChatActionType::MemberUnmuted => "bell",
            ChatActionType::MessagePinned => "pin",
            ChatActionType::MessageUnpinned => "pin.slash",
            ChatActionType::MessageDeleted => "trash",
            ChatActionType::SettingsChanged | ChatActionType::CustomizationChanged => "gearshape",
            ChatActionType::InviteLinkCreated | ChatActionType::InviteLinkRevoked | ChatActionType::LinkChanged => {
                "link"
            }
            ChatActionType::MemberRightsChanged | ChatActionType::PermissionsChanged => "lock.shield",
            ChatActionType::OwnershipTransferred => "crown",
            ChatActionType::TitleChanged | ChatActionType::DescriptionChanged => "pencil",
            ChatActionType::PhotoChanged | ChatActionType::PhotoRemoved => "photo",
            ChatActionType::MadePublic => "globe",
            ChatActionType::MadePrivate => "lock",
            ChatActionType::SlowModeChanged => "timer",
            ChatActionType::ReactionsChanged => "face.smiling",
            ChatActionType::ChatLinked | ChatActionType::ChatUnlinked => "link.circle",
            ChatActionType::RoleCreated | ChatActionType::RoleUpdated | ChatActionType::RoleDeleted => "tag",
            ChatActionType::RolesReordered => "arrow.up.arrow.down",
            ChatActionType::TopicCreated | ChatActionType::TopicEdited => "number",
            ChatActionType::TopicClosed => "lock",
            ChatActionType::TopicReopened => "lock.open",
            ChatActionType::TopicDeleted => "trash",
            ChatActionType::TopicAccessChanged => "eye.slash",
            ChatActionType::ChatCreated | ChatActionType::ChatDeleted | ChatActionType::ReactionsDeleted => {
                "clock.arrow.circlepath"
            }
        }
    }
}

pub type ChatReactionsType = rise_chat::ReactionsType;

pub trait ChatReactionsTypeWire: Sized {
    fn with_wire(wire: Option<&str>) -> Self;
}

impl ChatReactionsTypeWire for ChatReactionsType {
    fn with_wire(wire: Option<&str>) -> Self {
        let normalized = wire.unwrap_or("").to_uppercase();
        ChatReactionsType::ALL
            .into_iter()
            .find(|value| value.wire() == normalized || value.aliases().contains(&normalized.as_str()))
            .unwrap_or_else(|| {
                ChatReactionsType::ALL
                    .into_iter()
                    .find(|value| value.is_fallback())
                    .unwrap_or(ChatReactionsType::All)
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatReactionConfig {
    pub enabled: bool,
    pub emojis: Vec<String>,
    pub allows_custom_emoji: bool,
    pub restricted_emojis: Option<Vec<String>>,
}

impl ChatReactionConfig {
    pub fn new(enabled: bool, emojis: Vec<String>) -> Self {
        Self { enabled, emojis, allows_custom_emoji: false, restricted_emojis: None }
    }

    pub fn list_allows_custom_emoji(emojis: &[String]) -> bool {
        emojis.iter().any(|emoji| emoji.starts_with(MessageReaction::CUSTOM_EMOJI_PREFIX))
    }

    /// emojis is the whole allow list and may hold dozens, the bar is one unscrollable capsule
    /// so whatever passes the default width stays reachable through the expand chevron
    pub fn quick_bar_emojis(&self) -> Vec<String> {
        self.emojis.iter().take(ChatReactionCatalog::QUICK_BAR.len()).cloned().collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatReaction {
    pub emoji: String,
}

impl ChatReaction {
    pub fn id(&self) -> &str {
        &self.emoji
    }
}

pub enum ChatReactionCatalog {}

const CHAT_REACTION_CATALOG_EMOJIS: [&str; 38] = [
    "\u{1F44D}",
    "\u{1F44E}",
    "\u{2764}\u{FE0F}",
    "\u{1F525}",
    "\u{1F970}",
    "\u{1F44F}",
    "\u{1F601}",
    "\u{1F914}",
    "\u{1F92F}",
    "\u{1F631}",
    "\u{1F92C}",
    "\u{1F622}",
    "\u{1F389}",
    "\u{1F929}",
    "\u{1F92E}",
    "\u{1F4A9}",
    "\u{1F64F}",
    "\u{1F44C}",
    "\u{1F54A}",
    "\u{1F921}",
    "\u{1F971}",
    "\u{1F974}",
    "\u{1F60D}",
    "\u{1F433}",
    "\u{2764}\u{FE0F}\u{200D}\u{1F525}",
    "\u{1F31A}",
    "\u{1F4AF}",
    "\u{1F923}",
    "\u{26A1}\u{FE0F}",
    "\u{1F34C}",
    "\u{1F3C6}",
    "\u{1F494}",
    "\u{1F928}",
    "\u{1F610}",
    "\u{1F353}",
    "\u{1F37E}",
    "\u{1F48B}",
    "\u{1F608}",
];

static CHAT_REACTION_CATALOG_ALL: LazyLock<Vec<ChatReaction>> = LazyLock::new(|| {
    CHAT_REACTION_CATALOG_EMOJIS.iter().map(|emoji| ChatReaction { emoji: (*emoji).to_owned() }).collect()
});

static CHAT_REACTION_CATALOG_ALL_EMOJIS: LazyLock<Vec<String>> =
    LazyLock::new(|| CHAT_REACTION_CATALOG_ALL.iter().map(|reaction| reaction.emoji.clone()).collect());

impl ChatReactionCatalog {
    pub const QUICK_BAR: [&'static str; 7] = [
        "\u{2764}\u{FE0F}",
        "\u{1F44D}",
        "\u{1F44E}",
        "\u{1F525}",
        "\u{1F970}",
        "\u{1F44F}",
        "\u{1F601}",
    ];

    pub fn all() -> &'static [ChatReaction] {
        &CHAT_REACTION_CATALOG_ALL
    }

    pub fn all_emojis() -> &'static [String] {
        &CHAT_REACTION_CATALOG_ALL_EMOJIS
    }

    pub fn quick_bar() -> Vec<String> {
        Self::QUICK_BAR.iter().map(|emoji| (*emoji).to_owned()).collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LinkedChatInfo {
    pub id: String,
    pub title: String,
    pub logo_url: Option<String>,
    pub r#type: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChatAnalyticsModel {
    pub member_count: i64,
    pub new_members: i64,
    pub buckets: Vec<ChatAnalyticsPoint>,
    pub posts_count: i64,
    pub views_total: i64,
    pub reactions_total: i64,
    pub avg_views_per_post: i64,
    pub stats_available: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChatAnalyticsPoint {
    pub day_start_ms: i64,
    pub count: i64,
}

impl ChatAnalyticsPoint {
    pub fn id(&self) -> i64 {
        self.day_start_ms
    }
}

pub enum ChatPermKey {}

impl ChatPermKey {
    pub const TEXT: &'static str = ChatAllowKey::Text.raw_value();
    pub const LINKS: &'static str = ChatAllowKey::Links.raw_value();
    pub const POLLS: &'static str = ChatAllowKey::Polls.raw_value();
    pub const WEB_PREVIEWS: &'static str = ChatAllowKey::WebPagePreviews.raw_value();
    pub const PHOTO: &'static str = ChatAllowKey::Photo.raw_value();
    pub const VIDEO: &'static str = ChatAllowKey::Video.raw_value();
    pub const STICKERS: &'static str = ChatAllowKey::Stickers.raw_value();
    pub const GIF: &'static str = ChatAllowKey::Gif.raw_value();
    pub const MUSIC: &'static str = ChatAllowKey::Music.raw_value();
    pub const FILES: &'static str = ChatAllowKey::Files.raw_value();
    pub const VOICE: &'static str = ChatAllowKey::VoiceMessages.raw_value();
    pub const VIDEO_MESSAGES: &'static str = ChatAllowKey::VideoMessages.raw_value();
    pub const MEDIA: [&'static str; 8] = [
        Self::PHOTO,
        Self::VIDEO,
        Self::STICKERS,
        Self::GIF,
        Self::MUSIC,
        Self::FILES,
        Self::VOICE,
        Self::VIDEO_MESSAGES,
    ];
}

pub enum ChatPermissionJSON {}

impl ChatPermissionJSON {
    pub fn parse(raw: Option<&str>) -> HashMap<String, bool> {
        let Some(raw) = raw else {
            return HashMap::new();
        };
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(raw) else {
            return HashMap::new();
        };
        let mut out: HashMap<String, bool> = HashMap::new();
        for (k, v) in obj {
            match v {
                Value::Bool(b) => {
                    out.insert(k, b);
                }
                Value::Number(n) => {
                    out.insert(k, n.as_f64().is_some_and(|n| n != 0.0));
                }
                _ => {}
            }
        }
        out
    }

    pub fn serialize(dict: &HashMap<String, bool>) -> String {
        serde_json::to_string(dict).unwrap_or_else(|_| "{}".to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSettingsModel {
    pub chat_id: String,
    pub reactions_type: ChatReactionsType,
    pub allowed_reactions: Vec<String>,
    pub max_reaction_count: i64,
    pub slow_mode: i64,
    pub members_can_add_members: bool,
    pub members_can_change_pfp: bool,
    pub message_allow: HashMap<String, bool>,
    pub media_allow: HashMap<String, bool>,
    pub linked_chat_id: Option<String>,
    pub paid_reactions_allowed: Option<bool>,
}

impl Default for ChatSettingsModel {
    fn default() -> Self {
        Self {
            chat_id: String::new(),
            reactions_type: ChatReactionsType::All,
            allowed_reactions: Vec::new(),
            max_reaction_count: 11,
            slow_mode: 0,
            members_can_add_members: true,
            members_can_change_pfp: false,
            message_allow: HashMap::new(),
            media_allow: HashMap::new(),
            linked_chat_id: None,
            paid_reactions_allowed: None,
        }
    }
}

impl ChatSettingsModel {
    pub fn msg(&self, key: &str) -> bool {
        self.message_allow.get(key).copied().unwrap_or(true)
    }

    pub fn media(&self, key: &str) -> bool {
        self.media_allow.get(key).copied().unwrap_or(true)
    }

    pub fn media_enabled_count(&self) -> usize {
        ChatPermKey::MEDIA.iter().filter(|key| self.media(key)).count()
    }

    pub fn reaction_config(&self) -> ChatReactionConfig {
        match self.reactions_type {
            ChatReactionsType::None => ChatReactionConfig::new(false, Vec::new()),
            ChatReactionsType::Custom => {
                let emojis = self.allowed_reactions.clone();
                ChatReactionConfig {
                    enabled: !emojis.is_empty(),
                    allows_custom_emoji: ChatReactionConfig::list_allows_custom_emoji(&emojis),
                    restricted_emojis: Some(emojis.clone()),
                    emojis,
                }
            }
            ChatReactionsType::All => ChatReactionConfig {
                enabled: true,
                emojis: ChatReactionCatalog::quick_bar(),
                allows_custom_emoji: true,
                restricted_emojis: None,
            },
        }
    }
}

pub type ChatPerm = rise_canon::ChatPerm;

static CHAT_PERM_MASK_BY_NAME: LazyLock<HashMap<&'static str, i64>> =
    LazyLock::new(|| ChatPerm::BITS.iter().map(|bit| (bit.name, bit.mask)).collect());

static CHAT_PERM_OWNER_ROLES_NORMALIZED: LazyLock<Vec<String>> =
    LazyLock::new(|| ChatPerm::OWNER_ROLES.iter().map(|role| role.to_uppercase()).collect());

pub trait ChatPermExt {
    /// administrator bypasses the bit check entirely, product rule since canon stores only the bits
    fn has(perms: i64, bit: i64) -> bool;

    fn mask(name: &str) -> Option<i64>;

    /// the only owner check in the project, dont add another one
    /// canon compares case insensitively and doesnt trim whitespace because the server doesnt
    fn is_owner_role(role: Option<&str>) -> bool;

    /// legacy member role ADMIN is not in canon, the server folds it into the administrator bit
    /// kept apart from owner so owner never means admin
    fn is_owner_or_legacy_admin_role(role: Option<&str>) -> bool;
}

impl ChatPermExt for ChatPerm {
    fn has(perms: i64, bit: i64) -> bool {
        (perms & ChatPerm::ADMINISTRATOR) != 0 || (perms & bit) == bit
    }

    fn mask(name: &str) -> Option<i64> {
        CHAT_PERM_MASK_BY_NAME.get(name).copied()
    }

    fn is_owner_role(role: Option<&str>) -> bool {
        let Some(role) = role.filter(|role| !role.is_empty()) else {
            return false;
        };
        let candidate = if ChatPerm::OWNER_ROLE_TRIM { trimming_whitespaces_and_newlines(role) } else { role };
        if !ChatPerm::OWNER_ROLE_CASE_INSENSITIVE {
            return ChatPerm::OWNER_ROLES.contains(&candidate);
        }
        let upper = candidate.to_uppercase();
        CHAT_PERM_OWNER_ROLES_NORMALIZED.iter().any(|owner| *owner == upper)
    }

    fn is_owner_or_legacy_admin_role(role: Option<&str>) -> bool {
        let Some(role) = role.filter(|role| !role.is_empty()) else {
            return false;
        };
        Self::is_owner_role(Some(role)) || role.to_uppercase() == "ADMIN"
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChatRole {
    pub id: String,
    pub name: String,
    pub color: i64,
    pub position: i64,
    pub permissions: i64,
    pub mentionable: bool,
    pub hoist: bool,
    pub is_default: bool,
}

impl ChatRole {
    pub fn swift_color(&self) -> Option<Hsla> {
        rgb_color(self.color)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChatSection {
    pub id: String,
    pub name: String,
    pub position: i64,
    pub name_entities: Option<String>,
}

impl ChatSection {
    pub fn name_custom_emojis(&self) -> Vec<MessageCustomEmojiEntity> {
        CommentEntities::normalized(self.name_entities.as_deref()).1
    }
}

pub type ChatPermOverwrite = rise_canon::ChatPermissionOverwrite;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OverwriteScope {
    Topic,
    Section,
}

pub fn rgb_int(r: f64, g: f64, b: f64) -> i64 {
    (((r * 255.0) as i64) << 16) | (((g * 255.0) as i64) << 8) | ((b * 255.0) as i64)
}
