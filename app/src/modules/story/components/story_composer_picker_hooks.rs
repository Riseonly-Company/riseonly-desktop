use gpui::{AnyElement, App, ElementId, Entity, Window};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerResult, MediaPickerTrackSelection};

pub type StoryComposerPickerComplete = Box<dyn Fn(MediaPickerResult, &mut Window, &mut App)>;
pub type StoryComposerPickerDismissed = Box<dyn Fn(&mut Window, &mut App)>;

pub struct StoryComposerPickerHooks {
    pub media_picker_sheet: fn(
        id: ElementId,
        is_presented: Binding<bool>,
        configuration: MediaPickerConfiguration,
        theme_store: Entity<ThemeStore>,
        initial_track: Option<MediaPickerTrackSelection>,
        on_complete: StoryComposerPickerComplete,
        on_dismissed: StoryComposerPickerDismissed,
    ) -> AnyElement,
}

pub(crate) static STORY_COMPOSER_PICKER_HOOKS: FeatureHookSlot<StoryComposerPickerHooks> =
    FeatureHookSlot::new("STORY_COMPOSER_PICKER_HOOKS");

pub fn set_story_composer_picker_hooks(hooks: StoryComposerPickerHooks) {
    let _ = STORY_COMPOSER_PICKER_HOOKS.set(hooks);
}

pub fn story_composer_picker_hooks() -> Option<&'static StoryComposerPickerHooks> {
    STORY_COMPOSER_PICKER_HOOKS.get()
}
