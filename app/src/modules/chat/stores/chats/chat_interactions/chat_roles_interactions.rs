use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, Global};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;

use super::chat_interactions::ChatInteractionsStore;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatVoiceConnectGateDecision, ChatVoiceConnectGatePolicy};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{EffectivePermissionsDTO, EffectivePermissionsDTOExt};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

#[derive(Default)]
struct MentionedRoleFetchLedger {
    requested: HashSet<String>,
}

impl Global for MentionedRoleFetchLedger {}

impl ChatInteractionsStore {
    pub fn load_roles(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_forum_structure_action(chat_id, false, None, None, cx));
        self.load_effective_permissions(chat_id, None, false, cx);
    }

    pub fn resolve_mentioned_role(&mut self, chat_id: &str, role_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() || role_id.is_empty() {
            return;
        }
        let Some(uid) = AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|uid| !uid.is_empty())
        else {
            return;
        };
        let inserted = cx
            .default_global::<MentionedRoleFetchLedger>()
            .requested
            .insert(format!("{uid}\u{1F}{chat_id}\u{1F}{role_id}"));
        if !inserted {
            return;
        }
        let chat_id = chat_id.to_owned();
        cx.defer(move |cx| {
            ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_forum_structure_action(&chat_id, true, None, None, cx));
        });
    }

    pub fn load_effective_permissions(&mut self, chat_id: &str, topic_id: Option<&str>, force: bool, cx: &mut Context<Self>) {
        self.load_effective_permissions_on_result(chat_id, topic_id, force, None, cx);
    }

    pub fn load_effective_permissions_on_result(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        force: bool,
        on_result: Option<Rc<dyn Fn(Option<EffectivePermissionsDTO>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_forum_chat_id(Some(chat_id))) {
            if let Some(on_result) = on_result {
                let permissions = GuideSandbox::shared(cx).read(cx).forum_permissions();
                cx.defer(move |cx| on_result(Some(permissions), cx));
            }
            return;
        }
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.load_effective_permissions(chat_id, topic_id, force, on_result, cx);
        });
    }

    pub fn resolve_voice_connect_permission(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        on_result: Box<dyn FnOnce(ChatVoiceConnectGateDecision, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let normalized_topic_id = topic_id.map(trimming_whitespaces_and_newlines);
        let cached_permissions = ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.effective_permissions(chat_id, normalized_topic_id, cx))
            .map(|permissions| permissions.mask());

        match ChatVoiceConnectGatePolicy::decision(cached_permissions) {
            ChatVoiceConnectGateDecision::Allowed => {
                cx.defer(move |cx| on_result(ChatVoiceConnectGateDecision::Allowed, cx));
            }
            ChatVoiceConnectGateDecision::Denied => {
                cx.defer(move |cx| on_result(ChatVoiceConnectGateDecision::Denied, cx));
            }
            ChatVoiceConnectGateDecision::RefreshRequired => {
                let on_result = RefCell::new(Some(on_result));
                self.load_effective_permissions_on_result(
                    chat_id,
                    normalized_topic_id,
                    false,
                    Some(Rc::new(move |permissions: Option<EffectivePermissionsDTO>, cx: &mut App| {
                        let Some(on_result) = on_result.borrow_mut().take() else {
                            return;
                        };
                        match permissions {
                            Some(value) => on_result(ChatVoiceConnectGatePolicy::decision(Some(value.mask())), cx),
                            None => on_result(ChatVoiceConnectGateDecision::RefreshRequired, cx),
                        }
                    })),
                    cx,
                );
            }
        }
    }

    pub fn create_role(
        &mut self,
        chat_id: &str,
        name: &str,
        color: i64,
        permissions: i64,
        mentionable: bool,
        hoist: bool,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.create_role_action(chat_id, name.to_owned(), color, permissions, mentionable, hoist, on_result, cx);
        });
    }

    pub fn update_role(
        &mut self,
        chat_id: &str,
        role_id: &str,
        name: &str,
        color: i64,
        permissions: i64,
        mentionable: bool,
        hoist: bool,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_role_action(
                chat_id,
                role_id,
                Some(name.to_owned()),
                Some(color),
                Some(permissions),
                Some(mentionable),
                Some(hoist),
                on_result,
                cx,
            );
        });
    }

    pub fn delete_role(&mut self, chat_id: &str, role_id: &str, cx: &mut Context<Self>) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.delete_role_action(chat_id, role_id, cx));
    }

    pub fn assign_role(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        role_id: &str,
        assign: bool,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() || participant_id.is_empty() || role_id.is_empty() {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(false, cx));
            }
            Notifier::send(
                tr("save_failed"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
            return;
        }
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.assign_role_action(chat_id, participant_id, role_id, assign, on_result, cx);
        });
    }
}
