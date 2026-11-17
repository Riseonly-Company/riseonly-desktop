#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use gpui::{App, IntoElement, RenderOnce, Window};
use rise_theme::ThemeStore;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationMode, BottomSheetDebugRegistrationModule,
};
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;

pub enum MediaBottomSheetPreviews {}

impl MediaBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        vec![
            BottomSheetDebugRegistration::new(
                "media.picker",
                BottomSheetDebugRegistrationModule::Media,
                "media_picker_title",
                "MediaPickerSheet",
                |open: Binding<bool>, _: &mut Window, _: &mut App| DebugMediaPicker { is_presented: open },
            )
            .with_mode(BottomSheetDebugRegistrationMode::Live),
        ]
    }
}

#[derive(IntoElement)]
struct DebugMediaPicker {
    is_presented: Binding<bool>,
}

impl RenderOnce for DebugMediaPicker {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_presented = self.is_presented.clone();
        MediaPickerSheet::new(
            "debug-media-picker",
            self.is_presented,
            MediaPickerConfiguration::chat_attachments(),
            ThemeStore::shared(cx),
            move |_, window, cx| is_presented.set(false, window, cx),
        )
    }
}
