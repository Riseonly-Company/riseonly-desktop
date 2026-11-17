use rise_canon::{rise_proto, rise_response};

use super::bot_types::{
    BotCallbackEnqueueResult, BotCommandModel, BotMiniAppLaunchData, BotMiniAppModel, BotModel,
    BotPublicCommandsModel, BotTagAvailability, BotTokenResult, BotWebhookInfoModel,
};

pub type BotWireDTO = rise_proto::bot::Bot;

pub trait BotWireDTOExt {
    fn to_bot_model(&self) -> BotModel;
}

impl BotWireDTOExt for rise_proto::bot::Bot {
    fn to_bot_model(&self) -> BotModel {
        BotModel {
            id: self.id.clone(),
            owner_user_id: self.owner_user_id.clone(),
            bot_user_id: self.bot_user_id.clone(),
            private_chat_id: self.private_chat_id.clone(),
            tag: self.tag.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            avatar_url: self.avatar_url.clone(),
            banner_url: self.banner_url.clone(),
            is_active: self.is_active,
            can_join_groups: self.can_join_groups,
            privacy_mode: self.privacy_mode,
            created_at: self.created_at,
            updated_at: self.updated_at,
            bot_user_chat_id: self.bot_user_chat_id.clone(),
            is_owned_by_requester: self.is_owned_by_requester,
            moderation_status: self.moderation_status.clone(),
            is_system: self.is_system,
        }
    }
}

pub type BotCommandWireDTO = rise_proto::bot::BotCommand;

pub trait BotCommandWireDTOExt {
    fn to_bot_command_model(&self) -> BotCommandModel;
}

impl BotCommandWireDTOExt for rise_proto::bot::BotCommand {
    fn to_bot_command_model(&self) -> BotCommandModel {
        BotCommandModel { command: self.command.clone(), description: self.description.clone() }
    }
}

pub type BotWebhookInfoWireDTO = rise_proto::bot::WebhookInfo;

pub trait BotWebhookInfoWireDTOExt {
    fn to_bot_webhook_info_model(&self) -> BotWebhookInfoModel;
}

impl BotWebhookInfoWireDTOExt for rise_proto::bot::WebhookInfo {
    fn to_bot_webhook_info_model(&self) -> BotWebhookInfoModel {
        BotWebhookInfoModel {
            bot_id: self.bot_id.clone(),
            url: self.url.clone(),
            enabled: self.enabled,
            has_custom_certificate: self.has_custom_certificate,
            pending_update_count: self.pending_update_count,
            last_error_date: self.last_error_date,
            last_error_message: self.last_error_message.clone(),
            max_connections: self.max_connections,
            allowed_updates: self.allowed_updates.clone(),
            updated_at: self.updated_at,
        }
    }
}

pub type BotMiniAppWireDTO = rise_proto::bot::MiniApp;

pub trait BotMiniAppWireDTOExt {
    fn to_bot_mini_app_model(&self) -> BotMiniAppModel;
}

impl BotMiniAppWireDTOExt for rise_proto::bot::MiniApp {
    fn to_bot_mini_app_model(&self) -> BotMiniAppModel {
        BotMiniAppModel {
            id: self.id.clone(),
            bot_id: self.bot_id.clone(),
            short_name: self.short_name.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            production_url: self.production_url.clone(),
            development_url: self.development_url.clone(),
            icon_url: self.icon_url.clone(),
            is_active: self.is_active,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

pub type BotCreateGatewayDTO = rise_proto::bot::CreateBotResponse;

pub type BotListGatewayDTO = rise_proto::bot::ListBotsResponse;
pub type BotGatewayDTO = rise_proto::bot::GetBotResponse;

pub type BotTagAvailabilityGatewayDTO = rise_proto::bot::CheckBotTagAvailabilityResponse;

pub trait BotTagAvailabilityGatewayDTOExt {
    fn to_bot_tag_availability(&self) -> BotTagAvailability;
}

impl BotTagAvailabilityGatewayDTOExt for rise_proto::bot::CheckBotTagAvailabilityResponse {
    fn to_bot_tag_availability(&self) -> BotTagAvailability {
        BotTagAvailability {
            available: self.available,
            reason: self.reason.clone(),
            suggestions: self.suggestions.clone(),
        }
    }
}

pub type BotStatusGatewayDTO = rise_proto::bot::StatusResponse;

pub type BotTokenGatewayDTO = rise_response::bot::RevealBotToken;

pub trait BotTokenGatewayDTOExt {
    fn to_bot_token_result(&self) -> BotTokenResult;
}

impl BotTokenGatewayDTOExt for rise_proto::bot::TokenResponse {
    fn to_bot_token_result(&self) -> BotTokenResult {
        BotTokenResult { bot_id: self.bot_id.clone(), token: self.token.clone(), rotated_at: self.rotated_at }
    }
}

pub type BotCommandsGatewayDTO = rise_proto::bot::CommandsResponse;

pub type BotPublicCommandsGatewayDTO = rise_proto::bot::PublicCommandsResponse;

pub trait BotPublicCommandsGatewayDTOExt {
    fn to_bot_public_commands_model(&self) -> BotPublicCommandsModel;
}

impl BotPublicCommandsGatewayDTOExt for rise_proto::bot::PublicCommandsResponse {
    fn to_bot_public_commands_model(&self) -> BotPublicCommandsModel {
        BotPublicCommandsModel {
            commands: self.commands.iter().map(BotCommandWireDTOExt::to_bot_command_model).collect(),
            resolved_scope: self.resolved_scope.clone(),
            resolved_language_code: self.resolved_language_code.clone(),
        }
    }
}

pub type BotWebhookGatewayDTO = rise_proto::bot::WebhookInfoResponse;
pub type BotMiniAppGatewayDTO = rise_proto::bot::MiniAppResponse;

pub type BotMiniAppListGatewayDTO = rise_proto::bot::ListMiniAppsResponse;

/// answer type of create_mini_app_launch_data and get_mini_app_launch
/// resolve_mini_app_link lacks load_receipt in canon so that reply is decoded separately
pub type BotMiniAppLaunchGatewayDTO = rise_response::bot::GetMiniAppLaunch;

pub trait BotMiniAppLaunchGatewayDTOExt {
    fn to_bot_mini_app_launch_data(&self) -> BotMiniAppLaunchData;
}

impl BotMiniAppLaunchGatewayDTOExt for rise_proto::bot::MiniAppLaunchDataResponse {
    fn to_bot_mini_app_launch_data(&self) -> BotMiniAppLaunchData {
        BotMiniAppLaunchData {
            launch_url: self.launch_url.clone(),
            init_data: self.init_data.clone(),
            expires_at: self.expires_at,
        }
    }
}

pub type BotCallbackEnqueueGatewayDTO = rise_proto::bot::EnqueueCallbackQueryResponse;

pub trait BotCallbackEnqueueGatewayDTOExt {
    fn to_bot_callback_enqueue_result(&self) -> BotCallbackEnqueueResult;
}

impl BotCallbackEnqueueGatewayDTOExt for rise_proto::bot::EnqueueCallbackQueryResponse {
    fn to_bot_callback_enqueue_result(&self) -> BotCallbackEnqueueResult {
        BotCallbackEnqueueResult { update_id: self.update_id, callback_query_id: self.callback_query_id.clone() }
    }
}
