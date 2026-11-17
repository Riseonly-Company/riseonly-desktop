use std::rc::Rc;
use std::time::Duration;

use gpui::App;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::feature_hooks::FeatureHookSlot;

const DEFAULT_BACKGROUND_WORK_RETRY_INTERVAL: Duration = Duration::from_millis(120);

pub struct ChatDecorativeBackdropHooks {
    pub background_work_retry_interval: fn() -> Duration,
    pub chat_presentation_epoch: fn(&App) -> Rc<dyn Fn(&App) -> Option<Uuid>>,
    pub structural_updates_blocked: fn(&App) -> Option<Rc<dyn Fn(&App) -> bool>>,
}

pub(crate) static CHAT_DECORATIVE_BACKDROP_HOOKS: FeatureHookSlot<ChatDecorativeBackdropHooks> = FeatureHookSlot::new("CHAT_DECORATIVE_BACKDROP_HOOKS");

/// second install is ignored, first owner keeps the slot
pub fn set_chat_decorative_backdrop_hooks(hooks: ChatDecorativeBackdropHooks) {
    let _ = CHAT_DECORATIVE_BACKDROP_HOOKS.set(hooks);
}

pub(crate) fn chat_decorative_backdrop_hooks() -> Option<&'static ChatDecorativeBackdropHooks> {
    CHAT_DECORATIVE_BACKDROP_HOOKS.get()
}

impl ChatDecorativeBackdropHooks {
    pub(crate) fn resolved_background_work_retry_interval() -> Duration {
        chat_decorative_backdrop_hooks()
            .map_or(DEFAULT_BACKGROUND_WORK_RETRY_INTERVAL, |hooks| (hooks.background_work_retry_interval)())
    }

    pub(crate) fn resolved_chat_presentation_epoch(cx: &App) -> Rc<dyn Fn(&App) -> Option<Uuid>> {
        match chat_decorative_backdrop_hooks() {
            Some(hooks) => (hooks.chat_presentation_epoch)(cx),
            None => Rc::new(|_: &App| None),
        }
    }

    pub(crate) fn resolved_structural_updates_blocked(cx: &App) -> Option<Rc<dyn Fn(&App) -> bool>> {
        chat_decorative_backdrop_hooks().and_then(|hooks| (hooks.structural_updates_blocked)(cx))
    }
}
