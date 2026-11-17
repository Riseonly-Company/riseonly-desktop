use gpui::{AnyElement, ElementId, Entity, IntoElement};
use rise_core::foundation::foundation_url::Url;
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::editor::media_editor_presentation_host::MediaEditorPresentationHost;
use crate::modules::media_picker::editor::media_editor_presentation_state::MediaEditorPresentationState;
use crate::modules::media_picker::editor::media_editor_screen::MediaEditorScreen;
use crate::modules::media_picker::editor::media_editor_view_hooks::{
    MediaEditorViewCancel, MediaEditorViewDone, MediaEditorViewHooks, set_media_editor_view_hooks,
};
use crate::modules::media_picker::editor::media_editor_window_presenter::MediaEditorWindowOutcomeHandler;
use crate::modules::media_picker::library::media_asset::MediaAsset;
use crate::modules::media_picker::library::photo_library_fetcher::PhotoLibraryFetcher;
use crate::modules::media_picker::selection::media_selection_state::{MediaSelectionEdit, MediaSelectionState};

#[allow(clippy::too_many_arguments)]
fn reedit_screen(
    id: ElementId,
    selection: Entity<MediaSelectionState>,
    caption: Binding<String>,
    media_asset: MediaAsset,
    theme_store: Entity<ThemeStore>,
    configuration: MediaPickerConfiguration,
    initial_edit: Option<MediaSelectionEdit>,
    original: CleverImage,
    on_cancel: MediaEditorViewCancel,
    on_done: MediaEditorViewDone,
) -> AnyElement {
    MediaEditorScreen::new(
        id,
        selection,
        caption,
        media_asset,
        None,
        theme_store,
        configuration,
        initial_edit,
        false,
        on_cancel,
        on_done,
    )
    .source_image_override(Some(original))
    .enables_standalone_swipe_dismiss(false)
    .editor_presentation(None)
    .embed_in_hero_host(false)
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn external_video_screen(
    id: ElementId,
    selection: Entity<MediaSelectionState>,
    caption: Binding<String>,
    media_asset: MediaAsset,
    theme_store: Entity<ThemeStore>,
    configuration: MediaPickerConfiguration,
    source_url: Url,
    on_cancel: MediaEditorViewCancel,
    on_done: MediaEditorViewDone,
) -> AnyElement {
    MediaEditorScreen::new(
        id,
        selection,
        caption,
        media_asset,
        None,
        theme_store,
        configuration,
        None,
        true,
        on_cancel,
        on_done,
    )
    .source_video_url_override(Some(source_url))
    .enables_standalone_swipe_dismiss(true)
    .standalone_dismiss_cancels(true)
    .embed_in_hero_host(false)
    .into_any_element()
}

fn presentation_host(
    state: Entity<MediaEditorPresentationState>,
    theme_store: Entity<ThemeStore>,
    configuration: MediaPickerConfiguration,
    selection: Entity<MediaSelectionState>,
    caption: Binding<String>,
    on_outcome: MediaEditorWindowOutcomeHandler,
    fetcher: Entity<PhotoLibraryFetcher>,
) -> AnyElement {
    MediaEditorPresentationHost::new(state, theme_store, configuration, selection, caption, on_outcome)
        .fetcher(Some(fetcher))
        .into_any_element()
}

pub fn install_media_editor_view_hooks() {
    set_media_editor_view_hooks(MediaEditorViewHooks { reedit_screen, external_video_screen, presentation_host });
}
