use std::rc::Rc;

use gpui::{AnyElement, App, ElementId, Entity, Window};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatDraftCustomEmoji;
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;

/// the caption bar of the picker, built by the chat ui
pub struct MediaPickerCaptionBarHooks {
    pub caption_bar: fn(
        id: ElementId,
        selection: Entity<MediaSelectionState>,
        theme_store: Entity<ThemeStore>,
        caption: Binding<String>,
        caption_custom_emojis: Option<Binding<Vec<ChatDraftCustomEmoji>>>,
        on_picker_visible_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
        on_send: Rc<dyn Fn(&mut Window, &mut App)>,
    ) -> AnyElement,
}

pub(crate) static MEDIA_PICKER_CAPTION_BAR_HOOKS: FeatureHookSlot<MediaPickerCaptionBarHooks> =
    FeatureHookSlot::new("MEDIA_PICKER_CAPTION_BAR_HOOKS");

pub fn set_media_picker_caption_bar_hooks(hooks: MediaPickerCaptionBarHooks) {
    let _ = MEDIA_PICKER_CAPTION_BAR_HOOKS.set(hooks);
}

pub fn media_picker_caption_bar_hooks() -> Option<&'static MediaPickerCaptionBarHooks> {
    MEDIA_PICKER_CAPTION_BAR_HOOKS.get()
}
