use gpui::{App, ElementId, IntoElement};

use crate::core::ui::lottie_sticker::lottie_sticker_view::LottieStickerView;
use crate::modules::media_picker::editor::editor_sticker_lottie_hooks::{
    EditorStickerLottieHooks, set_editor_sticker_lottie_hooks,
};

pub fn install_editor_sticker_lottie_hooks() {
    set_editor_sticker_lottie_hooks(EditorStickerLottieHooks {
        live_player: |id: ElementId, file_url: String, side: f32, on_first_frame: Box<dyn Fn(&mut App)>| {
            LottieStickerView::new(id, file_url)
                .size(side)
                .r#loop(true)
                .on_first_frame(on_first_frame)
                .into_any_element()
        },
    });
}
