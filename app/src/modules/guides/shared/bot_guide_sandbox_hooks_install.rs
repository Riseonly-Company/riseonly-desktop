use gpui::App;

use crate::modules::bot::stores::bot::bot_actions::bot_types::{
    BotCommandModel, BotMiniAppLaunchData, BotMiniAppModel, BotMiniAppUpdateValues, BotModel, BotTokenResult,
    BotUpdateValues, BotWebhookInfoModel,
};
use crate::modules::bot::stores::bot::bot_services::bot_guide_sandbox_hooks::{
    BotGuideSandboxHooks, set_bot_guide_sandbox_hooks,
};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

pub(crate) fn install_bot_guide_sandbox_hooks() {
    let _ = set_bot_guide_sandbox_hooks(BotGuideSandboxHooks {
        owns_practice_bot,
        owned_bots_snapshot,
        practice_bot,
        practice_commands,
        practice_webhook,
        practice_mini_apps,
        create_practice_bot,
        update_practice_bot,
        delete_practice_bot,
        practice_token,
        set_practice_commands,
        delete_practice_commands,
        set_practice_webhook,
        delete_practice_webhook,
        create_practice_mini_app,
        update_practice_mini_app,
        delete_practice_mini_app,
        practice_mini_app_launch,
    });
}

fn owns_practice_bot(bot_id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_practice_bot(bot_id))
}

fn owned_bots_snapshot(real: Vec<BotModel>, cx: &App) -> Vec<BotModel> {
    match GuideSandbox::existing(cx) {
        Some(sandbox) => sandbox.read(cx).owned_bots_snapshot(real),
        None => real,
    }
}

fn practice_bot(id: &str, cx: &App) -> Option<BotModel> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).practice_bot(id))
}

fn practice_commands(bot_id: &str, scope: &str, language_code: &str, cx: &App) -> Option<Vec<BotCommandModel>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).practice_commands(bot_id, scope, language_code))
}

fn practice_webhook(bot_id: &str, cx: &App) -> Option<Option<BotWebhookInfoModel>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).practice_webhook(bot_id))
}

fn practice_mini_apps(bot_id: &str, cx: &App) -> Option<Vec<BotMiniAppModel>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).practice_mini_apps(bot_id))
}

fn create_practice_bot(
    name: &str,
    tag: &str,
    description: &str,
    can_join_groups: bool,
    privacy_mode: bool,
    cx: &mut App,
) -> Option<(BotModel, String)> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| {
        sandbox.create_practice_bot(name, tag, description, can_join_groups, privacy_mode, cx)
    })
}

fn update_practice_bot(bot_id: &str, values: &BotUpdateValues, cx: &mut App) -> Option<BotModel> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.update_practice_bot(bot_id, values, cx))
}

fn delete_practice_bot(bot_id: &str, cx: &mut App) -> bool {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_practice_bot(bot_id, cx))
}

fn practice_token(bot_id: &str, cx: &App) -> Option<BotTokenResult> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).practice_token(bot_id))
}

fn set_practice_commands(
    bot_id: &str,
    commands: Vec<BotCommandModel>,
    scope: &str,
    language_code: &str,
    cx: &mut App,
) -> bool {
    GuideSandbox::shared(cx)
        .update(cx, |sandbox, cx| sandbox.set_practice_commands(bot_id, commands, scope, language_code, cx))
}

fn delete_practice_commands(bot_id: &str, scope: &str, language_code: &str, cx: &mut App) -> bool {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_practice_commands(bot_id, scope, language_code, cx))
}

fn set_practice_webhook(
    bot_id: &str,
    url: &str,
    max_connections: i64,
    allowed_updates: Vec<String>,
    cx: &mut App,
) -> Option<BotWebhookInfoModel> {
    GuideSandbox::shared(cx)
        .update(cx, |sandbox, cx| sandbox.set_practice_webhook(bot_id, url, max_connections, allowed_updates, cx))
}

fn delete_practice_webhook(bot_id: &str, cx: &mut App) -> bool {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_practice_webhook(bot_id, cx))
}

#[allow(clippy::too_many_arguments)]
fn create_practice_mini_app(
    bot_id: &str,
    short_name: &str,
    title: &str,
    description: &str,
    production_url: &str,
    development_url: &str,
    icon_url: &str,
    cx: &mut App,
) -> Option<BotMiniAppModel> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| {
        sandbox.create_practice_mini_app(
            bot_id,
            short_name,
            title,
            description,
            production_url,
            development_url,
            icon_url,
            cx,
        )
    })
}

fn update_practice_mini_app(
    bot_id: &str,
    mini_app_id: &str,
    values: &BotMiniAppUpdateValues,
    cx: &mut App,
) -> Option<BotMiniAppModel> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.update_practice_mini_app(bot_id, mini_app_id, values, cx))
}

fn delete_practice_mini_app(bot_id: &str, mini_app_id: &str, cx: &mut App) -> bool {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.delete_practice_mini_app(bot_id, mini_app_id, cx))
}

fn practice_mini_app_launch(
    bot_id: &str,
    mini_app_id: &str,
    prefer_development: bool,
    cx: &App,
) -> Option<BotMiniAppLaunchData> {
    GuideSandbox::existing(cx)
        .and_then(|sandbox| sandbox.read(cx).practice_mini_app_launch(bot_id, mini_app_id, prefer_development))
}
