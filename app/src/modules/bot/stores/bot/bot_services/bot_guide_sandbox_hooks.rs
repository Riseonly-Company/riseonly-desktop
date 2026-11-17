use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{
    BotCommandModel, BotMiniAppLaunchData, BotMiniAppModel, BotMiniAppUpdateValues, BotModel, BotTokenResult,
    BotUpdateValues, BotWebhookInfoModel,
};

pub(crate) struct BotGuideSandboxHooks {
    pub owns_practice_bot: fn(bot_id: &str, cx: &App) -> bool,
    pub owned_bots_snapshot: fn(real: Vec<BotModel>, cx: &App) -> Vec<BotModel>,
    pub practice_bot: fn(id: &str, cx: &App) -> Option<BotModel>,
    pub practice_commands:
        fn(bot_id: &str, scope: &str, language_code: &str, cx: &App) -> Option<Vec<BotCommandModel>>,
    pub practice_webhook: fn(bot_id: &str, cx: &App) -> Option<Option<BotWebhookInfoModel>>,
    pub practice_mini_apps: fn(bot_id: &str, cx: &App) -> Option<Vec<BotMiniAppModel>>,
    pub create_practice_bot: fn(
        name: &str,
        tag: &str,
        description: &str,
        can_join_groups: bool,
        privacy_mode: bool,
        cx: &mut App,
    ) -> Option<(BotModel, String)>,
    pub update_practice_bot: fn(bot_id: &str, values: &BotUpdateValues, cx: &mut App) -> Option<BotModel>,
    pub delete_practice_bot: fn(bot_id: &str, cx: &mut App) -> bool,
    pub practice_token: fn(bot_id: &str, cx: &App) -> Option<BotTokenResult>,
    pub set_practice_commands:
        fn(bot_id: &str, commands: Vec<BotCommandModel>, scope: &str, language_code: &str, cx: &mut App) -> bool,
    pub delete_practice_commands: fn(bot_id: &str, scope: &str, language_code: &str, cx: &mut App) -> bool,
    pub set_practice_webhook: fn(
        bot_id: &str,
        url: &str,
        max_connections: i64,
        allowed_updates: Vec<String>,
        cx: &mut App,
    ) -> Option<BotWebhookInfoModel>,
    pub delete_practice_webhook: fn(bot_id: &str, cx: &mut App) -> bool,
    pub create_practice_mini_app: fn(
        bot_id: &str,
        short_name: &str,
        title: &str,
        description: &str,
        production_url: &str,
        development_url: &str,
        icon_url: &str,
        cx: &mut App,
    ) -> Option<BotMiniAppModel>,
    pub update_practice_mini_app:
        fn(bot_id: &str, mini_app_id: &str, values: &BotMiniAppUpdateValues, cx: &mut App) -> Option<BotMiniAppModel>,
    pub delete_practice_mini_app: fn(bot_id: &str, mini_app_id: &str, cx: &mut App) -> bool,
    pub practice_mini_app_launch:
        fn(bot_id: &str, mini_app_id: &str, prefer_development: bool, cx: &App) -> Option<BotMiniAppLaunchData>,
}

pub(crate) static HOOKS: FeatureHookSlot<BotGuideSandboxHooks> = FeatureHookSlot::new("HOOKS");

static UNINSTALLED: BotGuideSandboxHooks = BotGuideSandboxHooks {
    owns_practice_bot: |_, _| false,
    owned_bots_snapshot: |real, _| real,
    practice_bot: |_, _| None,
    practice_commands: |_, _, _, _| None,
    practice_webhook: |_, _| None,
    practice_mini_apps: |_, _| None,
    create_practice_bot: |_, _, _, _, _, _| None,
    update_practice_bot: |_, _, _| None,
    delete_practice_bot: |_, _| false,
    practice_token: |_, _| None,
    set_practice_commands: |_, _, _, _, _| false,
    delete_practice_commands: |_, _, _, _| false,
    set_practice_webhook: |_, _, _, _, _| None,
    delete_practice_webhook: |_, _| false,
    create_practice_mini_app: |_, _, _, _, _, _, _, _| None,
    update_practice_mini_app: |_, _, _, _| None,
    delete_practice_mini_app: |_, _, _| false,
    practice_mini_app_launch: |_, _, _, _| None,
};

pub(crate) fn set_bot_guide_sandbox_hooks(hooks: BotGuideSandboxHooks) -> Result<(), BotGuideSandboxHooks> {
    HOOKS.set(hooks)
}

pub(crate) fn bot_guide_sandbox_hooks() -> &'static BotGuideSandboxHooks {
    HOOKS.get().unwrap_or(&UNINSTALLED)
}
