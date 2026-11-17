use std::sync::LazyLock;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RisePositionKey, RisePositionKeyOrder,
};
use rise_engine::rise_store::rise_store_models::RiseStoreNamespace;

use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::modules::chat::engine::core::rise_conversation_entity_records::RiseMessageLocalPhase;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;

static CHATS: LazyLock<RiseStoreNamespace> = LazyLock::new(|| RiseStoreNamespace::new("conversation.chats.v1"));
static MESSAGES: LazyLock<RiseStoreNamespace> = LazyLock::new(|| RiseStoreNamespace::new("conversation.messages.v1"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseConversationStoreLayoutChatView {
    Inbox,
    Archive,
    Requests,
}

impl RiseConversationStoreLayoutChatView {
    pub const ALL_CASES: [Self; 3] = [Self::Inbox, Self::Archive, Self::Requests];

    pub fn all_cases() -> &'static [Self] {
        &Self::ALL_CASES
    }

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Inbox => "chat.inbox",
            Self::Archive => "chat.archive",
            Self::Requests => "chat.requests",
        }
    }
}

pub enum RiseConversationStoreLayout {}

impl RiseConversationStoreLayout {
    const PENDING_POSITION_BASE_MICROS: i64 = 4_000_000_000_000_000;

    pub fn chats() -> RiseStoreNamespace {
        CHATS.clone()
    }

    pub fn messages() -> RiseStoreNamespace {
        MESSAGES.clone()
    }

    pub fn message_view(scope: &RiseMessageScope) -> String {
        Self::encoded(&["message.timeline", &scope.storage_key()])
    }

    /// inverse of message_view, none for any other view of the messages namespace
    pub fn message_scope(from_view: &str) -> Option<RiseMessageScope> {
        let prefix = Self::encoded(&["message.timeline"]) + "|";
        let rest = from_view.strip_prefix(prefix.as_str())?;
        let (declared_length, storage_key) = rest.split_once(':')?;
        if declared_length.parse::<i64>().ok()? != storage_key.len() as i64 {
            return None;
        }
        RiseMessageScope::parse(storage_key)
    }

    pub fn chat_position(chat: &RiseStoredChat) -> RisePositionKey {
        let pinned = chat.is_pinned == Some(true);
        let message_activity = Self::normalized_timestamp(
            chat.last_message
                .as_ref()
                .and_then(|message| message.created_at.or(message.timestamp))
                .or(chat.updated_at)
                .or(chat.last_activity)
                .or(chat.created_at)
                .unwrap_or(0.0),
        );
        let activity = message_activity.max(Self::normalized_timestamp(chat.last_activity.unwrap_or(0.0)));
        let activity_micros = Self::clamped_micros(activity);
        let pin_position = chat.pin_position.unwrap_or(i64::MAX);
        let identity = Self::canonical_chat_server_id(chat);
        RisePositionKey::make(|key| {
            key.append_signed_int64(if pinned { 0 } else { 1 });
            key.append_signed_int64(if pinned { pin_position } else { 0 });
            key.append_signed_int64_with_order(activity_micros, RisePositionKeyOrder::Descending);
            key.append_string_with_order(&identity, RisePositionKeyOrder::Ascending);
        })
    }

    /// pending row sits outside server order, in the feed tail by send time
    /// else the ack reorders it since the server stamps created_at on receipt
    /// failed row goes back to its own time, a retry puts it in the tail
    pub fn message_position(
        message: &MessageWireDTO,
        local_id: &str,
        local_phase: RiseMessageLocalPhase,
    ) -> RisePositionKey {
        let timestamp = Self::normalized_timestamp(message.created_at.or(message.timestamp).unwrap_or(0.0));
        let is_pending = local_phase != RiseMessageLocalPhase::Remote && local_phase != RiseMessageLocalPhase::Failed;
        let micros = if is_pending {
            Self::PENDING_POSITION_BASE_MICROS.wrapping_add(Self::clamped_micros(timestamp))
        } else {
            Self::clamped_micros(timestamp)
        };
        let identity = if is_pending {
            local_id.to_owned()
        } else {
            Self::canonical_message_server_id(message).unwrap_or_else(|| local_id.to_owned())
        };
        RisePositionKey::make(|key| {
            key.append_signed_int64_with_order(micros, RisePositionKeyOrder::Ascending);
            key.append_string_with_order(&identity, RisePositionKeyOrder::Descending);
        })
    }

    pub fn canonical_chat_server_id(chat: &RiseStoredChat) -> String {
        if !GetSaiServerId::is_temp_id(Some(chat.id.as_str())) {
            return chat.id.clone();
        }
        Self::normalized_id(chat.server_id.as_deref()).map_or_else(|| chat.id.clone(), str::to_owned)
    }

    pub fn canonical_message_server_id(message: &MessageWireDTO) -> Option<String> {
        for raw in [message.server_id.as_deref(), message.id.as_deref()] {
            let Some(id) = Self::normalized_id(raw) else {
                continue;
            };
            if GetSaiServerId::is_temp_id(Some(id)) {
                continue;
            }
            return Some(id.to_owned());
        }
        None
    }

    pub fn message_aliases(message: &MessageWireDTO) -> Vec<RiseEntityAlias> {
        let mut aliases: Vec<RiseEntityAlias> = Vec::new();
        if let Some(server_id) = Self::canonical_message_server_id(message) {
            Self::append_alias(&mut aliases, RiseEntityAlias::new(RiseEntityAliasKind::server_id(), server_id));
        }
        if let Some(id) = Self::normalized_id(message.id.as_deref()) {
            Self::append_alias(&mut aliases, RiseEntityAlias::new(Self::identifier_kind(id), id));
        }
        if let Some(temp_id) = Self::normalized_id(message.temp_id.as_deref()) {
            Self::append_alias(&mut aliases, RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), temp_id));
        }
        aliases
    }

    pub fn chat_aliases(chat: &RiseStoredChat) -> Vec<RiseEntityAlias> {
        let mut aliases: Vec<RiseEntityAlias> = Vec::new();
        for raw in [Some(chat.id.as_str()), chat.server_id.as_deref()] {
            let Some(id) = Self::normalized_id(raw) else {
                continue;
            };
            Self::append_alias(&mut aliases, RiseEntityAlias::new(Self::identifier_kind(id), id));
        }
        aliases
    }

    pub fn local_message_id(message: &MessageWireDTO) -> String {
        if let Some(temp_id) = Self::normalized_id(message.temp_id.as_deref()) {
            return temp_id.to_owned();
        }
        if let Some(id) = Self::normalized_id(message.id.as_deref())
            && GetSaiServerId::is_temp_id(Some(id))
        {
            return id.to_owned();
        }
        if let Some(server_id) = Self::canonical_message_server_id(message) {
            return server_id;
        }
        format!("temp_{}", Uuid::new_v4().uuid_string().to_lowercase())
    }

    pub fn local_chat_id(chat: &RiseStoredChat) -> String {
        let id = trimming_whitespaces_and_newlines(&chat.id);
        if GetSaiServerId::is_temp_id(Some(id)) {
            return id.to_owned();
        }
        Self::canonical_chat_server_id(chat)
    }

    pub fn normalized_timestamp(raw: f64) -> f64 {
        if raw > 1_000_000_000_000.0 { raw / 1_000.0 } else { raw }
    }

    fn identifier_kind(id: &str) -> RiseEntityAliasKind {
        if GetSaiServerId::is_temp_id(Some(id)) {
            RiseEntityAliasKind::temporary_id()
        } else {
            RiseEntityAliasKind::server_id()
        }
    }

    fn append_alias(aliases: &mut Vec<RiseEntityAlias>, alias: RiseEntityAlias) {
        if !aliases.contains(&alias) {
            aliases.push(alias);
        }
    }

    fn clamped_micros(seconds: f64) -> i64 {
        if !seconds.is_finite() || seconds <= 0.0 {
            return 0;
        }
        let value = seconds * 1_000_000.0;
        if value >= i64::MAX as f64 {
            return i64::MAX;
        }
        value.trunc() as i64
    }

    fn normalized_id(raw: Option<&str>) -> Option<&str> {
        raw.map(trimming_whitespaces_and_newlines).filter(|value| !value.is_empty())
    }

    fn encoded(components: &[&str]) -> String {
        components
            .iter()
            .map(|component| format!("{}:{component}", component.len()))
            .collect::<Vec<_>>()
            .join("|")
    }
}

/// extra methods for the conversation only alias kinds
pub trait RiseEntityAliasKindExt {
    fn request_id() -> RiseEntityAliasKind;
    fn web_app_id() -> RiseEntityAliasKind;
}

impl RiseEntityAliasKindExt for RiseEntityAliasKind {
    fn request_id() -> RiseEntityAliasKind {
        RiseEntityAliasKind::new("request")
    }

    fn web_app_id() -> RiseEntityAliasKind {
        RiseEntityAliasKind::new("webapp")
    }
}
