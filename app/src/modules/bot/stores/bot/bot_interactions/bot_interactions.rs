use std::pin::pin;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Task};
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::RiseAppEvents;
use rise_engine::{CancellationError, RiseWireError};
use rise_i18n::language_store::LanguageStore;
use rise_i18n::tr;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use rise_canon::RiseTimingCanon;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};
use crate::core::rise_engine_bridge::rise_engine_app_runtime::{RISE_ENGINE_CONNECTION_READY, RiseEngineAppRuntimeError};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::shared::bot_start_payload::BotStartPayload;
use crate::modules::bot::shared::bot_ui_validation::{BotFieldRules, BotFormValidation};
use crate::modules::bot::stores::bot::bot_actions::bot_actions::{
    BotActionError, BotActionFinished, BotActionResult, BotActionsStore, BotCreateResult, BotInlineSendResult,
};
use crate::modules::bot::stores::bot::bot_actions::bot_inline_types::BotInlineResultModel;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{
    BotCallbackEnqueueResult, BotCommandModel, BotMediaKind, BotMiniAppLaunchData, BotMiniAppModel,
    BotMiniAppUpdateValues, BotModel, BotTagAvailability, BotTagRules, BotTokenResult, BotUpdateValues,
    BotWebhookInfoModel,
};
use crate::modules::bot::stores::bot::bot_services::bot_guide_sandbox_hooks::bot_guide_sandbox_hooks;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::engine::core::rise_conversation_request_state::RiseConversationStateError;
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

const START_LINK_READY_POLL: Duration = Duration::from_millis(500);

pub struct BotInteractionsStore {
    actions: Entity<BotActionsStore>,
    service: Entity<BotServicesStore>,
}

#[allow(clippy::too_many_arguments)]
impl BotInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self { actions: BotActionsStore::shared(cx), service: BotServicesStore::shared(cx) }
    }

    pub fn resolve_start_link(&mut self, tag: &str, cx: &mut Context<Self>) -> Task<anyhow::Result<BotModel>> {
        if !BotTagRules::is_valid(tag) {
            return Task::ready(Err(RiseConversationStateError::InvalidPresentationValue.into()));
        }
        let deadline = RiseClock::monotonic()
            + RiseCanonEndpoints::required(rise_wire_methods::bot::SERVICE, rise_wire_methods::bot::GET_BOT_BY_TAG)
                .client_timeout();
        let tag = tag.to_owned();
        cx.spawn(async move |this, cx| {
            loop {
                let (sender, mut ready) = mpsc::unbounded::<()>();
                let _ready_subscription =
                    RiseAppEvents::default_center().observe(RISE_ENGINE_CONNECTION_READY, move |_| {
                        let _ = sender.unbounded_send(());
                    });
                let attempt = this.update(cx, |_, cx| BotActionsStore::resolve_start_link_action(&tag, cx));
                let attempt = match attempt {
                    Ok(attempt) => attempt,
                    Err(error) => return Err(error),
                };
                let error = match attempt.await {
                    Ok(bot) => return Ok(bot),
                    Err(error) => error,
                };
                let remaining = deadline.saturating_sub(RiseClock::monotonic());
                if !Self::engine_not_ready(&error) || remaining.is_zero() {
                    return Err(error);
                }
                let timer = cx.background_executor().timer(remaining.min(START_LINK_READY_POLL));
                futures::future::select(pin!(ready.next()), pin!(timer)).await;
            }
        })
    }

    fn engine_not_ready(error: &anyhow::Error) -> bool {
        if error.downcast_ref::<CancellationError>().is_some()
            || error.downcast_ref::<RiseEngineAppRuntimeError>().is_some()
        {
            return true;
        }
        if CanonErrors::is_connection_failure(error) {
            return true;
        }
        if matches!(
            error.downcast_ref::<RiseWireError>(),
            Some(RiseWireError::Shutdown | RiseWireError::StaleSession | RiseWireError::StaleAccount)
        ) {
            return true;
        }
        matches!(
            error.downcast_ref::<RiseConversationStateError>(),
            Some(RiseConversationStateError::MissingActiveSession | RiseConversationStateError::StaleAccount)
        )
    }

    pub fn start_linked_bot(
        &mut self,
        bot: &BotModel,
        start_param: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if !bot.is_active || bot.is_moderation_disabled() || !start_param.is_none_or(BotStartPayload::is_valid) {
            return None;
        }

        let mut participant = User {
            id: bot.bot_user_id.clone(),
            name: bot.name.clone(),
            tag: bot.tag.clone(),
            ..User::default()
        };
        participant.more.logo = bot.avatar_url.clone();
        participant.is_bot = true;
        participant.bot_is_active = true;
        let chat_id = ChatInteractionsStore::shared(cx)
            .update(cx, |chats, cx| chats.direct_conversation_id(&participant, cx));
        if chat_id.is_empty() {
            return None;
        }

        let text = start_param.map_or_else(|| "/start".to_owned(), |param| format!("/start {param}"));
        MessageInteractionsStore::shared(cx).update(cx, |messages, cx| {
            messages.send_text(&chat_id, &text, None, None, None, None, None, None, None, None, None, None, cx);
        });
        Some(chat_id)
    }

    pub fn load_owned_bots(&mut self, cx: &mut Context<Self>) {
        if self.actions.read(cx).list_bots_sai.is_pending() {
            return;
        }

        let reflect_pending_in_ui = BotServicesStore::owned_bots(cx).is_empty();
        BotActionsStore::list_bots_action(None, reflect_pending_in_ui, cx);
    }

    pub fn refresh_owned_bots(&mut self, cx: &mut Context<Self>) {
        if self.actions.read(cx).list_bots_sai.is_pending() {
            return;
        }

        BotActionsStore::list_bots_action(None, false, cx);
    }

    pub fn load_more_owned_bots(&mut self, cx: &mut Context<Self>) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };

        let cache_key = BotServicesStore::owned_bots_cache_key(&user_id);
        let can_request = Self::can_request_trailing(&cache_key, cx);

        let has_more = BotServicesStore::owned_bots_has_more(cx);
        let relative_id = BotServicesStore::owned_bots_relative_id(cx);
        let Some(relative_id) = relative_id.filter(|relative_id| has_more && !relative_id.is_empty() && can_request)
        else {
            return;
        };

        BotActionsStore::list_bots_action(Some(&relative_id), false, cx);
    }

    pub fn load_bot(&mut self, bot_id: &str, cx: &mut Context<Self>) {
        if bot_id.is_empty() || self.actions.read(cx).get_bot_sai.is_pending() {
            return;
        }
        if Self::owns_practice_bot(bot_id, cx) {
            return;
        }

        let reflect_pending_in_ui = BotServicesStore::bot(bot_id, cx).is_none();
        BotActionsStore::get_bot_action(bot_id, reflect_pending_in_ui, cx);
    }

    pub fn load_bot_by_tag(&mut self, raw_tag: &str, cx: &mut Context<Self>) {
        let tag = BotTagRules::sanitize(raw_tag, false, false);
        if !BotTagRules::is_valid(&tag) || self.actions.read(cx).get_bot_by_tag_sai.is_pending() {
            return;
        }

        let reflect_pending_in_ui = BotServicesStore::bot_by_tag(&tag, cx).is_none();
        BotActionsStore::get_bot_by_tag_action(&tag, reflect_pending_in_ui, cx);
    }

    pub fn check_tag_availability(
        &mut self,
        raw_tag: &str,
        on_result: Option<Rc<dyn Fn(String, BotTagAvailability, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let tag = BotTagRules::sanitize(raw_tag, true, false);
        if !BotTagRules::is_valid(&tag) || self.actions.read(cx).check_bot_tag_availability_sai.is_pending() {
            return;
        }

        let checked_tag = tag.clone();
        BotActionsStore::check_bot_tag_availability_action(
            &tag,
            Some(Rc::new(move |availability: BotTagAvailability, cx: &mut App| {
                if let Some(on_result) = &on_result {
                    on_result(checked_tag.clone(), availability, cx);
                }
            })),
            cx,
        );
    }

    pub fn create_bot(
        &mut self,
        name: &str,
        raw_tag: &str,
        description: &str,
        avatar_url: &str,
        banner_url: &str,
        can_join_groups: bool,
        privacy_mode: bool,
        on_result: Option<BotCreateResult>,
        cx: &mut Context<Self>,
    ) {
        let tag = BotTagRules::sanitize(raw_tag, true, false);
        if self.actions.read(cx).create_bot_sai.is_pending()
            || trimming_whitespaces_and_newlines(name).is_empty()
            || !BotTagRules::is_valid(&tag)
        {
            return;
        }

        let practice = (bot_guide_sandbox_hooks().create_practice_bot)(name, &tag, description, can_join_groups, privacy_mode, cx);
        if let Some((bot, token)) = practice {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(bot, token, cx));
            }
            return;
        }

        BotActionsStore::create_bot_action(
            name,
            &tag,
            description,
            avatar_url,
            banner_url,
            can_join_groups,
            privacy_mode,
            on_result,
            cx,
        );
    }

    pub fn update_bot(
        &mut self,
        bot_id: &str,
        values: BotUpdateValues,
        on_result: Option<BotActionResult<BotModel>>,
        cx: &mut Context<Self>,
    ) {
        if bot_id.is_empty() || self.actions.read(cx).update_bot_sai.is_pending() {
            return;
        }

        let practice = (bot_guide_sandbox_hooks().update_practice_bot)(bot_id, &values, cx);
        if let Some(updated) = practice {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(updated, cx));
            }
            return;
        }

        let mut normalized = values;
        if let Some(raw_tag) = normalized.tag.as_deref() {
            let tag = BotTagRules::sanitize(raw_tag, true, false);
            if !BotTagRules::is_valid(&tag) {
                return;
            }
            normalized.tag = Some(tag);
        }
        BotActionsStore::update_bot_action(bot_id, &normalized, on_result, cx);
    }

    pub fn update_bot_avatar(
        &mut self,
        bot_id: &str,
        local_file: SaiWsOptimisticLocalFile,
        on_result: Option<BotActionResult<String>>,
        cx: &mut Context<Self>,
    ) {
        if !self.can_update_bot_media(bot_id, BotMediaKind::Avatar, cx) {
            return;
        }

        BotActionsStore::update_bot_avatar_action(bot_id, local_file, on_result, cx);
    }

    pub fn update_bot_banner(
        &mut self,
        bot_id: &str,
        local_file: SaiWsOptimisticLocalFile,
        on_result: Option<BotActionResult<String>>,
        cx: &mut Context<Self>,
    ) {
        if !self.can_update_bot_media(bot_id, BotMediaKind::Banner, cx) {
            return;
        }

        BotActionsStore::update_bot_banner_action(bot_id, local_file, on_result, cx);
    }

    fn can_update_bot_media(&mut self, bot_id: &str, kind: BotMediaKind, cx: &mut Context<Self>) -> bool {
        if bot_id.is_empty()
            || self.actions.read(cx).update_bot_sai.is_pending()
            || self.service.read(cx).is_media_uploading(bot_id, kind)
        {
            return false;
        }
        !Self::owns_practice_bot(bot_id, cx)
    }

    pub fn delete_bot(
        &mut self,
        bot_id: &str,
        on_result: Option<BotActionFinished>,
        cx: &mut Context<Self>,
    ) {
        if bot_id.is_empty() || self.actions.read(cx).delete_bot_sai.is_pending() {
            return;
        }

        if (bot_guide_sandbox_hooks().delete_practice_bot)(bot_id, cx) {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(cx));
            }
            return;
        }

        BotActionsStore::delete_bot_action(bot_id, on_result, cx);
    }

    pub fn reveal_token(
        &mut self,
        bot_id: &str,
        on_result: Option<BotActionResult<BotTokenResult>>,
        cx: &mut Context<Self>,
    ) {
        if bot_id.is_empty() || self.actions.read(cx).reveal_bot_token_sai.is_pending() {
            return;
        }
        if let Some(token) = Self::practice_token(bot_id, cx) {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(token, cx));
            }
            return;
        }

        BotActionsStore::reveal_bot_token_action(bot_id, on_result, cx);
    }

    pub fn rotate_token(
        &mut self,
        bot_id: &str,
        on_result: Option<BotActionResult<BotTokenResult>>,
        cx: &mut Context<Self>,
    ) {
        if bot_id.is_empty() || self.actions.read(cx).rotate_bot_token_sai.is_pending() {
            return;
        }
        if let Some(token) = Self::practice_token(bot_id, cx) {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(token, cx));
            }
            return;
        }

        BotActionsStore::rotate_bot_token_action(bot_id, on_result, cx);
    }

    pub fn load_commands(
        &mut self,
        bot_id: &str,
        scope: &str,
        language_code: &str,
        on_result: Option<BotActionResult<Vec<BotCommandModel>>>,
        cx: &mut Context<Self>,
    ) -> bool {
        if bot_id.is_empty() || self.actions.read(cx).get_bot_commands_sai.is_pending() {
            return false;
        }
        if Self::owns_practice_bot(bot_id, cx) {
            if let Some(on_result) = on_result {
                let commands = BotServicesStore::commands(bot_id, scope, language_code, cx);
                cx.defer(move |cx| on_result(commands, cx));
            }
            return true;
        }
        if Self::effective_user_id_for_wire(cx).is_none() {
            return false;
        }

        let reflect_pending_in_ui = on_result.is_some()
            || BotServicesStore::commands(bot_id, scope, language_code, cx).is_empty();
        BotActionsStore::get_bot_commands_action(bot_id, scope, language_code, reflect_pending_in_ui, on_result, cx);
        true
    }

    pub fn set_commands(
        &mut self,
        bot_id: &str,
        commands: Vec<BotCommandModel>,
        scope: &str,
        language_code: &str,
        on_result: Option<BotActionResult<Vec<BotCommandModel>>>,
        cx: &mut Context<Self>,
    ) {
        if bot_id.is_empty() || self.actions.read(cx).set_bot_commands_sai.is_pending() {
            return;
        }
        let practice = (bot_guide_sandbox_hooks().set_practice_commands)(bot_id, commands.clone(), scope, language_code, cx);
        if practice {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(commands, cx));
            }
            return;
        }

        BotActionsStore::set_bot_commands_action(bot_id, &commands, scope, language_code, on_result, cx);
    }

    pub fn delete_commands(
        &mut self,
        bot_id: &str,
        scope: &str,
        language_code: &str,
        on_result: Option<BotActionFinished>,
        cx: &mut Context<Self>,
    ) {
        if bot_id.is_empty() || self.actions.read(cx).delete_bot_commands_sai.is_pending() {
            return;
        }
        let practice = (bot_guide_sandbox_hooks().delete_practice_commands)(bot_id, scope, language_code, cx);
        if practice {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(cx));
            }
            return;
        }

        BotActionsStore::delete_bot_commands_action(bot_id, scope, language_code, on_result, cx);
    }

    pub fn load_public_commands(
        &mut self,
        bot_id: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        chat_id: Option<&str>,
        language_code: Option<&str>,
        chat_type: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if self.actions.read(cx).get_public_bot_commands_sai.is_pending() {
            return;
        }

        let language_code =
            language_code.map(str::to_owned).unwrap_or_else(|| LanguageStore::shared(cx).code().to_owned());
        BotActionsStore::get_public_bot_commands_action(
            bot_id,
            tag,
            bot_user_id,
            chat_id,
            &language_code,
            chat_type,
            false,
            cx,
        );
    }

    pub fn load_webhook(&mut self, bot_id: &str, cx: &mut Context<Self>) {
        if bot_id.is_empty() || self.actions.read(cx).get_bot_webhook_info_sai.is_pending() {
            return;
        }
        if Self::owns_practice_bot(bot_id, cx) {
            return;
        }

        let reflect_pending_in_ui = BotServicesStore::webhook(bot_id, cx).is_none();
        BotActionsStore::get_bot_webhook_info_action(bot_id, reflect_pending_in_ui, cx);
    }

    pub fn set_webhook(
        &mut self,
        bot_id: &str,
        url: &str,
        secret_token: &str,
        max_connections: i64,
        allowed_updates: Vec<String>,
        drop_pending_updates: bool,
        on_result: Option<BotActionResult<Option<BotWebhookInfoModel>>>,
        cx: &mut Context<Self>,
    ) {
        if bot_id.is_empty() || url.is_empty() || self.actions.read(cx).set_bot_webhook_sai.is_pending() {
            return;
        }
        let practice = (bot_guide_sandbox_hooks().set_practice_webhook)(bot_id, url, max_connections, allowed_updates.clone(), cx);
        if let Some(webhook) = practice {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(Some(webhook), cx));
            }
            return;
        }

        BotActionsStore::set_bot_webhook_action(
            bot_id,
            url,
            secret_token,
            max_connections,
            &allowed_updates,
            drop_pending_updates,
            on_result,
            cx,
        );
    }

    pub fn delete_webhook(
        &mut self,
        bot_id: &str,
        drop_pending_updates: bool,
        on_result: Option<BotActionFinished>,
        cx: &mut Context<Self>,
    ) {
        if bot_id.is_empty() || self.actions.read(cx).delete_bot_webhook_sai.is_pending() {
            return;
        }
        if (bot_guide_sandbox_hooks().delete_practice_webhook)(bot_id, cx) {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(cx));
            }
            return;
        }

        BotActionsStore::delete_bot_webhook_action(bot_id, drop_pending_updates, on_result, cx);
    }

    pub fn load_mini_apps(&mut self, bot_id: &str, cx: &mut Context<Self>) {
        if bot_id.is_empty() || self.actions.read(cx).list_mini_apps_sai.is_pending() {
            return;
        }
        if Self::owns_practice_bot(bot_id, cx) {
            return;
        }

        let reflect_pending_in_ui = BotServicesStore::mini_apps(bot_id, cx).is_empty();
        BotActionsStore::list_mini_apps_action(bot_id, None, reflect_pending_in_ui, cx);
    }

    pub fn refresh_mini_apps(&mut self, bot_id: &str, cx: &mut Context<Self>) {
        if bot_id.is_empty() || self.actions.read(cx).list_mini_apps_sai.is_pending() {
            return;
        }
        if Self::owns_practice_bot(bot_id, cx) {
            return;
        }

        BotActionsStore::list_mini_apps_action(bot_id, None, false, cx);
    }

    pub fn load_more_mini_apps(&mut self, bot_id: &str, cx: &mut Context<Self>) {
        let Some(user_id) = Self::effective_user_id_for_wire(cx) else {
            return;
        };
        if bot_id.is_empty() {
            return;
        }

        let cache_key = BotServicesStore::mini_apps_cache_key(&user_id, bot_id);
        let can_request = Self::can_request_trailing(&cache_key, cx);

        let has_more = BotServicesStore::mini_apps_has_more(bot_id, cx);
        let relative_id = BotServicesStore::mini_apps_relative_id(bot_id, cx);
        let Some(relative_id) = relative_id.filter(|relative_id| has_more && !relative_id.is_empty() && can_request)
        else {
            return;
        };

        BotActionsStore::list_mini_apps_action(bot_id, Some(&relative_id), false, cx);
    }

    pub fn load_public_mini_apps(
        &mut self,
        bot_id: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        on_finished: Option<BotActionFinished>,
        cx: &mut Context<Self>,
    ) {
        if on_finished.is_none() && self.actions.read(cx).list_public_mini_apps_sai.is_pending() {
            return;
        }

        BotActionsStore::list_public_mini_apps_action(bot_id, tag, bot_user_id, None, false, on_finished, cx);
    }

    pub fn mini_app_title(
        bot_id: Option<&str>,
        mini_app_id: Option<&str>,
        short_name: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        cx: &mut App,
    ) -> Option<String> {
        let mini_app_id = mini_app_id.filter(|mini_app_id| !mini_app_id.is_empty());
        let mut pool: Vec<BotMiniAppModel> = Vec::new();
        if let Some(tag) = tag.filter(|tag| !tag.is_empty()) {
            pool.extend(BotServicesStore::public_mini_apps(None, Some(tag), bot_user_id, cx));
            if bot_user_id.is_some() {
                pool.extend(BotServicesStore::public_mini_apps(None, Some(tag), None, cx));
            }
        }
        if let Some(bot_id) = bot_id.filter(|bot_id| !bot_id.is_empty()) {
            pool.extend(BotServicesStore::public_mini_apps(Some(bot_id), None, None, cx));
            if let Some(mini_app_id) = mini_app_id
                && let Some(owned) = BotServicesStore::mini_app(bot_id, mini_app_id, cx)
            {
                pool.push(owned);
            }
        }
        let slug = short_name.map(BotFormValidation::normalized_mini_app_short_name).unwrap_or_default();
        let found = pool.iter().find(|app| {
            if mini_app_id.is_some_and(|mini_app_id| app.id == mini_app_id) {
                return true;
            }
            !slug.is_empty() && BotFormValidation::normalized_mini_app_short_name(&app.short_name) == slug
        })?;
        let title = trimming_whitespaces_and_newlines(&found.title);
        if title.is_empty() {
            return None;
        }
        Some(title.to_owned())
    }

    pub fn load_more_public_mini_apps(
        &mut self,
        bot_id: Option<&str>,
        tag: Option<&str>,
        bot_user_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let cache_key = BotServicesStore::public_mini_apps_cache_key(bot_id, tag, bot_user_id);
        let can_request = Self::can_request_trailing(&cache_key, cx);

        let has_more = BotServicesStore::public_mini_apps_has_more(bot_id, tag, bot_user_id, cx);
        let relative_id =
            BotServicesStore::public_mini_apps_relative_id(bot_id, tag, bot_user_id, cx);
        let Some(relative_id) = relative_id.filter(|relative_id| has_more && !relative_id.is_empty() && can_request)
        else {
            return;
        };

        BotActionsStore::list_public_mini_apps_action(bot_id, tag, bot_user_id, Some(&relative_id), false, None, cx);
    }

    pub fn load_mini_app(&mut self, bot_id: &str, mini_app_id: &str, cx: &mut Context<Self>) {
        if bot_id.is_empty() || mini_app_id.is_empty() || self.actions.read(cx).get_mini_app_sai.is_pending() {
            return;
        }
        if Self::owns_practice_bot(bot_id, cx) {
            return;
        }

        let reflect_pending_in_ui = BotServicesStore::mini_app(bot_id, mini_app_id, cx).is_none();
        BotActionsStore::get_mini_app_action(bot_id, mini_app_id, reflect_pending_in_ui, cx);
    }

    pub fn create_mini_app(
        &mut self,
        bot_id: &str,
        short_name: &str,
        title: &str,
        description: &str,
        production_url: &str,
        development_url: &str,
        icon_url: &str,
        on_result: Option<BotActionResult<BotMiniAppModel>>,
        cx: &mut Context<Self>,
    ) {
        if self.actions.read(cx).create_mini_app_sai.is_pending() {
            return;
        }
        let practice = (bot_guide_sandbox_hooks().create_practice_mini_app)(
                bot_id,
                short_name,
                title,
                description,
                production_url,
                development_url,
                icon_url,
                cx,
            );
        if let Some(created) = practice {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(created, cx));
            }
            return;
        }

        BotActionsStore::create_mini_app_action(
            bot_id,
            short_name,
            title,
            description,
            production_url,
            development_url,
            icon_url,
            on_result,
            cx,
        );
    }

    pub fn update_mini_app(
        &mut self,
        bot_id: &str,
        mini_app_id: &str,
        values: BotMiniAppUpdateValues,
        on_result: Option<BotActionResult<BotMiniAppModel>>,
        cx: &mut Context<Self>,
    ) {
        if self.actions.read(cx).update_mini_app_sai.is_pending() {
            return;
        }
        let practice =
            (bot_guide_sandbox_hooks().update_practice_mini_app)(bot_id, mini_app_id, &values, cx);
        if let Some(updated) = practice {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(updated, cx));
            }
            return;
        }

        BotActionsStore::update_mini_app_action(bot_id, mini_app_id, &values, on_result, cx);
    }

    pub fn delete_mini_app(
        &mut self,
        bot_id: &str,
        mini_app_id: &str,
        on_result: Option<BotActionFinished>,
        cx: &mut Context<Self>,
    ) {
        if self.actions.read(cx).delete_mini_app_sai.is_pending() {
            return;
        }
        if (bot_guide_sandbox_hooks().delete_practice_mini_app)(bot_id, mini_app_id, cx) {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(cx));
            }
            return;
        }

        BotActionsStore::delete_mini_app_action(bot_id, mini_app_id, on_result, cx);
    }

    pub fn create_mini_app_launch_data(
        &mut self,
        bot_id: &str,
        mini_app_id: &str,
        chat_id: Option<&str>,
        query_id: Option<&str>,
        prefer_development: bool,
        start_param: Option<&str>,
        on_result: Option<BotActionResult<BotMiniAppLaunchData>>,
        on_error: Option<BotActionError>,
        cx: &mut Context<Self>,
    ) {
        if self.actions.read(cx).create_mini_app_launch_data_sai.is_pending() {
            return;
        }

        let practice = (bot_guide_sandbox_hooks().practice_mini_app_launch)(bot_id, mini_app_id, prefer_development, cx);
        if let Some(launch) = practice {
            if let Some(on_result) = on_result {
                cx.defer(move |cx| on_result(launch, cx));
            }
            return;
        }

        BotActionsStore::create_mini_app_launch_data_action(
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

    pub fn get_mini_app_launch(
        &mut self,
        bot_id: &str,
        mini_app_id: &str,
        chat_id: Option<&str>,
        query_id: Option<&str>,
        prefer_development: bool,
        start_param: Option<&str>,
        on_result: Option<BotActionResult<BotMiniAppLaunchData>>,
        on_error: Option<BotActionError>,
        cx: &mut Context<Self>,
    ) {
        if self.actions.read(cx).get_mini_app_launch_sai.is_pending() {
            return;
        }

        BotActionsStore::get_mini_app_launch_action(
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

    pub fn mini_app_launch_failure_text(&self, error: &anyhow::Error) -> String {
        if CanonErrors::is_connection_loss(error) {
            tr("sync_error_unavailable")
        } else {
            CanonErrors::refusal_text(error)
        }
    }

    pub fn enqueue_callback_query(
        &mut self,
        bot_id: &str,
        callback_query_id: &str,
        chat_id: Option<&str>,
        message_id: Option<&str>,
        inline_message_id: Option<&str>,
        data: Option<&str>,
        game_short_name: Option<&str>,
        on_result: Option<BotActionResult<BotCallbackEnqueueResult>>,
        on_error: Option<BotActionError>,
        cx: &mut Context<Self>,
    ) {
        let has_message = message_id.is_some_and(|id| !id.is_empty());
        let has_inline_message = inline_message_id.is_some_and(|id| !id.is_empty());
        if bot_id.is_empty() || !(has_message || has_inline_message) {
            return;
        }

        BotActionsStore::enqueue_callback_query_action(
            bot_id,
            callback_query_id,
            chat_id,
            message_id,
            inline_message_id,
            data,
            game_short_name,
            on_result,
            on_error,
            cx,
        );
    }

    pub fn resolve_mini_app_link(
        &mut self,
        raw_tag: &str,
        raw_slug: &str,
        chat_id: Option<&str>,
        query_id: Option<&str>,
        start_param: Option<&str>,
        prefer_development: bool,
        on_result: Option<BotActionResult<BotMiniAppLaunchData>>,
        on_error: Option<BotActionError>,
        cx: &mut Context<Self>,
    ) {
        let tag = BotTagRules::sanitize(raw_tag, false, false);
        let slug = trimming_whitespaces_and_newlines(raw_slug).to_lowercase();
        if self.actions.read(cx).resolve_mini_app_link_sai.is_pending()
            || !BotTagRules::is_valid(&tag)
            || !BotFieldRules::mini_app_short_name().is_satisfied(&slug)
        {
            return;
        }

        BotActionsStore::resolve_mini_app_link_action(
            &tag,
            &slug,
            chat_id,
            query_id,
            start_param,
            prefer_development,
            on_result,
            on_error,
            cx,
        );
    }

    pub fn clear_sensitive_results(&mut self, cx: &mut Context<Self>) {
        self.actions.update(cx, |actions, cx| {
            actions.create_bot_sai.reset_to_idle();
            actions.reveal_bot_token_sai.reset_to_idle();
            actions.rotate_bot_token_sai.reset_to_idle();
            actions.create_mini_app_launch_data_sai.reset_to_idle();
            actions.get_mini_app_launch_sai.reset_to_idle();
            actions.resolve_mini_app_link_sai.reset_to_idle();
            cx.notify();
        });
    }

    pub fn clear_settings_errors(&mut self, cx: &mut Context<Self>) {
        self.actions.update(cx, |actions, cx| {
            if !actions.update_bot_sai.is_pending() {
                actions.update_bot_sai.reset_to_idle();
            }
            if !actions.delete_bot_sai.is_pending() {
                actions.delete_bot_sai.reset_to_idle();
            }
            cx.notify();
        });
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.clear_sensitive_results(cx);
        self.clear_settings_errors(cx);
    }

    pub fn load_mention_candidates(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        composer_scope: &str,
        debounced: bool,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() || composer_scope.is_empty() {
            return;
        }

        let cache_key =
            BotServicesStore::mention_candidates_cache_key(chat_id, topic_id, query, cx);
        if cache_key.is_empty() {
            return;
        }

        let delay_milliseconds =
            if debounced { RiseTimingCanon::debounce("bot_mention_candidates").client_debounce_ms } else { 0 };
        let action = Self::mention_candidates_request(chat_id, topic_id, query, None, composer_scope);
        self.service.update(cx, |service, cx| {
            service.schedule_inline_composer_request(composer_scope, &cache_key, delay_milliseconds, None, action, cx);
        });
    }

    pub fn load_more_mention_candidates(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        composer_scope: &str,
        cx: &mut Context<Self>,
    ) {
        let cache_key =
            BotServicesStore::mention_candidates_cache_key(chat_id, topic_id, query, cx);
        let has_more =
            BotServicesStore::mention_candidates_has_more(chat_id, topic_id, query, cx);
        let cursor =
            BotServicesStore::mention_candidates_cursor(chat_id, topic_id, query, cx);
        let Some(cursor) = cursor.filter(|cursor| has_more && !cursor.is_empty()) else {
            return;
        };
        if !Self::can_request_trailing(&cache_key, cx) {
            return;
        }

        let action = Self::mention_candidates_request(chat_id, topic_id, query, Some(cursor), composer_scope);
        self.service.update(cx, |service, cx| {
            service.schedule_inline_composer_request(composer_scope, &cache_key, 0, None, action, cx);
        });
    }

    fn mention_candidates_request(
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        cursor: Option<String>,
        composer_scope: &str,
    ) -> Box<dyn FnOnce(&mut App) + 'static> {
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        let query = query.to_owned();
        let composer_scope = composer_scope.to_owned();
        Box::new(move |cx: &mut App| {
            BotActionsStore::get_mention_candidates_action(
                &chat_id,
                topic_id.as_deref(),
                &query,
                cursor.as_deref(),
                &composer_scope,
                cx,
            );
        })
    }

    pub fn load_inline_commands(
        &mut self,
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        language_code: &str,
        composer_scope: &str,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() || BotServicesStore::normalized_bot_tag(bot_tag).is_empty() || composer_scope.is_empty() {
            return;
        }

        let cache_key = BotServicesStore::inline_commands_cache_key(bot_tag, chat_id, topic_id, language_code, cx);
        if cache_key.is_empty() {
            return;
        }

        let action: Box<dyn FnOnce(&mut App) + 'static> = {
            let bot_tag = bot_tag.to_owned();
            let chat_id = chat_id.to_owned();
            let topic_id = topic_id.map(str::to_owned);
            let language_code = language_code.to_owned();
            let composer_scope = composer_scope.to_owned();
            Box::new(move |cx: &mut App| {
                BotActionsStore::get_inline_bot_commands_action(
                    &bot_tag,
                    &chat_id,
                    topic_id.as_deref(),
                    &language_code,
                    &composer_scope,
                    cx,
                );
            })
        };
        self.service.update(cx, |service, cx| {
            service.schedule_inline_composer_request(composer_scope, &cache_key, 0, None, action, cx);
        });
    }

    pub fn load_chat_commands(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        language_code: &str,
        composer_scope: &str,
        cx: &mut Context<Self>,
    ) {
        if chat_id.is_empty() || composer_scope.is_empty() {
            return;
        }

        let cache_key = BotServicesStore::chat_commands_cache_key(chat_id, topic_id, language_code, cx);
        if cache_key.is_empty() {
            return;
        }

        let action: Box<dyn FnOnce(&mut App) + 'static> = {
            let chat_id = chat_id.to_owned();
            let topic_id = topic_id.map(str::to_owned);
            let language_code = language_code.to_owned();
            let composer_scope = composer_scope.to_owned();
            Box::new(move |cx: &mut App| {
                BotActionsStore::get_chat_bot_commands_action(
                    &chat_id,
                    topic_id.as_deref(),
                    &language_code,
                    &composer_scope,
                    cx,
                );
            })
        };
        self.service.update(cx, |service, cx| {
            service.schedule_inline_composer_request(composer_scope, &cache_key, 0, None, action, cx);
        });
    }

    pub fn query_inline_results(
        &mut self,
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        composer_scope: &str,
        debounced: bool,
        cx: &mut Context<Self>,
    ) {
        let normalized_query = BotServicesStore::normalized_inline_result_query(query);
        if chat_id.is_empty()
            || BotServicesStore::normalized_bot_tag(bot_tag).is_empty()
            || normalized_query.is_empty()
            || composer_scope.is_empty()
        {
            return;
        }

        let cache_key = BotServicesStore::inline_results_cache_key(bot_tag, chat_id, topic_id, &normalized_query, cx);
        if cache_key.is_empty() {
            return;
        }

        let delay_milliseconds =
            if debounced { RiseTimingCanon::debounce("bot_inline_results").client_debounce_ms } else { 0 };
        let response_timeout =
            RiseCanonEndpoints::required(rise_wire_methods::bot::SERVICE, rise_wire_methods::bot::QUERY_INLINE_RESULTS)
                .client_timeout();
        let action = Self::inline_results_request(bot_tag, chat_id, topic_id, &normalized_query, None, composer_scope);
        self.service.update(cx, |service, cx| {
            service.schedule_inline_composer_request(
                composer_scope,
                &cache_key,
                delay_milliseconds,
                Some(response_timeout),
                action,
                cx,
            );
        });
    }

    pub fn load_more_inline_results(
        &mut self,
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        composer_scope: &str,
        cx: &mut Context<Self>,
    ) {
        let page = BotServicesStore::inline_results(bot_tag, chat_id, topic_id, query, cx);
        let cache_key =
            BotServicesStore::inline_results_cache_key(bot_tag, chat_id, topic_id, query, cx);
        let Some(offset) = page.next_offset.filter(|offset| page.has_more && !offset.is_empty()) else {
            return;
        };
        if !Self::can_request_trailing(&cache_key, cx) {
            return;
        }

        let action = Self::inline_results_request(bot_tag, chat_id, topic_id, query, Some(offset), composer_scope);
        self.service.update(cx, |service, cx| {
            service.schedule_inline_composer_request(composer_scope, &cache_key, 0, None, action, cx);
        });
    }

    fn inline_results_request(
        bot_tag: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        query: &str,
        offset: Option<String>,
        composer_scope: &str,
    ) -> Box<dyn FnOnce(&mut App) + 'static> {
        let bot_tag = bot_tag.to_owned();
        let chat_id = chat_id.to_owned();
        let topic_id = topic_id.map(str::to_owned);
        let query = query.to_owned();
        let composer_scope = composer_scope.to_owned();
        Box::new(move |cx: &mut App| {
            BotActionsStore::query_inline_results_action(
                &bot_tag,
                &chat_id,
                topic_id.as_deref(),
                &query,
                offset.as_deref(),
                &composer_scope,
                cx,
            );
        })
    }

    pub fn send_inline_result(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        query_id: &str,
        result: BotInlineResultModel,
        on_result: Option<BotInlineSendResult>,
        cx: &mut Context<Self>,
    ) -> bool {
        if chat_id.is_empty() || query_id.is_empty() || result.id.is_empty() {
            return false;
        }

        let send_key = BotServicesStore::inline_send_key(query_id, &result.id);
        if !self.service.update(cx, |service, cx| service.begin_inline_result_send(&send_key, cx)) {
            return false;
        }

        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let resolved_reply_to_id = reply_to_id.map(|reply_to_id| {
            MessageServicesStore::shared(cx).update(cx, |messages, cx| messages.server_message_id(reply_to_id, &scope, cx))
        });

        let request_id = Uuid::new_v4().uuid_string().to_lowercase();
        BotActionsStore::send_inline_result_action(
            chat_id,
            topic_id,
            post_topic_id,
            thread_root_id,
            resolved_reply_to_id.as_deref(),
            query_id,
            &result,
            &request_id,
            on_result,
            cx,
        );
        true
    }

    pub fn cancel_inline_composer(&mut self, scope: &str, cx: &mut Context<Self>) {
        self.service.update(cx, |service, cx| service.cancel_inline_composer_request(scope, cx));
    }

    fn owns_practice_bot(bot_id: &str, cx: &App) -> bool {
        (bot_guide_sandbox_hooks().owns_practice_bot)(bot_id, cx)
    }

    fn practice_token(bot_id: &str, cx: &App) -> Option<BotTokenResult> {
        (bot_guide_sandbox_hooks().practice_token)(bot_id, cx)
    }

    fn effective_user_id_for_wire(cx: &mut App) -> Option<String> {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().filter(|user_id| !user_id.is_empty())
    }

    fn can_request_trailing(cache_key: &str, cx: &mut App) -> bool {
        SaiWs::pagination_state(cache_key, cx).read(cx).trailing().can_request_automatically()
    }
}
