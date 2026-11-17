use gpui::App;

use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatPerm, ChatPermExt, ChatType};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::EffectivePermissionsDTOExt;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

pub enum ChatProfilePermissions {}

impl ChatProfilePermissions {
    pub fn can_manage(chat: &ChatInfo) -> bool {
        chat.can_manage_chat || Self::is_owner_or_admin(chat)
    }

    pub fn is_owner(chat: &ChatInfo) -> bool {
        ChatPerm::is_owner_role(chat.role.as_deref())
    }

    pub fn is_owner_or_admin(chat: &ChatInfo) -> bool {
        ChatPerm::is_owner_or_legacy_admin_role(chat.role.as_deref())
    }

    pub fn has(chat: &ChatInfo, perm: i64, cx: &mut App) -> bool {
        if Self::is_owner(chat) {
            return true;
        }
        if let Some(mask) = Self::loaded_mask(chat, cx) {
            return ChatPerm::has(mask, perm);
        }
        Self::is_owner_or_admin(chat)
    }

    pub fn manages_chat(chat: &ChatInfo, cx: &mut App) -> bool {
        if Self::is_owner_or_admin(chat) {
            return true;
        }
        let Some(mask) = Self::loaded_mask(chat, cx) else {
            return chat.can_manage_chat && !Self::is_outside_group(chat);
        };
        ChatPerm::has(mask, ChatPerm::MANAGE_MESSAGES) || ChatPerm::has(mask, ChatPerm::MANAGE_GROUP)
    }

    pub fn can_edit_profile(chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type != ChatType::Group && chat.r#type != ChatType::Channel {
            return false;
        }
        if Self::has(chat, ChatPerm::MANAGE_GROUP, cx) {
            return true;
        }
        chat.r#type == ChatType::Group
            && !Self::is_outside_group(chat)
            && ChatServicesStore::shared(cx)
                .read(cx)
                .chat_settings(&chat.id, cx)
                .is_some_and(|settings| settings.members_can_change_pfp)
    }

    pub fn manages_reactions(chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type != ChatType::Group && chat.r#type != ChatType::Channel {
            return false;
        }
        if Self::is_owner(chat) {
            return true;
        }
        let Some(permissions) = Self::loaded_mask(chat, cx) else {
            return false;
        };
        ChatPerm::has(permissions, ChatPerm::MANAGE_GROUP) || ChatPerm::has(permissions, ChatPerm::MANAGE_REACTIONS)
    }

    pub fn manages_forum_structure(chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type != ChatType::Group || !chat.topics_enabled {
            return false;
        }
        Self::has(chat, ChatPerm::MANAGE_TOPICS, cx)
            || Self::has(chat, ChatPerm::MANAGE_SECTIONS, cx)
            || Self::has(chat, ChatPerm::MANAGE_VOICE_CHANNEL, cx)
    }

    pub fn reads_moderation_lists(chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type != ChatType::Group && chat.r#type != ChatType::Channel {
            return false;
        }
        Self::has(chat, ChatPerm::BAN_MEMBERS, cx) || Self::has(chat, ChatPerm::VIEW_AUDIT_LOG, cx)
    }

    pub fn manages_roles(chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type != ChatType::Group && chat.r#type != ChatType::Channel {
            return false;
        }
        Self::has(chat, ChatPerm::MANAGE_ROLES, cx)
    }

    pub fn distributes_invites(chat: &ChatInfo, cx: &mut App) -> bool {
        if chat.r#type != ChatType::Group && chat.r#type != ChatType::Channel {
            return false;
        }
        if Self::is_owner(chat) {
            return true;
        }
        let Some(mask) = Self::loaded_mask(chat, cx).filter(|mask| ChatPerm::has(*mask, ChatPerm::CREATE_INVITES))
        else {
            return false;
        };
        if ChatPerm::has(mask, ChatPerm::MANAGE_GROUP)
            || ChatPerm::has(mask, ChatPerm::ADMINISTRATOR)
            || Self::is_owner_or_admin(chat)
        {
            return true;
        }
        chat.r#type == ChatType::Group
            && ChatServicesStore::shared(cx)
                .read(cx)
                .chat_settings(&chat.id, cx)
                .is_some_and(|settings| settings.members_can_add_members)
    }

    pub fn lists_invite_links(chat: &ChatInfo, cx: &mut App) -> bool {
        if Self::is_owner(chat) {
            return true;
        }
        if Self::loaded_mask(chat, cx).is_some() {
            return Self::distributes_invites(chat, cx);
        }
        chat.can_invite_users
    }

    pub fn shares_place_links(chat: &ChatInfo) -> bool {
        (chat.r#type == ChatType::Group || chat.r#type == ChatType::Channel)
            && ChatServicesStore::is_joined_member_role(chat.role.as_deref())
    }

    pub fn sets_auto_delete(chat: &ChatInfo, cx: &mut App) -> bool {
        match chat.r#type {
            ChatType::Direct => true,
            ChatType::Group | ChatType::Channel => Self::has(chat, ChatPerm::MANAGE_GROUP, cx),
            ChatType::Favourites => Self::can_manage(chat),
        }
    }

    /// a group or channel card whose role isnt a joined one, a public preview or a chat you were
    /// removed from
    /// its cached mask and can_manage_chat are leftovers so they grant nothing
    fn is_outside_group(chat: &ChatInfo) -> bool {
        (chat.r#type == ChatType::Group || chat.r#type == ChatType::Channel)
            && !ChatServicesStore::is_joined_member_role(chat.role.as_deref())
    }

    fn loaded_mask(chat: &ChatInfo, cx: &mut App) -> Option<i64> {
        if chat.id.is_empty() || Self::is_outside_group(chat) {
            return None;
        }
        let store = ChatServicesStore::shared(cx);
        store.update(cx, |store, cx| store.effective_permissions(&chat.id, None, cx)).map(|permissions| permissions.mask())
    }
}
