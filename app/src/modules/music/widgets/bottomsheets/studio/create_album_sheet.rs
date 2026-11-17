use std::rc::Rc;

use gpui::{
    App, Div, ElementId, Entity, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div,
    px, transparent_black,
};
use rise_catalog::MusicGenre;
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_i18n::tr;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, vstack};

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::SheetFloatingHeader;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::input_ui::InputOutlinedRowUi;
use crate::modules::music::shared::music_field_rules::MusicFieldRules;
use crate::modules::music::shared::utils::music::music_models::display_genre;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::music_cover_picker_ui::MusicCoverPickerUi;

/// dismiss callback run from the alert button, a system dialog answers without a window
pub type StudioSheetDismiss = Rc<dyn Fn(&mut App)>;

#[derive(Default)]
struct CreateAlbumSheetState {
    album_title: String,
    genre: String,
    local_cover_file: Option<SaiWsOptimisticLocalFile>,
    cover_preview: Option<CleverImage>,
}

#[derive(IntoElement)]
pub struct CreateAlbumSheet {
    id: ElementId,
    on_dismiss: StudioSheetDismiss,
}

impl CreateAlbumSheet {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), on_dismiss: Rc::new(|_: &mut App| {}) }
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(on_dismiss);
        self
    }

    fn can_submit(state: &CreateAlbumSheetState, is_submitting: bool) -> bool {
        let title = trimming_whitespaces(&state.album_title);
        !title.is_empty()
            && MusicFieldRules::album_title().is_satisfied(title)
            && !state.genre.is_empty()
            && state.cover_preview.is_some()
            && state.local_cover_file.is_some()
            && !is_submitting
    }

    fn submit_album(state: &Entity<CreateAlbumSheetState>, on_dismiss: &StudioSheetDismiss, cx: &mut App) {
        let actions = MusicActionsStore::shared(cx);
        let slice = Rc::clone(&actions.read(cx).request_album_sai);
        let is_submitting = slice.is_pending() || slice.data().is_some_and(|data| data.cover_uploading == Some(true));
        let sheet = state.read(cx);
        if !Self::can_submit(sheet, is_submitting) {
            return;
        }
        let Some(local_cover_file) = sheet.local_cover_file.clone() else {
            return;
        };
        let title = trimming_whitespaces(&sheet.album_title).to_owned();
        let genre = sheet.genre.clone();
        let on_dismiss = Rc::clone(on_dismiss);
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.request_album(
                &title,
                &genre,
                local_cover_file,
                Some(Rc::new(move |cx: &mut App| {
                    let on_dismiss = Rc::clone(&on_dismiss);
                    NativeConfirmAlert::present_notice(
                        &tr("music_album_create_success_title"),
                        &tr("music_album_create_success_msg"),
                        None,
                        None,
                        move |cx: &mut App| on_dismiss(cx),
                        cx,
                    );
                })),
                Some(Rc::new(|message: String, cx: &mut App| {
                    NativeConfirmAlert::present_notice(
                        &tr("music_album_create_error_title"),
                        &message,
                        None,
                        None,
                        |_: &mut App| {},
                        cx,
                    );
                })),
                cx,
            );
        });
    }

    fn genre_chips(id: &ElementId, state: &Entity<CreateAlbumSheetState>, selected: &str, theme: &AppTheme) -> Div {
        let mut chips = hstack(8.0).w_full().flex_wrap();
        for genre in MusicGenre::ALL {
            let wire = genre.wire();
            let is_selected = selected == wire;
            let state = state.clone();
            chips = chips.child(
                plain((id.clone(), wire))
                    .on_press(move |_, cx| {
                        state.update(cx, |state, cx| {
                            state.genre = if state.genre == wire { String::new() } else { wire.to_owned() };
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .px(px(12.0))
                            .py(px(7.0))
                            .capsule()
                            .swiftui_font(Font::system_weight(13.0, if is_selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }))
                            .text_color(if is_selected { theme.primary._100 } else { theme.text.secondary })
                            .bg(if is_selected { theme.primary._100.opacity(0.15) } else { theme.bg._200 })
                            .stroke_border(if is_selected { theme.primary._100 } else { transparent_black() }, 1.0)
                            .child(display_genre(wire)),
                    ),
            );
        }
        vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(SecondaryText::new(tr("music_artist_genre")).px(12.0))
            .child(chips)
    }
}

impl RenderOnce for CreateAlbumSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let state = window.use_keyed_state((self.id.clone(), "state"), cx, |_, _| CreateAlbumSheetState::default());
        let actions = MusicActionsStore::shared(cx);
        let slice = Rc::clone(&actions.read(cx).request_album_sai);
        let upload = slice.data();
        let cover_upload_progress = upload.as_ref().and_then(|data| data.cover_upload_progress).unwrap_or(0.0);
        let is_cover_uploading = upload.as_ref().is_some_and(|data| data.cover_uploading == Some(true));
        let is_submitting = slice.is_pending() || is_cover_uploading;
        let sheet = state.read(cx);
        let can_submit = Self::can_submit(sheet, is_submitting);
        let has_local_cover = sheet.local_cover_file.is_some();
        let genre = sheet.genre.clone();

        let local_cover_file = Binding::entity(
            state.clone(),
            |state: &CreateAlbumSheetState| state.local_cover_file.clone(),
            |state: &mut CreateAlbumSheetState, value, _| state.local_cover_file = value,
        );
        let cover_preview = Binding::entity(
            state.clone(),
            |state: &CreateAlbumSheetState| state.cover_preview.clone(),
            |state: &mut CreateAlbumSheetState, value, _| state.cover_preview = value,
        );
        let album_title = Binding::entity(
            state.clone(),
            |state: &CreateAlbumSheetState| state.album_title.clone(),
            |state: &mut CreateAlbumSheetState, value, _| state.album_title = value,
        );

        let submit_state = state.clone();
        let on_dismiss = Rc::clone(&self.on_dismiss);
        let uploading_hint = is_cover_uploading
            .then(|| SecondaryText::new(tr("music_upload_cover_uploading")).px(12.0).tac(TextAlignT::Center));
        let button_title: SharedString =
            if is_submitting { tr("music_uploading").into() } else { tr("music_create_album").into() };

        div().size_full().bg(theme.bg._100).child(
            scroll_view((self.id.clone(), "scroll")).child(
                vstack(16.0)
                    .w_full()
                    .child({
                        let leading_dismiss = Rc::clone(&self.on_dismiss);
                        SheetFloatingHeader::new((self.id.clone(), "header"), tr("music_create_album"), move |_, cx| leading_dismiss(cx))
                            .shows_chrome(false)
                    })
                    .child(
                        MusicCoverPickerUi::new((self.id.clone(), "cover"), local_cover_file, cover_preview)
                            .placeholder_label(Some(tr("music_upload_album_cover")))
                            .cover_upload_progress(cover_upload_progress)
                            .show_upload_overlay(is_cover_uploading && has_local_cover),
                    )
                    .child(
                        vstack(12.0)
                            .w_full()
                            .px(px(16.0))
                            .child(
                                vstack(6.0)
                                    .w_full()
                                    .align(HorizontalAlignment::Leading)
                                    .child(SecondaryText::new(tr("music_album_title")).px(12.0))
                                    .child(div().w_full().child(InputOutlinedRowUi::new(
                                        (self.id.clone(), "title"),
                                        MusicFieldRules::album_title().binding(album_title),
                                        tr("music_album_title_placeholder"),
                                        div(),
                                    ))),
                            )
                            .child(Self::genre_chips(&self.id, &state, &genre, &theme)),
                    )
                    .children(uploading_hint)
                    .child(
                        div().w_full().px(px(16.0)).pb(px(32.0)).child(
                            ButtonUi::new((self.id.clone(), "submit"))
                                .disabled(!can_submit)
                                .on_press(move |_, cx| Self::submit_album(&submit_state, &on_dismiss, cx))
                                .child(MainText::new(button_title)),
                        ),
                    ),
            ),
        )
    }
}
