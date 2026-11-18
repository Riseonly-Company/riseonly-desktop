use gpui::{App, AsyncApp, Context, Entity};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;
use crate::modules::user::stores::user::user_services::user_guide_sandbox_hooks::user_guide_sandbox_hooks;
use crate::modules::user::stores::user_achievements::user_achievements_actions::user_achievements_actions::UserAchievementsActionsStore;
use crate::modules::user::stores::user_achievements::user_achievements_services::user_achievements_services::UserAchievementsServicesStore;

#[derive(Clone, Debug, PartialEq)]
pub struct RiseAchievementShareIntent {
    pub id: Uuid,
    pub route: RootRoute,
}

impl RiseAchievementShareIntent {
    pub fn new(route: RootRoute) -> Self {
        Self::with_id(Uuid::new_v4(), route)
    }

    pub fn with_id(id: Uuid, route: RootRoute) -> Self {
        Self { id, route }
    }
}

pub struct UserAchievementsInteractionsStore {
    pending_share_intent: Option<RiseAchievementShareIntent>,
    pending_detail_id: Option<i64>,
}

impl UserAchievementsInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { pending_share_intent: None, pending_detail_id: None }
    }

    fn actions(cx: &mut App) -> Entity<UserAchievementsActionsStore> {
        UserAchievementsActionsStore::shared(cx)
    }

    pub fn pending_share_intent(&self) -> Option<&RiseAchievementShareIntent> {
        self.pending_share_intent.as_ref()
    }

    pub fn share_achievement_to_profile(&mut self, achievement: &RiseAchievement, cx: &mut Context<Self>) {
        if achievement.id <= 0 {
            return;
        }
        GuideSignalBus::emit(RiseGuideSignal::AchievementShareStarted);

        self.pending_share_intent = Some(RiseAchievementShareIntent::new(RootRoute::CreatePostWithAchievement {
            achievement_id: achievement.id,
        }));
        cx.notify();
    }

    pub fn consume_pending_share_intent(&mut self, cx: &mut Context<Self>) {
        self.pending_share_intent = None;
        cx.notify();
    }

    pub fn pending_detail_id(&self) -> Option<i64> {
        self.pending_detail_id
    }

    /// opens the earned card from a push or a feed row, the own list reloads since the grant may be newer
    pub fn open_earned_achievement(&mut self, id: i64, cx: &mut Context<Self>) {
        if id <= 0 {
            return;
        }
        self.pending_detail_id = Some(id);
        cx.notify();
        Self::actions(cx).update(cx, |actions, cx| actions.load_action(true, cx));
    }

    pub fn consume_pending_detail(&mut self, cx: &mut Context<Self>) {
        if self.pending_detail_id.take().is_some() {
            cx.notify();
        }
    }

    pub fn open_achievements(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let Some(user_id) = Self::normalized(user_id) else {
            let user_waiting = UserAchievementsServicesStore::shows_initial_skeleton(cx);
            actions.update(cx, |actions, cx| actions.load_action(user_waiting, cx));
            return;
        };

        actions.update(cx, |actions, cx| actions.load_showcase_action(user_id, true, cx));
    }

    pub fn resolve_shared_achievement(&mut self, author_id: Option<&str>, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        actions.update(cx, |actions, cx| actions.load_action(false, cx));
        let Some(author_id) = Self::normalized(author_id) else {
            return;
        };
        let viewer_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone());
        if viewer_id.as_deref() == Some(author_id.as_str()) {
            return;
        }

        actions.update(cx, |actions, cx| actions.load_showcase_action(author_id, false, cx));
    }

    pub async fn refresh_achievements(user_id: Option<String>, cx: &mut AsyncApp) {
        let Some(user_id) = Self::normalized(user_id.as_deref()) else {
            UserAchievementsActionsStore::await_load_action(true, cx).await;
            return;
        };

        UserAchievementsActionsStore::await_load_showcase_action(&user_id, true, cx).await;
    }

    pub fn retry_achievements(&mut self, user_id: Option<&str>, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let Some(user_id) = Self::normalized(user_id) else {
            actions.update(cx, |actions, cx| actions.load_action(true, cx));
            return;
        };

        actions.update(cx, |actions, cx| actions.load_showcase_action(user_id, true, cx));
    }

    pub fn set_hidden(&mut self, achievement: &RiseAchievement, hidden: bool, cx: &mut Context<Self>) {
        let hooks = user_guide_sandbox_hooks();
        if (hooks.owns_achievements_scene)(cx) {
            (hooks.set_achievement_hidden)(achievement.id, hidden, cx);
            return;
        }
        if achievement.id <= 0 {
            return;
        }
        if achievement.hidden == hidden {
            return;
        }
        if UserAchievementsServicesStore::is_mutating(achievement.id, cx) {
            return;
        }

        let achievement_id = achievement.id;
        Self::actions(cx).update(cx, |actions, cx| actions.set_hidden_action(achievement_id, hidden, cx));
    }

    pub fn mark_seen(&mut self, ids: &[i64], cx: &mut Context<Self>) {
        let normalized: Vec<i64> = ids.iter().copied().filter(|id| *id > 0).collect();
        if normalized.is_empty() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.mark_seen_action(normalized, cx));
    }

    pub fn mark_visible_achievements_seen(&mut self, cx: &mut Context<Self>) {
        let unseen = UserAchievementsServicesStore::unseen_achievement_ids(cx);
        if unseen.is_empty() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.mark_seen_action(unseen, cx));
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        if value.is_empty() {
            return None;
        }

        Some(value.to_owned())
    }
}
