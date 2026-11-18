use gpui::{App, Context, Entity};

use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;
use crate::modules::user::engine::core::rise_user_achievements_resource_repository::RiseUserAchievementsCacheSource;
use crate::modules::user::engine::rise_user_achievements_presentation::{
    RiseUserAchievementsLoadState, RiseUserAchievementsPresentation, RiseUserAchievementsShowcase,
    RiseUserAchievementsSnapshot,
};
use crate::modules::user::stores::user::user_services::user_guide_sandbox_hooks::user_guide_sandbox_hooks;

pub struct UserAchievementsServicesStore {}

impl UserAchievementsServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn snapshot(cx: &mut App) -> RiseUserAchievementsSnapshot {
        let real = RiseUserAchievementsPresentation::shared(cx).read(cx).snapshot().clone();
        (user_guide_sandbox_hooks().achievements_snapshot)(real, cx)
    }

    pub fn own_achievement(id: i64, cx: &mut App) -> Option<RiseAchievement> {
        RiseUserAchievementsPresentation::shared(cx)
            .read(cx)
            .snapshot()
            .achievements
            .iter()
            .find(|achievement| achievement.id == id)
            .cloned()
    }

    pub fn achievements(cx: &mut App) -> Vec<RiseAchievement> {
        Self::snapshot(cx).achievements
    }

    pub fn visible_achievements(cx: &mut App) -> Vec<RiseAchievement> {
        Self::snapshot(cx).visible_achievements
    }

    pub fn hidden_achievements(cx: &mut App) -> Vec<RiseAchievement> {
        Self::snapshot(cx).hidden_achievements
    }

    pub fn total(cx: &mut App) -> i64 {
        Self::snapshot(cx).total
    }

    pub fn hidden_count(cx: &mut App) -> i64 {
        Self::snapshot(cx).hidden_count
    }

    pub fn catalog_total(cx: &mut App) -> i64 {
        Self::snapshot(cx).catalog_total
    }

    pub fn earned_count(cx: &mut App) -> i64 {
        Self::snapshot(cx).earned_count()
    }

    pub fn progress_total(cx: &mut App) -> i64 {
        Self::snapshot(cx).progress_total()
    }

    pub fn achievements_rule(cx: &mut App) -> String {
        Self::snapshot(cx).achievements_rule
    }

    pub fn load_state(cx: &mut App) -> RiseUserAchievementsLoadState {
        Self::snapshot(cx).load_state
    }

    pub fn cache_source(cx: &mut App) -> RiseUserAchievementsCacheSource {
        Self::snapshot(cx).cache_source
    }

    pub fn is_refreshing(cx: &mut App) -> bool {
        Self::snapshot(cx).is_refreshing
    }

    pub fn has_content(cx: &mut App) -> bool {
        Self::snapshot(cx).has_content()
    }

    pub fn error(cx: &mut App) -> Option<String> {
        Self::snapshot(cx).error
    }

    pub fn shows_initial_skeleton(cx: &mut App) -> bool {
        let snapshot = Self::snapshot(cx);
        !snapshot.has_content()
            && snapshot.load_state != RiseUserAchievementsLoadState::Loaded
            && snapshot.load_state != RiseUserAchievementsLoadState::Failed
    }

    pub fn shows_empty_state(cx: &mut App) -> bool {
        let snapshot = Self::snapshot(cx);
        !snapshot.has_content() && snapshot.load_state == RiseUserAchievementsLoadState::Loaded
    }

    pub fn shows_blocking_error(cx: &mut App) -> bool {
        let snapshot = Self::snapshot(cx);
        !snapshot.has_content() && snapshot.load_state == RiseUserAchievementsLoadState::Failed
    }

    pub fn inline_error(cx: &mut App) -> Option<String> {
        let snapshot = Self::snapshot(cx);
        if snapshot.has_content() { snapshot.error } else { None }
    }

    pub fn mutation_error(cx: &mut App) -> Option<String> {
        Self::snapshot(cx).mutation_error
    }

    pub fn is_mutating(achievement_id: i64, cx: &mut App) -> bool {
        Self::snapshot(cx).is_mutating(achievement_id)
    }

    pub fn unseen_achievement_ids(cx: &mut App) -> Vec<i64> {
        Self::snapshot(cx).unseen_ids()
    }

    pub fn showcase(user_id: &str, cx: &mut App) -> Option<RiseUserAchievementsShowcase> {
        Self::snapshot(cx).showcase(user_id).cloned()
    }

    pub fn showcase_achievements(user_id: &str, cx: &mut App) -> Vec<RiseAchievement> {
        match Self::showcase(user_id, cx) {
            Some(showcase) if showcase.is_visible => showcase.achievements,
            _ => Vec::new(),
        }
    }

    pub fn shows_showcase_skeleton(user_id: &str, cx: &mut App) -> bool {
        let Some(showcase) = Self::showcase(user_id, cx) else {
            return true;
        };

        showcase.achievements.is_empty()
            && showcase.load_state != RiseUserAchievementsLoadState::Loaded
            && showcase.load_state != RiseUserAchievementsLoadState::Failed
    }

    pub fn shows_showcase_hidden(user_id: &str, cx: &mut App) -> bool {
        let Some(showcase) = Self::showcase(user_id, cx) else {
            return false;
        };

        !showcase.is_visible && showcase.load_state == RiseUserAchievementsLoadState::Loaded
    }

    pub fn shows_showcase_empty_state(user_id: &str, cx: &mut App) -> bool {
        let Some(showcase) = Self::showcase(user_id, cx) else {
            return false;
        };

        showcase.achievements.is_empty() && showcase.load_state == RiseUserAchievementsLoadState::Loaded
    }

    pub fn shows_showcase_error(user_id: &str, cx: &mut App) -> bool {
        let Some(showcase) = Self::showcase(user_id, cx) else {
            return false;
        };

        showcase.achievements.is_empty() && showcase.load_state == RiseUserAchievementsLoadState::Failed
    }
}
