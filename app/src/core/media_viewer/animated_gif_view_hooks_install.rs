use crate::core::media_viewer::media_viewer_presentation_state::chat_media_presentation;
use crate::core::ui::animated_gif_view_hooks::{AnimatedGifViewHooks, set_animated_gif_view_hooks};

pub fn install_animated_gif_view_hooks() {
    set_animated_gif_view_hooks(AnimatedGifViewHooks {
        media_viewer_covers_page: |cx| chat_media_presentation(cx).read(cx).covers_page(),
        observe_media_viewer_cover: |mut on_change, cx| {
            let presentation = chat_media_presentation(cx);
            cx.observe(&presentation, move |presentation, cx| {
                let covers = presentation.read(cx).covers_page();
                on_change(covers, cx);
            })
        },
    });
}
