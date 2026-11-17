use std::rc::Rc;
use std::time::Duration;

use gpui::App;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::swiftui::environment::env;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop_hooks::{
    ChatDecorativeBackdropHooks, set_chat_decorative_backdrop_hooks,
};
use crate::modules::chat::pages::chats::embedded_forum_transition::{
    EmbeddedForumRailContextKey, EmbeddedForumTransitionMetrics,
};

fn background_work_retry_interval() -> Duration {
    Duration::from_secs_f64(EmbeddedForumTransitionMetrics::BACKGROUND_WORK_RETRY_INTERVAL)
}

fn chat_presentation_epoch(cx: &App) -> Rc<dyn Fn(&App) -> Option<Uuid>> {
    let store = env::<NavigationStoreKey>(cx);
    Rc::new(move |cx: &App| store.as_ref().and_then(|store| store.read(cx).chat_presentation_epoch()))
}

fn structural_updates_blocked(cx: &App) -> Option<Rc<dyn Fn(&App) -> bool>> {
    let rail = env::<EmbeddedForumRailContextKey>(cx)?;
    let transition = rail.transition;
    Some(Rc::new(move |cx: &App| transition.read(cx).structural_updates_blocked()))
}

pub fn install_chat_decorative_backdrop_hooks() {
    set_chat_decorative_backdrop_hooks(ChatDecorativeBackdropHooks {
        background_work_retry_interval,
        chat_presentation_epoch,
        structural_updates_blocked,
    });
}
