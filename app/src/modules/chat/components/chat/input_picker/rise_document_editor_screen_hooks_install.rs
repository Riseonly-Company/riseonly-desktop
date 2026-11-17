use gpui::{AnyElement, App, IntoElement, Window};

use crate::core::ui::rich_text::document::rise_document_editor_hooks::{
    RiseDocumentEditorHooks, RiseDocumentEditorPickerPanel, set_rise_document_editor_hooks,
};
use crate::modules::chat::components::chat::input_picker::chat_input_picker_panel::ChatInputPickerPanel;

fn chat_input_picker_panel(panel: RiseDocumentEditorPickerPanel, _: &mut Window, _: &mut App) -> AnyElement {
    let RiseDocumentEditorPickerPanel {
        panel_height,
        on_select_emoji,
        on_select_custom_emoji,
        on_backspace,
        uses_flat_chrome,
    } = panel;
    ChatInputPickerPanel::new(
        "rise-document-editor-picker-panel",
        panel_height,
        move |emoji, window, cx| on_select_emoji(emoji, window, cx),
        move |item, window, cx| on_select_custom_emoji(item, window, cx),
        move |window, cx| on_backspace(window, cx),
        |_, _, _| {},
        |_, _, _| {},
        |_, _| {},
        |_, _, _| {},
    )
    .uses_flat_chrome(uses_flat_chrome)
    .into_any_element()
}

pub fn install_rise_document_editor_screen_hooks() {
    set_rise_document_editor_hooks(RiseDocumentEditorHooks { chat_input_picker_panel });
}
