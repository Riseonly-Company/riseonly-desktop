use gpui::{App, Context, Subscription};

use crate::app::aside::aside_registry::AsideSection;
use crate::app::router::navigation_types::RootRoute;
use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::music::pages::now_playing::now_playing::NowPlayingHost;

/// what the full player asks the right panel of the front window
pub struct NowPlayingAsideHooks {
    pub observe_frontmost: fn(&mut Context<NowPlayingHost>) -> Subscription,
    pub has_frontmost: fn(&App) -> bool,
    pub set_default: fn(AsideSection, RootRoute, &mut App),
    pub clear_default: fn(AsideSection, &mut App),
}

pub(crate) static NOW_PLAYING_ASIDE_HOOKS: FeatureHookSlot<NowPlayingAsideHooks> = FeatureHookSlot::new("NOW_PLAYING_ASIDE_HOOKS");

pub fn set_now_playing_aside_hooks(hooks: NowPlayingAsideHooks) {
    let _ = NOW_PLAYING_ASIDE_HOOKS.set(hooks);
}

pub fn now_playing_aside_hooks() -> Option<&'static NowPlayingAsideHooks> {
    NOW_PLAYING_ASIDE_HOOKS.get()
}
