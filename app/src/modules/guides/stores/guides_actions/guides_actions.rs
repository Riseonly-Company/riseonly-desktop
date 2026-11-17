use gpui::{App, AppContext as _, AsyncApp, Context, Entity};

use crate::modules::guides::engine::guide_auto_start::GuideAutoStart;
use crate::modules::guides::engine::guide_runner::{GuideRunner, GuideRunnerOrigin};
use crate::modules::guides::engine::rise_guides_domain::RiseGuidesActionExecutor;

pub struct GuidesActionsStore {}

impl GuidesActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn refresh_action(&mut self, force: bool, cx: &mut Context<Self>) {
        let domain = RiseGuidesActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.refresh(force).await;
        })
        .detach();
    }

    pub fn set_auto_start_action(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let domain = RiseGuidesActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.set_auto_start(enabled).await;
        })
        .detach();
    }

    pub fn start_watching_triggers_action(&mut self, cx: &mut Context<Self>) {
        GuideAutoStart::shared(cx).update(cx, |auto_start, cx| auto_start.start_watching(cx));
    }

    pub fn stop_watching_triggers_action(&mut self, cx: &mut Context<Self>) {
        GuideAutoStart::shared(cx).update(cx, |auto_start, cx| auto_start.stop_watching(cx));
    }

    pub fn start_action(&mut self, key: String, origin: GuideRunnerOrigin, cx: &mut Context<Self>) {
        let domain = RiseGuidesActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let Some(guide) = domain.guide_by_key(&key).await else {
                return;
            };
            cx.update(|cx| {
                GuideRunner::shared(cx).update(cx, |runner, cx| runner.start(guide, origin, cx));
            });
        })
        .detach();
    }

    pub fn start_action_id(&mut self, id: i64, origin: GuideRunnerOrigin, cx: &mut Context<Self>) {
        let domain = RiseGuidesActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            let Some(guide) = domain.guide(id).await else {
                return;
            };
            cx.update(|cx| {
                GuideRunner::shared(cx).update(cx, |runner, cx| runner.start(guide, origin, cx));
            });
        })
        .detach();
    }
}
