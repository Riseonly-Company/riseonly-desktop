use gpui::App;
use rise_core::foundation::foundation_date::TimeInterval;
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};

use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryGroupFlags;
use crate::modules::chat::shared::message_permissions::MessagePermissions;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType, User};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageReaction,
};
use crate::modules::post::stores::post_actions::post_types::AuthorInfoMore;
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;

fn effective_user_id_for_wire(cx: &App) -> Option<String> {
    rise_ui::shared::try_read::<AuthServicesStore>(cx).and_then(AuthServicesStore::effective_user_id_for_wire)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatSavedMessageAuthor {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo: Option<String>,
}

/// saving to favourites is not a forward
/// message is made in your own chat but shown as the original author with avatar and name
/// no forwarded header, pure on purpose
pub enum ChatSavedMessageIdentity {}

impl ChatSavedMessageIdentity {
    pub fn author(
        message: &GetMessageMessage,
        is_saved_messages_chat: bool,
        viewer_user_id: Option<&str>,
    ) -> Option<ChatSavedMessageAuthor> {
        if !is_saved_messages_chat {
            return None;
        }
        let forward = message.forward_info.as_ref()?;
        let viewer = Self::normalized(viewer_user_id)?;
        let id = Self::normalized(forward.sender_id.as_deref())?;
        if RiseIdentity::is_viewer(Some(id.as_str()), Some(viewer.as_str())) {
            return None;
        }
        let name = Self::normalized(Some(forward.sender_name.as_str()))
            .or_else(|| Self::normalized(forward.from_chat_title.as_deref()))?;
        Some(ChatSavedMessageAuthor {
            id,
            name,
            tag: Self::normalized(forward.sender_tag.as_deref()).unwrap_or_default(),
            logo: Self::normalized(forward.sender_logo.as_deref())
                .or_else(|| Self::normalized(forward.from_chat_logo.as_deref())),
        })
    }

    /// row grouping key
    /// every saved message is authored by the viewer so sender_id would merge saves from
    /// different people into one group
    pub fn grouping_sender_id(
        message: &GetMessageMessage,
        is_saved_messages_chat: bool,
        viewer_user_id: Option<&str>,
    ) -> String {
        match Self::author(message, is_saved_messages_chat, viewer_user_id) {
            Some(author) => format!("saved:{}", author.id),
            None => message.sender_id.clone(),
        }
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(value?);
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }
}

pub enum ChatSavedMessagePresentation {}

impl ChatSavedMessagePresentation {
    pub fn is_saved_messages_chat(chat: &ChatInfo) -> bool {
        chat.r#type == ChatType::Favourites
    }

    pub fn author(message: &GetMessageMessage, chat: &ChatInfo, cx: &App) -> Option<ChatSavedMessageAuthor> {
        if chat.r#type != ChatType::Favourites {
            return None;
        }
        ChatSavedMessageIdentity::author(message, true, effective_user_id_for_wire(cx).as_deref())
    }

    /// saved message already shows as its author so a forwarded header would repeat the same name
    pub fn hides_forward_header(message: &GetMessageMessage, chat: &ChatInfo, cx: &App) -> bool {
        Self::author(message, chat, cx).is_some()
    }
}

pub enum RiseMessageSenderIdentity {}

impl RiseMessageSenderIdentity {
    /// row author as the list shows it
    /// in favourites the person whose message was saved, elsewhere the sender
    pub fn participant(message: &GetMessageMessage, chat: &ChatInfo, cx: &App) -> User {
        if let Some(saved) = ChatSavedMessagePresentation::author(message, chat, cx) {
            return User {
                id: saved.id,
                name: saved.name,
                tag: saved.tag,
                more: AuthorInfoMore { logo: saved.logo.unwrap_or_default(), ..AuthorInfoMore::default() },
                ..User::default()
            };
        }
        User {
            id: message.sender_id.clone(),
            name: message.sender_name.clone(),
            tag: message.sender_tag.clone(),
            more: AuthorInfoMore {
                logo: message.sender_logo.clone().unwrap_or_default(),
                ..AuthorInfoMore::default()
            },
            is_deleted: message.sender_is_deleted,
            ..User::default()
        }
    }
}

pub enum ChatMessageGrouping {}

impl ChatMessageGrouping {
    pub const WINDOW: TimeInterval = 600.0;

    /// first and last in group flags for message lists rendered row by row
    /// the manual list does this in its own preparator off main thread, this is the same
    /// contract for other screens
    pub fn flags(messages: &[GetMessageMessage], chat: &ChatInfo, cx: &App) -> Vec<ChatMessageRenderEntryGroupFlags> {
        if messages.is_empty() {
            return Vec::new();
        }
        let saved_messages_chat = ChatSavedMessagePresentation::is_saved_messages_chat(chat);
        let viewer_user_id = effective_user_id_for_wire(cx);
        let authors: Vec<String> = messages
            .iter()
            .map(|message| {
                ChatSavedMessageIdentity::grouping_sender_id(message, saved_messages_chat, viewer_user_id.as_deref())
            })
            .collect();

        let mut result = Vec::with_capacity(messages.len());
        for (index, message) in messages.iter().enumerate() {
            if message.content_type == MessageContentType::System {
                result.push(ChatMessageRenderEntryGroupFlags { is_first: true, is_last: true });
                continue;
            }
            let previous = index.checked_sub(1).map(|previous_index| &messages[previous_index]);
            let next = messages.get(index + 1);
            let is_first = previous.is_none_or(|previous| {
                previous.content_type == MessageContentType::System
                    || authors[index - 1] != authors[index]
                    || (message.created_at - previous.created_at).abs() > Self::WINDOW
            });
            let is_last = next.is_none_or(|next| {
                next.content_type == MessageContentType::System
                    || authors[index + 1] != authors[index]
                    || (next.created_at - message.created_at).abs() > Self::WINDOW
            });
            result.push(ChatMessageRenderEntryGroupFlags { is_first, is_last });
        }
        result
    }
}

pub enum ChatMessageReactionPolicy {}

impl ChatMessageReactionPolicy {
    pub fn can_add(reaction: &str, message: &GetMessageMessage, chat: &ChatInfo, cx: &mut App) -> bool {
        if message.is_system()
            || message.content_type == MessageContentType::Call
            || message.is_pending_outgoing()
            || message.is_temp
            || reaction.is_empty()
        {
            return false;
        }

        let sai_server_id = message.sai_server_id();
        let server_message_id = trimming_whitespaces_and_newlines(&sai_server_id);
        if server_message_id.is_empty() || GetSaiServerId::is_temp_id(Some(server_message_id)) {
            return false;
        }
        let reaction_config = ChatServicesStore::shared(cx).read(cx).reaction_config(&chat.id, &chat.allowed_reactions);
        if !reaction_config.enabled || !MessagePermissions::new(message, chat, false).can_add_reactions(cx) {
            return false;
        }
        if reaction.starts_with(MessageReaction::CUSTOM_EMOJI_PREFIX)
            && (!reaction_config.allows_custom_emoji
                || character_count(reaction) <= character_count(MessageReaction::CUSTOM_EMOJI_PREFIX))
        {
            return false;
        }
        match &reaction_config.restricted_emojis {
            Some(allowed) => allowed.iter().any(|emoji| emoji == reaction),
            None => true,
        }
    }
}

/// double tap quick reaction
/// one rule for every screen with a message bubble: chat, pinned and favourites tab of the profile
pub enum ChatQuickReactionPolicy {}

impl ChatQuickReactionPolicy {
    /// ce:id has to survive this read
    /// the unicode only normalizer rejects anything over one grapheme and rewrote custom quick
    /// reactions to the default
    pub fn default_raw(cx: &App) -> String {
        let quick_reaction = rise_ui::shared::try_read::<AuthServicesStore>(cx)
            .and_then(AuthServicesStore::profile)
            .map(|profile| profile.quick_reaction.as_str());
        EmojiPickerCatalog::normalized_selectable_emoji_or_custom_ref(quick_reaction)
            .unwrap_or_else(|| ProfileModel::DEFAULT_QUICK_REACTION.to_owned())
    }

    pub fn resolved(message: &GetMessageMessage, chat: &ChatInfo, raw: &str, cx: &mut App) -> Option<String> {
        let reaction = if raw.starts_with(MessageReaction::CUSTOM_EMOJI_PREFIX) {
            raw.to_owned()
        } else {
            EmojiPickerCatalog::normalized_selectable_emoji(Some(raw))
                .unwrap_or_else(|| ProfileModel::DEFAULT_QUICK_REACTION.to_owned())
        };
        if ChatMessageReactionPolicy::can_add(&reaction, message, chat, cx) {
            return Some(reaction);
        }
        let fallback = ChatServicesStore::shared(cx)
            .read(cx)
            .reaction_config(&chat.id, &chat.allowed_reactions)
            .emojis
            .into_iter()
            .next()?;
        if fallback == reaction || !ChatMessageReactionPolicy::can_add(&fallback, message, chat, cx) {
            return None;
        }
        Some(fallback)
    }
}
