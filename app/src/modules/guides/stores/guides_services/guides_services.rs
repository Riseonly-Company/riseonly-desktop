use gpui::{App, Context, Entity, Subscription};

use crate::modules::guides::engine::guide_runner::GuideRunner;
use crate::modules::guides::engine::rise_guides_models::RiseGuide;
use crate::modules::guides::engine::rise_guides_presentation::RiseGuidesPresentation;

pub struct GuidesServicesStore {
    presentation: Entity<RiseGuidesPresentation>,
    runner: Entity<GuideRunner>,
    _subscriptions: [Subscription; 2],
}

impl GuidesServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RiseGuidesPresentation::shared(cx);
        let runner = GuideRunner::shared(cx);
        let subscriptions =
            [cx.observe(&presentation, |_, _, cx| cx.notify()), cx.observe(&runner, |_, _, cx| cx.notify())];
        Self { presentation, runner, _subscriptions: subscriptions }
    }

    /// guides that can be offered now, finished and skipped ones removed
    pub fn guides(&self, cx: &App) -> Vec<RiseGuide> {
        self.presentation.read(cx).snapshot().guides.clone()
    }

    /// everything this build can run, finished guides included
    pub fn catalogue(&self, cx: &App) -> Vec<RiseGuide> {
        self.presentation.read(cx).snapshot().catalogue.clone()
    }

    pub fn has_guides(&self, cx: &App) -> bool {
        self.presentation.read(cx).has_guides()
    }

    pub fn is_refreshing(&self, cx: &App) -> bool {
        self.presentation.read(cx).snapshot().is_refreshing
    }

    pub fn is_running(&self, cx: &App) -> bool {
        self.runner.read(cx).running_guide_key().is_some()
    }

    /// the panda tips switch, whether guides may start on their own
    pub fn auto_start(&self, cx: &App) -> bool {
        self.presentation.read(cx).snapshot().auto_start
    }

    pub fn guide(&self, key: &str, cx: &App) -> Option<RiseGuide> {
        self.presentation.read(cx).snapshot().guide(key).cloned()
    }

    pub fn has_guide(&self, key: &str, cx: &App) -> bool {
        self.presentation.read(cx).snapshot().guide(key).is_some()
    }
}
