use std::sync::Arc;

use gpui::{App, AsyncApp, Context, Entity};

use crate::modules::user::engine::rise_user_domain::{RiseUserActionExecutor, RiseUserDomain};

pub struct UserAchievementsActionsStore {}

impl UserAchievementsActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    async fn require_domain(cx: &mut AsyncApp) -> Option<Arc<RiseUserDomain>> {
        let domain = cx.update(RiseUserActionExecutor::require_domain);
        domain.await.ok()
    }

    pub fn load_action(&mut self, force: bool, cx: &mut Context<Self>) {
        cx.spawn(async move |_, cx| {
            Self::await_load_action(force, cx).await;
        })
        .detach();
    }

    pub async fn await_load_action(force: bool, cx: &mut AsyncApp) {
        let Some(domain) = Self::require_domain(cx).await else {
            return;
        };

        domain.load_achievements(force).await;
    }

    pub fn load_showcase_action(&mut self, user_id: String, force: bool, cx: &mut Context<Self>) {
        cx.spawn(async move |_, cx| {
            Self::await_load_showcase_action(&user_id, force, cx).await;
        })
        .detach();
    }

    pub async fn await_load_showcase_action(user_id: &str, force: bool, cx: &mut AsyncApp) {
        let Some(domain) = Self::require_domain(cx).await else {
            return;
        };

        domain.load_user_achievements(user_id, force).await;
    }

    pub fn set_hidden_action(&mut self, achievement_id: i64, hidden: bool, cx: &mut Context<Self>) {
        cx.spawn(async move |_, cx| {
            let Some(domain) = Self::require_domain(cx).await else {
                return;
            };

            let _ = domain.set_achievement_hidden(achievement_id, hidden).await;
        })
        .detach();
    }

    pub fn mark_seen_action(&mut self, ids: Vec<i64>, cx: &mut Context<Self>) {
        cx.spawn(async move |_, cx| {
            let Some(domain) = Self::require_domain(cx).await else {
                return;
            };

            domain.mark_achievements_seen(&ids).await;
        })
        .detach();
    }
}
