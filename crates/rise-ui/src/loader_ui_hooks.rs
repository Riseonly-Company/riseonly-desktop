use std::any::Any;
use std::sync::{Arc, OnceLock};

use gpui::{AnyElement, App, Window};

/// a decoded animation, opaque to the kit
pub type LottieAnimation = Arc<dyn Any + Send + Sync>;

pub struct LoaderUiHooks {
    pub riseonly_loader_animation: fn() -> Option<LottieAnimation>,
    pub lottie_view: fn(animation: &LottieAnimation, playing: bool, window: &mut Window, cx: &mut App) -> AnyElement,
}

static LOADER_UI_HOOKS: OnceLock<LoaderUiHooks> = OnceLock::new();

pub fn set_loader_ui_hooks(hooks: LoaderUiHooks) {
    let _ = LOADER_UI_HOOKS.set(hooks);
}

pub fn loader_ui_hooks() -> Option<&'static LoaderUiHooks> {
    LOADER_UI_HOOKS.get()
}
