use std::collections::HashMap;

use gpui::{App, Context};
use rise_canon::ChatPerm;
use rise_canon::RiseIdentityCanon;
use rise_engine::rise_store::rise_store_entities::RiseEntityViewLoadState;
use rise_i18n::tr;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftStore;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationPresentation, RiseMessageScopePresentationSnapshot, RiseMessageWireSnapshot,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatMember, ChatPermOverwrite, ChatRole, ChatTopic, OverwriteScope,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

const OPTIMISTIC_TEMPORARY_PREFIX: &str = RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX;

/// fake community people and rights
/// roles and members, plus overrides and channel overwrites the lessons edit
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuideSandboxRolesScene {
    pub roles: Vec<ChatRole>,
    pub panda_role_ids: Vec<String>,
    pub panda_rank: String,
    pub panda_rank_color: String,
    pub panda_is_admin: bool,
    pub member_overrides: HashMap<String, GuideSandboxMemberOverride>,
    pub overwrites: HashMap<String, Vec<ChatPermOverwrite>>,
    pub created_roles: i64,
}

impl GuideSandboxRolesScene {
    pub fn new(roles: Vec<ChatRole>) -> Self {
        Self {
            roles,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GuideSandboxMemberOverride {
    pub allow: i64,
    pub deny: i64,
}

impl GuideSandbox {
    const EVERYONE_ROLE_NAME: &'static str = "@everyone";
    const FRIENDS_ROLE_COLOR: i64 = 0x32ADE6;
    const MODERATORS_ROLE_COLOR: i64 = 0xFF9500;
    const TEAM_TOPIC_ICON: &'static str = "\u{1F4AC}";
    const ADMIN_MEMBER_ROLE: &'static str = "ADMIN";
    const PLAIN_MEMBER_ROLE: &'static str = "MEMBER";
    const FALLBACK_OWNER_ROLE: &'static str = "OWNER";

    /// true if this chat is the community of an armed roles lesson
    /// stores fetching or writing its roles answer from here instead
    pub fn owns_roles(&self, id: Option<&str>) -> bool {
        self.is_armed && self.roles.is_some() && id == Some(&*Self::forum_chat_id())
    }

    pub fn arm_roles(&mut self, cx: &mut Context<Self>) {
        let mentionable = ChatPerm::NEW_ROLE_DEFAULT | ChatPerm::ATTACH_MEDIA | ChatPerm::ADD_REACTIONS;
        self.roles = Some(GuideSandboxRolesScene::new(vec![
            ChatRole {
                id: Self::everyone_role_id().to_owned(),
                name: Self::EVERYONE_ROLE_NAME.to_owned(),
                color: 0,
                position: 0,
                permissions: ChatPerm::NEW_ROLE_DEFAULT
                    | ChatPerm::ADD_REACTIONS
                    | ChatPerm::CONNECT_VOICE
                    | ChatPerm::SPEAK,
                mentionable: false,
                hoist: false,
                is_default: true,
            },
            ChatRole {
                id: Self::friends_role_id().to_owned(),
                name: tr("guide_scene_role_friends"),
                color: Self::FRIENDS_ROLE_COLOR,
                position: 1,
                permissions: mentionable,
                mentionable: true,
                hoist: false,
                is_default: false,
            },
            ChatRole {
                id: Self::moderators_role_id().to_owned(),
                name: tr("guide_scene_role_moderators"),
                color: Self::MODERATORS_ROLE_COLOR,
                position: 2,
                permissions: mentionable
                    | ChatPerm::MANAGE_MESSAGES
                    | ChatPerm::PIN_MESSAGES
                    | ChatPerm::MUTE_MEMBERS,
                mentionable: true,
                hoist: true,
                is_default: false,
            },
        ]));
        cx.notify();
    }

    /// highest first, same order the structure projection sorts real roles
    pub fn forum_roles(&self) -> Vec<ChatRole> {
        let mut roles = self.roles.as_ref().map(|scene| scene.roles.clone()).unwrap_or_default();
        roles.sort_by(|left, right| right.position.cmp(&left.position));
        roles
    }

    pub fn team_topic(&self) -> Option<ChatTopic> {
        self.roles.as_ref()?;
        Some(ChatTopic {
            section_id: Self::forum_section_id().to_owned(),
            updated_at: Self::scene_stamp(),
            channel_kind: ChatChannelKind::Text,
            voice_config: None,
            ..ChatTopic::new(
                Self::team_topic_id().to_owned(),
                tr("guide_scene_team_topic"),
                Self::TEAM_TOPIC_ICON.to_owned(),
                0,
                false,
                false,
            )
        })
    }

    pub fn forum_members(&self, scope: &str, cx: &App) -> Option<Vec<ChatMember>> {
        let scene = self.roles.as_ref().filter(|_| self.is_armed)?;
        let base = ChatServicesStore::member_scope_key(&Self::forum_chat_id(), None);
        if scope != base && !scope.starts_with(&format!("{base}::t::")) {
            return None;
        }
        Some(vec![Self::reader_member(cx), self.panda_member(scene)])
    }

    fn reader_member(cx: &App) -> ChatMember {
        let auth = AuthServicesStore::try_read(cx);
        let profile = auth.and_then(AuthServicesStore::profile);
        ChatMember {
            user_id: auth.and_then(AuthServicesStore::effective_user_id_for_wire).unwrap_or_default(),
            name: profile.map(|profile| profile.name.clone()).unwrap_or_default(),
            tag: profile.map(|profile| profile.tag.clone()).unwrap_or_default(),
            avatar_url: profile.map(|profile| profile.more.logo.clone()).unwrap_or_default(),
            role: ChatPerm::OWNER_ROLES
                .first()
                .map(|role| (*role).to_owned())
                .unwrap_or_else(|| Self::FALLBACK_OWNER_ROLE.to_owned()),
            is_online: true,
            is_premium: profile.is_some_and(|profile| profile.more.is_premium),
            premium_badge_emoji: profile.and_then(|profile| profile.more.premium_badge_emoji.clone()),
            ..ChatMember::default()
        }
    }

    fn panda_member(&self, scene: &GuideSandboxRolesScene) -> ChatMember {
        let display_color = self
            .forum_roles()
            .into_iter()
            .find(|role| scene.panda_role_ids.contains(&role.id) && role.color != 0)
            .map(|role| role.color)
            .unwrap_or(0);
        ChatMember {
            user_id: Self::panda_user_id().to_owned(),
            name: tr("guide_scene_sender"),
            tag: tr("guide_scene_sender_tag"),
            role: if scene.panda_is_admin { Self::ADMIN_MEMBER_ROLE } else { Self::PLAIN_MEMBER_ROLE }.to_owned(),
            is_online: true,
            custom_title: scene.panda_rank.clone(),
            custom_color: scene.panda_rank_color.clone(),
            role_ids: scene.panda_role_ids.clone(),
            display_color,
            ..ChatMember::default()
        }
    }

    pub fn overwrites(&self, scope: OverwriteScope, scope_id: &str) -> Option<Vec<ChatPermOverwrite>> {
        let scene = self.roles.as_ref().filter(|_| self.is_armed)?;
        Some(scene.overwrites.get(&Self::overwrite_key(scope, scope_id)).cloned().unwrap_or_default())
    }

    pub fn member_override(&self, participant_id: &str) -> Option<GuideSandboxMemberOverride> {
        let scene = self.roles.as_ref().filter(|_| self.is_armed)?;
        Some(scene.member_overrides.get(participant_id).copied().unwrap_or_default())
    }

    fn overwrite_key(scope: OverwriteScope, scope_id: &str) -> String {
        let scope = if scope == OverwriteScope::Topic { "topic" } else { "section" };
        format!("{scope}|{scope_id}")
    }

    pub fn create_role(
        &mut self,
        name: String,
        color: i64,
        permissions: i64,
        mentionable: bool,
        hoist: bool,
        cx: &mut Context<Self>,
    ) -> Option<ChatRole> {
        let scene = self.roles.as_mut()?;
        scene.created_roles += 1;
        let role = ChatRole {
            id: format!("{OPTIMISTIC_TEMPORARY_PREFIX}guide_role_{}", scene.created_roles),
            name,
            color,
            position: scene.roles.iter().map(|role| role.position).max().unwrap_or(0) + 1,
            permissions,
            mentionable,
            hoist,
            is_default: false,
        };
        scene.roles.push(role.clone());
        cx.notify();
        Some(role)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_role(
        &mut self,
        id: &str,
        name: Option<String>,
        color: Option<i64>,
        permissions: Option<i64>,
        mentionable: Option<bool>,
        hoist: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let Some(role) = self.roles.as_mut().and_then(|scene| scene.roles.iter_mut().find(|role| role.id == id)) else {
            return;
        };
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            role.name = name;
        }
        if let Some(color) = color {
            role.color = color;
        }
        if let Some(permissions) = permissions {
            role.permissions = permissions;
        }
        if let Some(mentionable) = mentionable {
            role.mentionable = mentionable;
        }
        if let Some(hoist) = hoist {
            role.hoist = hoist;
        }
        cx.notify();
    }

    pub fn delete_role(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(scene) = self.roles.as_mut() else {
            return;
        };
        scene.roles.retain(|role| !(role.id == id && !role.is_default));
        scene.panda_role_ids.retain(|role_id| role_id != id);
        cx.notify();
    }

    pub fn reorder_roles(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let Some(scene) = self.roles.as_mut() else {
            return;
        };
        let count = ids.len() as i64;
        for (index, id) in ids.iter().enumerate() {
            if let Some(role) = scene.roles.iter_mut().find(|role| &role.id == id) {
                role.position = count - index as i64;
            }
        }
        cx.notify();
    }

    pub fn assign_role(&mut self, participant_id: &str, role_id: &str, assign: bool, cx: &mut Context<Self>) {
        let Some(scene) = self.roles.as_mut() else {
            return;
        };
        if participant_id != &*Self::panda_user_id() {
            return;
        }
        scene.panda_role_ids.retain(|id| id != role_id);
        if assign {
            scene.panda_role_ids.push(role_id.to_owned());
        }
        cx.notify();
    }

    pub fn set_overwrite(
        &mut self,
        scope: OverwriteScope,
        scope_id: &str,
        overwrite: ChatPermOverwrite,
        cx: &mut Context<Self>,
    ) {
        let Some(scene) = self.roles.as_mut() else {
            return;
        };
        let list = scene.overwrites.entry(Self::overwrite_key(scope, scope_id)).or_default();
        list.retain(|existing| {
            !(existing.target_type == overwrite.target_type && existing.target_id == overwrite.target_id)
        });
        list.push(overwrite);
        cx.notify();
    }

    pub fn remove_overwrite(
        &mut self,
        scope: OverwriteScope,
        scope_id: &str,
        target_type: &str,
        target_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(scene) = self.roles.as_mut() else {
            return;
        };
        if let Some(list) = scene.overwrites.get_mut(&Self::overwrite_key(scope, scope_id)) {
            list.retain(|existing| !(existing.target_type == target_type && existing.target_id == target_id));
        }
        cx.notify();
    }

    pub fn set_member_override(&mut self, participant_id: &str, allow: i64, deny: i64, cx: &mut Context<Self>) {
        let Some(scene) = self.roles.as_mut() else {
            return;
        };
        scene
            .member_overrides
            .insert(participant_id.to_owned(), GuideSandboxMemberOverride { allow, deny });
        cx.notify();
    }

    pub fn edit_member(
        &mut self,
        participant_id: &str,
        role: Option<&str>,
        rank: Option<String>,
        rank_color: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(scene) = self.roles.as_mut() else {
            return;
        };
        if participant_id != &*Self::panda_user_id() {
            return;
        }
        if let Some(role) = role {
            scene.panda_is_admin = role.to_uppercase() == Self::ADMIN_MEMBER_ROLE;
        }
        if let Some(rank) = rank {
            scene.panda_rank = rank;
        }
        if let Some(rank_color) = rank_color {
            scene.panda_rank_color = rank_color;
        }
        cx.notify();
    }

    pub fn publish_team_channel(&self, cx: &mut App) {
        if !self.is_armed || self.roles.is_none() {
            return;
        }
        RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
            let Some(account_id) = presentation.account_id().filter(|id| !id.is_empty()).map(str::to_owned) else {
                return;
            };
            let scope = RiseMessageScope::new(
                Self::forum_chat_id().to_owned(),
                Some(Self::team_topic_id().to_owned()),
                None,
            );
            let source = RiseMessageWireSnapshot {
                account_id,
                scope,
                revision: 1,
                rows: Vec::new(),
                local_ids: Vec::new(),
                entity_stamps: Vec::new(),
                anchor_local_id: None,
                has_cached_before: false,
                has_cached_after: false,
                before_cursor: None,
                after_cursor: None,
                has_more_before: false,
                has_more_after: false,
                load_state: RiseEntityViewLoadState::Loaded,
                error_code: None,
            };
            presentation.publish_message_scope(
                RiseMessageScopePresentationSnapshot {
                    revision: u64::from(u32::MAX),
                    source,
                    messages: Vec::new(),
                    wires: Vec::new(),
                    visible_local_ids: Vec::new(),
                    content_stamps_by_local_id: HashMap::new(),
                    anchor_offset: 0,
                    pinned_messages: Vec::new(),
                    latest_reply_keyboard_message: None,
                },
                cx,
            );
        });
    }

    pub fn withdraw_team_channel(&self, cx: &mut App) {
        RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.remove_message_scope(
                &RiseMessageScope::new(
                    Self::forum_chat_id().to_owned(),
                    Some(Self::team_topic_id().to_owned()),
                    None,
                ),
                cx,
            );
        });
        ChatDraftStore::shared(cx).update(cx, |drafts, cx| drafts.clear_chat(&Self::forum_chat_id(), cx));
        ChatScrollPositionStore::clear(&Self::forum_chat_id(), cx);
    }
}
