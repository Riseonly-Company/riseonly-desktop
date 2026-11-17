use gpui::StyleRefinement;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::sticker_hold_overlay_hooks::{
    StickerHoldOverlayHooks, set_sticker_hold_overlay_hooks,
};
use crate::modules::chat::components::chat::input_picker::sticker_hold_overlay::StickerHoldOverlay;

pub fn install_sticker_hold_overlay_hooks() {
    set_sticker_hold_overlay_hooks(StickerHoldOverlayHooks {
        hosted_overlay: |id, window, cx| {
            let overlay = window.use_keyed_state((id, "sticker-hold-overlay"), cx, |_, cx| {
                StickerHoldOverlay::new(cx)
            });
            if overlay.read(cx).is_idle(cx) {
                overlay
                    .cached(StyleRefinement::default().absolute().top_0().left_0().size_full())
                    .replay_when_moved()
                    .into_any_element()
            } else {
                div().absolute().inset_0().child(overlay).into_any_element()
            }
        },
    });
}
