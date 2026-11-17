use std::rc::Rc;

use gpui::{AnyElement, App, Window};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

/// arguments this editor passes to the chat input picker panel
/// stickers and gifs and pack settings are screens of their own, never inline content
/// so the editor gives them no op callbacks
pub(crate) struct RiseDocumentEditorPickerPanel {
    pub(crate) panel_height: f32,
    pub(crate) on_select_emoji: Rc<dyn Fn(String, &mut Window, &mut App)>,
    pub(crate) on_select_custom_emoji: Rc<dyn Fn(StickerItem, &mut Window, &mut App)>,
    pub(crate) on_backspace: Rc<dyn Fn(&mut Window, &mut App)>,
    pub(crate) uses_flat_chrome: bool,
}

pub(crate) struct RiseDocumentEditorHooks {
    pub(crate) chat_input_picker_panel: fn(RiseDocumentEditorPickerPanel, &mut Window, &mut App) -> AnyElement,
}

pub(crate) static RISE_DOCUMENT_EDITOR_HOOKS: FeatureHookSlot<RiseDocumentEditorHooks> = FeatureHookSlot::new("RISE_DOCUMENT_EDITOR_HOOKS");

/// installed once, a second install keeps the first since the slot is process wide
pub(crate) fn set_rise_document_editor_hooks(hooks: RiseDocumentEditorHooks) {
    let _ = RISE_DOCUMENT_EDITOR_HOOKS.set(hooks);
}

pub(crate) fn rise_document_editor_hooks() -> Option<&'static RiseDocumentEditorHooks> {
    RISE_DOCUMENT_EDITOR_HOOKS.get()
}
