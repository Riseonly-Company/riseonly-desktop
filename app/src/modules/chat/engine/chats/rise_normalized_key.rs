use std::sync::LazyLock;

use rise_canon::rise_key;

use crate::modules::chat::engine::chats::rise_chat_repository_models::RiseChatWireCodec;

/// keys of normalized chat payloads in camelCase
/// a key renamed in canon breaks compile here instead of reading nothing
pub enum RiseNormalizedKey {}

impl RiseNormalizedKey {
    pub fn auto_delete_messages_after() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::AUTO_DELETE_MESSAGES_AFTER));
        KEY.as_str()
    }

    pub fn avatar_url() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::AVATAR_URL));
        KEY.as_str()
    }

    pub fn banner_url() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::BANNER_URL));
        KEY.as_str()
    }

    pub fn can_accept_join_requests() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::CAN_ACCEPT_JOIN_REQUESTS));
        KEY.as_str()
    }

    pub fn can_manage_chat() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::CAN_MANAGE_CHAT));
        KEY.as_str()
    }

    pub fn can_pin_messages() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::CAN_PIN_MESSAGES));
        KEY.as_str()
    }

    pub fn chat_id() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::CHAT_ID));
        KEY.as_str()
    }

    pub fn chat_info() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::CHAT_INFO));
        KEY.as_str()
    }

    pub fn content_type() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::CONTENT_TYPE));
        KEY.as_str()
    }

    pub fn created_at() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::CREATED_AT));
        KEY.as_str()
    }

    pub fn deleted_by() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::events::DELETED_BY));
        KEY.as_str()
    }

    pub fn deleted_message_ids() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::message::DELETED_MESSAGE_IDS));
        KEY.as_str()
    }

    pub fn display_color() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::DISPLAY_COLOR));
        KEY.as_str()
    }

    pub fn forward_in_chat_enabled() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::FORWARD_IN_CHAT_ENABLED));
        KEY.as_str()
    }

    pub fn has_attachments() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::HAS_ATTACHMENTS));
        KEY.as_str()
    }

    pub fn has_live_voice() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::HAS_LIVE_VOICE));
        KEY.as_str()
    }

    pub fn has_reactions() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::HAS_REACTIONS));
        KEY.as_str()
    }

    pub fn is_archived() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::IS_ARCHIVED));
        KEY.as_str()
    }

    pub fn is_message_request() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::IS_MESSAGE_REQUEST));
        KEY.as_str()
    }

    pub fn is_muted() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::IS_MUTED));
        KEY.as_str()
    }

    pub fn is_pinned() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::IS_PINNED));
        KEY.as_str()
    }

    pub fn is_public() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::IS_PUBLIC));
        KEY.as_str()
    }

    pub fn is_read() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::IS_READ));
        KEY.as_str()
    }

    pub fn is_system_message() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::IS_SYSTEM_MESSAGE));
        KEY.as_str()
    }

    pub fn joined_at() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::JOINED_AT));
        KEY.as_str()
    }

    pub fn last_activity() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::LAST_ACTIVITY));
        KEY.as_str()
    }

    pub fn last_message() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::LAST_MESSAGE));
        KEY.as_str()
    }

    pub fn logo_url() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::LOGO_URL));
        KEY.as_str()
    }

    pub fn member_count() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::MEMBER_COUNT));
        KEY.as_str()
    }

    pub fn message_id() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::message::MESSAGE_ID));
        KEY.as_str()
    }

    pub fn message_ids() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::MESSAGE_IDS));
        KEY.as_str()
    }

    pub fn message_request_state() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::MESSAGE_REQUEST_STATE));
        KEY.as_str()
    }

    pub fn mute_scopes() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::MUTE_SCOPES));
        KEY.as_str()
    }

    pub fn mute_until() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::MUTE_UNTIL));
        KEY.as_str()
    }

    pub fn new_role() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::events::NEW_ROLE));
        KEY.as_str()
    }

    pub fn notifications_enabled() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::NOTIFICATIONS_ENABLED));
        KEY.as_str()
    }

    pub fn pin_position() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::PIN_POSITION));
        KEY.as_str()
    }

    pub fn request_id() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::REQUEST_ID));
        KEY.as_str()
    }

    pub fn role_ids() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::ROLE_IDS));
        KEY.as_str()
    }

    pub fn sender_id() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::SENDER_ID));
        KEY.as_str()
    }

    pub fn sender_name() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::SENDER_NAME));
        KEY.as_str()
    }

    pub fn settings_changed() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::events::SETTINGS_CHANGED));
        KEY.as_str()
    }

    pub fn system_message_type() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::SYSTEM_MESSAGE_TYPE));
        KEY.as_str()
    }

    pub fn target_user_id() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::message::TARGET_USER_ID));
        KEY.as_str()
    }

    pub fn temp_id() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::message::TEMP_ID));
        KEY.as_str()
    }

    pub fn topic_id() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::TOPIC_ID));
        KEY.as_str()
    }

    pub fn topics_enabled() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::TOPICS_ENABLED));
        KEY.as_str()
    }

    pub fn unread_count() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::UNREAD_COUNT));
        KEY.as_str()
    }

    pub fn unread_info() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::UNREAD_INFO));
        KEY.as_str()
    }

    pub fn unread_media_count() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::UNREAD_MEDIA_COUNT));
        KEY.as_str()
    }

    pub fn unread_mentions_count() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::UNREAD_MENTIONS_COUNT));
        KEY.as_str()
    }

    pub fn unread_replies_count() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::UNREAD_REPLIES_COUNT));
        KEY.as_str()
    }

    pub fn unread_reactions_count() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::UNREAD_REACTIONS_COUNT));
        KEY.as_str()
    }

    pub fn updated_at() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::UPDATED_AT));
        KEY.as_str()
    }

    pub fn user_id() -> &'static str {
        static KEY: LazyLock<String> =
            LazyLock::new(|| RiseChatWireCodec::normalized_key(rise_key::chat::USER_ID));
        KEY.as_str()
    }
}
