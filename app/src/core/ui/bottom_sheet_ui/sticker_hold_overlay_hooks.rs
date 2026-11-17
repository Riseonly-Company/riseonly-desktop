use gpui::{AnyElement, App, ElementId, Window};

use crate::core::feature_hooks::FeatureHookSlot;

pub struct StickerHoldOverlayHooks {
    pub hosted_overlay: fn(ElementId, &mut Window, &mut App) -> AnyElement,
}

pub(crate) static STICKER_HOLD_OVERLAY_HOOKS: FeatureHookSlot<StickerHoldOverlayHooks> =
    FeatureHookSlot::new("STICKER_HOLD_OVERLAY_HOOKS");

pub fn set_sticker_hold_overlay_hooks(hooks: StickerHoldOverlayHooks) {
    let _ = STICKER_HOLD_OVERLAY_HOOKS.set(hooks);
}

pub fn sticker_hold_overlay_hooks() -> Option<&'static StickerHoldOverlayHooks> {
    STICKER_HOLD_OVERLAY_HOOKS.get()
}
