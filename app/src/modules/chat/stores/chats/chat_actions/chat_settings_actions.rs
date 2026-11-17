use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_request;
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_errors::{ErrorCategory, ErrorCode};

use super::chat_actions::ChatActionsStore;
use super::chat_roles_actions::{ChatRolesResultCallback, chat_services, fire_result, typed_payload, wire_user_id};
use super::chat_types::{ChatPermissionJSON, ChatReactionCatalog, ChatReactionsType, ChatSettingsModel, LinkedChatInfo};
use super::chat_wire_types::{
    ChatAnalyticsGatewayDTO, ChatAnalyticsGatewayDTOExt, ChatSettingsGatewayDTO, ChatSettingsGatewayDTOExt,
    LinkedChatGatewayDTO, LinkedChatGatewayDTOExt,
};
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::sai::Sai;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const LINKED_CHAT_OPTIMISTIC_TYPE: &str = "GROUP";

fn settings_read_opts(method: &str, cache_key: String, reflect_pending_in_ui: bool) -> SaiWsRequestOptions {
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
        ..SaiWsRequestOptionsInit::default()
    })
}

fn settings_write_opts(method: &str, allow_concurrent_while_pending: bool) -> SaiWsRequestOptions {
    SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: rise_wire_methods::chat::SERVICE.to_owned(),
        method: method.to_owned(),
        reload_if_cached: true,
        allow_concurrent_while_pending,
        reflect_pending_in_ui: false,
        send_immediately: true,
        store_response_in_cache: false,
        ..SaiWsRequestOptionsInit::default()
    })
}

fn settings_of(chat_id: &str, cx: &mut App) -> Option<ChatSettingsModel> {
    let services = ChatServicesStore::shared(cx);
    services.read(cx).settings_by_chat.get(chat_id).cloned()
}

fn put_settings(chat_id: &str, model: Option<ChatSettingsModel>, cx: &mut App) {
    chat_services(cx, |services, cx| {
        if services.settings_by_chat.get(chat_id) == model.as_ref() {
            return;
        }
        match model {
            Some(model) => services.settings_by_chat.insert(chat_id.to_owned(), model),
            None => services.settings_by_chat.remove(chat_id),
        };
        cx.notify();
    });
}

fn linked_chat_of(chat_id: &str, cx: &mut App) -> Option<LinkedChatInfo> {
    let services = ChatServicesStore::shared(cx);
    services.read(cx).linked_chat_by_chat.get(chat_id).cloned()
}

fn put_linked_chat(chat_id: &str, card: Option<LinkedChatInfo>, cx: &mut App) {
    chat_services(cx, |services, cx| {
        if services.linked_chat_by_chat.get(chat_id) == card.as_ref() {
            return;
        }
        match card {
            Some(card) => services.linked_chat_by_chat.insert(chat_id.to_owned(), card),
            None => services.linked_chat_by_chat.remove(chat_id),
        };
        cx.notify();
    });
}

fn forget_linked_chat(chat_id: &str, cx: &mut App) {
    put_linked_chat(chat_id, None, cx);
    SaiWsResponseCache::remove(&ChatServicesStore::linked_chat_cache_key(chat_id), SaiCacheCleanLayer::Everywhere, cx);
    if let Some(mut model) = settings_of(chat_id, cx).filter(|model| model.linked_chat_id.is_some()) {
        model.linked_chat_id = None;
        put_settings(chat_id, Some(model), cx);
    }
    let _ = chat_services(cx, |services, cx| services.apply_linked_chat_mutation_local(chat_id, None, cx));
}

fn drop_unbacked_linked_card(chat_id: &str, linked_id: Option<&str>, cx: &mut App) {
    let linked_id = linked_id.filter(|id| !id.is_empty());
    let card = linked_chat_of(chat_id, cx);
    if card.as_ref().is_some_and(|card| linked_id == Some(card.id.as_str())) {
        return;
    }
    if card.is_some() || linked_id.is_none() {
        put_linked_chat(chat_id, None, cx);
        SaiWsResponseCache::remove(&ChatServicesStore::linked_chat_cache_key(chat_id), SaiCacheCleanLayer::Everywhere, cx);
    }
}

fn settings_deny_card(chat_id: &str, card_id: &str, cx: &mut App) -> bool {
    settings_of(chat_id, cx).is_some_and(|settings| settings.linked_chat_id.as_deref() != Some(card_id))
}

fn adopt_updated_settings(chat_id: &str, dto: &ChatSettingsGatewayDTO, cx: &mut App) {
    put_settings(chat_id, Some(dto.to_model(chat_id)), cx);
    if let Some(root) = SaiJSONCodec::encode(dto) {
        let _ = SaiUpdater::replace_root(
            &ChatServicesStore::settings_cache_key(chat_id),
            root,
            SaiUpdaterWriteScope::All,
            cx,
        );
    }
}

fn empty_settings(chat_id: &str) -> ChatSettingsModel {
    ChatSettingsModel { chat_id: chat_id.to_owned(), ..ChatSettingsModel::default() }
}

impl ChatActionsStore {
    pub fn get_chat_settings_action(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if wire_user_id(cx).is_none() || chat_id.is_empty() {
            return;
        }
        chat_services(cx, |services, cx| services.hydrate_chat_settings_from_disk(chat_id, cx));

        let reflect_pending = settings_of(chat_id, cx).is_none();
        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = typed_payload::<ChatSettingsGatewayDTO>(payload) else {
                    tracing::warn!(target: "riseonly::X1048", chat_id = %chat_id, "settings payload not decoded");
                    return;
                };
                tracing::warn!(
                    target: "riseonly::X1048",
                    chat_id = %chat_id,
                    linked = ?dto.linked_chat_id,
                    card = ?linked_chat_of(&chat_id, cx).map(|card| card.id),
                    "settings answer"
                );
                put_settings(&chat_id, Some(dto.to_model(&chat_id)), cx);
                drop_unbacked_linked_card(&chat_id, dto.linked_chat_id.as_deref(), cx);
                chat_services(cx, |services, cx| services.reconcile_slow_mode(&chat_id, cx));
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                let message = format!("get_chat_settings({chat_id}) error: {error}");
                Sai::log("Chat", move || message, SaiLogKind::Network, cx);
            })
        };

        SaiWs::request_slice(
            &self.get_chat_settings_sai,
            rise_request::chat::GetChatSettings::new(chat_id.to_owned()),
            settings_read_opts(
                rise_wire_methods::chat::GET_CHAT_SETTINGS,
                ChatServicesStore::settings_cache_key(chat_id),
                reflect_pending,
            ),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn update_paid_reactions_action(&mut self, chat_id: &str, allowed: bool, cx: &mut Context<Self>) {
        if wire_user_id(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let previous = settings_of(chat_id, cx);
        let mut model = previous.clone().unwrap_or_else(|| empty_settings(chat_id));
        model.paid_reactions_allowed = Some(allowed);
        put_settings(chat_id, Some(model), cx);

        let body = rise_request::chat::UpdateChatSettings {
            paid_reactions_allowed: Some(allowed),
            ..rise_request::chat::UpdateChatSettings::new(chat_id.to_owned())
        };

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = typed_payload::<ChatSettingsGatewayDTO>(payload) else {
                    return;
                };
                adopt_updated_settings(&chat_id, &dto, cx);
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                let message = format!("update_chat_settings({chat_id}) sparks error: {error}");
                Sai::log("Chat", move || message, SaiLogKind::Network, cx);
                put_settings(&chat_id, previous.clone(), cx);
                Self::report_member_mutation_failure(error, cx);
            })
        };

        SaiWs::request_slice(
            &self.update_reactions_sai,
            body,
            settings_write_opts(rise_wire_methods::chat::UPDATE_CHAT_SETTINGS, true),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn update_reactions_action(
        &mut self,
        chat_id: &str,
        r#type: ChatReactionsType,
        allowed: &[String],
        cx: &mut Context<Self>,
    ) {
        if wire_user_id(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let previous = settings_of(chat_id, cx);
        let mut model = previous.clone().unwrap_or_else(|| empty_settings(chat_id));
        model.reactions_type = r#type;
        model.allowed_reactions = match r#type {
            ChatReactionsType::Custom => allowed.to_vec(),
            ChatReactionsType::All => ChatReactionCatalog::all_emojis().to_vec(),
            ChatReactionsType::None => Vec::new(),
        };
        let allowed_reactions = model.allowed_reactions.clone();
        let local_reactions = if model.reaction_config().enabled { allowed_reactions.clone() } else { Vec::new() };
        put_settings(chat_id, Some(model), cx);
        chat_services(cx, |services, cx| services.set_allowed_reactions_local(chat_id, local_reactions, cx));

        let body = rise_request::chat::UpdateChatSettings {
            allowed_reactions: Some(allowed_reactions),
            allowed_reactions_type: Some(r#type.wire().to_owned()),
            ..rise_request::chat::UpdateChatSettings::new(chat_id.to_owned())
        };

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = typed_payload::<ChatSettingsGatewayDTO>(payload) else {
                    return;
                };
                adopt_updated_settings(&chat_id, &dto, cx);
                let chat_id = chat_id.clone();
                cx.defer(move |cx| {
                    ChatActionsStore::shared(cx)
                        .update(cx, |actions, cx| actions.get_chat_by_id_action(&chat_id, true, cx));
                });
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                let message = format!("update_chat_settings({chat_id}) reactions error: {error}");
                Sai::log("Chat", move || message, SaiLogKind::Network, cx);
                put_settings(&chat_id, previous.clone(), cx);
                let revert_reactions = previous
                    .as_ref()
                    .filter(|settings| settings.reaction_config().enabled)
                    .map(|settings| settings.allowed_reactions.clone())
                    .unwrap_or_default();
                chat_services(cx, |services, cx| services.set_allowed_reactions_local(&chat_id, revert_reactions, cx));
                Self::report_member_mutation_failure(error, cx);
            })
        };

        SaiWs::request_slice(
            &self.update_reactions_sai,
            body,
            settings_write_opts(rise_wire_methods::chat::UPDATE_CHAT_SETTINGS, true),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn get_linked_chat_action(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if wire_user_id(cx).is_none() || chat_id.is_empty() {
            return;
        }
        chat_services(cx, |services, cx| services.hydrate_linked_chat_from_disk(chat_id, cx));

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = typed_payload::<LinkedChatGatewayDTO>(payload) else {
                    tracing::warn!(target: "riseonly::X1048", chat_id = %chat_id, "linked chat payload not decoded");
                    return;
                };
                let info = dto.to_info();
                tracing::warn!(target: "riseonly::X1048", chat_id = %chat_id, card = ?info.as_ref().map(|card| card.id.clone()), "linked chat answer");
                match info {
                    Some(info) if !settings_deny_card(&chat_id, &info.id, cx) => put_linked_chat(&chat_id, Some(info), cx),
                    _ => forget_linked_chat(&chat_id, cx),
                }
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                let message = format!("get_linked_chat_by_chat_id({chat_id}) error: {error}");
                Sai::log("Chat", move || message, SaiLogKind::Network, cx);
                let code = CanonErrors::failure(error).and_then(|failure| failure.code);
                tracing::warn!(target: "riseonly::X1048", chat_id = %chat_id, code = ?code, %error, "linked chat failed");
                let link_gone = code.is_some_and(|code| code == ErrorCode::ChatLinkedChatNotFound.wire());
                if link_gone {
                    forget_linked_chat(&chat_id, cx);
                }
            })
        };

        SaiWs::request_slice(
            &self.get_linked_chat_sai,
            rise_request::chat::GetLinkedChatByChatId::new(chat_id.to_owned()),
            settings_read_opts(
                rise_wire_methods::chat::GET_LINKED_CHAT_BY_CHAT_ID,
                ChatServicesStore::linked_chat_cache_key(chat_id),
                false,
            ),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn link_chat_action(
        &mut self,
        chat_id: &str,
        group_id: &str,
        on_result: Option<ChatRolesResultCallback>,
        cx: &mut Context<Self>,
    ) {
        if wire_user_id(cx).is_none() || self.link_chat_sai.is_pending() || self.unlink_chat_sai.is_pending() {
            fire_result(&on_result, false, cx);
            return;
        }

        let previous_card = linked_chat_of(chat_id, cx);
        let previous_settings = settings_of(chat_id, cx);
        let group = chat_services(cx, |services, cx| services.current_chat_infos(cx))
            .into_iter()
            .find(|chat| chat.id == group_id);
        let optimistic_card = match group {
            Some(group) => LinkedChatInfo {
                id: group.id,
                title: group.title,
                logo_url: group.logo_url,
                r#type: LINKED_CHAT_OPTIMISTIC_TYPE.to_owned(),
            },
            None => LinkedChatInfo { id: group_id.to_owned(), ..LinkedChatInfo::default() },
        };
        put_linked_chat(chat_id, Some(optimistic_card.clone()), cx);
        let mut model = previous_settings.clone().unwrap_or_else(|| empty_settings(chat_id));
        model.linked_chat_id = Some(group_id.to_owned());
        put_settings(chat_id, Some(model), cx);
        let cache_snapshot = chat_services(cx, |services, cx| {
            services.apply_linked_chat_mutation_local(chat_id, Some(optimistic_card), cx)
        });

        let success: SaiWsOnSuccess = {
            let on_result = on_result.clone();
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| fire_result(&on_result, true, cx))
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let group_id = group_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                let message = format!("link_chat({chat_id}\u{2192}{group_id}) error: {error}");
                Sai::log("Chat", move || message, SaiLogKind::Network, cx);
                put_linked_chat(&chat_id, previous_card.clone(), cx);
                put_settings(&chat_id, previous_settings.clone(), cx);
                chat_services(cx, |services, cx| services.restore_linked_chat_mutation(&cache_snapshot, &chat_id, cx));
                Self::report_member_mutation_failure(error, cx);
                fire_result(&on_result, false, cx);
            })
        };

        SaiWs::request_slice(
            &self.link_chat_sai,
            rise_request::chat::LinkChat::new(chat_id.to_owned(), group_id.to_owned()),
            settings_write_opts(rise_wire_methods::chat::LINK_CHAT, false),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn unlink_chat_action(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if wire_user_id(cx).is_none() || self.link_chat_sai.is_pending() || self.unlink_chat_sai.is_pending() {
            tracing::warn!(
                target: "riseonly::X1048",
                chat_id,
                link_pending = self.link_chat_sai.is_pending(),
                unlink_pending = self.unlink_chat_sai.is_pending(),
                "unlink skipped"
            );
            return;
        }

        let previous_card = linked_chat_of(chat_id, cx);
        let previous_settings = settings_of(chat_id, cx);
        put_linked_chat(chat_id, None, cx);
        if let Some(mut model) = previous_settings.clone() {
            model.linked_chat_id = None;
            put_settings(chat_id, Some(model), cx);
        }
        let cache_snapshot =
            chat_services(cx, |services, cx| services.apply_linked_chat_mutation_local(chat_id, None, cx));

        let counterpart_id = previous_card
            .as_ref()
            .map(|card| card.id.as_str())
            .or_else(|| previous_settings.as_ref().and_then(|settings| settings.linked_chat_id.as_deref()))
            .map(|id| trimming_whitespaces_and_newlines(id).to_owned())
            .unwrap_or_default();
        let counterpart_card = if counterpart_id.is_empty() { None } else { linked_chat_of(&counterpart_id, cx) };
        let counterpart_settings = if counterpart_id.is_empty() { None } else { settings_of(&counterpart_id, cx) };
        let counterpart_points_back = counterpart_card.as_ref().is_some_and(|card| card.id == chat_id)
            || counterpart_settings.as_ref().is_some_and(|settings| settings.linked_chat_id.as_deref() == Some(chat_id));
        let counterpart_snapshot = if counterpart_points_back {
            put_linked_chat(&counterpart_id, None, cx);
            if let Some(mut model) = counterpart_settings.clone() {
                model.linked_chat_id = None;
                put_settings(&counterpart_id, Some(model), cx);
            }
            Some(chat_services(cx, |services, cx| {
                services.apply_linked_chat_mutation_local(&counterpart_id, None, cx)
            }))
        } else {
            None
        };

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let counterpart_id = counterpart_id.clone();
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
                tracing::warn!(target: "riseonly::X1048", chat_id = %chat_id, counterpart = %counterpart_id, "unlink ack");
                forget_linked_chat(&chat_id, cx);
                if !counterpart_id.is_empty() && counterpart_points_back {
                    forget_linked_chat(&counterpart_id, cx);
                }
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                tracing::warn!(target: "riseonly::X1048", chat_id = %chat_id, %error, "unlink failed");
                let message = format!("unlink_chat({chat_id}) error: {error}");
                Sai::log("Chat", move || message, SaiLogKind::Network, cx);
                put_linked_chat(&chat_id, previous_card.clone(), cx);
                put_settings(&chat_id, previous_settings.clone(), cx);
                chat_services(cx, |services, cx| services.restore_linked_chat_mutation(&cache_snapshot, &chat_id, cx));
                if let Some(counterpart_snapshot) = &counterpart_snapshot {
                    put_linked_chat(&counterpart_id, counterpart_card.clone(), cx);
                    put_settings(&counterpart_id, counterpart_settings.clone(), cx);
                    chat_services(cx, |services, cx| {
                        services.restore_linked_chat_mutation(counterpart_snapshot, &counterpart_id, cx)
                    });
                }
                Self::report_member_mutation_failure(error, cx);
            })
        };

        SaiWs::request_slice(
            &self.unlink_chat_sai,
            rise_request::chat::UnlinkChat::new(chat_id.to_owned()),
            settings_write_opts(rise_wire_methods::chat::UNLINK_CHAT, false),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn get_chat_analytics_action(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if wire_user_id(cx).is_none() || chat_id.is_empty() {
            return;
        }
        chat_services(cx, |services, cx| services.hydrate_analytics_from_disk(chat_id, cx));

        let services = ChatServicesStore::shared(cx);
        let reflect_pending = !services.read(cx).analytics_by_chat.contains_key(chat_id);

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = typed_payload::<ChatAnalyticsGatewayDTO>(payload) else {
                    return;
                };
                chat_services(cx, |services, cx| {
                    services.analytics_by_chat.insert(chat_id.clone(), dto.to_model());
                    cx.notify();
                });
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                let message = format!("get_chat_analytics({chat_id}) error: {error}");
                Sai::log("Chat", move || message, SaiLogKind::Network, cx);
                let Some(failure) = CanonErrors::failure(error) else {
                    return;
                };
                if CanonErrors::category(failure.code.as_deref()) != ErrorCategory::Permission {
                    return;
                }
                Self::report_member_mutation_failure(error, cx);
            })
        };

        SaiWs::request_slice(
            &self.chat_analytics_sai,
            rise_request::chat::GetChatAnalytics::new(chat_id.to_owned()),
            settings_read_opts(
                rise_wire_methods::chat::GET_CHAT_ANALYTICS,
                ChatServicesStore::analytics_cache_key(chat_id),
                reflect_pending,
            ),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn update_permissions_action(
        &mut self,
        chat_id: &str,
        mutate: impl FnOnce(&mut ChatSettingsModel),
        cx: &mut Context<Self>,
    ) {
        if wire_user_id(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let previous = settings_of(chat_id, cx);
        let mut model = previous.clone().unwrap_or_else(|| empty_settings(chat_id));
        mutate(&mut model);

        let body = rise_request::chat::UpdateChatSettings {
            media_allow: Some(ChatPermissionJSON::serialize(&model.media_allow)),
            members_can_add_members: Some(model.members_can_add_members),
            members_can_change_pfp: Some(model.members_can_change_pfp),
            message_allow: Some(ChatPermissionJSON::serialize(&model.message_allow)),
            slow_mode: Some(model.slow_mode),
            ..rise_request::chat::UpdateChatSettings::new(chat_id.to_owned())
        };
        put_settings(chat_id, Some(model), cx);

        let success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = typed_payload::<ChatSettingsGatewayDTO>(payload) else {
                    return;
                };
                adopt_updated_settings(&chat_id, &dto, cx);
            })
        };
        let failure: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                let message = format!("update_chat_settings({chat_id}) permissions error: {error}");
                Sai::log("Chat", move || message, SaiLogKind::Network, cx);
                put_settings(&chat_id, previous.clone(), cx);
                Self::report_member_mutation_failure(error, cx);
            })
        };

        SaiWs::request_slice(
            &self.update_permissions_sai,
            body,
            settings_write_opts(rise_wire_methods::chat::UPDATE_CHAT_SETTINGS, true),
            Some(success),
            Some(failure),
            cx,
        );
    }
}
