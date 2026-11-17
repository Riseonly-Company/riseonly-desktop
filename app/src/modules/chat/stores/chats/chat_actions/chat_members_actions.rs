use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use gpui::App;
use rise_canon::rise_key::chat as key;
use rise_canon::rise_key::guide_screen::camel as guide_screen_camel_key;
use rise_canon::rise_key::message as message_key;
use rise_canon::rise_request;
use rise_canon::rise_wire_methods;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseJSONValue};
use rise_errors::ErrorCode;
use rise_i18n::tr;
use serde_json::{Map, Value};

use super::chat_actions::{ChatActionsStore, effective_user_id_for_wire};
use super::chat_folders_actions::chat_call_options;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationCursorFallback, SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiWsScreenRefresh, SaiWsScreenRefreshParams};
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::engine::chats::rise_chat_repository_models::RiseChatOptimisticOperation;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::chat::stores::chat_guide_sandbox_hooks::chat_guide_sandbox_hooks;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const MEMBER_ROLE_OWNER: &str = "OWNER";
const MEMBER_ROLE_ADMIN: &str = "ADMIN";
const SCREEN_CHAT_PROFILE: &str = "ChatProfile";
const ROUTE_PARAM_CURRENT_TAB: &str = "currentTab";

fn pagination_key_path(leaf: &str) -> String {
    format!("{}.{leaf}", key::PAGINATION)
}

fn member_list_pagination(relative_id: Option<&str>) -> SaiWsPaginationOptions {
    SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
        relative_id: relative_id.map(str::to_owned),
        response_relative_id_path: pagination_key_path(key::LAST_ID),
        response_relative_id_fallback: Some(SaiWsPaginationCursorFallback::LastItemId),
        has_more_path: Some(pagination_key_path(key::HAS_MORE)),
        duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
        ..SaiWsPaginationRelativeIdArgs::default()
    })
}

fn moderation_list_options(
    method: &str,
    cache_key: String,
    reflect_pending_in_ui: bool,
    path_to_array: &str,
    path_to_id: &str,
    relative_id: Option<&str>,
) -> SaiWsRequestOptions {
    SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: rise_wire_methods::chat::SERVICE.to_owned(),
        method: method.to_owned(),
        cache_key: Some(cache_key),
        reload_if_cached: true,
        allow_concurrent_while_pending: false,
        reflect_pending_in_ui,
        send_immediately: true,
        store_response_in_cache: true,
        persist_cache_to_disk: true,
        disk_cache_policy: Some(SaiWsDiskCachePolicy::Persistent),
        reload_when_only_disk_cached: true,
        preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
        path_to_array: Some(path_to_array.to_owned()),
        path_to_id: Some(path_to_id.to_owned()),
        pagination: Some(member_list_pagination(relative_id)),
        ..SaiWsRequestOptionsInit::default()
    })
}

fn silent_success() -> SaiWsOnSuccess {
    Rc::new(|_payload: Option<Box<dyn Any>>, _cx: &mut App| {})
}

fn log_only_error(label: &'static str) -> SaiWsOnError {
    Rc::new(move |error: &anyhow::Error, cx: &mut App| {
        Sai::log("Chat", || format!("{label} error: {error}"), SaiLogKind::Network, cx);
    })
}

fn notify_result(on_result: Option<&Rc<dyn Fn(bool, &mut App)>>, value: bool, cx: &mut App) {
    if let Some(on_result) = on_result {
        on_result(value, cx);
    }
}

fn set_edit_member_rule(body: &mut rise_request::chat::EditMemberRules, rule: &str, value: bool) -> bool {
    let slot = match rule {
        key::CAN_ACCEPT_JOIN_REQUESTS => &mut body.can_accept_join_requests,
        key::CAN_BAN_MEMBERS => &mut body.can_ban_members,
        key::CAN_CHANGE_INFO => &mut body.can_change_info,
        key::CAN_DELETE_MESSAGES => &mut body.can_delete_messages,
        key::CAN_EDIT_MESSAGES => &mut body.can_edit_messages,
        key::CAN_INVITE_USERS => &mut body.can_invite_users,
        key::CAN_KICK_MEMBERS => &mut body.can_kick_members,
        key::CAN_MANAGE_CHAT => &mut body.can_manage_chat,
        key::CAN_MANAGE_REACTIONS => &mut body.can_manage_reactions,
        key::CAN_MANAGE_STORIES => &mut body.can_manage_stories,
        key::CAN_MANAGE_VIDEO_CHATS => &mut body.can_manage_video_chats,
        key::CAN_PIN_MESSAGES => &mut body.can_pin_messages,
        key::CAN_PROMOTE_MEMBERS => &mut body.can_promote_members,
        key::CAN_REACT => &mut body.can_react,
        key::CAN_RESTRICT_MEMBERS => &mut body.can_restrict_members,
        key::CAN_SEND_AUDIO => &mut body.can_send_audio,
        key::CAN_SEND_DOCUMENTS => &mut body.can_send_documents,
        key::CAN_SEND_MEDIA => &mut body.can_send_media,
        key::CAN_SEND_POLLS => &mut body.can_send_polls,
        key::CAN_SEND_STICKERS => &mut body.can_send_stickers,
        key::CAN_SEND_VOICE => &mut body.can_send_voice,
        key::CAN_SEND_WEB_PAGE_PREVIEWS => &mut body.can_send_web_page_previews,
        _ => return false,
    };
    *slot = Some(value);
    true
}

fn guide_owns_roles(chat_id: &str, cx: &App) -> bool {
    (chat_guide_sandbox_hooks().owns_roles)(chat_id, cx)
}

impl ChatActionsStore {
    #[allow(clippy::too_many_arguments)]
    pub fn get_chat_members_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        relative_id: Option<&str>,
        up: bool,
        reload_if_cached: bool,
        screen_refresh_route_tab_key: Option<&str>,
        cx: &mut App,
    ) {
        if guide_owns_roles(chat_id, cx) {
            return;
        }
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let scope = ChatServicesStore::member_scope_key(chat_id, topic_id);
        let cache_key = ChatServicesStore::members_cache_key(&scope);
        let is_first_page = relative_id.is_none();
        let services = ChatServicesStore::shared(cx);
        services.update(cx, |store, cx| store.register_member_scope(chat_id, &scope, cx));

        let mut body = rise_request::chat::GetChatMembers::new(chat_id.to_owned());
        body.limit = Some(ChatServicesStore::MEMBER_PAGE_LIMIT);
        body.relative_id = relative_id.map(str::to_owned);
        body.topic_id = topic_id.map(str::to_owned);
        body.up = Some(up);

        let show_pending = is_first_page && services.update(cx, |store, cx| store.members(&scope, cx).is_empty());
        let screen_refresh = screen_refresh_route_tab_key.filter(|_| is_first_page).map(|route_tab_key| {
            let route_params = HashMap::from([
                (guide_screen_camel_key::CHAT_ID.to_owned(), chat_id.to_owned()),
                (ROUTE_PARAM_CURRENT_TAB.to_owned(), route_tab_key.to_owned()),
            ]);
            SaiWsScreenRefresh::new(
                true,
                vec![SCREEN_CHAT_PROFILE.to_owned()],
                Some(SaiWsScreenRefreshParams::PerScreen(HashMap::from([(
                    SCREEN_CHAT_PROFILE.to_owned(),
                    route_params,
                )]))),
            )
        });

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_wire_methods::chat::GET_CHAT_MEMBERS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: if is_first_page { reload_if_cached } else { true },
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: show_pending,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::Persistent),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(key::MEMBERS.to_owned()),
            path_to_id: Some(message_key::USER_ID.to_owned()),
            pagination: Some(member_list_pagination(relative_id)),
            screen_refresh,
            ..SaiWsRequestOptionsInit::default()
        });

        if is_first_page {
            tracing::warn!(target: "riseonly::X2107", scope = %scope, reload_if_cached, "member list first page requested");
            services.update(cx, |store, cx| store.begin_member_list_load(&scope, cx));
        }
        let on_success: SaiWsOnSuccess = {
            let scope = scope.clone();
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                let count = ChatServicesStore::shared(cx).update(cx, |store, cx| store.members(&scope, cx).len());
                tracing::warn!(target: "riseonly::X2107", scope = %scope, count, "member list response applied");
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.finish_member_list_load(&scope, cx));
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            tracing::warn!(target: "riseonly::X2107", scope = %scope, error = %error, "member list request failed");
            Sai::log("Chat", || format!("get_chat_members({scope}) error: {error}"), SaiLogKind::Network, cx);
            let refused = CanonErrors::failure(error)
                .is_some_and(|failure| failure.code.as_deref() == Some(ErrorCode::ChatPermissionDenied.wire()));
            ChatServicesStore::shared(cx)
                .update(cx, |store, cx| store.fail_member_list_load(&scope, is_first_page, refused, cx));
        });
        SaiWs::request_slice(&self.get_chat_members_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn ban_member_action(&self, chat_id: &str, participant_id: &str, reason: Option<&str>, cx: &mut App) {
        self.remove_member_action(chat_id, participant_id, reason, true, cx);
    }

    pub fn kick_member_action(&self, chat_id: &str, participant_id: &str, cx: &mut App) {
        self.remove_member_action(chat_id, participant_id, None, false, cx);
    }

    pub fn add_members_action(&self, chat_id: &str, user_ids: Vec<String>, added: i64, cx: &mut App) {
        if chat_id.is_empty() || effective_user_id_for_wire(cx).is_none() {
            return;
        }
        let Some((next, rest)) = user_ids.split_first() else {
            if added > 0 {
                let chat_id = chat_id.to_owned();
                cx.defer(move |cx| {
                    ChatInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.refresh_members(&chat_id, None, cx));
                    Notifier::send(
                        tr("chat_members_added"),
                        NotifierSendOptions {
                            r#type: NotifierType::Success,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                });
            }
            return;
        };
        let next = next.clone();
        let rest = rest.to_vec();

        let on_success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let rest = rest.clone();
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                ChatActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.add_members_action(&chat_id, rest.clone(), added + 1, cx);
                });
            })
        };
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let next = next.clone();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Sai::log("Chat", || format!("add_member({chat_id}/{next}) error: {error}"), SaiLogKind::Network, cx);
                Self::report_member_mutation_failure(error, cx);
                ChatActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.add_members_action(&chat_id, rest.clone(), added, cx);
                });
            })
        };
        SaiWs::request_slice(
            &self.add_member_sai,
            rise_request::chat::AddMember::new(chat_id.to_owned(), next),
            chat_call_options(rise_wire_methods::chat::ADD_MEMBER, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    fn remove_member_action(&self, chat_id: &str, participant_id: &str, reason: Option<&str>, ban: bool, cx: &mut App) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() || participant_id.is_empty() {
            return;
        }
        let services = ChatServicesStore::shared(cx);
        if !services.update(cx, |store, cx| store.begin_member_mutation(chat_id, cx)) {
            return;
        }

        let snapshot = services.update(cx, |store, cx| store.member_mutation_snapshot(chat_id, ban, cx));
        services.update(cx, |store, cx| store.remove_member_local(chat_id, participant_id, reason, ban, cx));

        let method = if ban { rise_wire_methods::chat::BAN_MEMBER } else { rise_wire_methods::chat::KICK_MEMBER };
        let on_success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.finish_member_mutation(&chat_id, cx));
            })
        };
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let participant_id = participant_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Sai::log(
                    "Chat",
                    || format!("{method}({chat_id}/{participant_id}) error: {error}"),
                    SaiLogKind::Network,
                    cx,
                );
                Self::report_member_mutation_failure(error, cx);
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.restore_member_mutation_snapshot(&snapshot, &chat_id, cx);
                    store.finish_member_mutation(&chat_id, cx);
                });
            })
        };

        let options = chat_call_options(method, false, false);
        if ban {
            let mut body = rise_request::chat::BanMember::new(chat_id.to_owned(), participant_id.to_owned());
            body.reason = reason.filter(|reason| !reason.is_empty()).map(str::to_owned);
            SaiWs::request_slice(&self.ban_member_sai, body, options, Some(on_success), Some(on_error), cx);
        } else {
            let body = rise_request::chat::KickMember::new(chat_id.to_owned(), participant_id.to_owned());
            SaiWs::request_slice(&self.kick_member_sai, body, options, Some(on_success), Some(on_error), cx);
        }
    }

    pub fn restrict_member_action(
        &self,
        chat_id: &str,
        participant_id: &str,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut App,
    ) {
        let rules = HashMap::from([
            (key::CAN_SEND_MEDIA.to_owned(), false),
            (key::CAN_SEND_STICKERS.to_owned(), false),
            (key::CAN_SEND_POLLS.to_owned(), false),
            (key::CAN_SEND_WEB_PAGE_PREVIEWS.to_owned(), false),
            (key::CAN_SEND_AUDIO.to_owned(), false),
            (key::CAN_SEND_VOICE.to_owned(), false),
            (key::CAN_SEND_DOCUMENTS.to_owned(), false),
            (key::CAN_REACT.to_owned(), false),
        ]);
        self.edit_member_rules_action(chat_id, participant_id, rules, on_result, cx);
    }

    pub fn edit_member_rules_action(
        &self,
        chat_id: &str,
        participant_id: &str,
        rules: HashMap<String, bool>,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut App,
    ) {
        if guide_owns_roles(chat_id, cx) {
            notify_result(on_result.as_ref(), true, cx);
            return;
        }
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() || participant_id.is_empty() {
            notify_result(on_result.as_ref(), false, cx);
            return;
        }
        let services = ChatServicesStore::shared(cx);
        if !services.update(cx, |store, cx| store.begin_member_mutation(chat_id, cx)) {
            notify_result(on_result.as_ref(), false, cx);
            return;
        }

        let snapshot = services.update(cx, |store, cx| store.member_mutation_snapshot(chat_id, false, cx));
        let fields: Map<String, Value> = rules.iter().map(|(rule, value)| (rule.clone(), Value::Bool(*value))).collect();
        services.update(cx, |store, cx| store.apply_member_fields_local(chat_id, participant_id, fields, cx));

        let mut body = rise_request::chat::EditMemberRules::new(chat_id.to_owned(), participant_id.to_owned());
        for (rule, value) in &rules {
            let applied = set_edit_member_rule(&mut body, rule, *value);
            debug_assert!(applied, "edit_member_rules has no rule {rule}");
        }

        let on_success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let on_result = on_result.clone();
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.finish_member_mutation(&chat_id, cx));
                notify_result(on_result.as_ref(), true, cx);
            })
        };
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let participant_id = participant_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Sai::log(
                    "Chat",
                    || format!("edit_member_rules({chat_id}/{participant_id}) error: {error}"),
                    SaiLogKind::Network,
                    cx,
                );
                Self::report_member_mutation_failure(error, cx);
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.restore_member_mutation_snapshot(&snapshot, &chat_id, cx);
                    store.finish_member_mutation(&chat_id, cx);
                });
                notify_result(on_result.as_ref(), false, cx);
            })
        };
        SaiWs::request_slice(
            &self.edit_member_rules_sai,
            body,
            chat_call_options(rise_wire_methods::chat::EDIT_MEMBER_RULES, false, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn edit_member_action(
        &self,
        chat_id: &str,
        participant_id: &str,
        role: Option<&str>,
        custom_title: Option<&str>,
        custom_color: Option<&str>,
        is_anonymous: Option<bool>,
        granted_rights: HashMap<String, bool>,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut App,
    ) {
        if guide_owns_roles(chat_id, cx) {
            (chat_guide_sandbox_hooks().edit_member)(
                participant_id,
                role,
                custom_title.map(str::to_owned),
                custom_color.map(str::to_owned),
                cx,
            );
            notify_result(on_result.as_ref(), true, cx);
            return;
        }
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() || participant_id.is_empty() {
            notify_result(on_result.as_ref(), false, cx);
            return;
        }
        let services = ChatServicesStore::shared(cx);
        if !services.update(cx, |store, cx| store.begin_member_mutation(chat_id, cx)) {
            notify_result(on_result.as_ref(), false, cx);
            return;
        }

        let snapshot = services.update(cx, |store, cx| store.member_mutation_snapshot(chat_id, false, cx));
        let mut optimistic_fields = Map::new();
        if let Some(role) = role {
            optimistic_fields.insert(key::ROLE.to_owned(), Value::String(role.to_owned()));
        }
        if let Some(custom_title) = custom_title {
            optimistic_fields.insert(key::CUSTOM_TITLE.to_owned(), Value::String(custom_title.to_owned()));
        }
        if let Some(custom_color) = custom_color {
            optimistic_fields.insert(key::CUSTOM_COLOR.to_owned(), Value::String(custom_color.to_owned()));
        }
        if let Some(is_anonymous) = is_anonymous {
            optimistic_fields.insert(key::IS_ANONYMOUS.to_owned(), Value::Bool(is_anonymous));
        }
        for (right, value) in granted_rights {
            optimistic_fields.insert(right, Value::Bool(value));
        }
        services.update(cx, |store, cx| {
            store.apply_member_fields_local(chat_id, participant_id, optimistic_fields, cx);
        });

        let mut body = rise_request::chat::EditMember::new(chat_id.to_owned(), participant_id.to_owned());
        body.role = role.map(str::to_owned);
        body.custom_title = custom_title.map(str::to_owned);
        body.custom_color = custom_color.map(str::to_owned);
        body.is_anonymous = is_anonymous;

        let role_changed = role.is_some();
        let on_success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let on_result = on_result.clone();
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.finish_member_mutation(&chat_id, cx);
                    if role_changed {
                        store.refresh_loaded_member_lists(&chat_id, cx);
                    }
                });
                notify_result(on_result.as_ref(), true, cx);
            })
        };
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let participant_id = participant_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Sai::log(
                    "Chat",
                    || format!("edit_member({chat_id}/{participant_id}) error: {error}"),
                    SaiLogKind::Network,
                    cx,
                );
                Self::report_member_mutation_failure(error, cx);
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.restore_member_mutation_snapshot(&snapshot, &chat_id, cx);
                    store.finish_member_mutation(&chat_id, cx);
                });
                notify_result(on_result.as_ref(), false, cx);
            })
        };
        SaiWs::request_slice(
            &self.edit_member_sai,
            body,
            chat_call_options(rise_wire_methods::chat::EDIT_MEMBER, false, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn transfer_ownership_action(
        &self,
        chat_id: &str,
        new_owner_id: &str,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut App,
    ) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            notify_result(on_result.as_ref(), false, cx);
            return;
        };
        if chat_id.is_empty() || new_owner_id.is_empty() {
            notify_result(on_result.as_ref(), false, cx);
            return;
        }
        let services = ChatServicesStore::shared(cx);
        if !services.update(cx, |store, cx| store.begin_member_mutation(chat_id, cx)) {
            notify_result(on_result.as_ref(), false, cx);
            return;
        }

        let viewer_role = if user_id == new_owner_id { MEMBER_ROLE_OWNER } else { MEMBER_ROLE_ADMIN };
        let snapshot = services.update(cx, |store, cx| store.member_mutation_snapshot(chat_id, false, cx));
        let previous_compatibility_chat = if RiseConversationPresentation::shared(cx).read(cx).is_active() {
            None
        } else {
            services.update(cx, |store, cx| store.chat_info(chat_id, cx))
        };
        if let Some(mut optimistic_chat) = previous_compatibility_chat.clone() {
            optimistic_chat.creator_info = Some(new_owner_id.to_owned());
            optimistic_chat.role = Some(viewer_role.to_owned());
            services.update(cx, |store, cx| {
                if store.contains_archived_chat(chat_id, cx) {
                    store.upsert_archived_chat(&optimistic_chat, false, cx);
                } else {
                    store.upsert_chat(optimistic_chat, cx);
                }
            });
        }
        let owner_fields = Map::from_iter([(key::ROLE.to_owned(), Value::String(MEMBER_ROLE_OWNER.to_owned()))]);
        services.update(cx, |store, cx| store.apply_member_fields_local(chat_id, new_owner_id, owner_fields, cx));
        if user_id != new_owner_id {
            let viewer_fields = Map::from_iter([(key::ROLE.to_owned(), Value::String(MEMBER_ROLE_ADMIN.to_owned()))]);
            services.update(cx, |store, cx| store.apply_member_fields_local(chat_id, &user_id, viewer_fields, cx));
        }

        let domain = RiseConversationDomainRegistry::shared(cx).read(cx).active();
        let slice = Rc::clone(&self.transfer_ownership_sai);
        let chat_id = chat_id.to_owned();
        let new_owner_id = new_owner_id.to_owned();
        cx.spawn(async move |cx| {
            let chat_token = match domain.as_ref() {
                Some(domain) => {
                    let domain = Arc::clone(domain);
                    let chat_id = chat_id.clone();
                    let operation = RiseChatOptimisticOperation::Patch(BTreeMap::from([
                        (key::ROLE.to_owned(), RiseJSONValue::String(viewer_role.to_owned())),
                        (key::CREATOR_INFO.to_owned(), RiseJSONValue::String(new_owner_id.clone())),
                    ]));
                    RiseAccountRuntime::current()
                        .spawn(async move { domain.chats.apply_optimistic(&chat_id, operation).await })
                        .await
                        .ok()
                        .and_then(Result::ok)
                }
                None => None,
            };
            cx.update(|cx| {
                let body = rise_request::chat::TransferOwnership::new(chat_id.clone(), new_owner_id);
                let on_success: SaiWsOnSuccess = {
                    let domain = domain.clone();
                    let chat_token = chat_token.clone();
                    let chat_id = chat_id.clone();
                    let on_result = on_result.clone();
                    Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                        if let (Some(domain), Some(chat_token)) = (domain.clone(), chat_token.clone()) {
                            RiseAccountRuntime::current().spawn(async move {
                                let _ = domain.chats.confirm_optimistic(&chat_token).await;
                            });
                        }
                        ChatServicesStore::shared(cx).update(cx, |store, cx| store.finish_member_mutation(&chat_id, cx));
                        notify_result(on_result.as_ref(), true, cx);
                    })
                };
                let on_error: SaiWsOnError = {
                    let chat_id = chat_id.clone();
                    Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                        Sai::log(
                            "Chat",
                            || format!("transfer_ownership({chat_id}) error: {error}"),
                            SaiLogKind::Network,
                            cx,
                        );
                        Self::report_member_mutation_failure(error, cx);
                        let services = ChatServicesStore::shared(cx);
                        services.update(cx, |store, cx| {
                            store.restore_member_mutation_snapshot(&snapshot, &chat_id, cx);
                            if let Some(previous) = previous_compatibility_chat.clone() {
                                if store.contains_archived_chat(&chat_id, cx) {
                                    store.upsert_archived_chat(&previous, false, cx);
                                } else {
                                    store.upsert_chat(previous, cx);
                                }
                            }
                        });
                        if let (Some(domain), Some(chat_token)) = (domain.clone(), chat_token.clone()) {
                            RiseAccountRuntime::current().spawn(async move {
                                let _ = domain.chats.rollback_optimistic(&chat_token).await;
                            });
                        }
                        services.update(cx, |store, cx| store.finish_member_mutation(&chat_id, cx));
                        notify_result(on_result.as_ref(), false, cx);
                    })
                };
                SaiWs::request_slice(
                    &slice,
                    body,
                    chat_call_options(rise_wire_methods::chat::TRANSFER_OWNERSHIP, false, false),
                    Some(on_success),
                    Some(on_error),
                    cx,
                );
            });
        })
        .detach();
    }

    pub fn get_admin_members_action(&self, chat_id: &str, relative_id: Option<&str>, cx: &mut App) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let reflect_pending = relative_id.is_none()
            && ChatServicesStore::shared(cx).update(cx, |store, cx| store.admins(chat_id, cx).is_empty());
        let mut body = rise_request::chat::GetAdminMembers::new(chat_id.to_owned());
        body.limit = Some(ChatServicesStore::moderation_page_limit());
        body.relative_id = relative_id.map(str::to_owned);
        body.up = Some(false);

        let options = moderation_list_options(
            rise_wire_methods::chat::GET_ADMIN_MEMBERS,
            ChatServicesStore::admins_cache_key(chat_id),
            reflect_pending,
            key::MEMBERS,
            message_key::USER_ID,
            relative_id,
        );
        SaiWs::request_slice(
            &self.admins_sai,
            body,
            options,
            Some(silent_success()),
            Some(log_only_error("get_admin_members")),
            cx,
        );
    }

    pub fn get_blocked_members_action(&self, chat_id: &str, relative_id: Option<&str>, cx: &mut App) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let reflect_pending = relative_id.is_none()
            && ChatServicesStore::shared(cx).update(cx, |store, cx| store.blocked(chat_id, cx).is_empty());
        let mut body = rise_request::chat::GetBlockedMembers::new(chat_id.to_owned());
        body.limit = Some(ChatServicesStore::moderation_page_limit());
        body.relative_id = relative_id.map(str::to_owned);
        body.up = Some(false);

        let options = moderation_list_options(
            rise_wire_methods::chat::GET_BLOCKED_MEMBERS,
            ChatServicesStore::blocked_cache_key(chat_id),
            reflect_pending,
            key::MEMBERS,
            message_key::USER_ID,
            relative_id,
        );
        SaiWs::request_slice(
            &self.blocked_sai,
            body,
            options,
            Some(silent_success()),
            Some(log_only_error("get_blocked_members")),
            cx,
        );
    }

    pub fn get_chat_actions_action(&self, chat_id: &str, relative_id: Option<&str>, cx: &mut App) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let reflect_pending = relative_id.is_none()
            && ChatServicesStore::shared(cx).update(cx, |store, cx| store.chat_actions(chat_id, cx).is_empty());
        let mut body = rise_request::chat::GetChatActionsHistory::new(chat_id.to_owned());
        body.limit = Some(ChatServicesStore::moderation_page_limit());
        body.relative_id = relative_id.map(str::to_owned);
        body.up = Some(false);

        let options = moderation_list_options(
            rise_wire_methods::chat::GET_CHAT_ACTIONS_HISTORY,
            ChatServicesStore::chat_actions_cache_key(chat_id),
            reflect_pending,
            key::ACTIONS,
            key::ACTION_ID,
            relative_id,
        );
        SaiWs::request_slice(
            &self.actions_sai,
            body,
            options,
            Some(silent_success()),
            Some(log_only_error("get_chat_actions_history")),
            cx,
        );
    }

    pub fn unban_member_action(&self, chat_id: &str, participant_id: &str, cx: &mut App) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() || participant_id.is_empty() {
            return;
        }

        let services = ChatServicesStore::shared(cx);
        let Some(removed) = services.update(cx, |store, cx| store.blocked_raw_row(chat_id, participant_id, cx)) else {
            return;
        };
        if !services.update(cx, |store, cx| store.begin_member_mutation(chat_id, cx)) {
            return;
        }
        services.update(cx, |store, cx| store.remove_blocked(chat_id, participant_id, cx));

        let on_success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.finish_member_mutation(&chat_id, cx));
            })
        };
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Sai::log("Chat", || format!("unban_member error: {error}"), SaiLogKind::Network, cx);
                Self::report_member_mutation_failure(error, cx);
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.restore_blocked(removed.clone(), &chat_id, cx);
                    store.finish_member_mutation(&chat_id, cx);
                });
            })
        };
        SaiWs::request_slice(
            &self.unban_member_sai,
            rise_request::chat::UnbanMember::new(chat_id.to_owned(), participant_id.to_owned()),
            chat_call_options(rise_wire_methods::chat::UNBAN_MEMBER, false, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn report_member_mutation_failure(error: &anyhow::Error, cx: &mut App) {
        if error.is::<CancellationError>() {
            return;
        }
        let text = if CanonErrors::is_connection_loss(error) {
            tr("chat_delete_failed_text")
        } else {
            CanonErrors::refusal_text(error)
        };
        Notifier::send(
            text,
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }
}
