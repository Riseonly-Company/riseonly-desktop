use gpui::{App, Empty, IntoElement, RenderOnce, Window};
use rise_theme::ThemeStore;
use rise_ui::swiftui::prelude::*;

use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItemKind;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;

/// mounted at the root not inside the comment input, the composer lives in the sheet footer
/// so overlays from there get clipped
/// full screen picker has to be a sibling of the sheet above it
#[derive(IntoElement, Default)]
pub struct CommentPhotoPickerHost;

impl CommentPhotoPickerHost {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for CommentPhotoPickerHost {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let interactions = CommentInteractionsStore::shared(cx);
        if !interactions.read(cx).photo_picker_visible {
            return Empty.into_any_element();
        }
        let theme_store = ThemeStore::shared(cx);
        let is_presented = Binding::entity(
            interactions.clone(),
            |store| store.photo_picker_visible,
            |store, visible, cx| {
                store.photo_picker_visible = visible;
                cx.notify();
            },
        );
        let caption = Binding::entity(
            interactions.clone(),
            |store| store.comment_text().to_owned(),
            |store, text: String, cx| store.set_comment_text(&text, cx),
        );
        div()
            .size_full()
            .child(
                MediaPickerSheet::new(
                    "comment-photo-picker",
                    is_presented,
                    MediaPickerConfiguration::comment_photo(),
                    theme_store,
                    move |result, _window, cx| {
                        interactions.update(cx, |store, cx| {
                            store.photo_picker_visible = false;
                            cx.notify();
                        });
                        if result.was_cancelled {
                            return;
                        }
                        let Some(item) = result
                            .items
                            .into_iter()
                            .find(|item| item.kind == MediaPickerItemKind::Image && !item.is_file)
                        else {
                            return;
                        };
                        let interactions = interactions.clone();
                        cx.defer(move |cx| {
                            interactions.update(cx, |store, cx| store.send_photo(item, cx));
                        });
                    },
                )
                .caption(Some(caption))
                .experimental_bottom_sheet(true),
            )
            .into_any_element()
    }
}
