use std::rc::Rc;

use gpui::{AnyElement, App, ElementId, Entity, IntoElement, Window};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::modules::chat::components::chat::composer::chat_composer_types::ChatDraftCustomEmoji;
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;
use crate::modules::media_picker::ui::media_picker_caption_bar_hooks::{
    MediaPickerCaptionBarHooks, set_media_picker_caption_bar_hooks,
};
use crate::modules::media_picker::ui::media_picker_selection_bar::MediaPickerCaptionBar;

fn caption_bar(
    id: ElementId,
    selection: Entity<MediaSelectionState>,
    theme_store: Entity<ThemeStore>,
    caption: Binding<String>,
    caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>,
    on_picker_visible_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
    on_send: Rc<dyn Fn(&mut Window, &mut App)>,
) -> AnyElement {
    let bar = MediaPickerCaptionBar::new(id, selection, theme_store, caption, move |window, cx| on_send(window, cx))
        .caption_custom_emojis(caption_custom_emojis);
    let bar = match on_picker_visible_change {
        Some(on_change) => bar.on_picker_visible_change(move |visible, window, cx| on_change(visible, window, cx)),
        None => bar,
    };
    bar.into_any_element()
}

pub fn install_media_picker_caption_bar_hooks() {
    set_media_picker_caption_bar_hooks(MediaPickerCaptionBarHooks { caption_bar });
}
