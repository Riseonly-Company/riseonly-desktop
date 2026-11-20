use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::ChatPerm;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_ui::native_confirm_alert::NativeConfirmAlert;

use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::chat::engine::chat_engine_hooks::ChatEngineRemovedChatName;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_roles_actions::ChatRolesResultCallback;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatMember, ChatType};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::{
    ChatInteractionsBoolResult, ChatInteractionsStore,
};
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::chat::stores::chats::chat_services::chat_services::{ChatRemovedFromChatNotice, ChatServicesStore};
use crate::modules::post::stores::post_actions::post_types::PostCollaboratorUser;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatRemovedChatName {
    pub title: String,
    pub is_channel: bool,
    pub announces: bool,
}

impl ChatRemovedChatName {
    pub fn new(title: String, is_channel: bool) -> Self {
        Self { title, is_channel, announces: true }
    }
}

impl From<ChatEngineRemovedChatName> for ChatRemovedChatName {
    fn from(name: ChatEngineRemovedChatName) -> Self {
        Self { title: name.title, is_channel: name.is_channel, announces: name.announces }
    }
}

fn send_bottom_notice(title: String, r#type: NotifierType, cx: &mut App) {
    Notifier::send(
        title,
        NotifierSendOptions { r#type, position: NotifierPosition::Bottom, ..NotifierSendOptions::default() },
        cx,
    );
}

fn send_removal_notice(title: String, cx: &mut App) {
    Notifier::send(
        title,
        NotifierSendOptions { r#type: NotifierType::Info, is_notification: true, ..NotifierSendOptions::default() },
        cx,
    );
}

impl ChatInteractionsStore {
    pub fn load_member_restriction_permissions(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        on_result: Box<dyn FnOnce(Option<i64>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let on_result = RefCell::new(Some(on_result));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_member_override_action(
                chat_id,
                participant_id,
                Some(Rc::new(move |permissions: Option<i64>, cx: &mut App| {
                    let on_result = on_result.borrow_mut().take();
                    if let Some(on_result) = on_result {
                        cx.defer(move |cx| on_result(permissions, cx));
                    }
                })),
                None,
                Rc::new(|_: i64, _: i64, _: &mut App| {}),
                cx,
            );
        });
    }

    pub fn load_member_override(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        on_result: Box<dyn FnOnce(i64, i64, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let on_result = RefCell::new(Some(on_result));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_member_override_action(
                chat_id,
                participant_id,
                None,
                None,
                Rc::new(move |allow: i64, deny: i64, cx: &mut App| {
                    let on_result = on_result.borrow_mut().take();
                    if let Some(on_result) = on_result {
                        cx.defer(move |cx| on_result(allow, deny, cx));
                    }
                }),
                cx,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_member_permissions(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        touched: i64,
        allow: i64,
        deny: i64,
        on_merged: Box<dyn FnOnce(i64, i64, &mut App)>,
        on_result: ChatInteractionsBoolResult,
        cx: &mut Context<Self>,
    ) {
        let answered = Rc::new(Cell::new(false));
        let on_result = Rc::new(RefCell::new(Some(on_result)));
        let on_merged = RefCell::new(Some(on_merged));
        let read_answered = answered.clone();
        let read_on_result = on_result.clone();
        let write_chat_id = chat_id.to_owned();
        let write_participant_id = participant_id.to_owned();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_member_override_action(
                chat_id,
                participant_id,
                Some(Rc::new(|_: Option<i64>, _: &mut App| {})),
                Some(Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                    if read_answered.replace(true) {
                        return;
                    }
                    ChatActionsStore::report_member_mutation_failure(error, cx);
                    let on_result = read_on_result.borrow_mut().take();
                    if let Some(on_result) = on_result {
                        on_result(false, cx);
                    }
                })),
                Rc::new(move |fresh_allow: i64, fresh_deny: i64, cx: &mut App| {
                    if answered.replace(true) {
                        return;
                    }
                    let merged_allow = (fresh_allow & !touched) | (allow & touched);
                    let merged_deny = (fresh_deny & !touched) | (deny & touched);
                    let on_result = on_result.borrow_mut().take().map(Self::once_result);
                    let on_merged = on_merged.borrow_mut().take();
                    let write_chat_id = write_chat_id.clone();
                    let write_participant_id = write_participant_id.clone();
                    cx.defer(move |cx| {
                        if let Some(on_merged) = on_merged {
                            on_merged(merged_allow, merged_deny, cx);
                        }
                        Self::actions(cx).update(cx, |actions, cx| {
                            actions.set_member_permissions_action(
                                &write_chat_id,
                                &write_participant_id,
                                merged_allow,
                                merged_deny,
                                on_result,
                                cx,
                            );
                        });
                    });
                }),
                cx,
            );
        });
    }

    pub(crate) fn once_result(on_result: ChatInteractionsBoolResult) -> ChatRolesResultCallback {
        let on_result = RefCell::new(Some(on_result));
        Rc::new(move |ok: bool, cx: &mut App| {
            let on_result = on_result.borrow_mut().take();
            if let Some(on_result) = on_result {
                on_result(ok, cx);
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn edit_member(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        role: Option<&str>,
        custom_title: Option<&str>,
        custom_color: Option<&str>,
        is_anonymous: Option<bool>,
        granted_rights: HashMap<String, bool>,
        on_result: Option<ChatInteractionsBoolResult>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.edit_member_action(
                chat_id,
                participant_id,
                role,
                custom_title,
                custom_color,
                is_anonymous,
                granted_rights,
                on_result.map(Self::once_result),
                cx,
            );
        });
    }

    pub fn edit_member_rules(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        rules: HashMap<String, bool>,
        on_result: Option<ChatInteractionsBoolResult>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx)
            .update(cx, |actions, cx| actions.edit_member_rules_action(chat_id, participant_id, rules, on_result.map(Self::once_result), cx));
    }

    pub fn ban_member(&mut self, chat_id: &str, participant_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.ban_member_action(chat_id, participant_id, None, cx));
    }

    pub fn add_members(&mut self, chat_id: &str, users: &[PostCollaboratorUser], cx: &mut Context<Self>) {
        let user_ids: Vec<String> = users.iter().map(|user| user.id.clone()).collect();
        Self::actions(cx).update(cx, |actions, cx| actions.add_members_action(chat_id, user_ids, 0, cx));
    }

    pub fn remove_member(&mut self, chat_id: &str, participant_id: &str, cx: &mut Context<Self>) {
        let chat = Self::service(cx).read(cx).chat_info(chat_id, cx);
        let kicks_only = chat.is_some_and(|chat| {
            !ChatProfilePermissions::has(&chat, ChatPerm::BAN_MEMBERS, cx)
                && ChatProfilePermissions::has(&chat, ChatPerm::KICK_MEMBERS, cx)
        });
        if kicks_only {
            Self::actions(cx).update(cx, |actions, cx| actions.kick_member_action(chat_id, participant_id, cx));
        } else {
            Self::actions(cx).update(cx, |actions, cx| actions.ban_member_action(chat_id, participant_id, None, cx));
        }
    }

    pub fn confirm_remove_member(&mut self, chat_id: &str, member: &ChatMember, cx: &mut Context<Self>) {
        if chat_id.is_empty() || member.user_id.is_empty() {
            return;
        }
        let chat = Self::service(cx).read(cx).chat_info(chat_id, cx);
        let name = trimming_whitespaces_and_newlines(&member.name);
        let title = if name.is_empty() { format!("@{}", member.tag) } else { name.to_owned() };
        let message =
            chat.as_ref().map(|chat| trimming_whitespaces_and_newlines(&chat.title).to_owned()).unwrap_or_default();
        let is_channel = chat.as_ref().is_some_and(|chat| chat.r#type == ChatType::Channel);
        let confirm_title = if is_channel { tr("chat_member_remove_channel") } else { tr("chat_member_remove") };
        let owned_chat_id = chat_id.to_owned();
        let participant_id = member.user_id.clone();
        NativeConfirmAlert::present_delete(
            &title,
            &message,
            Some(confirm_title),
            None,
            true,
            None,
            None,
            move |cx: &mut App| {
                Self::shared(cx).update(cx, |store, cx| store.remove_member(&owned_chat_id, &participant_id, cx));
            },
            cx,
        );
    }

    pub fn announce_removed_from_chat(
        &mut self,
        chat_id: &str,
        stored: Option<ChatRemovedChatName>,
        cx: &mut Context<Self>,
    ) {
        let service = Self::service(cx);
        service.update(cx, |service, cx| {
            service.removed_from_chat_notice = Some(ChatRemovedFromChatNotice::new(chat_id.to_owned()));
            cx.notify();
        });
        let known_removal = stored.as_ref().is_none_or(|stored| stored.announces);
        let live = service
            .read(cx)
            .chat_info(chat_id, cx)
            .map(|chat| ChatRemovedChatName::new(chat.title, chat.r#type == ChatType::Channel));
        let has_title = |name: &&ChatRemovedChatName| !trimming_whitespaces_and_newlines(&name.title).is_empty();
        let named = live.as_ref().filter(has_title).or_else(|| stored.as_ref().filter(has_title));
        let Some(named) = named else {
            let is_channel = live.as_ref().or(stored.as_ref()).is_some_and(|name| name.is_channel);
            let title = if is_channel {
                tr("chat_no_longer_member_unnamed_channel_toast")
            } else {
                tr("chat_no_longer_member_unnamed_group_toast")
            };
            send_removal_notice(title, cx);
            return;
        };
        let name = trimming_whitespaces_and_newlines(&named.title);
        let template = match (known_removal, named.is_channel) {
            (true, true) => tr("chat_removed_you_channel_toast"),
            (true, false) => tr("chat_removed_you_group_toast"),
            (false, true) => tr("chat_no_longer_member_channel_toast"),
            (false, false) => tr("chat_no_longer_member_group_toast"),
        };
        send_removal_notice(template.replace("{name}", name), cx);
    }

    pub fn restrict_member(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        display_name: &str,
        cx: &mut Context<Self>,
    ) {
        let display_name = display_name.to_owned();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.restrict_member_action(
                chat_id,
                participant_id,
                Some(Rc::new(move |ok: bool, cx: &mut App| {
                    if ok {
                        Self::notify_member_restricted_name(&display_name, cx);
                    }
                })),
                cx,
            );
        });
    }

    pub fn notify_member_restricted(&mut self, member: &ChatMember, cx: &mut Context<Self>) {
        let name = if member.name.is_empty() { &member.tag } else { &member.name };
        Self::notify_member_restricted_name(name, cx);
    }

    fn notify_member_restricted_name(name: &str, cx: &mut App) {
        send_bottom_notice(tr("chat_member_restricted_toast").replace("{name}", name), NotifierType::Success, cx);
    }

    pub fn load_members(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        refresh_route_tab_key: Option<&str>,
        force_refresh: bool,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() {
            return;
        }

        let scope = ChatServicesStore::member_scope_key(chat_id, topic_id);
        let service = Self::service(cx);
        let mut force_refresh = force_refresh;
        if service.read(cx).is_member_list_loading(&scope) {
            let cache_key = ChatServicesStore::members_cache_key(&scope);
            let abandoned = service.read(cx).is_member_list_load_abandoned(&scope);
            if !abandoned
                && SaiWsPendingRegistry::shared(cx).read(cx).has_in_flight_request(Some(cache_key.as_str()))
            {
                tracing::warn!(target: "riseonly::X2107", scope = %scope, "member list open skipped: still loading");
                return;
            }
            tracing::warn!(target: "riseonly::X2306", scope = %scope, abandoned, "member list loading state stale, asking again");
            force_refresh = true;
        }

        if service.read(cx).chat_info(chat_id, cx).is_some_and(|chat| chat.is_community_preview()) {
            return;
        }

        let force_refresh = force_refresh || Self::cached_roster_disagrees(chat_id, topic_id, &scope, cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_chat_members_action(chat_id, topic_id, None, false, force_refresh, refresh_route_tab_key, cx);
        });
    }

    fn cached_roster_disagrees(chat_id: &str, topic_id: Option<&str>, scope: &str, cx: &mut App) -> bool {
        if topic_id.is_some() {
            return false;
        }
        let service = Self::service(cx);
        let Some(expected) = service.read(cx).chat_info(chat_id, cx).map(|chat| chat.member_count).filter(|count| *count > 0)
        else {
            return false;
        };
        let (loaded, has_more) =
            service.update(cx, |service, cx| (service.members(scope, cx).len(), service.members_has_more(scope, cx)));
        let disagrees = loaded > 0 && !has_more && loaded as i64 != expected;
        if disagrees {
            tracing::warn!(target: "riseonly::X2306", scope = %scope, loaded, expected, "cached roster disagrees with headcount, refetching");
        }
        disagrees
    }

    pub fn refresh_members(&mut self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }

        let service = Self::service(cx);
        if service.read(cx).chat_info(chat_id, cx).is_some_and(|chat| chat.is_community_preview()) {
            return;
        }

        let scope = ChatServicesStore::member_scope_key(chat_id, topic_id);
        service.update(cx, |service, cx| service.clear_members(&scope, cx));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_chat_members_action(chat_id, topic_id, None, false, true, None, cx);
        });
    }

    pub fn load_more_members(&mut self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }

        let scope = ChatServicesStore::member_scope_key(chat_id, topic_id);
        let service = Self::service(cx);

        if !service.update(cx, |service, cx| service.members_has_more(&scope, cx)) {
            return;
        }

        let Some(last_id) = service.update(cx, |service, cx| service.members_last_id(&scope, cx)).filter(|last_id| !last_id.is_empty()) else {
            return;
        };

        let cache_key = ChatServicesStore::members_cache_key(&scope);
        if !SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().can_request_automatically() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_chat_members_action(chat_id, topic_id, Some(&last_id), false, true, None, cx);
        });
    }

    pub fn load_admins(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_admin_members_action(chat_id, None, cx));
    }

    pub fn load_more_admins(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if !service.update(cx, |service, cx| service.admins_has_more(chat_id, cx)) {
            return;
        }
        let Some(last) = service.update(cx, |service, cx| service.admins_last_id(chat_id, cx)) else {
            return;
        };
        let cache_key = ChatServicesStore::admins_cache_key(chat_id);
        if !SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().can_request_automatically() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.get_admin_members_action(chat_id, Some(&last), cx));
    }

    pub fn load_blocked(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_blocked_members_action(chat_id, None, cx));
    }

    pub fn load_more_blocked(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if !service.update(cx, |service, cx| service.blocked_has_more(chat_id, cx)) {
            return;
        }
        let Some(last) = service.update(cx, |service, cx| service.blocked_last_id(chat_id, cx)) else {
            return;
        };
        let cache_key = ChatServicesStore::blocked_cache_key(chat_id);
        if !SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().can_request_automatically() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.get_blocked_members_action(chat_id, Some(&last), cx));
    }

    pub fn unban(&mut self, chat_id: &str, participant_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.unban_member_action(chat_id, participant_id, cx));
    }

    pub fn load_chat_actions(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_chat_actions_action(chat_id, None, cx));
    }

    pub fn load_more_chat_actions(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if !service.update(cx, |service, cx| service.chat_actions_has_more(chat_id, cx)) {
            return;
        }
        let Some(last) = service.update(cx, |service, cx| service.chat_actions_last_id(chat_id, cx)) else {
            return;
        };
        let cache_key = ChatServicesStore::chat_actions_cache_key(chat_id);
        if !SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().can_request_automatically() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.get_chat_actions_action(chat_id, Some(&last), cx));
    }
}
