use gpui::App;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::engine::rise_user_achievements_presentation::RiseUserAchievementsSnapshot;
use crate::modules::user::stores::user::user_services::user_guide_sandbox_hooks::{
    UserGuideSandboxHooks, set_user_guide_sandbox_hooks,
};

pub fn install_user_guide_sandbox_hooks() {
    let _ = set_user_guide_sandbox_hooks(UserGuideSandboxHooks {
        owns_user_id,
        owns_theme_scene,
        owns_profile_look_scene,
        owns_goals_scene,
        owns_achievements_scene,
        achievements_snapshot,
        set_achievement_hidden,
        panda_block_scope,
        set_panda_block_scope,
        set_practice_mute_all,
        set_practice_mute_exception,
        absorb_mute_overrides,
    });
}

fn owns_user_id(id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_user_id(id))
}

fn owns_theme_scene(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_theme_scene())
}

fn owns_profile_look_scene(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_profile_look_scene())
}

fn owns_goals_scene(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_goals_scene())
}

fn owns_achievements_scene(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_achievements_scene())
}

fn achievements_snapshot(real: RiseUserAchievementsSnapshot, cx: &App) -> RiseUserAchievementsSnapshot {
    match GuideSandbox::existing(cx) {
        Some(sandbox) => sandbox.read(cx).achievements_snapshot(real),
        None => real,
    }
}

fn set_achievement_hidden(id: i64, hidden: bool, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_achievement_hidden(id, hidden, cx));
}

fn panda_block_scope(cx: &App) -> Option<RiseBlockScope> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).panda_block_scope)
}

fn set_panda_block_scope(scope: Option<RiseBlockScope>, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| {
        sandbox.panda_block_scope = scope;
        cx.notify();
    });
}

fn set_practice_mute_all(mute_all: bool, cx: &mut App) -> bool {
    if !owns_mute_scene(cx) {
        return false;
    }
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_practice_mute_all(mute_all, cx))
}

fn set_practice_mute_exception(chat_id: &str, add: bool, cx: &mut App) -> bool {
    if !owns_mute_scene(cx) {
        return false;
    }
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_practice_mute_exception(chat_id, add, cx))
}

fn absorb_mute_overrides(mute_all: Option<bool>, chat_ids: Option<&[String]>, cx: &mut App) -> bool {
    if !owns_mute_scene(cx) {
        return false;
    }
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.absorb_mute_overrides(mute_all, chat_ids, cx))
}

fn owns_mute_scene(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_mute_scene())
}
