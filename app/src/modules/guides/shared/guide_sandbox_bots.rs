use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use gpui::Context;
use rise_canon::rise_wire_methods::bot;
use rise_canon::RiseGuideSignal;
use rise_canon::RiseIdentityCanon;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::shared::bot_ui_validation::BotCreateFlowState;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{
    BotCommandModel, BotMiniAppLaunchData, BotMiniAppModel, BotMiniAppUpdateValues, BotModel, BotTokenResult,
    BotUpdateValues, BotWebhookInfoModel,
};
use crate::modules::guides::engine::rise_guides_models::RiseGuide;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

const OPTIMISTIC_TEMPORARY_PREFIX: &str = RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX;

/// the bot lessons on the reader's real bot screens
/// practice bot lives on the computer so the id wall stops requests naming it
/// creation lesson starts without it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuideSandboxBotsScene {
    pub bot: Option<BotModel>,
    pub commands: HashMap<String, Vec<BotCommandModel>>,
    pub webhook: Option<BotWebhookInfoModel>,
    pub mini_apps: Vec<BotMiniAppModel>,
}

const PRACTICE_BOT_TOKEN: &str = "7000000000:temp_guide_practice_token_not_for_use";

static BOTS_REFUSED_METHODS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    [
        bot::CREATE_BOT,
        bot::UPDATE_BOT,
        bot::DELETE_BOT,
        bot::REVEAL_BOT_TOKEN,
        bot::ROTATE_BOT_TOKEN,
        bot::SET_BOT_COMMANDS,
        bot::DELETE_BOT_COMMANDS,
        bot::SET_BOT_WEBHOOK,
        bot::DELETE_BOT_WEBHOOK,
        bot::CREATE_MINI_APP,
        bot::UPDATE_MINI_APP,
        bot::DELETE_MINI_APP,
    ]
    .into_iter()
    .map(|method| format!("{}.{method}", bot::SERVICE))
    .collect()
});

impl GuideSandbox {
    pub fn practice_bot_id() -> String {
        format!("{OPTIMISTIC_TEMPORARY_PREFIX}guide_bot")
    }

    pub fn arm_bots(&mut self, guide: &RiseGuide, cx: &mut Context<Self>) {
        if !self.scenes.contains(&Self::bots_scene_id()) {
            return;
        }
        let teaches_creation = guide
            .steps
            .iter()
            .any(|step| step.advance_signal == Some(RiseGuideSignal::BotCreated));
        let bot = if teaches_creation {
            None
        } else {
            Some(Self::make_practice_bot(
                tr("guide_scene_bot_name"),
                "panda_helper_bot".to_owned(),
                tr("guide_scene_bot_text"),
                true,
                true,
                cx,
            ))
        };
        self.bots_scene = Some(GuideSandboxBotsScene {
            bot,
            ..GuideSandboxBotsScene::default()
        });
        Self::refuse_methods(&BOTS_REFUSED_METHODS);
        cx.notify();
    }

    pub fn withdraw_bots(&mut self, cx: &mut Context<Self>) {
        if self.bots_scene.is_none() {
            return;
        }
        self.bots_scene = None;
        cx.notify();
        let flow = BotCreateFlowState::shared(cx);
        let practice_bot_id = Self::practice_bot_id();
        if flow.read(cx).created_bot.as_ref().is_some_and(|bot| bot.id == practice_bot_id) {
            flow.update(cx, |flow, cx| flow.clear_creation_result(cx));
        }
        GuideFacts::set(RiseGuideSignal::CreateBotFormOpened, false, cx);
        GuideFacts::set(RiseGuideSignal::CreateBotSettingsOpened, false, cx);
        GuideFacts::set(RiseGuideSignal::CreateBotFormValid, false, cx);
        GuideFacts::set(RiseGuideSignal::BotCommandDrafted, false, cx);
        GuideFacts::set(RiseGuideSignal::BotCommandsValid, false, cx);
        GuideFacts::set(RiseGuideSignal::BotWebhookReady, false, cx);
        GuideFacts::set(RiseGuideSignal::BotMiniAppReady, false, cx);
    }

    pub fn owns_bots_scene(&self) -> bool {
        self.is_armed && self.bots_scene.is_some()
    }

    pub fn owns_practice_bot(&self, bot_id: &str) -> bool {
        self.owns_bots_scene()
            && bot_id == Self::practice_bot_id()
            && self.bots_scene.as_ref().is_some_and(|scene| scene.bot.is_some())
    }

    pub fn owned_bots_snapshot(&self, real: Vec<BotModel>) -> Vec<BotModel> {
        let Some(bot) = self
            .bots_scene
            .as_ref()
            .filter(|_| self.owns_bots_scene())
            .and_then(|scene| scene.bot.clone())
        else {
            return real;
        };
        let mut list = vec![bot];
        list.extend(real);
        list
    }

    pub fn practice_bot(&self, id: &str) -> Option<BotModel> {
        if !self.owns_practice_bot(id) {
            return None;
        }
        self.bots_scene.as_ref().and_then(|scene| scene.bot.clone())
    }

    pub fn practice_commands(&self, bot_id: &str, scope: &str, language_code: &str) -> Option<Vec<BotCommandModel>> {
        if !self.owns_practice_bot(bot_id) {
            return None;
        }
        Some(
            self.bots_scene
                .as_ref()
                .and_then(|scene| scene.commands.get(&commands_key(scope, language_code)).cloned())
                .unwrap_or_default(),
        )
    }

    /// none inside the outer option means the practice bot has no webhook yet
    pub fn practice_webhook(&self, bot_id: &str) -> Option<Option<BotWebhookInfoModel>> {
        if !self.owns_practice_bot(bot_id) {
            return None;
        }
        Some(self.bots_scene.as_ref().and_then(|scene| scene.webhook.clone()))
    }

    pub fn practice_mini_apps(&self, bot_id: &str) -> Option<Vec<BotMiniAppModel>> {
        if !self.owns_practice_bot(bot_id) {
            return None;
        }
        Some(self.bots_scene.as_ref().map(|scene| scene.mini_apps.clone()).unwrap_or_default())
    }

    pub fn create_practice_bot(
        &mut self,
        name: &str,
        tag: &str,
        description: &str,
        can_join_groups: bool,
        privacy_mode: bool,
        cx: &mut Context<Self>,
    ) -> Option<(BotModel, String)> {
        if !self.owns_bots_scene() {
            return None;
        }
        let bot = Self::make_practice_bot(
            trimming_whitespaces_and_newlines(name).to_owned(),
            tag.to_owned(),
            trimming_whitespaces_and_newlines(description).to_owned(),
            can_join_groups,
            privacy_mode,
            cx,
        );
        if let Some(scene) = self.bots_scene.as_mut() {
            scene.bot = Some(bot.clone());
        }
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::BotCreated);
        Some((bot, PRACTICE_BOT_TOKEN.to_owned()))
    }

    pub fn update_practice_bot(
        &mut self,
        bot_id: &str,
        values: &BotUpdateValues,
        cx: &mut Context<Self>,
    ) -> Option<BotModel> {
        if !self.owns_practice_bot(bot_id) {
            return None;
        }
        let scene = self.bots_scene.as_mut()?;
        let bot = scene.bot.as_ref()?;
        let updated = BotModel {
            id: bot.id.clone(),
            owner_user_id: bot.owner_user_id.clone(),
            bot_user_id: bot.bot_user_id.clone(),
            private_chat_id: bot.private_chat_id.clone(),
            tag: values.tag.clone().unwrap_or_else(|| bot.tag.clone()),
            name: values.name.clone().unwrap_or_else(|| bot.name.clone()),
            description: values.description.clone().unwrap_or_else(|| bot.description.clone()),
            avatar_url: bot.avatar_url.clone(),
            banner_url: bot.banner_url.clone(),
            is_active: bot.is_active,
            can_join_groups: values.can_join_groups.unwrap_or(bot.can_join_groups),
            privacy_mode: values.privacy_mode.unwrap_or(bot.privacy_mode),
            created_at: bot.created_at,
            updated_at: now_epoch_ms(),
            bot_user_chat_id: bot.bot_user_chat_id.clone(),
            is_owned_by_requester: true,
            moderation_status: BotModel::DEFAULT_MODERATION_STATUS.to_owned(),
            is_system: false,
        };
        scene.bot = Some(updated.clone());
        cx.notify();
        Some(updated)
    }

    pub fn delete_practice_bot(&mut self, bot_id: &str, cx: &mut Context<Self>) -> bool {
        if !self.owns_practice_bot(bot_id) {
            return false;
        }
        if let Some(scene) = self.bots_scene.as_mut() {
            scene.bot = None;
        }
        cx.notify();
        true
    }

    pub fn practice_token(&self, bot_id: &str) -> Option<BotTokenResult> {
        if !self.owns_practice_bot(bot_id) {
            return None;
        }
        Some(BotTokenResult {
            bot_id: bot_id.to_owned(),
            token: PRACTICE_BOT_TOKEN.to_owned(),
            rotated_at: now_epoch_ms(),
        })
    }

    pub fn set_practice_commands(
        &mut self,
        bot_id: &str,
        commands: Vec<BotCommandModel>,
        scope: &str,
        language_code: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.owns_practice_bot(bot_id) {
            return false;
        }
        if let Some(scene) = self.bots_scene.as_mut() {
            scene.commands.insert(commands_key(scope, language_code), commands);
        }
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::BotCommandsSaved);
        true
    }

    pub fn delete_practice_commands(
        &mut self,
        bot_id: &str,
        scope: &str,
        language_code: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.owns_practice_bot(bot_id) {
            return false;
        }
        if let Some(scene) = self.bots_scene.as_mut() {
            scene.commands.remove(&commands_key(scope, language_code));
        }
        cx.notify();
        true
    }

    pub fn set_practice_webhook(
        &mut self,
        bot_id: &str,
        url: &str,
        max_connections: i64,
        allowed_updates: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Option<BotWebhookInfoModel> {
        if !self.owns_practice_bot(bot_id) {
            return None;
        }
        let webhook = BotWebhookInfoModel {
            bot_id: bot_id.to_owned(),
            url: url.to_owned(),
            enabled: true,
            has_custom_certificate: false,
            pending_update_count: 0,
            last_error_date: 0,
            last_error_message: String::new(),
            max_connections,
            allowed_updates,
            updated_at: now_epoch_ms(),
        };
        if let Some(scene) = self.bots_scene.as_mut() {
            scene.webhook = Some(webhook.clone());
        }
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::BotWebhookSaved);
        Some(webhook)
    }

    pub fn delete_practice_webhook(&mut self, bot_id: &str, cx: &mut Context<Self>) -> bool {
        if !self.owns_practice_bot(bot_id) {
            return false;
        }
        if let Some(scene) = self.bots_scene.as_mut() {
            scene.webhook = None;
        }
        cx.notify();
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_practice_mini_app(
        &mut self,
        bot_id: &str,
        short_name: &str,
        title: &str,
        description: &str,
        production_url: &str,
        development_url: &str,
        icon_url: &str,
        cx: &mut Context<Self>,
    ) -> Option<BotMiniAppModel> {
        if !self.owns_practice_bot(bot_id) {
            return None;
        }
        let scene = self.bots_scene.as_mut()?;
        let now = now_epoch_ms();
        let mini_app = BotMiniAppModel {
            id: format!("{}_app_{}", Self::practice_bot_id(), scene.mini_apps.len() + 1),
            bot_id: bot_id.to_owned(),
            short_name: short_name.to_owned(),
            title: title.to_owned(),
            description: description.to_owned(),
            production_url: production_url.to_owned(),
            development_url: development_url.to_owned(),
            icon_url: icon_url.to_owned(),
            is_active: true,
            created_at: now,
            updated_at: now,
        };
        scene.mini_apps.insert(0, mini_app.clone());
        cx.notify();
        GuideSignalBus::emit(RiseGuideSignal::BotMiniAppCreated);
        Some(mini_app)
    }

    pub fn update_practice_mini_app(
        &mut self,
        bot_id: &str,
        mini_app_id: &str,
        values: &BotMiniAppUpdateValues,
        cx: &mut Context<Self>,
    ) -> Option<BotMiniAppModel> {
        if !self.owns_practice_bot(bot_id) {
            return None;
        }
        let scene = self.bots_scene.as_mut()?;
        let app = scene.mini_apps.iter_mut().find(|app| app.id == mini_app_id)?;
        let updated = BotMiniAppModel {
            id: app.id.clone(),
            bot_id: app.bot_id.clone(),
            short_name: app.short_name.clone(),
            title: values.title.clone().unwrap_or_else(|| app.title.clone()),
            description: values.description.clone().unwrap_or_else(|| app.description.clone()),
            production_url: values.production_url.clone().unwrap_or_else(|| app.production_url.clone()),
            development_url: values.development_url.clone().unwrap_or_else(|| app.development_url.clone()),
            icon_url: values.icon_url.clone().unwrap_or_else(|| app.icon_url.clone()),
            is_active: values.is_active.unwrap_or(app.is_active),
            created_at: app.created_at,
            updated_at: now_epoch_ms(),
        };
        *app = updated.clone();
        cx.notify();
        Some(updated)
    }

    pub fn delete_practice_mini_app(&mut self, bot_id: &str, mini_app_id: &str, cx: &mut Context<Self>) -> bool {
        if !self.owns_practice_bot(bot_id) {
            return false;
        }
        if let Some(scene) = self.bots_scene.as_mut() {
            scene.mini_apps.retain(|app| app.id != mini_app_id);
        }
        cx.notify();
        true
    }

    /// practice mini app opens its own address, no server to sign a launch
    pub fn practice_mini_app_launch(
        &self,
        bot_id: &str,
        mini_app_id: &str,
        prefer_development: bool,
    ) -> Option<BotMiniAppLaunchData> {
        if !self.owns_practice_bot(bot_id) {
            return None;
        }
        let app = self.bots_scene.as_ref()?.mini_apps.iter().find(|app| app.id == mini_app_id)?;
        let url = if prefer_development && !app.development_url.is_empty() {
            app.development_url.clone()
        } else {
            app.production_url.clone()
        };
        Some(BotMiniAppLaunchData {
            launch_url: url,
            init_data: String::new(),
            expires_at: 0,
        })
    }

    fn make_practice_bot(
        name: String,
        tag: String,
        description: String,
        can_join_groups: bool,
        privacy_mode: bool,
        cx: &mut Context<Self>,
    ) -> BotModel {
        let now = (Self::scene_stamp() * 1000.0) as i64;
        let owner_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        BotModel {
            id: Self::practice_bot_id(),
            owner_user_id,
            bot_user_id: Self::practice_bot_id(),
            private_chat_id: String::new(),
            tag,
            name,
            description,
            avatar_url: String::new(),
            banner_url: String::new(),
            is_active: true,
            can_join_groups,
            privacy_mode,
            created_at: now,
            updated_at: now,
            bot_user_chat_id: String::new(),
            is_owned_by_requester: true,
            moderation_status: BotModel::DEFAULT_MODERATION_STATUS.to_owned(),
            is_system: false,
        }
    }
}

fn now_epoch_ms() -> i64 {
    (Date::now().time_interval_since_1970() * 1000.0) as i64
}

fn commands_key(scope: &str, language_code: &str) -> String {
    format!("{scope}|{language_code}")
}
