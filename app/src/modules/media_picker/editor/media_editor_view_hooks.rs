use gpui::{AnyElement, App, ElementId, Entity, Window};
use rise_core::foundation::foundation_url::Url;
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::editor::media_editor_presentation_state::MediaEditorPresentationState;
use crate::modules::media_picker::editor::media_editor_window_presenter::{
    MediaEditorOutcome, MediaEditorWindowOutcomeHandler,
};
use crate::modules::media_picker::library::media_asset::MediaAsset;
use crate::modules::media_picker::library::photo_library_fetcher::PhotoLibraryFetcher;
use crate::modules::media_picker::selection::media_selection_state::{MediaSelectionEdit, MediaSelectionState};

pub type MediaEditorViewCancel = Box<dyn Fn(&mut Window, &mut App)>;
pub type MediaEditorViewDone = Box<dyn Fn(MediaEditorOutcome, &mut Window, &mut App)>;

pub struct MediaEditorViewHooks {
    pub reedit_screen: fn(
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
    ) -> AnyElement,
    pub external_video_screen: fn(
        id: ElementId,
        selection: Entity<MediaSelectionState>,
        caption: Binding<String>,
        media_asset: MediaAsset,
        theme_store: Entity<ThemeStore>,
        configuration: MediaPickerConfiguration,
        source_url: Url,
        on_cancel: MediaEditorViewCancel,
        on_done: MediaEditorViewDone,
    ) -> AnyElement,
    pub presentation_host: fn(
        state: Entity<MediaEditorPresentationState>,
        theme_store: Entity<ThemeStore>,
        configuration: MediaPickerConfiguration,
        selection: Entity<MediaSelectionState>,
        caption: Binding<String>,
        on_outcome: MediaEditorWindowOutcomeHandler,
        fetcher: Entity<PhotoLibraryFetcher>,
    ) -> AnyElement,
}

pub(crate) static MEDIA_EDITOR_VIEW_HOOKS: FeatureHookSlot<MediaEditorViewHooks> =
    FeatureHookSlot::new("MEDIA_EDITOR_VIEW_HOOKS");

pub fn set_media_editor_view_hooks(hooks: MediaEditorViewHooks) {
    let _ = MEDIA_EDITOR_VIEW_HOOKS.set(hooks);
}

pub fn media_editor_view_hooks() -> Option<&'static MediaEditorViewHooks> {
    MEDIA_EDITOR_VIEW_HOOKS.get()
}
