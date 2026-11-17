use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

use rise_canon::rise_key;
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_engine::{CancellationError, RiseJSONValue, RiseWireRemoteError};
use rise_errors::ErrorCode;

use crate::modules::chat::engine::bot_model_hooks::{BotGatewayDTO, BotModel, BotWireDTOExt};
use crate::modules::chat::engine::core::rise_conversation_request_state::RiseConversationStateError;
use crate::modules::chat::engine::core::rise_conversation_rpc::{RiseConversationEndpoints, RiseConversationRPC};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomain;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseBotStartLinkError {
    ModerationDisabled,
}

impl fmt::Display for RiseBotStartLinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RiseBotStartLinkError.moderationDisabled")
    }
}

impl std::error::Error for RiseBotStartLinkError {}

impl RiseConversationDomain {
    pub async fn resolve_bot_start_link(&self, tag: &str) -> anyhow::Result<BotModel> {
        if !self.is_active_account_session().await {
            return Err(CancellationError.into());
        }
        let body = RiseJSONValue::Object(BTreeMap::from([
            (rise_key::otp::USER_ID.to_owned(), RiseJSONValue::String(self.account_id.clone())),
            (rise_key::chat::TAG.to_owned(), RiseJSONValue::String(tag.to_owned())),
        ]));
        let response: BotGatewayDTO = RiseConversationRPC::new(self.engine.clone())
            .call(RiseConversationEndpoints::get_bot_by_tag(), body)
            .await
            .map_err(|error| {
                let is_disabled = error
                    .downcast_ref::<RiseWireRemoteError>()
                    .is_some_and(|remote| remote.code == ErrorCode::BotModerationDisabled.wire());
                if is_disabled { RiseBotStartLinkError::ModerationDisabled.into() } else { error }
            })?;
        if !self.is_active_account_session().await {
            return Err(CancellationError.into());
        }
        let bot = response.bot.as_ref().map(BotWireDTOExt::to_bot_model);
        if response.success && bot.as_ref().is_some_and(BotModel::is_moderation_disabled) {
            return Err(RiseBotStartLinkError::ModerationDisabled.into());
        }
        match bot {
            Some(bot)
                if response.success
                    && bot.is_active
                    && !bot.is_moderation_disabled()
                    && !bot.bot_user_id.is_empty()
                    && compare_case_insensitive(&bot.tag, tag) == Ordering::Equal =>
            {
                Ok(bot)
            }
            _ => Err(RiseConversationStateError::InvalidPresentationValue.into()),
        }
    }
}
