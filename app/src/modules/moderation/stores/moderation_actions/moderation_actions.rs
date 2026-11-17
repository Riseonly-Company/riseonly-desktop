use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::rise_key::{bot as bot_keys, report as report_keys};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiTypedSlice, SaiWireEmptyDTO};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::moderation::stores::moderation_services::moderation_services::ModerationServicesStore;

use super::moderation_types::ModeratorKindFilter;
use super::moderation_wire_types::{
    AllModeratorsGatewayDTO, ModeratorProfileGatewayDTO, ModeratorStatsGatewayDTO, ModeratorWithUserWireDTO,
    ReportsQueueGatewayDTO,
};

pub type ModerationActionDone = Rc<dyn Fn(&mut App)>;

pub struct ModerationActionsStore {
    pub my_moderator_profile_sai: Rc<SaiTypedSlice<ModeratorProfileGatewayDTO>>,
    pub all_moderators_sai: Rc<SaiTypedSlice<AllModeratorsGatewayDTO>>,
    pub moderator_profile_sai: Rc<SaiTypedSlice<ModeratorWithUserWireDTO>>,
    pub update_permissions_sai: Rc<SaiTypedSlice<ModeratorWithUserWireDTO>>,
    pub remove_moderator_sai: Rc<SaiTypedSlice<SaiWireEmptyDTO>>,
    pub moderator_stats_sai: Rc<SaiTypedSlice<ModeratorStatsGatewayDTO>>,
    pub moderator_history_sai: Rc<SaiTypedSlice<ReportsQueueGatewayDTO>>,
}

impl ModerationActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            my_moderator_profile_sai: SaiTypedSlice::new(cx),
            all_moderators_sai: SaiTypedSlice::new(cx),
            moderator_profile_sai: SaiTypedSlice::new(cx),
            update_permissions_sai: SaiTypedSlice::new(cx),
            remove_moderator_sai: SaiTypedSlice::new(cx),
            moderator_stats_sai: SaiTypedSlice::new(cx),
            moderator_history_sai: SaiTypedSlice::new(cx),
        }
    }

    fn my_id(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .filter(|id| !id.is_empty())
    }

    fn paginated_options(
        method: &str,
        cache_key: String,
        relative_id: Option<i64>,
        fetch_if_have_data: bool,
        path_to_array: String,
        path_to_id: String,
    ) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::report::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: fetch_if_have_data,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: relative_id.is_none(),
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(path_to_array),
            path_to_id: Some(path_to_id),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: relative_id.map(|id| id.to_string()),
                response_relative_id_path: report_keys::camel::RELATIVE_ID.to_owned(),
                has_more_path: Some(report_keys::camel::IS_HAVE_MORE_BOT_OR_TOP.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn cached_read_options(method: &str, cache_key: String, fetch_if_have_data: bool) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::report::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: fetch_if_have_data,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn mutation_options(method: &str) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::report::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn decoded<T: DeserializeOwned + 'static>(payload: Option<Box<dyn Any>>) -> Option<T> {
        let payload = payload?;
        match payload.downcast::<T>() {
            Ok(dto) => Some(*dto),
            Err(payload) => SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>()),
        }
    }

    pub fn get_my_moderator_profile_action(&self, fetch_if_have_data: bool, cx: &mut App) {
        let Some(me) = Self::my_id(cx) else {
            return;
        };

        let cache_key = SaiCache::new(["getMyModeratorProfile", me.as_str()]).key;
        let body = rise_canon::rise_request::report::GetMyModeratorProfile {};
        let options =
            Self::cached_read_options(rise_canon::rise_wire_methods::report::GET_MY_MODERATOR_PROFILE, cache_key, fetch_if_have_data);

        let on_success: SaiWsOnSuccess = Rc::new(|payload: Option<Box<dyn Any>>, cx: &mut App| {
            let dto = Self::decoded::<ModeratorProfileGatewayDTO>(payload);
            ModerationServicesStore::shared(cx)
                .update(cx, |services, cx| services.get_my_moderator_profile_success_handler(dto, cx));
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            ModerationServicesStore::shared(cx)
                .update(cx, |services, cx| services.get_my_moderator_profile_error_handler(error, cx));
        });

        SaiWs::request_slice(&self.my_moderator_profile_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn report_chat_spam_action(&self, chat_id: &str, message_id: Option<&str>, cx: &mut App) {
        ModerationInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.report_chat_spam(chat_id, message_id, cx));
    }

    pub fn get_all_moderators_action(
        &self,
        relative_id: Option<i64>,
        kind: ModeratorKindFilter,
        fetch_if_have_data: bool,
        cx: &mut App,
    ) {
        if Self::my_id(cx).is_none() {
            return;
        }

        let cache_key = ModerationServicesStore::moderators_cache_key(kind, cx);
        let mut body = rise_canon::rise_request::report::GetAllModerators {
            limit: Some(ModerationServicesStore::moderators_limit()),
            up: Some(false),
            include_inactive: Some(true),
            ..rise_canon::rise_request::report::GetAllModerators::default()
        };
        if let Some(relative_id) = relative_id {
            body.relative_id = Some(relative_id);
        }
        if let Some(wire) = kind.wire_value() {
            body.kind = Some(wire.to_owned());
        }

        let options = Self::paginated_options(
            rise_canon::rise_wire_methods::report::GET_ALL_MODERATORS,
            cache_key,
            relative_id,
            fetch_if_have_data,
            report_keys::MODERATORS.to_owned(),
            format!("{}.{}", report_keys::PROFILE, bot_keys::USER_ID),
        );

        let on_success: SaiWsOnSuccess = Rc::new(|_payload: Option<Box<dyn Any>>, cx: &mut App| {
            ModerationServicesStore::shared(cx)
                .update(cx, |services, cx| services.get_all_moderators_success_handler(cx));
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            ModerationServicesStore::shared(cx)
                .update(cx, |services, cx| services.get_all_moderators_error_handler(error, cx));
        });

        SaiWs::request_slice(&self.all_moderators_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn get_moderator_profile_action(&self, target_user_id: &str, fetch_if_have_data: bool, cx: &mut App) {
        let Some(me) = Self::my_id(cx) else {
            return;
        };
        if target_user_id.is_empty() {
            return;
        }

        let cache_key = SaiCache::new(["getModeratorProfile", me.as_str(), target_user_id]).key;
        let body = rise_canon::rise_request::report::GetModeratorProfile { target_user_id: Some(target_user_id.to_owned()) };
        let options = Self::cached_read_options(rise_canon::rise_wire_methods::report::GET_MODERATOR_PROFILE, cache_key, fetch_if_have_data);

        let success_target = target_user_id.to_owned();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let row = Self::decoded::<ModeratorWithUserWireDTO>(payload);
            ModerationServicesStore::shared(cx).update(cx, |services, cx| {
                services.get_moderator_profile_success_handler(row, &success_target, cx)
            });
        });
        let error_target = target_user_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            ModerationServicesStore::shared(cx)
                .update(cx, |services, cx| services.get_moderator_profile_error_handler(error, &error_target, cx));
        });

        SaiWs::request_slice(&self.moderator_profile_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn update_moderator_permissions_action(
        &self,
        target_user_id: &str,
        permissions_json: Option<String>,
        is_active: Option<bool>,
        on_done: Option<ModerationActionDone>,
        cx: &mut App,
    ) {
        if Self::my_id(cx).is_none() || target_user_id.is_empty() {
            return;
        }

        let revert = ModerationServicesStore::shared(cx)
            .update(cx, |services, cx| services.apply_update_optimistic(target_user_id, is_active, cx));

        let mut body = rise_canon::rise_request::report::UpdateModeratorPermissions {
            target_user_id: Some(target_user_id.to_owned()),
            ..rise_canon::rise_request::report::UpdateModeratorPermissions::default()
        };
        if let Some(permissions_json) = permissions_json.filter(|json| !json.is_empty()) {
            body.permissions_json = Some(permissions_json);
        }
        if let Some(is_active) = is_active {
            body.is_active = Some(is_active);
        }

        let options = Self::mutation_options(rise_canon::rise_wire_methods::report::UPDATE_MODERATOR_PERMISSIONS);

        let success_target = target_user_id.to_owned();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let updated = Self::decoded::<ModeratorWithUserWireDTO>(payload);
            ModerationServicesStore::shared(cx).update(cx, |services, cx| {
                services.update_moderator_permissions_success_handler(updated, &success_target, cx)
            });
            if let Some(on_done) = &on_done {
                on_done(cx);
            }
        });
        let error_target = target_user_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let revert = revert.clone();
            ModerationServicesStore::shared(cx).update(cx, |services, cx| {
                services.update_moderator_permissions_error_handler(error, &error_target, revert, cx)
            });
        });

        SaiWs::request_slice(&self.update_permissions_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn remove_moderator_action(&self, target_user_id: &str, on_done: Option<ModerationActionDone>, cx: &mut App) {
        if Self::my_id(cx).is_none() || target_user_id.is_empty() {
            return;
        }

        let revert = ModerationServicesStore::shared(cx)
            .update(cx, |services, cx| services.apply_remove_moderator_optimistic(target_user_id, cx));

        let body = rise_canon::rise_request::report::RemoveModerator { target_user_id: Some(target_user_id.to_owned()) };
        let options = Self::mutation_options(rise_canon::rise_wire_methods::report::REMOVE_MODERATOR);

        let success_target = target_user_id.to_owned();
        let on_success: SaiWsOnSuccess = Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
            ModerationServicesStore::shared(cx)
                .update(cx, |services, cx| services.remove_moderator_success_handler(&success_target, cx));
            if let Some(on_done) = &on_done {
                on_done(cx);
            }
        });
        let error_target = target_user_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let revert = revert.clone();
            ModerationServicesStore::shared(cx).update(cx, |services, cx| {
                services.remove_moderator_error_handler(error, &error_target, revert, cx)
            });
        });

        SaiWs::request_slice(&self.remove_moderator_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn get_moderator_stats_action(&self, target_user_id: &str, fetch_if_have_data: bool, cx: &mut App) {
        let Some(me) = Self::my_id(cx) else {
            return;
        };
        if target_user_id.is_empty() {
            return;
        }

        let cache_key = SaiCache::new(["getModeratorStats", me.as_str(), target_user_id]).key;
        let body = rise_canon::rise_request::report::GetModeratorStats { target_user_id: Some(target_user_id.to_owned()) };
        let options = Self::cached_read_options(rise_canon::rise_wire_methods::report::GET_MODERATOR_STATS, cache_key, fetch_if_have_data);

        let success_target = target_user_id.to_owned();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let dto = Self::decoded::<ModeratorStatsGatewayDTO>(payload);
            ModerationServicesStore::shared(cx)
                .update(cx, |services, cx| services.get_moderator_stats_success_handler(dto, &success_target, cx));
        });
        let error_target = target_user_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            ModerationServicesStore::shared(cx)
                .update(cx, |services, cx| services.get_moderator_stats_error_handler(error, &error_target, cx));
        });

        SaiWs::request_slice(&self.moderator_stats_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn get_moderator_history_action(
        &self,
        target_user_id: &str,
        relative_id: Option<i64>,
        fetch_if_have_data: bool,
        cx: &mut App,
    ) {
        if Self::my_id(cx).is_none() || target_user_id.is_empty() {
            return;
        }

        let cache_key = ModerationServicesStore::moderator_history_cache_key(target_user_id, cx);
        let mut body = rise_canon::rise_request::report::GetModeratorHistory {
            limit: Some(ModerationServicesStore::history_limit()),
            target_user_id: Some(target_user_id.to_owned()),
            up: Some(false),
            ..rise_canon::rise_request::report::GetModeratorHistory::default()
        };
        if let Some(relative_id) = relative_id {
            body.relative_id = Some(relative_id);
        }

        let options = Self::paginated_options(
            rise_canon::rise_wire_methods::report::GET_MODERATOR_HISTORY,
            cache_key,
            relative_id,
            fetch_if_have_data,
            report_keys::REPORTS.to_owned(),
            report_keys::ID.to_owned(),
        );

        let success_target = target_user_id.to_owned();
        let on_success: SaiWsOnSuccess = Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
            ModerationServicesStore::shared(cx)
                .update(cx, |services, cx| services.get_moderator_history_success_handler(&success_target, cx));
        });
        let error_target = target_user_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            ModerationServicesStore::shared(cx)
                .update(cx, |services, cx| services.get_moderator_history_error_handler(error, &error_target, cx));
        });

        SaiWs::request_slice(&self.moderator_history_sai, body, options, Some(on_success), Some(on_error), cx);
    }
}
