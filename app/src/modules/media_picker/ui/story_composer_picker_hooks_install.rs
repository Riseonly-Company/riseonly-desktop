use gpui::{AnyElement, ElementId, Entity, IntoElement};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerTrackSelection;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::story::components::story_composer_picker_hooks::{
    StoryComposerPickerComplete, StoryComposerPickerDismissed, StoryComposerPickerHooks,
    set_story_composer_picker_hooks,
};

fn media_picker_sheet(
    id: ElementId,
    is_presented: Binding<bool>,
    configuration: MediaPickerConfiguration,
    theme_store: Entity<ThemeStore>,
    initial_track: Option<MediaPickerTrackSelection>,
    on_complete: StoryComposerPickerComplete,
    on_dismissed: StoryComposerPickerDismissed,
) -> AnyElement {
    MediaPickerSheet::new(id, is_presented, configuration, theme_store, on_complete)
        .initial_track(initial_track)
        .experimental_bottom_sheet(true)
        .on_dismissed(on_dismissed)
        .into_any_element()
}

pub fn install_story_composer_picker_hooks() {
    set_story_composer_picker_hooks(StoryComposerPickerHooks { media_picker_sheet });
}
