use gpui::{App, Subscription};

use crate::core::feature_hooks::FeatureHookSlot;

pub struct AnimatedGifViewHooks {
    pub media_viewer_covers_page: fn(&mut App) -> bool,
    pub observe_media_viewer_cover: fn(Box<dyn FnMut(bool, &mut App)>, &mut App) -> Subscription,
}

pub(crate) static ANIMATED_GIF_VIEW_HOOKS: FeatureHookSlot<AnimatedGifViewHooks> =
    FeatureHookSlot::new("ANIMATED_GIF_VIEW_HOOKS");

pub fn set_animated_gif_view_hooks(hooks: AnimatedGifViewHooks) {
    let _ = ANIMATED_GIF_VIEW_HOOKS.set(hooks);
}

pub fn animated_gif_view_hooks() -> Option<&'static AnimatedGifViewHooks> {
    ANIMATED_GIF_VIEW_HOOKS.get()
}
