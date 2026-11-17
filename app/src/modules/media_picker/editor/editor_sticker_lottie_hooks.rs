use gpui::{AnyElement, App, ElementId};

use crate::core::feature_hooks::FeatureHookSlot;

pub struct EditorStickerLottieHooks {
    pub live_player: fn(ElementId, String, f32, Box<dyn Fn(&mut App)>) -> AnyElement,
}

pub(crate) static EDITOR_STICKER_LOTTIE_HOOKS: FeatureHookSlot<EditorStickerLottieHooks> =
    FeatureHookSlot::new("EDITOR_STICKER_LOTTIE_HOOKS");

pub fn set_editor_sticker_lottie_hooks(hooks: EditorStickerLottieHooks) {
    let _ = EDITOR_STICKER_LOTTIE_HOOKS.set(hooks);
}

pub fn editor_sticker_lottie_hooks() -> Option<&'static EditorStickerLottieHooks> {
    EDITOR_STICKER_LOTTIE_HOOKS.get()
}
