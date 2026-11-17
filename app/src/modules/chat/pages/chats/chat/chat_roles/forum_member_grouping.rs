use std::collections::HashMap;

use gpui::{App, Hsla};
use rise_canon::RiseGuideAnchor;
use rise_i18n::{tr, tr_plural};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatInfo, ChatMember, ChatPerm, ChatPermExt, ChatRole, ChatType,
};
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;

#[derive(Clone, Debug)]
pub struct MemberRoleGroup {
    pub id: String,
    pub title: String,
    pub color: Option<Hsla>,
    pub members: Vec<ChatMember>,
    pub display_count: Option<i64>,
}

impl MemberRoleGroup {
    pub fn count(&self) -> i64 {
        self.display_count.unwrap_or(self.members.len() as i64)
    }
}

/// what the viewer may do from a member hold menu, read once per list render
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForumMemberGroupingViewerRights {
    pub manage: bool,
    pub restrict: bool,
    pub remove: bool,
}

pub enum ForumMemberGrouping {}

impl ForumMemberGrouping {
    /// unloaded_total is the whole roster size while pages are missing
    /// unloaded members have no role so the plain members group counts them
    /// the owner has no section of their own
    pub fn groups(
        members: &[ChatMember],
        chat_id: &str,
        unloaded_total: Option<i64>,
        cx: &mut App,
    ) -> Vec<MemberRoleGroup> {
        let roles = ChatServicesStore::shared(cx).update(cx, |services, cx| services.roles(chat_id, cx));
        let mut role_by_id: HashMap<&str, &ChatRole> = HashMap::with_capacity(roles.len());
        for role in &roles {
            role_by_id.entry(role.id.as_str()).or_insert(role);
        }

        let mut by_role: HashMap<&str, Vec<ChatMember>> = HashMap::new();
        let mut rest: Vec<ChatMember> = Vec::new();
        for member in members {
            let top = member
                .role_ids
                .iter()
                .filter_map(|role_id| role_by_id.get(role_id.as_str()).copied())
                .filter(|role| role.hoist && !role.is_default)
                .rev()
                .max_by_key(|role| role.position);
            match top {
                Some(top) => by_role.entry(top.id.as_str()).or_default().push(member.clone()),
                None => rest.push(member.clone()),
            }
        }

        let rest_sorted = Self::presence_ordered(&rest, cx);

        let mut out = Vec::new();
        for role in roles.iter().filter(|role| role.hoist && !role.is_default) {
            if let Some(role_members) = by_role.remove(role.id.as_str()).filter(|list| !list.is_empty()) {
                out.push(MemberRoleGroup {
                    id: role.id.clone(),
                    title: role.name.clone(),
                    color: role.swift_color(),
                    members: role_members,
                    display_count: None,
                });
            }
        }
        if !rest_sorted.is_empty() {
            let rest_count = rest_sorted.len() as i64;
            let rest_total =
                unloaded_total.map(|total| rest_count.max(total - (members.len() as i64 - rest_count)));
            out.push(MemberRoleGroup {
                id: "rest".to_owned(),
                title: tr("forum_role_members"),
                color: None,
                members: rest_sorted,
                display_count: rest_total,
            });
        }
        out
    }

    /// live presence order like telegram, online first then most recently seen
    /// ties keep the server order so a roster with no presence doesnt reshuffle
    pub fn presence_ordered(members: &[ChatMember], cx: &mut App) -> Vec<ChatMember> {
        let presence = PresenceServicesStore::shared(cx);
        let presence = presence.read(cx);
        let mut ranked: Vec<(bool, i64, &ChatMember)> = members
            .iter()
            .map(|member| {
                (
                    presence.is_online(&member.user_id, member.is_online),
                    presence.last_seen(&member.user_id, member.last_seen).unwrap_or(0),
                    member,
                )
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        ranked.into_iter().map(|(_, _, member)| member.clone()).collect()
    }

    /// shows n members and m online from the loaded roster and live presence
    pub fn members_line(chat_id: &str, member_count: i64, members_text: Option<String>, cx: &mut App) -> String {
        let members = ChatServicesStore::shared(cx).update(cx, |services, cx| services.members_in_memory(chat_id, cx));
        let presence = PresenceServicesStore::shared(cx);
        let presence = presence.read(cx);
        let online_count =
            members.iter().filter(|member| presence.is_online(&member.user_id, member.is_online)).count();
        let members_text = members_text.unwrap_or_else(|| tr_plural("chat_members_count", member_count));
        if online_count > 0 {
            format!("{members_text}, {online_count} {}", tr("chat_status_online"))
        } else {
            members_text
        }
    }

    pub fn should_group(chat_id: &str, topic_id: Option<&str>, chat: Option<&ChatInfo>, cx: &mut App) -> bool {
        topic_id.is_some()
            || chat.is_some_and(|chat| chat.topics_enabled)
            || !ChatServicesStore::shared(cx)
                .update(cx, |services, cx| services.custom_roles(chat_id, cx))
                .is_empty()
    }

    pub fn my_user_id(cx: &mut App) -> String {
        let auth = AuthServicesStore::shared(cx);
        auth.read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default()
    }

    /// follows the server mask once loaded
    /// before that only the owner and an admin get the moderation items
    pub fn viewer_rights(chat_id: &str, cx: &mut App) -> ForumMemberGroupingViewerRights {
        let Some(chat) = ChatServicesStore::shared(cx).update(cx, |services, cx| services.chat_info(chat_id, cx))
        else {
            return ForumMemberGroupingViewerRights::default();
        };
        ForumMemberGroupingViewerRights {
            manage: ChatProfilePermissions::has(&chat, ChatPerm::MANAGE_MEMBERS, cx)
                || ChatProfilePermissions::has(&chat, ChatPerm::MANAGE_ROLES, cx),
            restrict: chat.r#type != ChatType::Channel
                && ChatProfilePermissions::has(&chat, ChatPerm::MUTE_MEMBERS, cx),
            remove: ChatProfilePermissions::has(&chat, ChatPerm::BAN_MEMBERS, cx)
                || ChatProfilePermissions::has(&chat, ChatPerm::KICK_MEMBERS, cx),
        }
    }

    /// an anonymous admin speaks as the group so an ordinary member sees a plain member
    /// no admin pill or rank or role group
    /// owner and admins see the truth
    pub fn viewer_visible_members(members: Vec<ChatMember>, chat_id: &str, cx: &mut App) -> Vec<ChatMember> {
        if !members.iter().any(|member| member.is_anonymous) || Self::viewer_sees_anonymous_admins(chat_id, cx) {
            return members;
        }
        members
            .into_iter()
            .map(|member| if member.is_anonymous && !member.is_owner() { Self::masked_anonymous(member) } else { member })
            .collect()
    }

    pub fn viewer_visible_admins(admins: Vec<ChatMember>, chat_id: &str, cx: &mut App) -> Vec<ChatMember> {
        if !admins.iter().any(|admin| admin.is_anonymous) || Self::viewer_sees_anonymous_admins(chat_id, cx) {
            return admins;
        }
        admins.into_iter().filter(|admin| !admin.is_anonymous || admin.is_owner()).collect()
    }

    fn viewer_sees_anonymous_admins(chat_id: &str, cx: &mut App) -> bool {
        ChatServicesStore::shared(cx)
            .update(cx, |services, cx| services.chat_info(chat_id, cx))
            .is_some_and(|chat| ChatProfilePermissions::is_owner_or_admin(&chat))
    }

    fn masked_anonymous(member: ChatMember) -> ChatMember {
        ChatMember {
            role: "MEMBER".to_owned(),
            custom_title: String::new(),
            custom_color: String::new(),
            role_ids: Vec::new(),
            display_color: 0,
            ..member
        }
    }

    pub fn member_actions(
        member: &ChatMember,
        chat_id: &str,
        rights: ForumMemberGroupingViewerRights,
        cx: &mut App,
    ) -> Vec<HoldContextMenuAction> {
        let is_self = !member.user_id.is_empty() && member.user_id == Self::my_user_id(cx);
        if is_self {
            if !rights.manage {
                return Vec::new();
            }
            return vec![
                HoldContextMenuAction::new("manage", tr("member_manage_roles"))
                    .system_image("person.badge.shield.checkmark"),
            ];
        }
        let mut items = Vec::new();
        if rights.manage {
            items.push(
                HoldContextMenuAction::new("manage", tr("member_manage_roles"))
                    .system_image("person.badge.shield.checkmark")
                    .guide_anchor(RiseGuideAnchor::MemberMenuManage),
            );
        }
        items.push(HoldContextMenuAction::new("mention", tr("ctx_mention")).system_image("at"));
        items.push(HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"));
        items.push(
            HoldContextMenuAction::new("report", tr("ctx_report"))
                .system_image("exclamationmark.bubble")
                .destructive(true),
        );
        if (rights.restrict || rights.remove) && Self::can_moderate(member, chat_id, cx) {
            if rights.restrict && !member.is_admin() {
                items.push(
                    HoldContextMenuAction::new("restrict", tr("ctx_restrict"))
                        .system_image("nosign")
                        .destructive(true),
                );
            }
            if rights.remove {
                items.push(
                    HoldContextMenuAction::new("remove", tr("ctx_remove_member"))
                        .system_image("person.crop.circle.badge.minus")
                        .destructive(true),
                );
            }
        }
        items
    }

    pub fn can_moderate(member: &ChatMember, chat_id: &str, cx: &mut App) -> bool {
        if member.is_owner() {
            return false;
        }
        let my_user_id = Self::my_user_id(cx);
        let services = ChatServicesStore::shared(cx);
        let Some(me) = services.update(cx, |services, cx| services.member(chat_id, &my_user_id, cx)) else {
            if member.is_admin() {
                let chat = services.update(cx, |services, cx| services.chat_info(chat_id, cx));
                return ChatPerm::is_owner_role(chat.and_then(|chat| chat.role).as_deref());
            }
            return true;
        };
        let roles = services.update(cx, |services, cx| services.roles(chat_id, cx));
        let mine = Self::rank_position(&me.role, &me.role_ids, &roles);
        let theirs = Self::rank_position(&member.role, &member.role_ids, &roles);
        mine > theirs || (mine == 0 && theirs == 0)
    }

    fn rank_position(role: &str, role_ids: &[String], roles: &[ChatRole]) -> i64 {
        if ChatPerm::is_owner_role(Some(role)) {
            return i64::from(i32::MAX);
        }
        let top = roles
            .iter()
            .filter(|candidate| !candidate.is_default && role_ids.contains(&candidate.id))
            .map(|candidate| candidate.position)
            .max()
            .unwrap_or(0);
        if role.to_uppercase() == "ADMIN" {
            return top.max(i64::from(i32::MAX) - 1);
        }
        top
    }
}
