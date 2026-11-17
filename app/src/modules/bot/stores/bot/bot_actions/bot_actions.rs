use std::any::Any;
use std::future::Future;
use std::rc::Rc;

use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::{RiseRequestBody, rise_key, rise_request, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::{CancellationError, RiseJSONValue};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::file_manager::{CocoaError, CocoaErrorCode};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::bot_inline_types::BotInlineResultModel;
use super::bot_inline_wire_types::{
    BotChatCommandsGatewayDTO, BotInlineCommandsGatewayDTO, BotInlineResultsGatewayDTO, BotMentionCandidatesGatewayDTO,
};
use super::bot_types::{
    BotCallbackEnqueueResult, BotCommandModel, BotMediaKind, BotMiniAppLaunchData, BotMiniAppModel,
    BotMiniAppUpdateValues, BotModel, BotTagAvailability, BotTagRules, BotTokenResult, BotUpdateValues,
    BotWebhookInfoModel,
};
use super::bot_wire_types::{
    BotCallbackEnqueueGatewayDTO, BotCallbackEnqueueGatewayDTOExt, BotCommandWireDTOExt, BotCommandsGatewayDTO,
    BotCreateGatewayDTO, BotGatewayDTO, BotListGatewayDTO, BotMiniAppGatewayDTO, BotMiniAppLaunchGatewayDTO,
    BotMiniAppLaunchGatewayDTOExt, BotMiniAppListGatewayDTO, BotMiniAppWireDTOExt, BotPublicCommandsGatewayDTO,
    BotStatusGatewayDTO, BotTagAvailabilityGatewayDTO, BotTagAvailabilityGatewayDTOExt, BotTokenGatewayDTO,
    BotTokenGatewayDTOExt, BotWebhookGatewayDTO, BotWebhookInfoWireDTOExt, BotWireDTOExt,
};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::{
    SaiWsOptimisticLocalFile, SaiWsOptimisticUpdate,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::core::rise_conversation_request_state::RiseConversationStateError;
use crate::modules::chat::engine::messages::rise_message_contracts::RiseMessageRepositoryError;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    MessagePayloadCodec, MessageWireDTO,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::share_extension::services::rise_share_app_bridge::RiseShareAppBridge;

pub type BotActionResult<T> = Rc<dyn Fn(T, &mut App)>;
pub type BotActionFinished = Rc<dyn Fn(&mut App)>;
pub type BotActionError = Rc<dyn Fn(&anyhow::Error, &mut App)>;
pub type BotCreateResult = Rc<dyn Fn(BotModel, String, &mut App)>;
pub type BotInlineSendResult = Rc<dyn Fn(bool, Option<&anyhow::Error>, &mut App)>;

struct InlineComposerRequest {
    action: &'static str,
    invalid_fallback: &'static str,
    rejected_fallback: &'static str,
    composer_scope: String,
    cache_key: String,
    generation: Option<i64>,
}

pub struct BotActionsStore {
    pub create_bot_sai: Rc<SaiTypedSlice<BotCreateGatewayDTO>>,
    pub list_bots_sai: Rc<SaiTypedSlice<BotListGatewayDTO>>,
    pub get_bot_sai: Rc<SaiTypedSlice<BotGatewayDTO>>,
    pub get_bot_by_tag_sai: Rc<SaiTypedSlice<BotGatewayDTO>>,
    pub check_bot_tag_availability_sai: Rc<SaiTypedSlice<BotTagAvailabilityGatewayDTO>>,
    pub update_bot_sai: Rc<SaiTypedSlice<BotGatewayDTO>>,
    pub delete_bot_sai: Rc<SaiTypedSlice<BotStatusGatewayDTO>>,
    pub reveal_bot_token_sai: Rc<SaiTypedSlice<BotTokenGatewayDTO>>,
    pub rotate_bot_token_sai: Rc<SaiTypedSlice<BotTokenGatewayDTO>>,
    pub set_bot_commands_sai: Rc<SaiTypedSlice<BotCommandsGatewayDTO>>,
    pub get_bot_commands_sai: Rc<SaiTypedSlice<BotCommandsGatewayDTO>>,
    pub get_public_bot_commands_sai: Rc<SaiTypedSlice<BotPublicCommandsGatewayDTO>>,
    pub delete_bot_commands_sai: Rc<SaiTypedSlice<BotCommandsGatewayDTO>>,
    pub set_bot_webhook_sai: Rc<SaiTypedSlice<BotWebhookGatewayDTO>>,
    pub delete_bot_webhook_sai: Rc<SaiTypedSlice<BotWebhookGatewayDTO>>,
    pub get_bot_webhook_info_sai: Rc<SaiTypedSlice<BotWebhookGatewayDTO>>,
    pub create_mini_app_sai: Rc<SaiTypedSlice<BotMiniAppGatewayDTO>>,
    pub list_mini_apps_sai: Rc<SaiTypedSlice<BotMiniAppListGatewayDTO>>,
    pub list_public_mini_apps_sai: Rc<SaiTypedSlice<BotMiniAppListGatewayDTO>>,
    pub get_mini_app_sai: Rc<SaiTypedSlice<BotMiniAppGatewayDTO>>,
    pub update_mini_app_sai: Rc<SaiTypedSlice<BotMiniAppGatewayDTO>>,
    pub delete_mini_app_sai: Rc<SaiTypedSlice<BotStatusGatewayDTO>>,
    pub create_mini_app_launch_data_sai: Rc<SaiTypedSlice<BotMiniAppLaunchGatewayDTO>>,
    pub get_mini_app_launch_sai: Rc<SaiTypedSlice<BotMiniAppLaunchGatewayDTO>>,
    pub resolve_mini_app_link_sai: Rc<SaiTypedSlice<BotMiniAppLaunchGatewayDTO>>,
    pub enqueue_callback_query_sai: Rc<SaiTypedSlice<BotCallbackEnqueueGatewayDTO>>,
    pub get_mention_candidates_sai: Rc<SaiTypedSlice<BotMentionCandidatesGatewayDTO>>,
    pub get_inline_bot_commands_sai: Rc<SaiTypedSlice<BotInlineCommandsGatewayDTO>>,
    pub get_chat_bot_commands_sai: Rc<SaiTypedSlice<BotChatCommandsGatewayDTO>>,
    pub query_inline_results_sai: Rc<SaiTypedSlice<BotInlineResultsGatewayDTO>>,
}

impl BotActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            create_bot_sai: SaiTypedSlice::new(cx),
            list_bots_sai: SaiTypedSlice::new(cx),
            get_bot_sai: SaiTypedSlice::new(cx),
            get_bot_by_tag_sai: SaiTypedSlice::new(cx),
            check_bot_tag_availability_sai: SaiTypedSlice::new(cx),
            update_bot_sai: SaiTypedSlice::new(cx),
            delete_bot_sai: SaiTypedSlice::new(cx),
            reveal_bot_token_sai: SaiTypedSlice::new(cx),
            rotate_bot_token_sai: SaiTypedSlice::new(cx),
            set_bot_commands_sai: SaiTypedSlice::new(cx),
            get_bot_commands_sai: SaiTypedSlice::new(cx),
            get_public_bot_commands_sai: SaiTypedSlice::new(cx),
            delete_bot_commands_sai: SaiTypedSlice::new(cx),
            set_bot_webhook_sai: SaiTypedSlice::new(cx),
            delete_bot_webhook_sai: SaiTypedSlice::new(cx),
            get_bot_webhook_info_sai: SaiTypedSlice::new(cx),
            create_mini_app_sai: SaiTypedSlice::new(cx),
            list_mini_apps_sai: SaiTypedSlice::new(cx),
            list_public_mini_apps_sai: SaiTypedSlice::new(cx),
            get_mini_app_sai: SaiTypedSlice::new(cx),
            update_mini_app_sai: SaiTypedSlice::new(cx),
            delete_mini_app_sai: SaiTypedSlice::new(cx),
            create_mini_app_launch_data_sai: SaiTypedSlice::new(cx),
            get_mini_app_launch_sai: SaiTypedSlice::new(cx),
            resolve_mini_app_link_sai: SaiTypedSlice::new(cx),
            enqueue_callback_query_sai: SaiTypedSlice::new(cx),
            get_mention_candidates_sai: SaiTypedSlice::new(cx),
            get_inline_bot_commands_sai: SaiTypedSlice::new(cx),
            get_chat_bot_commands_sai: SaiTypedSlice::new(cx),
            query_inline_results_sai: SaiTypedSlice::new(cx),
        }
    }

    fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .filter(|user_id| !user_id.is_empty())
    }

    fn slices_did_change(cx: &mut App) {
        cx.defer(|cx| {
            Self::shared(cx).update(cx, |_, cx| cx.notify());
        });
    }

    fn services<Output>(
        cx: &mut App,
        work: impl FnOnce(&mut BotServicesStore, &mut Context<BotServicesStore>) -> Output,
    ) -> Output {
        BotServicesStore::shared(cx).update(cx, work)
    }

    fn log_network(text: String, cx: &mut App) {
        Sai::log("Bot", move || text, SaiLogKind::Network, cx);
    }

    fn log_only_on_error(label: &'static str) -> SaiWsOnError {
        Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("{label} error: {error}"), cx);
        })
    }

    fn ignore_success() -> SaiWsOnSuccess {
        Rc::new(|_payload: Option<Box<dyn Any>>, _cx: &mut App| {})
    }

    fn typed_payload<T: 'static>(payload: Option<Box<dyn Any>>) -> Option<T> {
        payload?.downcast::<T>().ok().map(|decoded| *decoded)
    }

    fn decoded_payload<T: DeserializeOwned + 'static>(payload: Option<Box<dyn Any>>) -> Option<T> {
        match payload?.downcast::<T>() {
            Ok(decoded) => Some(*decoded),
            Err(payload) => SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>()),
        }
    }

    fn send<T, Body>(
        slice: &Rc<SaiTypedSlice<T>>,
        body: Body,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: SaiWsOnError,
        cx: &mut App,
    ) where
        T: DeserializeOwned + 'static,
        Body: RiseRequestBody + 'static,
    {
        let on_success = on_success.map(|on_success| -> SaiWsOnSuccess {
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                Self::slices_did_change(cx);
                on_success(payload, cx);
            })
        });
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::slices_did_change(cx);
            on_error(error, cx);
        });
        SaiWs::request_slice(slice, body, options, on_success, Some(on_error), cx);
        Self::slices_did_change(cx);
    }

    fn relative_id_pagination(relative_id: Option<&str>, response_relative_id_path: Option<&str>) -> SaiWsPaginationOptions {
        let defaults = SaiWsPaginationRelativeIdArgs::default();
        SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
            relative_id: relative_id.map(str::to_owned),
            response_relative_id_path: response_relative_id_path
                .map_or(defaults.response_relative_id_path.clone(), str::to_owned),
            has_more_path: Some(rise_key::bot::HAS_MORE.to_owned()),
            duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
            ..defaults
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_bot_action(
        name: &str,
        tag: &str,
        description: &str,
        avatar_url: &str,
        banner_url: &str,
        can_join_groups: bool,
        privacy_mode: bool,
        on_result: Option<BotCreateResult>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if trimming_whitespaces_and_newlines(name).is_empty() || !BotTagRules::is_valid(tag) {
            return;
        }

        let request_id = Uuid::new_v4().uuid_string().to_lowercase();
        let snapshots = Rc::new(BotServicesStore::apply_create_bot_optimistic(
            &user_id,
            &request_id,
            name,
            tag,
            description,
            avatar_url,
            banner_url,
            can_join_groups,
            privacy_mode,
            cx,
        ));

        let body = rise_request::bot::CreateBot {
            avatar_url: Some(avatar_url.to_owned()),
            banner_url: Some(banner_url.to_owned()),
            can_join_groups: Some(can_join_groups),
            description: Some(description.to_owned()),
            name: Some(name.to_owned()),
            privacy_mode: Some(privacy_mode),
            request_id: Some(request_id.clone()),
            tag: Some(tag.to_owned()),
            ..Default::default()
        };

        let on_success: SaiWsOnSuccess = {
            let snapshots = Rc::clone(&snapshots);
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let created = Self::typed_payload::<BotCreateGatewayDTO>(payload)
                    .filter(|dto| !dto.token.is_empty())
                    .and_then(|dto| dto.bot.map(|bot| (bot, dto.token)));
                let Some((bot, token)) = created else {
                    BotServicesStore::restore_documents(&snapshots, cx);
                    return;
                };
                BotServicesStore::finalize_created_bot(&user_id, &request_id, &bot, cx);
                if let Some(on_result) = &on_result {
                    on_result(bot.to_bot_model(), token, cx);
                }
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            BotServicesStore::restore_documents(&snapshots, cx);
            Self::log_network(format!("create_bot error: {error}"), cx);
        });

        let slice = Self::shared(cx).read(cx).create_bot_sai.clone();
        let options = BotServicesStore::sensitive_request_options(rise_wire_methods::bot::CREATE_BOT, true);
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn list_bots_action(relative_id: Option<&str>, reflect_pending_in_ui: bool, cx: &mut App) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };

        let first_page = relative_id.is_none();
        let body = rise_request::bot::ListBots {
            limit: Some(BotServicesStore::page_limit()),
            relative_id: relative_id.map(str::to_owned),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::LIST_BOTS.to_owned(),
            cache_key: Some(BotServicesStore::owned_bots_cache_key(&user_id)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: first_page && reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            path_to_array: Some(rise_key::bot::BOTS.to_owned()),
            path_to_id: Some(rise_key::bot::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(relative_id, None)),
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Self::shared(cx).read(cx).list_bots_sai.clone();
        Self::send(&slice, body, options, None, Self::log_only_on_error("list_bots"), cx);
    }

    pub fn get_bot_action(bot_id: &str, reflect_pending_in_ui: bool, cx: &mut App) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() {
            return;
        }

        let body = rise_request::bot::GetBot { bot_id: Some(bot_id.to_owned()) };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::GET_BOT.to_owned(),
            cache_key: Some(BotServicesStore::bot_cache_key(&user_id, bot_id)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Self::shared(cx).read(cx).get_bot_sai.clone();
        Self::send(&slice, body, options, Some(Self::ignore_success()), Self::log_only_on_error("get_bot"), cx);
    }

    pub fn get_bot_by_tag_action(tag: &str, reflect_pending_in_ui: bool, cx: &mut App) {
        if Self::effective_user_id_for_wire(cx).is_none() || !BotTagRules::is_valid(tag) {
            return;
        }

        let body = rise_request::bot::GetBotByTag { tag: Some(tag.to_owned()) };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::GET_BOT_BY_TAG.to_owned(),
            cache_key: Some(BotServicesStore::bot_by_tag_cache_key(tag)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Self::shared(cx).read(cx).get_bot_by_tag_sai.clone();
        Self::send(&slice, body, options, Some(Self::ignore_success()), Self::log_only_on_error("get_bot_by_tag"), cx);
    }

    pub fn resolve_start_link_action(tag: &str, cx: &mut App) -> Task<anyhow::Result<BotModel>> {
        let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() else {
            return Task::ready(Err(RiseConversationStateError::MissingActiveSession.into()));
        };
        let tag = tag.to_owned();
        cx.spawn(async move |_: &mut AsyncApp| {
            Self::on_account_runtime(async move { domain.resolve_bot_start_link(&tag).await }).await?
        })
    }

    pub fn check_bot_tag_availability_action(
        tag: &str,
        on_result: Option<BotActionResult<BotTagAvailability>>,
        cx: &mut App,
    ) {
        if Self::effective_user_id_for_wire(cx).is_none() || !BotTagRules::is_valid(tag) {
            return;
        }

        let body = rise_request::bot::CheckBotTagAvailability { tag: Some(tag.to_owned()) };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::CHECK_BOT_TAG_AVAILABILITY.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(dto) = Self::typed_payload::<BotTagAvailabilityGatewayDTO>(payload) else {
                return;
            };
            if let Some(on_result) = &on_result {
                on_result(dto.to_bot_tag_availability(), cx);
            }
        });

        let slice = Self::shared(cx).read(cx).check_bot_tag_availability_sai.clone();
        Self::send(
            &slice,
            body,
            options,
            Some(on_success),
            Self::log_only_on_error("check_bot_tag_availability"),
            cx,
        );
    }

    pub fn update_bot_action(
        bot_id: &str,
        values: &BotUpdateValues,
        on_result: Option<BotActionResult<BotModel>>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        let slice = Self::shared(cx).read(cx).update_bot_sai.clone();
        if bot_id.is_empty() || slice.is_pending() {
            return;
        }
        if let Some(tag) = &values.tag
            && !BotTagRules::is_valid(tag)
        {
            return;
        }

        slice.mark_pending();
        let previous_tag = BotServicesStore::bot(bot_id, cx).map(|bot| bot.tag);
        let snapshots = Rc::new(BotServicesStore::bot_mutation_snapshot(&user_id, bot_id, cx));
        BotServicesStore::apply_bot_update_optimistic(&user_id, bot_id, values, cx);

        let body = rise_request::bot::UpdateBot {
            avatar_url: values.avatar_url.clone(),
            banner_url: values.banner_url.clone(),
            bot_id: Some(bot_id.to_owned()),
            can_join_groups: values.can_join_groups,
            description: values.description.clone(),
            name: values.name.clone(),
            privacy_mode: values.privacy_mode,
            tag: values.tag.clone(),
            ..Default::default()
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::UPDATE_BOT.to_owned(),
            reflect_pending_in_ui: true,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = {
            let snapshots = Rc::clone(&snapshots);
            let slice = Rc::clone(&slice);
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let dto = Self::typed_payload::<BotGatewayDTO>(payload);
                let Some(bot) = dto.and_then(|dto| dto.bot) else {
                    BotServicesStore::restore_documents(&snapshots, cx);
                    slice.mark_rejected(&anyhow::Error::new(CocoaError::new(CocoaErrorCode::CoderReadCorrupt)));
                    return;
                };
                BotServicesStore::confirm_updated_bot(&user_id, &bot, previous_tag.as_deref(), cx);
                if let Some(on_result) = &on_result {
                    on_result(bot.to_bot_model(), cx);
                }
            })
        };
        let on_error: SaiWsOnError = {
            let slice = Rc::clone(&slice);
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                BotServicesStore::restore_documents(&snapshots, cx);
                slice.mark_rejected(error);
                Self::log_network(format!("update_bot error: {error}"), cx);
            })
        };

        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn delete_bot_action(bot_id: &str, on_result: Option<BotActionFinished>, cx: &mut App) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() {
            return;
        }

        let snapshots = Rc::new(BotServicesStore::bot_mutation_snapshot(&user_id, bot_id, cx));
        BotServicesStore::apply_delete_bot_optimistic(&user_id, bot_id, cx);

        let body = rise_request::bot::DeleteBot { bot_id: Some(bot_id.to_owned()) };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::DELETE_BOT.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_result) = &on_result {
                on_result(cx);
            }
        });
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            BotServicesStore::restore_documents(&snapshots, cx);
            Self::log_network(format!("delete_bot error: {error}"), cx);
        });

        let slice = Self::shared(cx).read(cx).delete_bot_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn update_bot_avatar_action(
        bot_id: &str,
        local_file: SaiWsOptimisticLocalFile,
        on_result: Option<BotActionResult<String>>,
        cx: &mut App,
    ) {
        Self::update_bot_media_action(bot_id, BotMediaKind::Avatar, local_file, on_result, cx);
    }

    pub fn update_bot_banner_action(
        bot_id: &str,
        local_file: SaiWsOptimisticLocalFile,
        on_result: Option<BotActionResult<String>>,
        cx: &mut App,
    ) {
        Self::update_bot_media_action(bot_id, BotMediaKind::Banner, local_file, on_result, cx);
    }

    fn update_bot_media_action(
        bot_id: &str,
        kind: BotMediaKind,
        local_file: SaiWsOptimisticLocalFile,
        on_result: Option<BotActionResult<String>>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() {
            return;
        }

        let slice = Self::shared(cx).read(cx).update_bot_sai.clone();
        let previous_tag = BotServicesStore::bot(bot_id, cx).map(|bot| bot.tag);
        let snapshots = Rc::new(BotServicesStore::bot_mutation_snapshot(&user_id, bot_id, cx));
        Self::services(cx, |service, cx| service.begin_media_upload(bot_id, kind, cx));
        let bot_id = bot_id.to_owned();

        let on_remote_url = {
            let user_id = user_id.clone();
            let bot_id = bot_id.clone();
            move |url: String, cx: &mut App| {
                BotServicesStore::apply_bot_media_optimistic(&user_id, &bot_id, kind, &url, cx);
            }
        };
        let on_failure = {
            let slice = Rc::clone(&slice);
            move |error: &anyhow::Error, _cx: &mut App| {
                slice.mark_rejected(error);
            }
        };
        let mut optimistic = SaiWsOptimisticUpdate::new();
        optimistic.enabled = true;
        optimistic.files =
            Some(BotServicesStore::make_media_optimistic_files(&bot_id, kind, local_file, on_remote_url, on_failure));

        let body = rise_request::bot::UpdateBot { bot_id: Some(bot_id.clone()), ..Default::default() };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::UPDATE_BOT.to_owned(),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            optimistic_update: Some(optimistic),
            track_until_acknowledged: true,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = {
            let snapshots = Rc::clone(&snapshots);
            let bot_id = bot_id.clone();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let dto = Self::typed_payload::<BotGatewayDTO>(payload);
                let Some(bot) = dto.and_then(|dto| dto.bot) else {
                    BotServicesStore::restore_documents(&snapshots, cx);
                    Self::services(cx, |service, cx| service.finish_media_upload(&bot_id, kind, false, cx));
                    return;
                };
                BotServicesStore::confirm_updated_bot(&user_id, &bot, previous_tag.as_deref(), cx);
                Self::services(cx, |service, cx| service.finish_media_upload(&bot_id, kind, true, cx));
                let result = if kind == BotMediaKind::Avatar { bot.avatar_url } else { bot.banner_url };
                if let Some(on_result) = &on_result {
                    on_result(result, cx);
                }
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            BotServicesStore::restore_documents(&snapshots, cx);
            Self::services(cx, |service, cx| service.finish_media_upload(&bot_id, kind, false, cx));
            Self::log_network(format!("update_bot media error: {error}"), cx);
        });

        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn reveal_bot_token_action(bot_id: &str, on_result: Option<BotActionResult<BotTokenResult>>, cx: &mut App) {
        let slice = Self::shared(cx).read(cx).reveal_bot_token_sai.clone();
        Self::request_token_action(&slice, rise_wire_methods::bot::REVEAL_BOT_TOKEN, bot_id, on_result, cx);
    }

    pub fn rotate_bot_token_action(bot_id: &str, on_result: Option<BotActionResult<BotTokenResult>>, cx: &mut App) {
        let slice = Self::shared(cx).read(cx).rotate_bot_token_sai.clone();
        Self::request_token_action(&slice, rise_wire_methods::bot::ROTATE_BOT_TOKEN, bot_id, on_result, cx);
    }

    fn request_token_action(
        slice: &Rc<SaiTypedSlice<BotTokenGatewayDTO>>,
        method: &'static str,
        bot_id: &str,
        on_result: Option<BotActionResult<BotTokenResult>>,
        cx: &mut App,
    ) {
        if Self::effective_user_id_for_wire(cx).is_none() || bot_id.is_empty() {
            return;
        }

        let options = BotServicesStore::sensitive_request_options(method, true);
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(dto) = Self::typed_payload::<BotTokenGatewayDTO>(payload) else {
                return;
            };
            if let Some(on_result) = &on_result {
                on_result(dto.to_bot_token_result(), cx);
            }
        });
        let on_error = Self::log_only_on_error(method);
        let bot_id = Some(bot_id.to_owned());
        if method == rise_wire_methods::bot::ROTATE_BOT_TOKEN {
            let body = rise_request::bot::RotateBotToken { bot_id };
            Self::send(slice, body, options, Some(on_success), on_error, cx);
        } else {
            let body = rise_request::bot::RevealBotToken { bot_id };
            Self::send(slice, body, options, Some(on_success), on_error, cx);
        }
    }

    pub fn set_bot_commands_action(
        bot_id: &str,
        commands: &[BotCommandModel],
        scope: &str,
        language_code: &str,
        on_result: Option<BotActionResult<Vec<BotCommandModel>>>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() {
            return;
        }

        let snapshots = Rc::new(BotServicesStore::commands_snapshot(&user_id, bot_id, scope, language_code, cx));
        BotServicesStore::apply_commands_optimistic(&user_id, bot_id, scope, language_code, commands, cx);

        let body = rise_request::bot::SetBotCommands {
            bot_id: Some(bot_id.to_owned()),
            commands: Some(
                commands
                    .iter()
                    .map(|command| rise_request::bot::set_bot_commands::CommandsElement {
                        command: Some(command.command.clone()),
                        description: Some(command.description.clone()),
                    })
                    .collect(),
            ),
            language_code: Some(language_code.to_owned()),
            scope: Some(scope.to_owned()),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::SET_BOT_COMMANDS.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = {
            let snapshots = Rc::clone(&snapshots);
            let bot_id = bot_id.to_owned();
            let scope = scope.to_owned();
            let language_code = language_code.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = Self::typed_payload::<BotCommandsGatewayDTO>(payload) else {
                    BotServicesStore::restore_documents(&snapshots, cx);
                    return;
                };
                BotServicesStore::confirm_commands(&user_id, &bot_id, &scope, &language_code, &dto, cx);
                if let Some(on_result) = &on_result {
                    on_result(dto.commands.iter().map(BotCommandWireDTOExt::to_bot_command_model).collect(), cx);
                }
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            BotServicesStore::restore_documents(&snapshots, cx);
            Self::log_network(format!("set_bot_commands error: {error}"), cx);
        });

        let slice = Self::shared(cx).read(cx).set_bot_commands_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn get_bot_commands_action(
        bot_id: &str,
        scope: &str,
        language_code: &str,
        reflect_pending_in_ui: bool,
        on_result: Option<BotActionResult<Vec<BotCommandModel>>>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() {
            return;
        }

        let body = rise_request::bot::GetBotCommands {
            bot_id: Some(bot_id.to_owned()),
            language_code: Some(language_code.to_owned()),
            scope: Some(scope.to_owned()),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::GET_BOT_COMMANDS.to_owned(),
            cache_key: Some(BotServicesStore::commands_cache_key(&user_id, bot_id, scope, language_code)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            allows_deferred_send: false,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            path_to_array: Some(rise_key::bot::COMMANDS.to_owned()),
            path_to_id: Some(rise_key::bot::COMMAND.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(dto) = Self::typed_payload::<BotCommandsGatewayDTO>(payload) else {
                return;
            };
            if let Some(on_result) = &on_result {
                on_result(dto.commands.iter().map(BotCommandWireDTOExt::to_bot_command_model).collect(), cx);
            }
        });

        let slice = Self::shared(cx).read(cx).get_bot_commands_sai.clone();
        Self::send(&slice, body, options, Some(on_success), Self::log_only_on_error("get_bot_commands"), cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_public_bot_commands_action(
        bot_id: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        chat_id: Option<&str>,
        language_code: &str,
        chat_type: Option<&str>,
        reflect_pending_in_ui: bool,
        cx: &mut App,
    ) {
        if Self::effective_user_id_for_wire(cx).is_none() {
            return;
        }
        if ![bot_id, tag, bot_user_id].into_iter().any(|value| value.is_some_and(|value| !value.is_empty())) {
            return;
        }

        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let body = rise_request::bot::GetPublicBotCommands {
            bot_id: bot_id.map(str::to_owned),
            bot_user_id: bot_user_id.map(str::to_owned),
            chat_id: chat_id.map(str::to_owned),
            chat_type: chat_type.map(str::to_owned),
            language_code: Some(language_code.to_owned()),
            tag: tag.map(str::to_owned),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::GET_PUBLIC_BOT_COMMANDS.to_owned(),
            cache_key: Some(BotServicesStore::public_commands_cache_key(
                bot_id,
                tag,
                bot_user_id,
                chat_id,
                language_code,
                chat_type,
            )),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            allows_deferred_send: false,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            path_to_array: Some(rise_key::bot::COMMANDS.to_owned()),
            path_to_id: Some(rise_key::bot::COMMAND.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Self::shared(cx).read(cx).get_public_bot_commands_sai.clone();
        let on_error: SaiWsOnError = {
            let slice = Rc::clone(&slice);
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Self::log_network(format!("get_public_bot_commands error: {error}"), cx);
                if !SaiAccountCacheScope::is_current_account_epoch(account_id.as_deref(), account_epoch.as_deref()) {
                    return;
                }
                slice.mark_rejected(error);
            })
        };
        Self::send(&slice, body, options, Some(Self::ignore_success()), on_error, cx);
    }

    pub fn delete_bot_commands_action(
        bot_id: &str,
        scope: &str,
        language_code: &str,
        on_result: Option<BotActionFinished>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        let slice = Self::shared(cx).read(cx).delete_bot_commands_sai.clone();
        if bot_id.is_empty() || slice.is_pending() {
            return;
        }

        let snapshots = BotServicesStore::commands_snapshot(&user_id, bot_id, scope, language_code, cx);
        let snapshots = Rc::new(snapshots);
        slice.mark_pending();
        BotServicesStore::apply_commands_optimistic(&user_id, bot_id, scope, language_code, &[], cx);

        let body = rise_request::bot::DeleteBotCommands {
            bot_id: Some(bot_id.to_owned()),
            language_code: Some(language_code.to_owned()),
            scope: Some(scope.to_owned()),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::DELETE_BOT_COMMANDS.to_owned(),
            reflect_pending_in_ui: true,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = {
            let snapshots = Rc::clone(&snapshots);
            let bot_id = bot_id.to_owned();
            let scope = scope.to_owned();
            let language_code = language_code.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = Self::typed_payload::<BotCommandsGatewayDTO>(payload) else {
                    BotServicesStore::restore_documents(&snapshots, cx);
                    return;
                };
                BotServicesStore::confirm_commands(&user_id, &bot_id, &scope, &language_code, &dto, cx);
                if let Some(on_result) = &on_result {
                    on_result(cx);
                }
            })
        };
        let on_error: SaiWsOnError = {
            let slice = Rc::clone(&slice);
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                slice.mark_rejected(error);
                BotServicesStore::restore_documents(&snapshots, cx);
                Self::log_network(format!("delete_bot_commands error: {error}"), cx);
            })
        };

        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_bot_webhook_action(
        bot_id: &str,
        url: &str,
        secret_token: &str,
        max_connections: i64,
        allowed_updates: &[String],
        drop_pending_updates: bool,
        on_result: Option<BotActionResult<Option<BotWebhookInfoModel>>>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() || url.is_empty() {
            return;
        }

        let snapshots = Rc::new(BotServicesStore::webhook_snapshot(&user_id, bot_id, cx));
        BotServicesStore::apply_webhook_optimistic(&user_id, bot_id, url, max_connections, allowed_updates, true, cx);

        let body = rise_request::bot::SetBotWebhook {
            allowed_updates: Some(allowed_updates.to_vec()),
            bot_id: Some(bot_id.to_owned()),
            drop_pending_updates: Some(drop_pending_updates),
            max_connections: Some(max_connections),
            secret_token: Some(secret_token.to_owned()),
            url: Some(url.to_owned()),
        };

        let on_success: SaiWsOnSuccess = {
            let snapshots = Rc::clone(&snapshots);
            let bot_id = bot_id.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = Self::typed_payload::<BotWebhookGatewayDTO>(payload) else {
                    BotServicesStore::restore_documents(&snapshots, cx);
                    return;
                };
                BotServicesStore::confirm_webhook(&user_id, &bot_id, &dto, cx);
                if let Some(on_result) = &on_result {
                    on_result(dto.webhook.as_ref().map(BotWebhookInfoWireDTOExt::to_bot_webhook_info_model), cx);
                }
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            BotServicesStore::restore_documents(&snapshots, cx);
            Self::log_network(format!("set_bot_webhook error: {error}"), cx);
        });

        let slice = Self::shared(cx).read(cx).set_bot_webhook_sai.clone();
        let options = BotServicesStore::sensitive_request_options(rise_wire_methods::bot::SET_BOT_WEBHOOK, false);
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn delete_bot_webhook_action(
        bot_id: &str,
        drop_pending_updates: bool,
        on_result: Option<BotActionFinished>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() {
            return;
        }

        let snapshots = Rc::new(BotServicesStore::webhook_snapshot(&user_id, bot_id, cx));
        BotServicesStore::apply_webhook_optimistic(&user_id, bot_id, "", 0, &[], false, cx);

        let body = rise_request::bot::DeleteBotWebhook {
            bot_id: Some(bot_id.to_owned()),
            drop_pending_updates: Some(drop_pending_updates),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::DELETE_BOT_WEBHOOK.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = {
            let snapshots = Rc::clone(&snapshots);
            let bot_id = bot_id.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = Self::typed_payload::<BotWebhookGatewayDTO>(payload) else {
                    BotServicesStore::restore_documents(&snapshots, cx);
                    return;
                };
                BotServicesStore::confirm_webhook(&user_id, &bot_id, &dto, cx);
                if let Some(on_result) = &on_result {
                    on_result(cx);
                }
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            BotServicesStore::restore_documents(&snapshots, cx);
            Self::log_network(format!("delete_bot_webhook error: {error}"), cx);
        });

        let slice = Self::shared(cx).read(cx).delete_bot_webhook_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn get_bot_webhook_info_action(bot_id: &str, reflect_pending_in_ui: bool, cx: &mut App) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() {
            return;
        }

        let body = rise_request::bot::GetBotWebhookInfo { bot_id: Some(bot_id.to_owned()) };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::GET_BOT_WEBHOOK_INFO.to_owned(),
            cache_key: Some(BotServicesStore::webhook_cache_key(&user_id, bot_id)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            allows_deferred_send: false,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Self::shared(cx).read(cx).get_bot_webhook_info_sai.clone();
        Self::send(
            &slice,
            body,
            options,
            Some(Self::ignore_success()),
            Self::log_only_on_error("get_bot_webhook_info"),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_mini_app_action(
        bot_id: &str,
        short_name: &str,
        title: &str,
        description: &str,
        production_url: &str,
        development_url: &str,
        icon_url: &str,
        on_result: Option<BotActionResult<BotMiniAppModel>>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() || short_name.is_empty() || title.is_empty() || production_url.is_empty() {
            return;
        }

        let temp_id = format!("temp_{}", Uuid::new_v4().uuid_string().to_lowercase());
        let snapshots = Rc::new(BotServicesStore::mini_app_mutation_snapshot(&user_id, bot_id, None, cx));
        BotServicesStore::apply_create_mini_app_optimistic(
            &user_id,
            bot_id,
            &temp_id,
            short_name,
            title,
            description,
            production_url,
            development_url,
            icon_url,
            cx,
        );

        let body = rise_request::bot::CreateMiniApp {
            bot_id: Some(bot_id.to_owned()),
            description: Some(description.to_owned()),
            development_url: Some(development_url.to_owned()),
            icon_url: Some(icon_url.to_owned()),
            production_url: Some(production_url.to_owned()),
            short_name: Some(short_name.to_owned()),
            title: Some(title.to_owned()),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::CREATE_MINI_APP.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = {
            let snapshots = Rc::clone(&snapshots);
            let bot_id = bot_id.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let dto = Self::typed_payload::<BotMiniAppGatewayDTO>(payload);
                let Some(mini_app) = dto.and_then(|dto| dto.mini_app) else {
                    BotServicesStore::restore_documents(&snapshots, cx);
                    return;
                };
                BotServicesStore::finalize_created_mini_app(&user_id, &bot_id, &temp_id, &mini_app, cx);
                if let Some(on_result) = &on_result {
                    on_result(mini_app.to_bot_mini_app_model(), cx);
                }
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            BotServicesStore::restore_documents(&snapshots, cx);
            Self::log_network(format!("create_mini_app error: {error}"), cx);
        });

        let slice = Self::shared(cx).read(cx).create_mini_app_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn list_mini_apps_action(bot_id: &str, relative_id: Option<&str>, reflect_pending_in_ui: bool, cx: &mut App) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() {
            return;
        }

        let first_page = relative_id.is_none();
        let body = rise_request::bot::ListMiniApps {
            bot_id: Some(bot_id.to_owned()),
            limit: Some(BotServicesStore::page_limit()),
            relative_id: relative_id.map(str::to_owned),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::LIST_MINI_APPS.to_owned(),
            cache_key: Some(BotServicesStore::mini_apps_cache_key(&user_id, bot_id)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: first_page && reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            path_to_array: Some(rise_key::bot::MINI_APPS.to_owned()),
            path_to_id: Some(rise_key::bot::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(relative_id, None)),
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Self::shared(cx).read(cx).list_mini_apps_sai.clone();
        Self::send(&slice, body, options, None, Self::log_only_on_error("list_mini_apps"), cx);
    }

    pub fn list_public_mini_apps_action(
        bot_id: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        relative_id: Option<&str>,
        reflect_pending_in_ui: bool,
        on_finished: Option<BotActionFinished>,
        cx: &mut App,
    ) {
        if Self::effective_user_id_for_wire(cx).is_none() {
            return;
        }
        if ![bot_id, tag, bot_user_id].into_iter().any(|value| value.is_some_and(|value| !value.is_empty())) {
            return;
        }

        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let first_page = relative_id.is_none();
        let body = rise_request::bot::ListPublicMiniApps {
            bot_id: bot_id.map(str::to_owned),
            bot_user_id: bot_user_id.map(str::to_owned),
            limit: Some(BotServicesStore::page_limit()),
            relative_id: relative_id.map(str::to_owned),
            tag: tag.map(str::to_owned),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::LIST_PUBLIC_MINI_APPS.to_owned(),
            cache_key: Some(BotServicesStore::public_mini_apps_cache_key(bot_id, tag, bot_user_id)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: first_page && reflect_pending_in_ui,
            send_immediately: true,
            allows_deferred_send: false,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            path_to_array: Some(rise_key::bot::MINI_APPS.to_owned()),
            path_to_id: Some(rise_key::bot::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(relative_id, None)),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success = on_finished.clone().map(|finished| -> SaiWsOnSuccess {
            let account_id = account_id.clone();
            let account_epoch = account_epoch.clone();
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                if !SaiAccountCacheScope::is_current_account_epoch(account_id.as_deref(), account_epoch.as_deref()) {
                    return;
                }
                finished(cx);
            })
        });
        let slice = Self::shared(cx).read(cx).list_public_mini_apps_sai.clone();
        let on_error: SaiWsOnError = {
            let slice = Rc::clone(&slice);
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Self::log_network(format!("list_public_mini_apps error: {error}"), cx);
                if !SaiAccountCacheScope::is_current_account_epoch(account_id.as_deref(), account_epoch.as_deref()) {
                    return;
                }
                if first_page {
                    slice.mark_rejected(error);
                }
                if let Some(on_finished) = &on_finished {
                    on_finished(cx);
                }
            })
        };
        Self::send(&slice, body, options, on_success, on_error, cx);
    }

    pub fn get_mini_app_action(bot_id: &str, mini_app_id: &str, reflect_pending_in_ui: bool, cx: &mut App) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() || mini_app_id.is_empty() {
            return;
        }

        let body = rise_request::bot::GetMiniApp {
            bot_id: Some(bot_id.to_owned()),
            mini_app_id: Some(mini_app_id.to_owned()),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::GET_MINI_APP.to_owned(),
            cache_key: Some(BotServicesStore::mini_app_cache_key(&user_id, bot_id, mini_app_id)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let slice = Self::shared(cx).read(cx).get_mini_app_sai.clone();
        Self::send(&slice, body, options, Some(Self::ignore_success()), Self::log_only_on_error("get_mini_app"), cx);
    }

    pub fn update_mini_app_action(
        bot_id: &str,
        mini_app_id: &str,
        values: &BotMiniAppUpdateValues,
        on_result: Option<BotActionResult<BotMiniAppModel>>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() || mini_app_id.is_empty() {
            return;
        }

        let snapshots = Rc::new(BotServicesStore::mini_app_mutation_snapshot(&user_id, bot_id, Some(mini_app_id), cx));
        BotServicesStore::apply_mini_app_update_optimistic(&user_id, bot_id, mini_app_id, values, cx);

        let body = rise_request::bot::UpdateMiniApp {
            bot_id: Some(bot_id.to_owned()),
            description: values.description.clone(),
            development_url: values.development_url.clone(),
            icon_url: values.icon_url.clone(),
            is_active: values.is_active,
            mini_app_id: Some(mini_app_id.to_owned()),
            production_url: values.production_url.clone(),
            title: values.title.clone(),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::UPDATE_MINI_APP.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = {
            let snapshots = Rc::clone(&snapshots);
            let bot_id = bot_id.to_owned();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let dto = Self::typed_payload::<BotMiniAppGatewayDTO>(payload);
                let Some(mini_app) = dto.and_then(|dto| dto.mini_app) else {
                    BotServicesStore::restore_documents(&snapshots, cx);
                    return;
                };
                BotServicesStore::confirm_updated_mini_app(&user_id, &bot_id, &mini_app, cx);
                if let Some(on_result) = &on_result {
                    on_result(mini_app.to_bot_mini_app_model(), cx);
                }
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            BotServicesStore::restore_documents(&snapshots, cx);
            Self::log_network(format!("update_mini_app error: {error}"), cx);
        });

        let slice = Self::shared(cx).read(cx).update_mini_app_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn delete_mini_app_action(bot_id: &str, mini_app_id: &str, on_result: Option<BotActionFinished>, cx: &mut App) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() || mini_app_id.is_empty() {
            return;
        }

        let snapshots = Rc::new(BotServicesStore::mini_app_mutation_snapshot(&user_id, bot_id, Some(mini_app_id), cx));
        BotServicesStore::apply_delete_mini_app_optimistic(&user_id, bot_id, mini_app_id, cx);

        let body = rise_request::bot::DeleteMiniApp {
            bot_id: Some(bot_id.to_owned()),
            mini_app_id: Some(mini_app_id.to_owned()),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::DELETE_MINI_APP.to_owned(),
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
            if let Some(on_result) = &on_result {
                on_result(cx);
            }
        });
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            BotServicesStore::restore_documents(&snapshots, cx);
            Self::log_network(format!("delete_mini_app error: {error}"), cx);
        });

        let slice = Self::shared(cx).read(cx).delete_mini_app_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_mini_app_launch_data_action(
        bot_id: &str,
        mini_app_id: &str,
        chat_id: Option<&str>,
        query_id: Option<&str>,
        prefer_development: bool,
        start_param: Option<&str>,
        on_result: Option<BotActionResult<BotMiniAppLaunchData>>,
        on_error: Option<BotActionError>,
        cx: &mut App,
    ) {
        let slice = Self::shared(cx).read(cx).create_mini_app_launch_data_sai.clone();
        Self::request_mini_app_launch_action(
            &slice,
            rise_wire_methods::bot::CREATE_MINI_APP_LAUNCH_DATA,
            bot_id,
            mini_app_id,
            chat_id,
            query_id,
            prefer_development,
            start_param,
            on_result,
            on_error,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn get_mini_app_launch_action(
        bot_id: &str,
        mini_app_id: &str,
        chat_id: Option<&str>,
        query_id: Option<&str>,
        prefer_development: bool,
        start_param: Option<&str>,
        on_result: Option<BotActionResult<BotMiniAppLaunchData>>,
        on_error: Option<BotActionError>,
        cx: &mut App,
    ) {
        let slice = Self::shared(cx).read(cx).get_mini_app_launch_sai.clone();
        Self::request_mini_app_launch_action(
            &slice,
            rise_wire_methods::bot::GET_MINI_APP_LAUNCH,
            bot_id,
            mini_app_id,
            chat_id,
            query_id,
            prefer_development,
            start_param,
            on_result,
            on_error,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_callback_query_action(
        bot_id: &str,
        callback_query_id: &str,
        chat_id: Option<&str>,
        message_id: Option<&str>,
        inline_message_id: Option<&str>,
        data: Option<&str>,
        game_short_name: Option<&str>,
        on_result: Option<BotActionResult<BotCallbackEnqueueResult>>,
        on_error: Option<BotActionError>,
        cx: &mut App,
    ) {
        if Self::effective_user_id_for_wire(cx).is_none() {
            return;
        }
        let has_target = [message_id, inline_message_id].into_iter().any(|value| value.is_some_and(|value| !value.is_empty()));
        if bot_id.is_empty() || !has_target {
            return;
        }

        let body = rise_request::bot::EnqueueCallbackQuery {
            bot_id: Some(bot_id.to_owned()),
            callback_query_id: Some(callback_query_id.to_owned()),
            chat_id: chat_id.map(str::to_owned),
            data: data.map(str::to_owned),
            game_short_name: game_short_name.map(str::to_owned),
            inline_message_id: inline_message_id.map(str::to_owned),
            message_id: message_id.map(str::to_owned),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::ENQUEUE_CALLBACK_QUERY.to_owned(),
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(dto) = Self::typed_payload::<BotCallbackEnqueueGatewayDTO>(payload) else {
                return;
            };
            if let Some(on_result) = &on_result {
                on_result(dto.to_bot_callback_enqueue_result(), cx);
            }
        });
        let on_failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
            Self::log_network(format!("enqueue_callback_query error: {error}"), cx);
        });

        let slice = Self::shared(cx).read(cx).enqueue_callback_query_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_failure, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resolve_mini_app_link_action(
        tag: &str,
        slug: &str,
        chat_id: Option<&str>,
        query_id: Option<&str>,
        start_param: Option<&str>,
        prefer_development: bool,
        on_result: Option<BotActionResult<BotMiniAppLaunchData>>,
        on_error: Option<BotActionError>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if !BotTagRules::is_valid(tag) || slug.is_empty() {
            return;
        }

        let body = rise_request::bot::ResolveMiniAppLink {
            chat_id: chat_id.map(str::to_owned),
            prefer_development: Some(prefer_development),
            query_id: query_id.map(str::to_owned),
            slug: slug.to_owned(),
            start_param: start_param.map(str::to_owned),
            tag: tag.to_owned(),
        };

        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(dto) = Self::typed_payload::<BotMiniAppLaunchGatewayDTO>(payload) else {
                return;
            };
            Self::services(cx, |service, cx| service.remember_mini_app_load(&dto, &user_id, cx));
            if let Some(on_result) = &on_result {
                on_result(dto.to_bot_mini_app_launch_data(), cx);
            }
        });
        let on_failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
            Self::log_network(format!("resolve_mini_app_link error: {error}"), cx);
        });

        let slice = Self::shared(cx).read(cx).resolve_mini_app_link_sai.clone();
        let options = BotServicesStore::sensitive_request_options(rise_wire_methods::bot::RESOLVE_MINI_APP_LINK, true);
        Self::send(&slice, body, options, Some(on_success), on_failure, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn request_mini_app_launch_action(
        slice: &Rc<SaiTypedSlice<BotMiniAppLaunchGatewayDTO>>,
        method: &'static str,
        bot_id: &str,
        mini_app_id: &str,
        chat_id: Option<&str>,
        query_id: Option<&str>,
        prefer_development: bool,
        start_param: Option<&str>,
        on_result: Option<BotActionResult<BotMiniAppLaunchData>>,
        on_error: Option<BotActionError>,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() || mini_app_id.is_empty() {
            return;
        }

        let mut options = BotServicesStore::sensitive_request_options(method, true);
        options.allows_deferred_send = false;

        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(dto) = Self::typed_payload::<BotMiniAppLaunchGatewayDTO>(payload) else {
                return;
            };
            Self::services(cx, |service, cx| service.remember_mini_app_load(&dto, &user_id, cx));
            if let Some(on_result) = &on_result {
                on_result(dto.to_bot_mini_app_launch_data(), cx);
            }
        });
        let on_failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if let Some(on_error) = &on_error {
                on_error(error, cx);
            }
            Self::log_network(format!("{method} error: {error}"), cx);
        });

        if method == rise_wire_methods::bot::CREATE_MINI_APP_LAUNCH_DATA {
            let body = rise_request::bot::CreateMiniAppLaunchData {
                bot_id: Some(bot_id.to_owned()),
                chat_id: chat_id.map(str::to_owned),
                mini_app_id: Some(mini_app_id.to_owned()),
                prefer_development: Some(prefer_development),
                query_id: query_id.map(str::to_owned),
                start_param: start_param.map(str::to_owned),
            };
            Self::send(slice, body, options, Some(on_success), on_failure, cx);
        } else {
            let body = rise_request::bot::GetMiniAppLaunch {
                bot_id: Some(bot_id.to_owned()),
                chat_id: chat_id.map(str::to_owned),
                mini_app_id: Some(mini_app_id.to_owned()),
                prefer_development: Some(prefer_development),
                query_id: query_id.map(str::to_owned),
                start_param: start_param.map(str::to_owned),
            };
            Self::send(slice, body, options, Some(on_success), on_failure, cx);
        }
    }

    fn inline_composer_callbacks<T: DeserializeOwned + 'static>(
        request: InlineComposerRequest,
        status: impl Fn(&T) -> (bool, String) + 'static,
    ) -> (SaiWsOnSuccess, SaiWsOnError) {
        let request = Rc::new(request);
        let on_success: SaiWsOnSuccess = {
            let request = Rc::clone(&request);
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(response) = Self::decoded_payload::<T>(payload) else {
                    let error = Self::bot_inline_request_error(None, request.invalid_fallback);
                    Self::fail_inline_composer_request(&request, &error, cx);
                    Self::log_network(format!("{} returned an undecodable response", request.action), cx);
                    return;
                };
                let (success, message) = status(&response);
                if !success {
                    let error = Self::bot_inline_request_error(Some(message.as_str()), request.rejected_fallback);
                    Self::fail_inline_composer_request(&request, &error, cx);
                    Self::log_network(format!("{} rejected: {error}", request.action), cx);
                    return;
                }
                Self::services(cx, |service, cx| {
                    service.finish_inline_composer_request(
                        &request.composer_scope,
                        &request.cache_key,
                        request.generation,
                        cx,
                    );
                });
            })
        };
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::fail_inline_composer_request(&request, error, cx);
            Self::log_network(format!("{} error: {error}", request.action), cx);
        });
        (on_success, on_error)
    }

    fn fail_inline_composer_request(request: &InlineComposerRequest, error: &anyhow::Error, cx: &mut App) {
        Self::services(cx, |service, cx| {
            service.fail_inline_composer_request(
                &request.composer_scope,
                &request.cache_key,
                error,
                request.generation,
                cx,
            );
        });
    }

    pub fn get_mention_candidates_action(
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        cursor: Option<&str>,
        composer_scope: &str,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if chat_id.is_empty() {
            return;
        }

        let normalized_query = BotServicesStore::normalized_inline_query(query);
        let cache_key =
            BotServicesStore::mention_candidates_cache_key_user_id(&user_id, chat_id, topic_id, &normalized_query);
        let body = rise_request::bot::GetMentionCandidates {
            chat_id: Some(chat_id.to_owned()),
            cursor: cursor.map(str::to_owned),
            limit: Some(BotServicesStore::inline_page_limit()),
            query: Self::optional_non_empty(&normalized_query),
            topic_id: topic_id.map(str::to_owned),
        };
        let has_no_candidates = BotServicesStore::mention_candidates(chat_id, topic_id, &normalized_query, cx).is_empty();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::GET_MENTION_CANDIDATES.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: cursor.is_none() && has_no_candidates,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(rise_key::bot::CANDIDATES.to_owned()),
            path_to_id: Some(rise_key::bot::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(cursor, Some(rise_key::bot::CURSOR))),
            ..SaiWsRequestOptionsInit::default()
        });

        let (on_success, on_error) = Self::inline_composer_callbacks(
            InlineComposerRequest {
                action: "get_mention_candidates",
                invalid_fallback: "The mention candidates response was invalid.",
                rejected_fallback: "Unable to load mention candidates.",
                composer_scope: composer_scope.to_owned(),
                cache_key,
                generation: None,
            },
            |response: &BotMentionCandidatesGatewayDTO| (response.success, response.error.clone()),
        );

        let slice = Self::shared(cx).read(cx).get_mention_candidates_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn get_inline_bot_commands_action(
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        language_code: &str,
        composer_scope: &str,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if chat_id.is_empty() {
            return;
        }

        let normalized_tag = BotServicesStore::normalized_bot_tag(bot_tag);
        if normalized_tag.is_empty() {
            return;
        }

        let cache_key =
            BotServicesStore::inline_commands_cache_key_user_id(&user_id, &normalized_tag, chat_id, topic_id, language_code);
        let body = rise_request::bot::GetInlineBotCommands {
            bot_tag: Some(normalized_tag.clone()),
            chat_id: Some(chat_id.to_owned()),
            language_code: Self::optional_non_empty(language_code),
            topic_id: topic_id.map(str::to_owned),
        };
        let has_no_commands = BotServicesStore::inline_commands(&normalized_tag, chat_id, topic_id, language_code, cx).is_none();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::GET_INLINE_BOT_COMMANDS.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: has_no_commands,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(rise_key::bot::COMMANDS.to_owned()),
            path_to_id: Some(rise_key::bot::COMMAND.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });

        let (on_success, on_error) = Self::inline_composer_callbacks(
            InlineComposerRequest {
                action: "get_inline_bot_commands",
                invalid_fallback: "The inline commands response was invalid.",
                rejected_fallback: "Unable to load inline bot commands.",
                composer_scope: composer_scope.to_owned(),
                cache_key,
                generation: None,
            },
            |response: &BotInlineCommandsGatewayDTO| (response.success, response.error.clone()),
        );

        let slice = Self::shared(cx).read(cx).get_inline_bot_commands_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn get_chat_bot_commands_action(
        chat_id: &str,
        topic_id: Option<&str>,
        language_code: &str,
        composer_scope: &str,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if chat_id.is_empty() {
            return;
        }

        let cache_key = BotServicesStore::chat_commands_cache_key_user_id(&user_id, chat_id, topic_id, language_code);
        let body = rise_canon::rise_request::bot::GetChatBotCommands {
            chat_id: Some(chat_id.to_owned()),
            language_code: Self::optional_non_empty(language_code),
            topic_id: topic_id.map(str::to_owned),
        };
        let has_no_commands = BotServicesStore::chat_commands(chat_id, topic_id, language_code, cx).is_none();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::bot::GET_CHAT_BOT_COMMANDS.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: has_no_commands,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            ..SaiWsRequestOptionsInit::default()
        });

        let (on_success, on_error) = Self::inline_composer_callbacks(
            InlineComposerRequest {
                action: "get_chat_bot_commands",
                invalid_fallback: "The group commands response was invalid.",
                rejected_fallback: "The group commands response was invalid.",
                composer_scope: composer_scope.to_owned(),
                cache_key,
                generation: None,
            },
            |_: &BotChatCommandsGatewayDTO| (true, String::new()),
        );

        let slice = Self::shared(cx).read(cx).get_chat_bot_commands_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    pub fn query_inline_results_action(
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        offset: Option<&str>,
        composer_scope: &str,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if chat_id.is_empty() {
            return;
        }

        let normalized_tag = BotServicesStore::normalized_bot_tag(bot_tag);
        let normalized_query = BotServicesStore::normalized_inline_result_query(query);
        if normalized_tag.is_empty() || normalized_query.is_empty() {
            return;
        }

        let cache_key =
            BotServicesStore::inline_results_cache_key_user_id(&user_id, &normalized_tag, chat_id, topic_id, &normalized_query);
        let body = rise_request::bot::QueryInlineResults {
            bot_tag: Some(normalized_tag.clone()),
            chat_id: Some(chat_id.to_owned()),
            limit: Some(BotServicesStore::inline_page_limit()),
            offset: offset.map(str::to_owned),
            query: Some(normalized_query.clone()),
            topic_id: topic_id.map(str::to_owned),
        };
        let Some(request_generation) = Self::services(cx, |service, _| {
            service.inline_composer_request_generation(composer_scope, &cache_key)
        }) else {
            return;
        };
        let has_no_results = BotServicesStore::inline_results(&normalized_tag, chat_id, topic_id, &normalized_query, cx).results.is_empty();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::bot::SERVICE.to_owned(),
            method: rise_wire_methods::bot::QUERY_INLINE_RESULTS.to_owned(),
            cache_key: Some(cache_key.clone()),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: offset.is_none() && has_no_results,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(rise_key::bot::RESULTS.to_owned()),
            path_to_id: Some(rise_key::bot::ID.to_owned()),
            pagination: Some(Self::relative_id_pagination(offset, Some(rise_key::bot::NEXT_OFFSET))),
            ..SaiWsRequestOptionsInit::default()
        });

        let (on_success, on_error) = Self::inline_composer_callbacks(
            InlineComposerRequest {
                action: "query_inline_results",
                invalid_fallback: "The inline results response was invalid.",
                rejected_fallback: "Unable to load inline results.",
                composer_scope: composer_scope.to_owned(),
                cache_key,
                generation: Some(request_generation),
            },
            |response: &BotInlineResultsGatewayDTO| (response.success, response.error.clone()),
        );

        let slice = Self::shared(cx).read(cx).query_inline_results_sai.clone();
        Self::send(&slice, body, options, Some(on_success), on_error, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_inline_result_action(
        chat_id: &str,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        query_id: &str,
        result: &BotInlineResultModel,
        request_id: &str,
        on_result: Option<BotInlineSendResult>,
        cx: &mut App,
    ) {
        let send_key = BotServicesStore::inline_send_key(query_id, &result.id);
        let has_user = Self::effective_user_id_for_wire(cx).is_some();
        if !has_user || chat_id.is_empty() || query_id.is_empty() || result.id.is_empty() || request_id.is_empty() {
            Self::services(cx, |service, cx| service.finish_inline_result_send(&send_key, cx));
            if let Some(on_result) = &on_result {
                on_result(false, None, cx);
            }
            return;
        }

        let wire_topic_id = post_topic_id.or(topic_id);
        let temp_id = format!("temp_{request_id}");
        let echo_request_id = request_id.to_owned();
        let optimistic_body = Value::Object(result.optimistic_message_body(wire_topic_id));
        let body = rise_request::bot::SendInlineResult {
            chat_id: Some(chat_id.to_owned()),
            query_id: Some(query_id.to_owned()),
            reply_to_id: reply_to_id.map(str::to_owned),
            request_id: Some(request_id.to_owned()),
            result_id: Some(result.id.clone()),
            thread_root_id: thread_root_id.map(str::to_owned),
            topic_id: wire_topic_id.map(str::to_owned),
        };
        let chat_id = chat_id.to_owned();
        let rise_scope = RiseMessageScope::new(
            chat_id.clone(),
            topic_id.or(wire_topic_id).map(str::to_owned),
            thread_root_id.map(str::to_owned),
        );

        cx.spawn(async move |cx: &mut AsyncApp| {
            let outcome: anyhow::Result<()> = async {
                let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                let row = cx.update(|cx| {
                    MessageServicesStore::shared(cx).update(cx, |service, cx| {
                        service.create_optimistic_message_data(&optimistic_body, &chat_id, &temp_id, &[], &[], &[], cx)
                    })
                });
                let row = Value::Object(row);
                let optimistic_wire: MessageWireDTO =
                    MessagePayloadCodec::decode(Some(&row)).ok_or(RiseMessageRepositoryError::InvalidWirePayload)?;
                let rise_body = RiseJSONValue::encode(&body)?;
                let (work_scope, work_temp_id, work_request_id) =
                    (rise_scope.clone(), temp_id.clone(), echo_request_id.clone());
                Self::on_account_runtime(async move {
                    domain.observe_messages(&work_scope).await?;
                    domain
                        .submit_inline_result(rise_body, optimistic_wire, &work_temp_id, &work_request_id, &work_scope)
                        .await?;
                    anyhow::Ok(())
                })
                .await?
            }
            .await;
            cx.update(|cx| {
                Self::services(cx, |service, cx| service.finish_inline_result_send(&send_key, cx));
                match outcome {
                    Ok(()) => {
                        RiseShareAppBridge::donate_outgoing_interaction(&chat_id, cx);
                        if let Some(on_result) = &on_result {
                            on_result(true, None, cx);
                        }
                    }
                    Err(error) => {
                        if let Some(on_result) = &on_result {
                            on_result(false, Some(&error), cx);
                        }
                        Self::log_network(format!("Rise send_inline_result error: {error}"), cx);
                    }
                }
            });
        })
        .detach();
    }

    async fn on_account_runtime<Work>(work: Work) -> anyhow::Result<Work::Output>
    where
        Work: Future + Send + 'static,
        Work::Output: Send + 'static,
    {
        RiseAccountRuntime::current().spawn(work).await.map_err(|_| CancellationError.into())
    }

    fn bot_inline_request_error(message: Option<&str>, fallback: &str) -> anyhow::Error {
        let normalized = message.map(trimming_whitespaces_and_newlines).filter(|message| !message.is_empty());
        anyhow::anyhow!("{}", normalized.unwrap_or(fallback))
    }

    fn optional_non_empty(value: &str) -> Option<String> {
        if value.is_empty() { None } else { Some(value.to_owned()) }
    }
}
