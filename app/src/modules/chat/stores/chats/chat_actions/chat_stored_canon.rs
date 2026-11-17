use rise_canon::rise_proto::chat::{ParticipantInfo, ProtoChatInfo, ProtoMessagePreview, ProtoSenderShort};
use rise_canon::rise_view::chat::{ArchivedChats, ChatById, ChatInfo, ChatPage, ChatUnread, MessageRequests};

use super::chat_archive_wire_types::RiseStoredArchivedChatPage;
use super::chat_message_requests_wire_types::RiseStoredMessageRequestPage;
use super::chat_wire_types::{
    RiseStoredChat, RiseStoredChatCreator, RiseStoredChatDetail, RiseStoredChatPage, RiseStoredChatParticipant,
    RiseStoredChatPreview, RiseStoredChatPreviewMedia, RiseStoredChatPreviewSender,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageSenderWireDTO;

impl RiseStoredChat {
    pub fn from_canon(chat: &ChatInfo) -> Self {
        Self {
            id: chat.id.clone(),
            server_id: None,
            title: Some(chat.title.clone()),
            r#type: Some(chat.r#type.clone()),
            description: chat.description.clone(),
            logo_url: chat.logo_url.clone(),
            banner_url: chat.banner_url.clone(),
            username: chat.username.clone(),
            tag: chat.tag.clone(),
            member_count: Some(chat.member_count),
            is_verified: Some(chat.is_verified),
            is_public: Some(chat.is_public),
            is_pinned: Some(chat.is_pinned),
            pin_position: chat.pin_position,
            is_muted: Some(chat.is_muted),
            unread_count: Some(chat.unread_count),
            role: chat.role.clone(),
            created_at: None,
            updated_at: Some(chat.updated_at as f64),
            joined_at: chat.joined_at.map(|value| value as f64),
            mute_until: chat.mute_until.map(|value| value as f64),
            last_activity: chat.last_activity.map(|value| value as f64),
            notifications_enabled: Some(chat.notifications_enabled),
            can_invite_users: Some(chat.can_invite_users),
            can_pin_messages: Some(chat.can_pin_messages),
            can_manage_chat: Some(chat.can_manage_chat),
            can_accept_join_requests: Some(chat.can_accept_join_requests),
            is_anonymous: Some(chat.is_anonymous),
            custom_title: chat.custom_title.clone(),
            custom_color: chat.custom_color.clone(),
            forward_in_chat_enabled: chat.forward_in_chat_enabled,
            allowed_reactions: Some(chat.allowed_reactions.clone()),
            topics_enabled: Some(chat.topics_enabled),
            auto_delete_messages_after: chat.auto_delete_messages_after,
            mute_scopes: chat.mute_scopes.clone(),
            participant: chat.participant.as_ref().map(RiseStoredChatParticipant::from_canon),
            last_message: chat.last_message.as_ref().map(|preview| RiseStoredChatPreview::from_canon(preview, &chat.id)),
            creator_info: chat.creator_info.as_deref().map(RiseStoredChatCreator::from_canon),
            is_self_chat: Some(chat.is_self_chat),
            chat_type_enum: Some(chat.chat_type_enum),
            is_message_request: Some(chat.is_message_request),
            message_request_state: chat.message_request_state.clone(),
            join_approval_required: Some(chat.join_approval_required),
            invite_link: chat.invite_link.clone(),
            can_share_invite: Some(chat.can_share_invite),
            has_live_voice: Some(chat.has_live_voice),
            wallpaper: chat.wallpaper.clone(),
            unread_info: chat.unread_info.clone(),
            unread_media_count: Some(chat.unread_media_count),
            ..Self::default()
        }
    }

    fn stored_sender(sender: &ProtoSenderShort) -> Option<MessageSenderWireDTO> {
        let value = serde_json::to_value(sender).ok()?;
        serde_json::from_value(value).ok()
    }

    pub fn from_canon_proto(chat: &ProtoChatInfo) -> Self {
        Self {
            id: chat.id.clone(),
            server_id: None,
            title: Some(chat.title.clone()),
            r#type: Some(chat.r#type.clone()),
            description: chat.description.clone(),
            logo_url: chat.logo_url.clone(),
            banner_url: chat.banner_url.clone(),
            username: chat.username.clone(),
            tag: chat.tag.clone(),
            member_count: Some(chat.member_count),
            is_verified: Some(chat.is_verified),
            is_public: Some(chat.is_public),
            is_pinned: Some(chat.is_pinned),
            pin_position: chat.pin_position,
            is_muted: Some(chat.is_muted),
            unread_count: Some(chat.unread_count),
            role: chat.role.clone(),
            created_at: None,
            updated_at: (chat.updated_at > 0).then_some(chat.updated_at as f64),
            joined_at: chat.joined_at.map(|value| value as f64),
            mute_until: chat.mute_until.map(|value| value as f64),
            last_activity: chat.last_activity.map(|value| value as f64),
            notifications_enabled: Some(chat.notifications_enabled),
            can_invite_users: Some(chat.can_invite_users),
            can_pin_messages: Some(chat.can_pin_messages),
            can_manage_chat: Some(chat.can_manage_chat),
            can_accept_join_requests: Some(chat.can_accept_join_requests),
            is_anonymous: Some(chat.is_anonymous),
            custom_title: chat.custom_title.clone(),
            custom_color: chat.custom_color.clone(),
            forward_in_chat_enabled: chat.forward_in_chat_enabled,
            allowed_reactions: Some(chat.allowed_reactions.clone()),
            topics_enabled: Some(chat.topics_enabled),
            auto_delete_messages_after: chat.auto_delete_messages_after,
            mute_scopes: chat.mute_scopes.clone(),
            participant: chat.participant.as_ref().map(RiseStoredChatParticipant::from_canon),
            last_message: chat.last_message.as_ref().map(|preview| RiseStoredChatPreview::from_canon(preview, &chat.id)),
            creator_info: chat
                .creator
                .as_ref()
                .and_then(Self::stored_sender)
                .map(RiseStoredChatCreator::User)
                .or_else(|| chat.creator_info.as_deref().map(RiseStoredChatCreator::from_canon))
                .or_else(|| chat.creator_id.clone().map(RiseStoredChatCreator::Id)),
            is_self_chat: Some(chat.is_self_chat),
            chat_type_enum: Some(chat.chat_type_enum),
            is_message_request: Some(chat.is_message_request),
            message_request_state: chat.message_request_state.clone(),
            join_approval_required: Some(chat.join_approval_required),
            invite_link: chat.invite_link.clone(),
            can_share_invite: Some(chat.can_share_invite),
            has_live_voice: Some(chat.has_live_voice),
            wallpaper: chat.wallpaper.clone(),
            unread_info: chat.unread_info.as_ref().and_then(|unread| {
                let value = serde_json::to_value(unread).ok()?;
                serde_json::from_value::<ChatUnread>(value).ok()
            }),
            unread_media_count: chat.unread_media_count,
            ..Self::default()
        }
    }
}

impl RiseStoredChatPreview {
    pub fn from_canon(preview: &ProtoMessagePreview, chat_id: &str) -> Self {
        Self {
            id: Some(preview.id.clone()),
            chat_id: Some(chat_id.to_owned()),
            topic_id: preview.topic_id.clone(),
            sender_id: Some(preview.sender_id.clone()),
            sender_name: Some(preview.sender_name.clone()),
            content: Some(preview.content.clone()),
            caption: preview.caption.clone(),
            content_type: Some(preview.content_type.clone()),
            timestamp: Some(preview.timestamp as f64),
            created_at: None,
            has_attachments: preview.has_attachments,
            has_reactions: preview.has_reactions,
            is_pinned: preview.is_pinned,
            is_read: preview.is_read,
            is_system_message: preview.is_system_message,
            system_message_type: preview.system_message_type.clone(),
            metadata: None,
            media_info: preview.media_info.as_deref().map(RiseStoredChatPreviewMedia::from_summary),
            sender: preview
                .sender
                .as_ref()
                .map(|sender| RiseStoredChatPreviewSender { is_deleted: Some(sender.is_deleted) }),
            ..Self::default()
        }
    }
}

impl RiseStoredChatParticipant {
    pub fn from_canon(participant: &ParticipantInfo) -> Self {
        Self {
            id: participant.id.clone(),
            name: Some(participant.name.clone()),
            tag: Some(participant.tag.clone()),
            logo: None,
            avatar_url: None,
            is_online: None,
            last_seen: None,
            is_premium: Some(participant.is_premium),
            is_official: Some(participant.is_official),
            more: participant.more.clone(),
            is_deleted: Some(participant.is_deleted),
            is_bot: Some(participant.is_bot),
            bot_owner_id: participant.bot_owner_id.clone(),
            bot_is_active: Some(participant.bot_is_active),
            bot_users: participant.bot_users,
            alias_tags: Some(participant.alias_tags.clone()),
            premium_badge_emoji: participant.premium_badge_emoji.clone(),
        }
    }
}

impl RiseStoredChatCreator {
    pub fn from_canon(raw: &str) -> Self {
        match serde_json::from_str::<MessageSenderWireDTO>(raw) {
            Ok(sender) => Self::User(sender),
            Err(_) => Self::Id(raw.to_owned()),
        }
    }
}

impl RiseStoredChatPage {
    pub fn from_canon(page: &ChatPage) -> Self {
        Self::new(
            Some(page.chats.iter().map(RiseStoredChat::from_canon).collect()),
            page.relative_id.clone(),
            Some(page.limit),
            Some(page.is_have_more),
            None,
            None,
        )
    }
}

impl RiseStoredArchivedChatPage {
    pub fn from_canon(page: &ArchivedChats) -> Self {
        Self::new(
            Some(page.chats.iter().map(RiseStoredChat::from_canon).collect()),
            page.relative_id.clone(),
            Some(page.limit),
            Some(page.is_have_more),
            Some(page.total_count),
            Some(page.total_unread_count),
            Some(page.unread_chats_count),
            None,
            None,
        )
    }
}

impl RiseStoredMessageRequestPage {
    pub fn from_canon(page: &MessageRequests) -> Self {
        Self {
            chats: Some(page.chats.iter().map(RiseStoredChat::from_canon).collect()),
            has_more: Some(page.has_more),
            relative_id: page.relative_id.clone(),
            total_count: Some(page.total_count),
        }
    }
}

impl RiseStoredChatDetail {
    pub fn from_canon(detail: &ChatById) -> Self {
        Self {
            chat: detail.chat.as_ref().map(RiseStoredChat::from_canon),
            is_archived: Some(detail.is_archived),
            success: None,
            error: None,
        }
    }
}
