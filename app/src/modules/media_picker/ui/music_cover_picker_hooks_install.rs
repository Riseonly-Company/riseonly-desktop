use gpui::{AnyElement, ElementId, Entity, IntoElement};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;
use crate::modules::music::widgets::music_cover_picker_hooks::{
    MusicCoverPickerComplete, MusicCoverPickerHooks, set_music_cover_picker_hooks,
};

fn render_media_picker_sheet(
    id: ElementId,
    is_presented: Binding<bool>,
    configuration: MediaPickerConfiguration,
    theme_store: Entity<ThemeStore>,
    on_complete: MusicCoverPickerComplete,
) -> AnyElement {
    MediaPickerSheet::new(id, is_presented, configuration, theme_store, move |result, window, cx| {
        on_complete(result, window, cx)
    })
    .experimental_bottom_sheet(true)
    .into_any_element()
}

pub fn install_music_cover_picker_hooks() {
    set_music_cover_picker_hooks(MusicCoverPickerHooks { render_media_picker_sheet });
}
