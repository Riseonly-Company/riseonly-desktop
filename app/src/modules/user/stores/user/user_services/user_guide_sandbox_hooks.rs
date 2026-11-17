use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::engine::rise_user_achievements_presentation::RiseUserAchievementsSnapshot;

pub struct UserGuideSandboxHooks {
    pub owns_user_id: fn(id: &str, cx: &App) -> bool,
    pub owns_theme_scene: fn(cx: &App) -> bool,
    pub owns_profile_look_scene: fn(cx: &App) -> bool,
    pub owns_goals_scene: fn(cx: &App) -> bool,
    pub owns_achievements_scene: fn(cx: &App) -> bool,
    pub achievements_snapshot: fn(real: RiseUserAchievementsSnapshot, cx: &App) -> RiseUserAchievementsSnapshot,
    pub set_achievement_hidden: fn(id: i64, hidden: bool, cx: &mut App),
    pub panda_block_scope: fn(cx: &App) -> Option<RiseBlockScope>,
    pub set_panda_block_scope: fn(scope: Option<RiseBlockScope>, cx: &mut App),
    pub set_practice_mute_all: fn(mute_all: bool, cx: &mut App) -> bool,
    pub set_practice_mute_exception: fn(chat_id: &str, add: bool, cx: &mut App) -> bool,
    pub absorb_mute_overrides: fn(mute_all: Option<bool>, chat_ids: Option<&[String]>, cx: &mut App) -> bool,
}

pub(crate) static HOOKS: FeatureHookSlot<UserGuideSandboxHooks> = FeatureHookSlot::new("HOOKS");

static UNINSTALLED: UserGuideSandboxHooks = UserGuideSandboxHooks {
    owns_user_id: |_, _| false,
    owns_theme_scene: |_| false,
    owns_profile_look_scene: |_| false,
    owns_goals_scene: |_| false,
    owns_achievements_scene: |_| false,
    achievements_snapshot: |real, _| real,
    set_achievement_hidden: |_, _, _| {},
    panda_block_scope: |_| None,
    set_panda_block_scope: |_, _| {},
    set_practice_mute_all: |_, _| false,
    set_practice_mute_exception: |_, _, _| false,
    absorb_mute_overrides: |_, _, _| false,
};

pub fn set_user_guide_sandbox_hooks(hooks: UserGuideSandboxHooks) -> Result<(), UserGuideSandboxHooks> {
    HOOKS.set(hooks)
}

pub fn user_guide_sandbox_hooks() -> &'static UserGuideSandboxHooks {
    HOOKS.get().unwrap_or(&UNINSTALLED)
}
