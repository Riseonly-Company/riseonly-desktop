#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use gpui::{App, Window};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::core::assets::RiseAssets;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};
use crate::modules::story::components::story_create_sheet::StoryCreateSheet;

pub enum StoryBottomSheetPreviews {}

impl StoryBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        vec![BottomSheetDebugRegistration::sheet(
            "stories.create",
            BottomSheetDebugRegistrationModule::Stories,
            "story_publish",
            "StoryCreateSheet",
            vec![0.93],
            None,
            |open: Binding<bool>, _, _| DebugStorySheet { is_presented: open },
        )]
    }
}

#[derive(IntoElement)]
struct DebugStorySheet {
    is_presented: Binding<bool>,
}

impl DebugStorySheet {
    const COVER_RESOURCE: &'static str = "audio/guide-rise-up-cover.jpg";
    const CAPTION: &'static str = "A moment worth sharing";

    fn cover() -> Option<Url> {
        let path = RiseAssets::discover().root()?.join(Self::COVER_RESOURCE);
        if path.is_file() { Url::from_file_path(&path) } else { None }
    }
}

impl RenderOnce for DebugStorySheet {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Some(cover) = Self::cover() else {
            return MainText::new(tr("error")).into_any_element();
        };
        let submit = self.is_presented.clone();
        StoryCreateSheet::new(
            "debug-story-create",
            MediaPickerItem {
                id: "debug-story".to_owned(),
                kind: MediaPickerItemKind::Image,
                url: cover,
                suggested_file_name: "preview.jpg".to_owned(),
                asset_identifier: None,
                image_payload: None,
                video_payload: None,
                audio_payload: None,
                is_file: false,
                has_spoiler: false,
                view_mode: None,
            },
            Self::CAPTION,
            self.is_presented,
            |_, _| {},
        )
        .on_submit(move |window, cx| submit.set(false, window, cx))
        .into_any_element()
    }
}
