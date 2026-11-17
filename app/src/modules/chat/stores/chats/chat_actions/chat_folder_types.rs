use gpui::Hsla;
use rise_canon::{rise_proto, rise_response};
use rise_chat::ContactFilter;
use rise_core::foundation::foundation_date::TimeInterval;
use rise_theme::color;

use super::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::MessageCustomEmojiEntity;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    CustomEmojiEntityWire, MessageEntitiesCodec,
};
use crate::modules::comment::stores::comment_actions::comment_types::CommentEntities;

pub type FolderChatType = rise_chat::FolderType;

pub trait FolderChatTypeExt {
    fn title_key(&self) -> &'static str;
    fn system_image(&self) -> &'static str;
    fn color(&self) -> Hsla;
}

impl FolderChatTypeExt for FolderChatType {
    fn title_key(&self) -> &'static str {
        self.i18n_key()
    }

    fn system_image(&self) -> &'static str {
        match self {
            FolderChatType::Contacts => "person.fill",
            FolderChatType::NonContacts => "person.crop.circle.badge.questionmark",
            FolderChatType::Groups => "person.2.fill",
            FolderChatType::Channels => "megaphone.fill",
            FolderChatType::Bots => "wand.and.stars",
        }
    }

    fn color(&self) -> Hsla {
        match self {
            FolderChatType::Contacts => color("#3478F6"),
            FolderChatType::NonContacts => color("#27C2C2"),
            FolderChatType::Groups => color("#34C759"),
            FolderChatType::Channels => color("#F0901E"),
            FolderChatType::Bots => color("#A45EF7"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatFolder {
    pub id: String,
    pub title: String,
    pub title_entities: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub included_chat_ids: Vec<String>,
    pub included_chat_types: Vec<String>,
    pub excluded_chat_ids: Vec<String>,
    pub excluded_chat_types: Vec<String>,
    pub pinned_chat_ids: Vec<String>,
    pub position: i64,
    pub created_at: TimeInterval,
    pub updated_at: TimeInterval,
}

impl ChatFolder {
    pub fn title_custom_emojis(&self) -> Vec<MessageCustomEmojiEntity> {
        CommentEntities::normalized(self.title_entities.as_deref()).1
    }

    pub fn dropping_orphan_object_marks(
        title: &str,
        entities: (Option<String>, Vec<MessageCustomEmojiEntity>),
    ) -> (String, Option<String>) {
        const MARK: u16 = 0xFFFC;
        let (json, custom_emojis) = entities;
        let units: Vec<u16> = title.encode_utf16().collect();
        if !units.contains(&MARK) {
            return (title.to_owned(), json);
        }
        let is_covered = |index: i64| {
            custom_emojis.iter().any(|entity| index >= entity.offset && index < entity.offset + entity.length.max(0))
        };
        let orphan_offsets: Vec<i64> = units
            .iter()
            .enumerate()
            .filter_map(|(index, unit)| i64::try_from(index).ok().filter(|index| *unit == MARK && !is_covered(*index)))
            .collect();
        if orphan_offsets.is_empty() {
            return (title.to_owned(), json);
        }
        let kept: Vec<u16> = units
            .iter()
            .enumerate()
            .filter(|(index, _)| i64::try_from(*index).map_or(true, |index| !orphan_offsets.contains(&index)))
            .map(|(_, unit)| *unit)
            .collect();
        let clean_title = String::from_utf16_lossy(&kept);
        if custom_emojis.is_empty() {
            return (clean_title, None);
        }
        let wires: Vec<CustomEmojiEntityWire> = custom_emojis
            .iter()
            .map(|entity| {
                let shift = i64::try_from(orphan_offsets.iter().filter(|offset| **offset < entity.offset).count())
                    .unwrap_or(i64::MAX);
                CustomEmojiEntityWire {
                    offset: entity.offset - shift,
                    length: entity.length,
                    custom_emoji_id: entity.custom_emoji_id.clone(),
                    file_url: entity.file_url.clone(),
                    asset_type: entity.asset_type.clone(),
                    width: entity.width,
                    height: entity.height,
                    pack_id: entity.pack_id.clone(),
                }
            })
            .collect();
        (clean_title, MessageEntitiesCodec::encode(&wires, None))
    }

    pub fn swift_color(&self) -> Option<Hsla> {
        let value = self.color.as_deref().filter(|value| !value.is_empty())?;
        Some(color(value))
    }
}

impl ChatFolder {
    fn chat_matches_type(&self, chat: &ChatInfo, type_raw: &str, is_contact: &dyn Fn(&str) -> bool) -> bool {
        let Some(t) = FolderChatType::from_wire(type_raw) else {
            return false;
        };
        let Some(folder_chat_type) = t.chat_type() else {
            return false;
        };
        if chat.r#type != folder_chat_type {
            return false;
        }
        match t.contact_filter() {
            ContactFilter::Any => true,
            ContactFilter::Contact => is_contact(&chat.participant.id),
            ContactFilter::NonContact => !is_contact(&chat.participant.id),
        }
    }

    pub fn contains(&self, chat: &ChatInfo, is_contact: &dyn Fn(&str) -> bool) -> bool {
        if self.excluded_chat_ids.contains(&chat.id) {
            return false;
        }
        if self.included_chat_ids.contains(&chat.id) {
            return true;
        }
        if self.excluded_chat_types.iter().any(|t| self.chat_matches_type(chat, t, is_contact)) {
            return false;
        }
        self.included_chat_types.iter().any(|t| self.chat_matches_type(chat, t, is_contact))
    }

    pub fn filter(&self, chats: &[ChatInfo], is_contact: &dyn Fn(&str) -> bool) -> Vec<ChatInfo> {
        chats.iter().filter(|chat| self.contains(chat, is_contact)).cloned().collect()
    }
}

pub enum ChatFolderPalette {}

impl ChatFolderPalette {
    pub const PRESETS: [&'static str; 8] =
        ["#3478F6", "#E0342E", "#F0901E", "#A45EF7", "#34C759", "#27C2C2", "#EA4C89", "#8E8E93"];
}

pub type ChatFolderWireDTO = rise_proto::chat::ProtoChatFolder;
pub type GetChatFoldersGatewayDTO = rise_response::chat::GetChatFolders;
pub type CreateChatFolderResultDTO = rise_response::chat::CreateChatFolder;
pub type EditChatFolderResultDTO = rise_response::chat::EditChatFolder;

pub trait ChatFolderWireDTOExt {
    fn to_chat_folder(&self) -> ChatFolder;
}

impl ChatFolderWireDTOExt for ChatFolderWireDTO {
    fn to_chat_folder(&self) -> ChatFolder {
        let (title, title_entities) = ChatFolder::dropping_orphan_object_marks(
            &self.title,
            CommentEntities::normalized(self.title_entities.as_deref()),
        );
        ChatFolder {
            id: self.id.clone(),
            title,
            title_entities,
            icon: self.icon.clone().filter(|icon| !icon.is_empty()),
            color: self.color.clone().filter(|color| !color.is_empty()),
            included_chat_ids: self.included_chat_ids.clone(),
            included_chat_types: self.included_chat_types.clone(),
            excluded_chat_ids: self.excluded_chat_ids.clone(),
            excluded_chat_types: self.excluded_chat_types.clone(),
            pinned_chat_ids: self.pinned_chat_ids.clone(),
            position: self.position,
            created_at: self.created_at as TimeInterval,
            updated_at: self.updated_at as TimeInterval,
        }
    }
}
