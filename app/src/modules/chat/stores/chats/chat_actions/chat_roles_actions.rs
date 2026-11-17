use std::any::Any;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::RiseGuideSignal;
use rise_canon::rise_key::chat as chat_key;
use rise_canon::rise_request;
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::chat_actions::ChatActionsStore;
use super::chat_types::{ChatPermOverwrite, ChatRole, OverwriteScope};
use super::chat_wire_types::{
    EffectivePermissionsDTO, GetOverwritesGatewayDTO, OverwriteWireDTOExt, RoleResultDTO, RoleWireDTOExt,
    SectionResultDTO, SectionWireDTOExt,
};
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::sai::sai::Sai;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiWsScreenRefresh;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::chat_engine_hooks::ChatEngineMainCallback;
use crate::modules::chat::stores::chat_guide_sandbox_hooks::{ChatGuideSandboxRolePatch, chat_guide_sandbox_hooks};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const TEMPORARY_ROLE_ID_PREFIX: &str = "temp-role-";

pub type ChatRolesResultCallback = Rc<dyn Fn(bool, &mut App)>;

pub(super) fn wire_user_id(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().filter(|user_id| !user_id.is_empty())
}

pub(super) fn chat_services<R>(
    cx: &mut App,
    update: impl FnOnce(&mut ChatServicesStore, &mut Context<ChatServicesStore>) -> R,
) -> R {
    ChatServicesStore::shared(cx).update(cx, update)
}

pub(super) fn fire_result(on_result: &Option<ChatRolesResultCallback>, succeeded: bool, cx: &mut App) {
    if let Some(on_result) = on_result {
        on_result(succeeded, cx);
    }
}

pub(super) fn typed_payload<T: DeserializeOwned + 'static>(payload: Option<Box<dyn Any>>) -> Option<T> {
    match payload?.downcast::<T>() {
        Ok(decoded) => Some(*decoded),
        Err(payload) => SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>()),
    }
}

fn guide_owns_roles(chat_id: &str, cx: &App) -> bool {
    (chat_guide_sandbox_hooks().owns_roles)(chat_id, cx)
}

fn guide_owns_forum(chat_id: &str, cx: &App) -> bool {
    (chat_guide_sandbox_hooks().owns_forum_chat_id)(chat_id, cx)
}

fn fire_main(callback: &Option<ChatEngineMainCallback>, cx: &mut App) {
    if let Some(callback) = callback {
        callback(cx);
    }
}

fn log_roles(message: String, cx: &mut App) {
    Sai::log("Roles", move || message, SaiLogKind::Network, cx);
}

fn reload_forum_structure(chat_id: String, cx: &mut App) {
    cx.defer(move |cx| {
        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.get_forum_structure_action(&chat_id, true, None, None, cx));
    });
}

fn role_still_listed(chat_id: &str, role_id: &str, cx: &mut App) -> bool {
    chat_services(cx, |services, cx| services.roles(chat_id, cx)).iter().any(|role| role.id == role_id)
}

impl ChatActionsStore {
    pub fn roles_read_opts(&self, method: &str, key: String, has_cache: bool) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(key),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: !has_cache,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::Persistent),
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            screen_refresh: Some(SaiWsScreenRefresh::new(
                true,
                ["ForumHome", "Chat", "ChatProfile", "ChatRoles", "ChatSettingsMembers", "ChatTopics", "Chats"]
                    .map(str::to_owned)
                    .to_vec(),
                None,
            )),
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn roles_has_cached_payload(&self, key: &str, cx: &mut App) -> bool {
        SaiWsResponseCache::memory_only(key, cx).is_some() || SaiWsResponseCache::disk_only(key, cx).is_some()
    }

    pub fn roles_write_opts(&self, method: &str) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: method.to_owned(),
            reflect_pending_in_ui: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    pub fn get_forum_structure_action(
        &mut self,
        chat_id: &str,
        force: bool,
        on_loaded: Option<ChatEngineMainCallback>,
        on_failed: Option<ChatEngineMainCallback>,
        cx: &mut Context<Self>,
    ) {
        if guide_owns_forum(chat_id, cx) {
            fire_main(&on_failed, cx);
            return;
        }
        if wire_user_id(cx).is_none() || chat_id.is_empty() {
            fire_main(&on_failed, cx);
            return;
        }

        let key = ChatServicesStore::forum_structure_cache_key(chat_id);
        let has_cache = self.roles_has_cached_payload(&key, cx);
        let mut options = self.roles_read_opts(rise_wire_methods::chat::GET_FORUM_STRUCTURE, key, has_cache);
        if force {
            options.reload_if_cached = true;
            options.force_bypass_cache_read = true;
        }
        let topics_before: HashSet<String> = if force {
            chat_services(cx, |services, cx| services.in_memory_forum_listed_topic_ids(chat_id, cx))
        } else {
            HashSet::new()
        };

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
                let chat_id = chat_id.clone();
                let topics_before = topics_before.clone();
                let on_loaded = on_loaded.clone();
                cx.defer(move |cx| {
                    chat_services(cx, |services, cx| services.reapply_failed_topic_previews(&chat_id, cx));
                    let topics = chat_services(cx, |services, cx| services.in_memory_forum_topics(&chat_id, cx));
                    ChatActionsStore::shared(cx)
                        .update(cx, |actions, cx| actions.adopt_forum_voice_state_action(&chat_id, &topics, cx));
                    let topics_after: HashSet<String> =
                        chat_services(cx, |services, cx| services.in_memory_forum_listed_topic_ids(&chat_id, cx));
                    chat_services(cx, |services, cx| {
                        services.note_lost_forum_topics(&chat_id, &topics_before, &topics_after, cx)
                    });
                    fire_main(&on_loaded, cx);
                });
            })
        };
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_roles(format!("get_forum_structure error: {error}"), cx);
            fire_main(&on_failed, cx);
        });

        SaiWs::request_slice(
            &self.forum_structure_sai,
            rise_request::chat::GetForumStructure::new(chat_id.to_owned()),
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn load_effective_permissions(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        force: bool,
        on_result: Option<Rc<dyn Fn(Option<EffectivePermissionsDTO>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if wire_user_id(cx).is_none() || chat_id.is_empty() {
            if let Some(on_result) = &on_result {
                on_result(None, cx);
            }
            return;
        }

        let resolved_topic_id = topic_id.map(|topic_id| trimming_whitespaces_and_newlines(topic_id).to_owned());
        let key = ChatServicesStore::effective_perms_cache_key(chat_id, resolved_topic_id.as_deref());
        let has_cache = self.roles_has_cached_payload(&key, cx);
        let mut options = self.roles_read_opts(rise_wire_methods::chat::GET_EFFECTIVE_PERMISSIONS, key, has_cache);
        if force {
            options.reload_if_cached = true;
            options.force_bypass_cache_read = true;
        }

        let mut body = rise_request::chat::GetEffectivePermissions::new(chat_id.to_owned());
        if let Some(resolved_topic_id) = &resolved_topic_id
            && !resolved_topic_id.is_empty()
        {
            body.topic_id = Some(resolved_topic_id.clone());
        }

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let on_result = on_result.clone();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let permissions = typed_payload::<EffectivePermissionsDTO>(payload).or_else(|| {
                    chat_services(cx, |services, cx| {
                        services.effective_permissions(&chat_id, resolved_topic_id.as_deref(), cx)
                    })
                });
                chat_services(cx, |services, cx| services.reconcile_slow_mode(&chat_id, cx));
                if let Some(on_result) = &on_result {
                    on_result(permissions, cx);
                }
            })
        };
        let failure: SaiWsOnError = Rc::new(move |_: &anyhow::Error, cx: &mut App| {
            if let Some(on_result) = &on_result {
                on_result(None, cx);
            }
        });

        SaiWs::request_slice(&self.eff_perms_sai, body, options, Some(success), Some(failure), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_role_action(
        &mut self,
        chat_id: &str,
        name: String,
        color: i64,
        permissions: i64,
        mentionable: bool,
        hoist: bool,
        on_result: Option<ChatRolesResultCallback>,
        cx: &mut Context<Self>,
    ) {
        if guide_owns_roles(chat_id, cx) {
            let created = (chat_guide_sandbox_hooks().create_role)(name, color, permissions, mentionable, hoist, cx);
            fire_result(&on_result, created.is_some(), cx);
            return;
        }
        if wire_user_id(cx).is_none() {
            fire_result(&on_result, false, cx);
            return;
        }
        if !chat_services(cx, |services, cx| services.begin_role_mutation(chat_id, cx)) {
            fire_result(&on_result, false, cx);
            return;
        }
        let snapshot = Rc::new(chat_services(cx, |services, cx| services.forum_structure_mutation_snapshot(chat_id, cx)));
        let had_forum_cache = chat_services(cx, |services, cx| services.has_forum_cache(chat_id, cx));
        let temporary_role_id = format!("{TEMPORARY_ROLE_ID_PREFIX}{}", Uuid::new_v4().uuid_string().to_lowercase());
        let position = chat_services(cx, |services, cx| services.roles(chat_id, cx))
            .iter()
            .map(|role| role.position)
            .max()
            .unwrap_or(0)
            + 1;
        if had_forum_cache {
            let optimistic = ChatRole {
                id: temporary_role_id.clone(),
                name: name.clone(),
                color,
                position,
                permissions,
                mentionable,
                hoist,
                is_default: false,
            };
            chat_services(cx, |services, cx| services.upsert_role_cache(optimistic, chat_id, cx));
        }

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let snapshot = Rc::clone(&snapshot);
            let on_result = on_result.clone();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                if had_forum_cache {
                    let Some(wire) = typed_payload::<RoleResultDTO>(payload).and_then(|result| result.role) else {
                        chat_services(cx, |services, cx| {
                            services.restore_forum_structure_mutation(&snapshot, &chat_id, cx);
                            services.finish_role_mutation(&chat_id, cx);
                        });
                        fire_result(&on_result, false, cx);
                        return;
                    };
                    chat_services(cx, |services, cx| {
                        services.remove_role_cache(&temporary_role_id, &chat_id, cx);
                        services.upsert_role_cache(wire.to_role(), &chat_id, cx);
                        services.recompute_effective_permissions_local(&chat_id, None, None, None, cx);
                    });
                } else {
                    reload_forum_structure(chat_id.clone(), cx);
                }
                chat_services(cx, |services, cx| services.finish_role_mutation(&chat_id, cx));
                fire_result(&on_result, true, cx);
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                log_roles(format!("create_role error: {error}"), cx);
                chat_services(cx, |services, cx| {
                    services.restore_forum_structure_mutation(&snapshot, &chat_id, cx);
                    services.finish_role_mutation(&chat_id, cx);
                });
                Self::report_member_mutation_failure(error, cx);
                fire_result(&on_result, false, cx);
            })
        };

        SaiWs::request_slice(
            &self.create_role_sai,
            rise_request::chat::CreateRole {
                chat_id: chat_id.to_owned(),
                color: Some(color),
                hoist: Some(hoist),
                mentionable: Some(mentionable),
                name,
                permissions: Some(permissions),
            },
            self.roles_write_opts(rise_wire_methods::chat::CREATE_ROLE),
            Some(success),
            Some(failure),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_role_action(
        &mut self,
        chat_id: &str,
        role_id: &str,
        name: Option<String>,
        color: Option<i64>,
        permissions: Option<i64>,
        mentionable: Option<bool>,
        hoist: Option<bool>,
        on_result: Option<ChatRolesResultCallback>,
        cx: &mut Context<Self>,
    ) {
        if guide_owns_roles(chat_id, cx) {
            (chat_guide_sandbox_hooks().update_role)(
                role_id,
                ChatGuideSandboxRolePatch { name, color, permissions, mentionable, hoist },
                cx,
            );
            GuideSignalBus::emit(RiseGuideSignal::RoleSaved);
            fire_result(&on_result, true, cx);
            return;
        }
        if wire_user_id(cx).is_none() {
            fire_result(&on_result, false, cx);
            return;
        }
        if !chat_services(cx, |services, cx| services.begin_role_mutation(chat_id, cx)) {
            fire_result(&on_result, false, cx);
            return;
        }
        let snapshot = Rc::new(chat_services(cx, |services, cx| services.forum_structure_mutation_snapshot(chat_id, cx)));
        let had_forum_cache = chat_services(cx, |services, cx| services.has_forum_cache(chat_id, cx));
        if had_forum_cache
            && let Some(mut role) = chat_services(cx, |services, cx| services.role(chat_id, role_id, cx))
        {
            if let Some(name) = &name {
                role.name = name.clone();
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
            chat_services(cx, |services, cx| {
                services.upsert_role_cache(role, chat_id, cx);
                services.recompute_effective_permissions_local(chat_id, None, None, None, cx);
            });
        }

        let body = rise_request::chat::UpdateRole {
            chat_id: chat_id.to_owned(),
            role_id: role_id.to_owned(),
            color,
            hoist,
            mentionable,
            name,
            permissions,
        };

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let snapshot = Rc::clone(&snapshot);
            let on_result = on_result.clone();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                if had_forum_cache {
                    let Some(wire) = typed_payload::<RoleResultDTO>(payload).and_then(|result| result.role) else {
                        chat_services(cx, |services, cx| {
                            services.restore_forum_structure_mutation(&snapshot, &chat_id, cx);
                            services.finish_role_mutation(&chat_id, cx);
                        });
                        fire_result(&on_result, false, cx);
                        return;
                    };
                    chat_services(cx, |services, cx| {
                        services.upsert_role_cache(wire.to_role(), &chat_id, cx);
                        services.recompute_effective_permissions_local(&chat_id, None, None, None, cx);
                    });
                } else {
                    reload_forum_structure(chat_id.clone(), cx);
                }
                chat_services(cx, |services, cx| services.finish_role_mutation(&chat_id, cx));
                GuideSignalBus::emit(RiseGuideSignal::RoleSaved);
                fire_result(&on_result, true, cx);
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                log_roles(format!("update_role error: {error}"), cx);
                chat_services(cx, |services, cx| {
                    services.restore_forum_structure_mutation(&snapshot, &chat_id, cx);
                    services.finish_role_mutation(&chat_id, cx);
                });
                Self::report_member_mutation_failure(error, cx);
                fire_result(&on_result, false, cx);
            })
        };

        SaiWs::request_slice(
            &self.update_role_sai,
            body,
            self.roles_write_opts(rise_wire_methods::chat::UPDATE_ROLE),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn delete_role_action(&mut self, chat_id: &str, role_id: &str, cx: &mut Context<Self>) {
        if guide_owns_roles(chat_id, cx) {
            (chat_guide_sandbox_hooks().delete_role)(role_id, cx);
            return;
        }
        if wire_user_id(cx).is_none() {
            return;
        }
        if !chat_services(cx, |services, cx| services.begin_role_mutation(chat_id, cx)) {
            return;
        }

        let snapshot = Rc::new(chat_services(cx, |services, cx| services.forum_structure_mutation_snapshot(chat_id, cx)));
        let had_forum_cache = chat_services(cx, |services, cx| services.has_forum_cache(chat_id, cx));
        if had_forum_cache {
            chat_services(cx, |services, cx| {
                services.remove_role_cache(role_id, chat_id, cx);
                services.recompute_effective_permissions_local(chat_id, None, None, None, cx);
                cx.notify();
            });
        }
        let still_listed = role_still_listed(chat_id, role_id, cx);
        tracing::warn!(target: "riseonly::X1039", had_forum_cache, still_listed, "delete role optimistic");

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let role_id = role_id.to_owned();
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
                chat_services(cx, |services, cx| {
                    services.remove_role_cache(&role_id, &chat_id, cx);
                    services.finish_role_mutation(&chat_id, cx);
                    cx.notify();
                });
                if role_still_listed(&chat_id, &role_id, cx) {
                    tracing::warn!(target: "riseonly::X1039", "delete role ack still listed, reloading structure");
                    reload_forum_structure(chat_id.clone(), cx);
                }
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                log_roles(format!("delete_role error: {error}"), cx);
                chat_services(cx, |services, cx| {
                    services.restore_forum_structure_mutation(&snapshot, &chat_id, cx);
                    services.finish_role_mutation(&chat_id, cx);
                });
                Self::report_member_mutation_failure(error, cx);
            })
        };

        SaiWs::request_slice(
            &self.delete_role_sai,
            rise_request::chat::DeleteRole::new(chat_id.to_owned(), role_id.to_owned()),
            self.roles_write_opts(rise_wire_methods::chat::DELETE_ROLE),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn reorder_roles_action(&mut self, chat_id: &str, role_ids: Vec<String>, cx: &mut Context<Self>) {
        if guide_owns_roles(chat_id, cx) {
            (chat_guide_sandbox_hooks().reorder_roles)(&role_ids, cx);
            GuideSignalBus::emit(RiseGuideSignal::RoleReordered);
            return;
        }
        if wire_user_id(cx).is_none() {
            return;
        }

        let previous: Vec<String> = chat_services(cx, |services, cx| services.custom_roles(chat_id, cx))
            .into_iter()
            .map(|role| role.id)
            .collect();
        chat_services(cx, |services, cx| services.reorder_roles_cache(chat_id, &role_ids, cx));

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {
            GuideSignalBus::emit(RiseGuideSignal::RoleReordered);
        });
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                log_roles(format!("reorder error: {error}"), cx);
                chat_services(cx, |services, cx| services.reorder_roles_cache(&chat_id, &previous, cx));
                Self::report_member_mutation_failure(error, cx);
            })
        };

        SaiWs::request_slice(
            &self.reorder_roles_sai,
            rise_request::chat::ReorderRoles { chat_id: chat_id.to_owned(), role_ids: Some(role_ids) },
            self.roles_write_opts(rise_wire_methods::chat::REORDER_ROLES),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn move_role_action(&mut self, chat_id: &str, role_id: &str, up: bool, cx: &mut Context<Self>) {
        let ordered = chat_services(cx, |services, cx| services.custom_roles(chat_id, cx));

        let Some(i) = ordered.iter().position(|role| role.id == role_id) else {
            return;
        };

        let Some(j) = (if up { i.checked_sub(1) } else { Some(i + 1) }) else {
            return;
        };

        if j >= ordered.len() {
            return;
        }

        let mut ids: Vec<String> = ordered.into_iter().map(|role| role.id).collect();
        ids.swap(i, j);

        self.reorder_roles_action(chat_id, ids, cx);
    }

    pub fn assign_role_action(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        role_id: &str,
        assign: bool,
        on_result: Option<ChatRolesResultCallback>,
        cx: &mut Context<Self>,
    ) {
        if guide_owns_roles(chat_id, cx) {
            (chat_guide_sandbox_hooks().assign_role)(participant_id, role_id, assign, cx);
            if assign {
                GuideSignalBus::emit(RiseGuideSignal::MemberRoleAssigned);
            }
            fire_result(&on_result, true, cx);
            return;
        }
        let uid = wire_user_id(cx);
        let began = uid.is_some() && chat_services(cx, |services, cx| services.begin_member_mutation(chat_id, cx));
        let Some(uid) = uid.filter(|_| began) else {
            Notifier::send(
                tr("save_failed"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
            fire_result(&on_result, false, cx);
            return;
        };

        let member_snapshot =
            Rc::new(chat_services(cx, |services, cx| services.member_mutation_snapshot(chat_id, false, cx)));
        let previous_effective = chat_services(cx, |services, cx| services.effective_permissions(chat_id, None, cx));
        let scope = ChatServicesStore::member_scope_key(chat_id, None);
        let cached_member_role_ids =
            chat_services(cx, |services, cx| services.member(&scope, participant_id, cx)).map(|member| member.role_ids);
        let previous_role_ids = cached_member_role_ids
            .or_else(|| {
                (participant_id == uid)
                    .then(|| previous_effective.as_ref().map(|effective| effective.role_ids.clone()))
                    .flatten()
            })
            .unwrap_or_default();
        let mut role_ids = previous_role_ids.clone();
        if assign {
            if !role_ids.iter().any(|id| id == role_id) {
                role_ids.push(role_id.to_owned());
            }
        } else {
            role_ids.retain(|id| id != role_id);
        }
        let mut member_fields = Map::new();
        member_fields.insert(chat_key::ROLE_IDS.to_owned(), Value::from(role_ids.clone()));
        let known_roles = chat_services(cx, |services, cx| services.custom_roles(chat_id, cx));
        if known_roles.iter().any(|role| role.id == role_id) {
            let display_color = known_roles
                .iter()
                .filter(|role| role.color != 0 && role_ids.contains(&role.id))
                .rev()
                .max_by_key(|role| role.position)
                .map_or(0, |role| role.color);
            member_fields.insert(chat_key::DISPLAY_COLOR.to_owned(), Value::from(display_color));
        }
        chat_services(cx, |services, cx| {
            services.apply_member_fields_local(chat_id, participant_id, member_fields, cx)
        });
        if participant_id == uid {
            chat_services(cx, |services, cx| {
                services.recompute_effective_permissions_local(chat_id, Some(role_ids), None, None, cx)
            });
        }

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let on_result = on_result.clone();
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
                chat_services(cx, |services, cx| services.finish_member_mutation(&chat_id, cx));
                if assign {
                    GuideSignalBus::emit(RiseGuideSignal::MemberRoleAssigned);
                }
                fire_result(&on_result, true, cx);
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let participant_id = participant_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                log_roles(format!("assign error: {error}"), cx);
                chat_services(cx, |services, cx| services.restore_member_mutation_snapshot(&member_snapshot, &chat_id, cx));
                if participant_id == uid {
                    let member_allow = previous_effective.as_ref().map_or(0, |effective| effective.member_allow);
                    let member_deny = previous_effective.as_ref().map_or(0, |effective| effective.member_deny);
                    let role_ids = previous_role_ids.clone();
                    chat_services(cx, |services, cx| {
                        services.recompute_effective_permissions_local(
                            &chat_id,
                            Some(role_ids),
                            Some(member_allow),
                            Some(member_deny),
                            cx,
                        )
                    });
                }
                chat_services(cx, |services, cx| services.finish_member_mutation(&chat_id, cx));
                Self::report_member_mutation_failure(error, cx);
                fire_result(&on_result, false, cx);
            })
        };

        if assign {
            SaiWs::request_slice(
                &self.assign_role_sai,
                rise_request::chat::AssignRole::new(chat_id.to_owned(), participant_id.to_owned(), role_id.to_owned()),
                self.roles_write_opts(rise_wire_methods::chat::ASSIGN_ROLE),
                Some(success),
                Some(failure),
                cx,
            );
        } else {
            SaiWs::request_slice(
                &self.assign_role_sai,
                rise_request::chat::UnassignRole::new(chat_id.to_owned(), participant_id.to_owned(), role_id.to_owned()),
                self.roles_write_opts(rise_wire_methods::chat::UNASSIGN_ROLE),
                Some(success),
                Some(failure),
                cx,
            );
        }
    }

    pub fn create_section_action(
        &mut self,
        chat_id: &str,
        name: &str,
        name_entities: Option<&str>,
        on_result: Option<ChatRolesResultCallback>,
        cx: &mut Context<Self>,
    ) {
        if wire_user_id(cx).is_none() {
            fire_result(&on_result, false, cx);
            return;
        }

        let mut body = rise_request::chat::CreateSection::new(chat_id.to_owned(), name.to_owned());
        body.name_entities = name_entities.map(str::to_owned);

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let on_result = on_result.clone();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                if let Some(section) = typed_payload::<SectionResultDTO>(payload).and_then(|result| result.section) {
                    chat_services(cx, |services, cx| {
                        services.insert_section_cache(section.to_section(), &chat_id, cx)
                    });
                }
                fire_result(&on_result, true, cx);
            })
        };
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::report_member_mutation_failure(error, cx);
            fire_result(&on_result, false, cx);
        });

        SaiWs::request_slice(
            &self.create_section_sai,
            body,
            self.roles_write_opts(rise_wire_methods::chat::CREATE_SECTION),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn delete_section_action(&mut self, chat_id: &str, section_id: &str, cx: &mut Context<Self>) {
        if wire_user_id(cx).is_none() {
            return;
        }

        chat_services(cx, |services, cx| services.remove_section_cache(section_id, chat_id, cx));

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let section_id = section_id.to_owned();
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
                SaiWsResponseCache::remove(
                    &ChatServicesStore::overwrites_cache_key(&chat_id, OverwriteScope::Section, &section_id),
                    SaiCacheCleanLayer::Everywhere,
                    cx,
                );
                chat_services(cx, |services, cx| {
                    services.invalidate_effective_permissions_scope(&chat_id, OverwriteScope::Section, &section_id, cx)
                });
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Self::report_member_mutation_failure(error, cx);
                reload_forum_structure(chat_id.clone(), cx);
            })
        };

        SaiWs::request_slice(
            &self.delete_section_sai,
            rise_request::chat::DeleteSection::new(chat_id.to_owned(), section_id.to_owned()),
            self.roles_write_opts(rise_wire_methods::chat::DELETE_SECTION),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn update_section_action(
        &mut self,
        chat_id: &str,
        section_id: &str,
        name: &str,
        name_entities: Option<&str>,
        on_result: Option<ChatRolesResultCallback>,
        cx: &mut Context<Self>,
    ) {
        if wire_user_id(cx).is_none() {
            fire_result(&on_result, false, cx);
            return;
        }

        let previous = chat_services(cx, |services, cx| services.sections(chat_id, cx))
            .into_iter()
            .find(|section| section.id == section_id);
        chat_services(cx, |services, cx| services.rename_section_cache(section_id, name, name_entities, chat_id, cx));

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let section_id = section_id.to_owned();
            let on_result = on_result.clone();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                if let Some(section) = typed_payload::<SectionResultDTO>(payload).and_then(|result| result.section) {
                    let section = section.to_section();
                    chat_services(cx, |services, cx| {
                        services.rename_section_cache(
                            &section_id,
                            &section.name,
                            section.name_entities.as_deref(),
                            &chat_id,
                            cx,
                        )
                    });
                }
                fire_result(&on_result, true, cx);
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let section_id = section_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                if let Some(previous) = &previous {
                    chat_services(cx, |services, cx| {
                        services.rename_section_cache(
                            &section_id,
                            &previous.name,
                            previous.name_entities.as_deref(),
                            &chat_id,
                            cx,
                        )
                    });
                }
                Self::report_member_mutation_failure(error, cx);
                fire_result(&on_result, false, cx);
            })
        };

        SaiWs::request_slice(
            &self.update_section_sai,
            rise_request::chat::UpdateSection {
                chat_id: chat_id.to_owned(),
                name: Some(name.to_owned()),
                name_entities: Some(name_entities.unwrap_or_default().to_owned()),
                section_id: section_id.to_owned(),
            },
            self.roles_write_opts(rise_wire_methods::chat::UPDATE_SECTION),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn reorder_sections_action(&mut self, chat_id: &str, section_ids: Vec<String>, cx: &mut Context<Self>) {
        if wire_user_id(cx).is_none() {
            return;
        }

        let previous: Vec<String> = chat_services(cx, |services, cx| services.sections(chat_id, cx))
            .into_iter()
            .map(|section| section.id)
            .collect();
        chat_services(cx, |services, cx| services.reorder_sections_cache(chat_id, &section_ids, cx));

        let success: SaiWsOnSuccess = Rc::new(|_: Option<Box<dyn Any>>, _: &mut App| {});
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                log_roles(format!("reorder sections error: {error}"), cx);
                chat_services(cx, |services, cx| services.reorder_sections_cache(&chat_id, &previous, cx));
                Self::report_member_mutation_failure(error, cx);
            })
        };

        SaiWs::request_slice(
            &self.reorder_sections_sai,
            rise_request::chat::ReorderSections { chat_id: chat_id.to_owned(), section_ids: Some(section_ids) },
            self.roles_write_opts(rise_wire_methods::chat::REORDER_SECTIONS),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn get_overwrites_action(
        &mut self,
        chat_id: &str,
        scope: OverwriteScope,
        scope_id: &str,
        on_result: Rc<dyn Fn(Vec<ChatPermOverwrite>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        if guide_owns_roles(chat_id, cx) {
            on_result((chat_guide_sandbox_hooks().overwrites)(scope, scope_id, cx).unwrap_or_default(), cx);
            return;
        }
        if wire_user_id(cx).is_none() {
            on_result(Vec::new(), cx);
            return;
        }

        let is_topic = scope == OverwriteScope::Topic;
        let method = if is_topic { rise_wire_methods::chat::GET_TOPIC_OVERWRITES } else { rise_wire_methods::chat::GET_SECTION_OVERWRITES };
        let key = ChatServicesStore::overwrites_cache_key(chat_id, scope, scope_id);
        let has_cache = self.roles_has_cached_payload(&key, cx);
        let options = self.roles_read_opts(method, key, has_cache);

        let success: SaiWsOnSuccess = {
            let on_result = Rc::clone(&on_result);
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let overwrites: Vec<ChatPermOverwrite> = typed_payload::<GetOverwritesGatewayDTO>(payload)
                    .map(|dto| dto.overwrites.iter().map(|overwrite| overwrite.to_overwrite()).collect())
                    .unwrap_or_default();
                on_result(overwrites, cx);
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let scope_id = scope_id.to_owned();
            Rc::new(move |_: &anyhow::Error, cx: &mut App| {
                let cached = chat_services(cx, |services, cx| {
                    services.cached_overwrites(&chat_id, scope, &scope_id, cx)
                });
                on_result(cached.unwrap_or_default(), cx);
            })
        };

        if is_topic {
            SaiWs::request_slice(
                &self.get_overwrites_sai,
                rise_request::chat::GetTopicOverwrites::new(chat_id.to_owned(), scope_id.to_owned()),
                options,
                Some(success),
                Some(failure),
                cx,
            );
        } else {
            SaiWs::request_slice(
                &self.get_overwrites_sai,
                rise_request::chat::GetSectionOverwrites::new(chat_id.to_owned(), scope_id.to_owned()),
                options,
                Some(success),
                Some(failure),
                cx,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_overwrite_action(
        &mut self,
        chat_id: &str,
        scope: OverwriteScope,
        scope_id: &str,
        target_type: &str,
        target_id: &str,
        allow: i64,
        deny: i64,
        on_result: Option<ChatRolesResultCallback>,
        cx: &mut Context<Self>,
    ) {
        let overwrite = ChatPermOverwrite {
            target_type: target_type.to_owned(),
            target_id: target_id.to_owned(),
            allow,
            deny,
        };
        if guide_owns_roles(chat_id, cx) {
            (chat_guide_sandbox_hooks().set_overwrite)(scope, scope_id, overwrite, cx);
            fire_result(&on_result, true, cx);
            return;
        }
        if wire_user_id(cx).is_none() {
            fire_result(&on_result, false, cx);
            return;
        }

        let is_topic = scope == OverwriteScope::Topic;
        let method = if is_topic { rise_wire_methods::chat::SET_TOPIC_OVERWRITE } else { rise_wire_methods::chat::SET_SECTION_OVERWRITE };

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let scope_id = scope_id.to_owned();
            let on_result = on_result.clone();
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
                let overwrite = overwrite.clone();
                chat_services(cx, |services, cx| {
                    services.upsert_overwrite_cache(&chat_id, scope, &scope_id, overwrite, cx);
                    services.recompute_effective_permissions_local(&chat_id, None, None, None, cx);
                });
                fire_result(&on_result, true, cx);
            })
        };
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_roles(format!("{method} error: {error}"), cx);
            Self::report_member_mutation_failure(error, cx);
            fire_result(&on_result, false, cx);
        });

        if is_topic {
            SaiWs::request_slice(
                &self.set_overwrite_sai,
                rise_request::chat::SetTopicOverwrite {
                    allow: Some(allow),
                    chat_id: chat_id.to_owned(),
                    deny: Some(deny),
                    scope_id: scope_id.to_owned(),
                    target_id: target_id.to_owned(),
                    target_type: target_type.to_owned(),
                },
                self.roles_write_opts(method),
                Some(success),
                Some(failure),
                cx,
            );
        } else {
            SaiWs::request_slice(
                &self.set_overwrite_sai,
                rise_request::chat::SetSectionOverwrite {
                    allow: Some(allow),
                    chat_id: chat_id.to_owned(),
                    deny: Some(deny),
                    scope_id: scope_id.to_owned(),
                    target_id: target_id.to_owned(),
                    target_type: target_type.to_owned(),
                },
                self.roles_write_opts(method),
                Some(success),
                Some(failure),
                cx,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn remove_overwrite_action(
        &mut self,
        chat_id: &str,
        scope: OverwriteScope,
        scope_id: &str,
        target_type: &str,
        target_id: &str,
        on_result: Option<ChatRolesResultCallback>,
        cx: &mut Context<Self>,
    ) {
        if guide_owns_roles(chat_id, cx) {
            (chat_guide_sandbox_hooks().remove_overwrite)(scope, scope_id, target_type, target_id, cx);
            fire_result(&on_result, true, cx);
            return;
        }
        if wire_user_id(cx).is_none() {
            fire_result(&on_result, false, cx);
            return;
        }

        let is_topic = scope == OverwriteScope::Topic;
        let method =
            if is_topic { rise_wire_methods::chat::REMOVE_TOPIC_OVERWRITE } else { rise_wire_methods::chat::REMOVE_SECTION_OVERWRITE };

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let scope_id = scope_id.to_owned();
            let target_type = target_type.to_owned();
            let target_id = target_id.to_owned();
            let on_result = on_result.clone();
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
                chat_services(cx, |services, cx| {
                    services.remove_overwrite_cache(&chat_id, scope, &scope_id, &target_type, &target_id, cx);
                    services.recompute_effective_permissions_local(&chat_id, None, None, None, cx);
                });
                fire_result(&on_result, true, cx);
            })
        };
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_roles(format!("{method} error: {error}"), cx);
            Self::report_member_mutation_failure(error, cx);
            fire_result(&on_result, false, cx);
        });

        if is_topic {
            SaiWs::request_slice(
                &self.remove_overwrite_sai,
                rise_request::chat::RemoveTopicOverwrite::new(
                    chat_id.to_owned(),
                    scope_id.to_owned(),
                    target_id.to_owned(),
                    target_type.to_owned(),
                ),
                self.roles_write_opts(method),
                Some(success),
                Some(failure),
                cx,
            );
        } else {
            SaiWs::request_slice(
                &self.remove_overwrite_sai,
                rise_request::chat::RemoveSectionOverwrite::new(
                    chat_id.to_owned(),
                    scope_id.to_owned(),
                    target_id.to_owned(),
                    target_type.to_owned(),
                ),
                self.roles_write_opts(method),
                Some(success),
                Some(failure),
                cx,
            );
        }
    }

    pub fn set_member_permissions_action(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        allow: i64,
        deny: i64,
        on_result: Option<ChatRolesResultCallback>,
        cx: &mut Context<Self>,
    ) {
        if guide_owns_roles(chat_id, cx) {
            (chat_guide_sandbox_hooks().set_member_override)(participant_id, allow, deny, cx);
            fire_result(&on_result, true, cx);
            return;
        }
        let Some(uid) = wire_user_id(cx) else {
            fire_result(&on_result, false, cx);
            return;
        };

        let override_key = ChatServicesStore::member_override_cache_key(chat_id, participant_id);
        let previous_override: Option<Map<String, Value>> = SaiWsResponseCache::current_payload(
            &override_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        )
        .and_then(|payload| payload.as_object().cloned());
        let previous_effective = chat_services(cx, |services, cx| services.effective_permissions(chat_id, None, cx));
        chat_services(cx, |services, cx| {
            services.upsert_member_override_cache(chat_id, participant_id, allow, deny, cx)
        });
        if participant_id == uid {
            chat_services(cx, |services, cx| {
                services.recompute_effective_permissions_local(chat_id, None, Some(allow), Some(deny), cx)
            });
        }

        let success: SaiWsOnSuccess = {
            let on_result = on_result.clone();
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| fire_result(&on_result, true, cx))
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let participant_id = participant_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Self::report_member_mutation_failure(error, cx);
                if let Some(previous_override) = &previous_override {
                    let _ = SaiUpdater::replace_root(
                        &override_key,
                        previous_override.clone(),
                        SaiUpdaterWriteScope::All,
                        cx,
                    );
                } else {
                    SaiWsResponseCache::remove(&override_key, SaiCacheCleanLayer::Everywhere, cx);
                }
                if participant_id == uid {
                    let role_ids = previous_effective.as_ref().map(|effective| effective.role_ids.clone());
                    let member_allow = previous_effective.as_ref().map_or(0, |effective| effective.member_allow);
                    let member_deny = previous_effective.as_ref().map_or(0, |effective| effective.member_deny);
                    chat_services(cx, |services, cx| {
                        services.recompute_effective_permissions_local(
                            &chat_id,
                            role_ids,
                            Some(member_allow),
                            Some(member_deny),
                            cx,
                        )
                    });
                }
                fire_result(&on_result, false, cx);
            })
        };

        SaiWs::request_slice(
            &self.set_member_perms_sai,
            rise_request::chat::SetMemberPermissions {
                allow: Some(allow),
                chat_id: chat_id.to_owned(),
                deny: Some(deny),
                participant_id: participant_id.to_owned(),
            },
            self.roles_write_opts(rise_wire_methods::chat::SET_MEMBER_PERMISSIONS),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn get_member_override_action(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        on_effective_permissions: Option<Rc<dyn Fn(Option<i64>, &mut App)>>,
        on_read_failed: Option<Rc<dyn Fn(&anyhow::Error, &mut App)>>,
        on_result: Rc<dyn Fn(i64, i64, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        if guide_owns_roles(chat_id, cx) {
            let invented = (chat_guide_sandbox_hooks().member_override)(participant_id, cx);
            on_result(invented.map_or(0, |value| value.0), invented.map_or(0, |value| value.1), cx);
            if let Some(on_effective_permissions) = &on_effective_permissions {
                on_effective_permissions(None, cx);
            }
            return;
        }
        if wire_user_id(cx).is_none() {
            on_result(0, 0, cx);
            if let Some(on_effective_permissions) = &on_effective_permissions {
                on_effective_permissions(None, cx);
            }
            return;
        }

        let key = ChatServicesStore::member_override_cache_key(chat_id, participant_id);
        let has_cache = self.roles_has_cached_payload(&key, cx);
        let mut options = self.roles_read_opts(rise_wire_methods::chat::GET_EFFECTIVE_PERMISSIONS, key, has_cache);
        if on_effective_permissions.is_some() {
            options.force_bypass_cache_read = true;
        }

        let success: SaiWsOnSuccess = {
            let on_result = Rc::clone(&on_result);
            let on_effective_permissions = on_effective_permissions.clone();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let dto = typed_payload::<EffectivePermissionsDTO>(payload);
                on_result(
                    dto.as_ref().map_or(0, |dto| dto.member_allow),
                    dto.as_ref().map_or(0, |dto| dto.member_deny),
                    cx,
                );
                if let Some(on_effective_permissions) = &on_effective_permissions {
                    on_effective_permissions(dto.as_ref().map(|dto| dto.permissions), cx);
                }
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let participant_id = participant_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                if let Some(on_read_failed) = &on_read_failed {
                    on_read_failed(error, cx);
                    return;
                }
                let cached = chat_services(cx, |services, cx| {
                    services.cached_member_override(&chat_id, &participant_id, cx)
                });
                on_result(cached.map_or(0, |value| value.0), cached.map_or(0, |value| value.1), cx);
                if let Some(on_effective_permissions) = &on_effective_permissions {
                    on_effective_permissions(None, cx);
                }
            })
        };

        SaiWs::request_slice(
            &self.member_override_sai,
            rise_request::chat::GetEffectivePermissions {
                participant_id: Some(participant_id.to_owned()),
                ..rise_request::chat::GetEffectivePermissions::new(chat_id.to_owned())
            },
            options,
            Some(success),
            Some(failure),
            cx,
        );
    }
}
