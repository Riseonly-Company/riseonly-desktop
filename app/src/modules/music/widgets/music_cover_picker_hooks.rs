use std::rc::Rc;

use gpui::{AnyElement, App, ElementId, Entity, IntoElement, Window, div};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerResult;

/// complete callback of the media picker sheet
pub(crate) type MusicCoverPickerComplete = Rc<dyn Fn(MediaPickerResult, &mut Window, &mut App)>;

pub(crate) struct MusicCoverPickerHooks {
    pub render_media_picker_sheet: fn(
        id: ElementId,
        is_presented: Binding<bool>,
        configuration: MediaPickerConfiguration,
        theme_store: Entity<ThemeStore>,
        on_complete: MusicCoverPickerComplete,
    ) -> AnyElement,
}

pub(crate) static MUSIC_COVER_PICKER_HOOKS: FeatureHookSlot<MusicCoverPickerHooks> = FeatureHookSlot::new("MUSIC_COVER_PICKER_HOOKS");

/// installs the hooks once, a second call keeps the first ones
pub(crate) fn set_music_cover_picker_hooks(hooks: MusicCoverPickerHooks) {
    let _ = MUSIC_COVER_PICKER_HOOKS.set(hooks);
}

impl MusicCoverPickerHooks {
    pub(crate) fn render_media_picker_sheet(
        id: ElementId,
        is_presented: Binding<bool>,
        configuration: MediaPickerConfiguration,
        theme_store: Entity<ThemeStore>,
        on_complete: MusicCoverPickerComplete,
    ) -> AnyElement {
        match MUSIC_COVER_PICKER_HOOKS.get() {
            Some(hooks) => (hooks.render_media_picker_sheet)(id, is_presented, configuration, theme_store, on_complete),
            None => div().into_any_element(),
        }
    }
}
