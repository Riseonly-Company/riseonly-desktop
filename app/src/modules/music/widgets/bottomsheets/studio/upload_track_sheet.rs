use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    App, AppContext, Div, ElementId, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, transparent_black,
};
use rise_catalog::MusicGenre;
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::uniform_type::UTType;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::{PlainButton, plain};
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::progress_view::progress_view_value;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::stacks::{HorizontalAlignment, StackAlign, hstack, vstack};
use rise_ui::switch_ui::SwitchUi;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::ui::bottom_sheet_ui::sheet_floating_header::SheetFloatingHeader;
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::input_ui::InputOutlinedRowUi;
use crate::modules::media_picker::api::media_picker_presenter_resolver::MediaPickerPresenterResolver;
use crate::modules::media_picker::send::document_export_bridge::DocumentExportBridge;
use crate::modules::music::shared::utils::music::music_models::{AlbumModel, display_genre};
use crate::modules::music::shared::utils::music::music_track_file_upload::{MusicTrackFileUpload, MusicTrackFileUploadError};
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::WireAlbumDTOExt;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::bottomsheets::studio::create_album_sheet::StudioSheetDismiss;
use crate::modules::music::widgets::music_cover_picker_ui::MusicCoverPickerUi;

const AUDIO_FILE_EXTENSIONS: [&str; 6] = ["mp3", "flac", "m4a", "aac", "ogg", "wav"];
const AUDIO_DURATION_MINIMUM_MS: i64 = 500;
const AUDIO_DURATION_FLOOR_MS: i64 = 1000;

#[derive(Default)]
struct UploadTrackSheetState {
    track_title: String,
    genre: String,
    selected_album_id: Option<String>,
    is_exclusive: bool,
    local_cover_file: Option<SaiWsOptimisticLocalFile>,
    cover_preview: Option<CleverImage>,
    audio_file_name: Option<String>,
    local_audio_file: Option<SaiWsOptimisticLocalFile>,
    audio_duration_ms: i64,
    audio_pick_id: Option<Uuid>,
}

impl UploadTrackSheetState {
    fn can_publish(&self, is_publishing: bool) -> bool {
        !trimming_whitespaces(&self.track_title).is_empty()
            && !self.genre.is_empty()
            && self.cover_preview.is_some()
            && self.local_cover_file.is_some()
            && self.local_audio_file.is_some()
            && !is_publishing
    }
}

#[derive(IntoElement)]
pub struct UploadTrackSheet {
    id: ElementId,
    preselected_album_id: Option<String>,
    on_dismiss: StudioSheetDismiss,
}

impl UploadTrackSheet {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), preselected_album_id: None, on_dismiss: Rc::new(|_: &mut App| {}) }
    }

    pub fn preselected_album_id(mut self, preselected_album_id: Option<String>) -> Self {
        self.preselected_album_id = preselected_album_id;
        self
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(on_dismiss);
        self
    }

    fn chip(id: impl Into<ElementId>, title: impl Into<SharedString>, is_selected: bool, theme: &AppTheme) -> PlainButton {
        let title: SharedString = title.into();
        plain(id).child(
            div()
                .px(px(12.0))
                .py(px(7.0))
                .capsule()
                .whitespace_nowrap()
                .swiftui_font(Font::system_weight(13.0, if is_selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }))
                .text_color(if is_selected { theme.primary._100 } else { theme.text.secondary })
                .bg(if is_selected { theme.primary._100.opacity(0.15) } else { theme.bg._200 })
                .stroke_border(if is_selected { theme.primary._100 } else { transparent_black() }, 1.0)
                .child(title),
        )
    }

    fn album_picker(
        id: &ElementId,
        state: &Entity<UploadTrackSheetState>,
        selected_album_id: Option<&str>,
        my_albums: &[AlbumModel],
        theme: &AppTheme,
    ) -> Div {
        let single_state = state.clone();
        let mut chips = hstack(8.0).child(
            Self::chip((id.clone(), "album-single"), tr("music_upload_single"), selected_album_id.is_none(), theme).on_press(
                move |_, cx| {
                    single_state.update(cx, |state, cx| {
                        state.selected_album_id = None;
                        cx.notify();
                    });
                },
            ),
        );
        for album in my_albums {
            let album_state = state.clone();
            let album_id = album.id.clone();
            chips = chips.child(
                Self::chip((id.clone(), album.id.clone()), album.title.clone(), selected_album_id == Some(album.id.as_str()), theme)
                    .on_press(move |_, cx| {
                        let album_id = album_id.clone();
                        album_state.update(cx, |state, cx| {
                            state.selected_album_id = Some(album_id);
                            cx.notify();
                        });
                    }),
            );
        }
        let hint = my_albums.is_empty().then(|| SecondaryText::new(tr("music_upload_single_hint")).px(12.0));
        vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(SecondaryText::new(tr("music_upload_album_or_single")).px(12.0))
            .child(div().id((id.clone(), "albums")).w_full().overflow_x_scroll().child(chips))
            .children(hint)
    }

    fn genre_chips(id: &ElementId, state: &Entity<UploadTrackSheetState>, selected: &str, theme: &AppTheme) -> Div {
        let mut chips = hstack(8.0).w_full().flex_wrap();
        for genre in MusicGenre::ALL {
            let wire = genre.wire();
            let state = state.clone();
            chips = chips.child(Self::chip((id.clone(), wire), display_genre(wire), selected == wire, theme).on_press(move |_, cx| {
                state.update(cx, |state, cx| {
                    state.genre = if state.genre == wire { String::new() } else { wire.to_owned() };
                    cx.notify();
                });
            }));
        }
        vstack(8.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(SecondaryText::new(tr("music_artist_genre")).px(12.0))
            .child(chips)
    }

    fn exclusive_toggle(id: &ElementId, state: &Entity<UploadTrackSheetState>, theme: &AppTheme) -> Div {
        let is_exclusive = Binding::entity(
            state.clone(),
            |state: &UploadTrackSheetState| state.is_exclusive,
            |state: &mut UploadTrackSheetState, value, _| state.is_exclusive = value,
        );
        hstack(12.0)
            .w_full()
            .p(px(14.0))
            .bg(theme.bg._200)
            .rounded(theme.radius._300)
            .stroke_border(theme.border._100, 1.0)
            .child(
                vstack(2.0)
                    .flex_1()
                    .min_w(px(0.0))
                    .align(HorizontalAlignment::Leading)
                    .child(MainText::new(tr("music_exclusive_toggle_title")).px(15.0).color(theme.text.primary))
                    .child(SecondaryText::new(tr("music_exclusive_toggle_hint")).px(12.0)),
            )
            .child(div().flex_none().child(SwitchUi::new((id.clone(), "exclusive"), is_exclusive)))
    }

    #[allow(clippy::too_many_arguments)]
    fn audio_picker_row(
        id: &ElementId,
        state: &Entity<UploadTrackSheetState>,
        audio_file_name: Option<String>,
        duration_label: String,
        is_audio_uploading: bool,
        audio_upload_progress: f64,
        theme: &AppTheme,
    ) -> PlainButton {
        let pick_state = state.clone();
        let trailing = if is_audio_uploading {
            div().w(px(28.0)).child(progress_view_value((audio_upload_progress / 100.0).clamp(0.0, 1.0))).into_any_element()
        } else {
            div().children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(17.0), theme.text.secondary, false)).into_any_element()
        };
        plain((id.clone(), "audio"))
            .w_full()
            .on_press(move |_, cx| Self::present_audio_document_picker(&pick_state, cx))
            .child(
                hstack(12.0)
                    .w_full()
                    .p(px(14.0))
                    .bg(theme.bg._200)
                    .rounded(theme.radius._300)
                    .stroke_border(theme.border._100, 1.0)
                    .children(IconUi::sized("waveform", px(20.0), theme.primary._100, false))
                    .child(
                        vstack(2.0)
                            .flex_1()
                            .min_w(px(0.0))
                            .align(HorizontalAlignment::Leading)
                            .child(
                                MainText::new(audio_file_name.unwrap_or_else(|| tr("music_upload_file")))
                                    .px(14.0)
                                    .color(theme.text.primary)
                                    .line_limit(1)
                                    .w_full(),
                            )
                            .child(SecondaryText::new(duration_label).px(12.0).line_limit(1).w_full()),
                    )
                    .child(div().flex_none().child(trailing)),
            )
    }

    fn labeled_field(id: ElementId, label: String, value: Binding<String>, placeholder: String) -> Div {
        vstack(6.0)
            .w_full()
            .align(HorizontalAlignment::Leading)
            .child(SecondaryText::new(label).px(12.0))
            .child(div().w_full().child(InputOutlinedRowUi::new(id, value, placeholder, div())))
    }

    fn present_audio_document_picker(state: &Entity<UploadTrackSheetState>, cx: &mut App) {
        let Some(presenter) = MediaPickerPresenterResolver::top_presenter(cx) else {
            return;
        };
        let mut audio_types = vec![UTType::audio(), UTType::mpeg4_audio()];
        audio_types.extend(AUDIO_FILE_EXTENSIONS.into_iter().filter_map(UTType::from_filename_extension));
        let state = state.clone();
        DocumentExportBridge::new(
            Some(audio_types),
            false,
            Box::new(move |urls: Vec<PathBuf>, cx: &mut App| {
                let Some(url) = urls.into_iter().next() else {
                    return;
                };
                Self::handle_audio_picked(&state, url, cx);
            }),
        )
        .present(presenter, cx);
    }

    fn present_audio_pick_error(message: &str, cx: &mut App) {
        NativeConfirmAlert::present_notice(&tr("upload_error_file_too_large"), message, None, None, |_: &mut App| {}, cx);
    }

    fn file_name(url: &Path) -> String {
        url.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| tr("music_upload_select_audio"))
    }

    fn prepare_failure_message(error: &anyhow::Error) -> String {
        error
            .downcast_ref::<MusicTrackFileUploadError>()
            .and_then(MusicTrackFileUploadError::error_description)
            .unwrap_or_else(|| error.to_string())
    }

    fn handle_audio_picked(state: &Entity<UploadTrackSheetState>, url: PathBuf, cx: &mut App) {
        if MusicTrackFileUpload::exceeds_track_file_limit(&url) {
            state.update(cx, |state, cx| {
                state.audio_file_name = None;
                state.local_audio_file = None;
                state.audio_duration_ms = 0;
                cx.notify();
            });
            Self::present_audio_pick_error(&tr("music_local_upload_too_large"), cx);
            return;
        }
        let name = Self::file_name(&url);
        let pick_id = Uuid::new_v4();
        let shown_name = name.clone();
        state.update(cx, |state, cx| {
            state.audio_file_name = Some(shown_name);
            state.local_audio_file = None;
            state.audio_duration_ms = 0;
            state.audio_pick_id = Some(pick_id);
            cx.notify();
        });
        let state = state.downgrade();
        cx.spawn(async move |cx| {
            let prepared = cx.background_spawn(async move { MusicTrackFileUpload::prepare_studio_audio(&url) }).await;
            let prepared = prepared.and_then(|copy| {
                let file_url = Url::from_file_path(&copy).ok_or(MusicTrackFileUploadError::MissingLocalFile)?;
                Ok((copy, file_url))
            });
            match prepared {
                Ok((copy, file_url)) => {
                    let duration = MusicTrackFileUpload::audio_duration_ms(copy.clone()).await;
                    let _ = state.update(cx, |state, cx| {
                        if state.audio_pick_id != Some(pick_id) {
                            return;
                        }
                        state.local_audio_file =
                            Some(SaiWsOptimisticLocalFile::new(file_url, name, MusicTrackFileUpload::mime_type(&copy)));
                        state.audio_duration_ms = duration;
                        cx.notify();
                    });
                }
                Err(error) => {
                    let is_current = state.update(cx, |state, cx| {
                        if state.audio_pick_id != Some(pick_id) {
                            return false;
                        }
                        state.audio_file_name = None;
                        cx.notify();
                        true
                    });
                    if matches!(is_current, Ok(true)) {
                        let message = Self::prepare_failure_message(&error);
                        cx.update(|cx| Self::present_audio_pick_error(&message, cx));
                    }
                }
            }
        })
        .detach();
    }

    fn publish_track(state: &Entity<UploadTrackSheetState>, on_dismiss: &StudioSheetDismiss, cx: &mut App) {
        let actions = MusicActionsStore::shared(cx);
        let slice = Rc::clone(&actions.read(cx).create_track_sai);
        let upload = slice.data();
        let is_publishing = slice.is_pending()
            || upload.as_ref().is_some_and(|data| data.cover_uploading == Some(true) || data.audio_uploading == Some(true));
        let sheet = state.read(cx);
        if !sheet.can_publish(is_publishing) {
            return;
        }
        let (Some(local_cover_file), Some(local_audio_file)) = (sheet.local_cover_file.clone(), sheet.local_audio_file.clone()) else {
            return;
        };
        let title = trimming_whitespaces(&sheet.track_title).to_owned();
        let genre = sheet.genre.clone();
        let album_id = sheet.selected_album_id.clone();
        let is_exclusive = sheet.is_exclusive;
        let on_dismiss = Rc::clone(on_dismiss);
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.publish_track(
                &title,
                &genre,
                local_cover_file,
                local_audio_file,
                album_id.as_deref(),
                is_exclusive,
                Some(Rc::new(move |cx: &mut App| {
                    let on_dismiss = Rc::clone(&on_dismiss);
                    NativeConfirmAlert::present_notice(
                        &tr("music_upload_success_title"),
                        &tr("music_upload_success_msg"),
                        None,
                        None,
                        move |cx: &mut App| on_dismiss(cx),
                        cx,
                    );
                })),
                Some(Rc::new(|message: String, cx: &mut App| {
                    NativeConfirmAlert::present_notice(
                        &tr("music_upload_error_title"),
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
}

impl RenderOnce for UploadTrackSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let state = window.use_keyed_state((self.id.clone(), "state"), cx, |_, _| UploadTrackSheetState::default());
        let actions = MusicActionsStore::shared(cx);
        let my_albums: Vec<AlbumModel> = actions
            .read(cx)
            .list_my_albums_sai
            .data()
            .map(|data| data.albums)
            .unwrap_or_default()
            .iter()
            .filter_map(|album| album.as_album_model())
            .collect();
        let slice = Rc::clone(&actions.read(cx).create_track_sai);
        let upload = slice.data();
        let cover_upload_progress = upload.as_ref().and_then(|data| data.cover_upload_progress).unwrap_or(0.0);
        let audio_upload_progress = upload.as_ref().and_then(|data| data.audio_upload_progress).unwrap_or(0.0);
        let is_cover_uploading = upload.as_ref().is_some_and(|data| data.cover_uploading == Some(true));
        let is_audio_uploading = upload.as_ref().is_some_and(|data| data.audio_uploading == Some(true));
        let is_publishing = slice.is_pending() || is_cover_uploading || is_audio_uploading;

        let sheet = state.read(cx);
        let can_publish = sheet.can_publish(is_publishing);
        let has_local_cover = sheet.local_cover_file.is_some();
        let genre = sheet.genre.clone();
        let selected_album_id = sheet.selected_album_id.clone();
        let audio_file_name = sheet.audio_file_name.clone();
        let duration_label = if is_audio_uploading {
            tr("music_upload_file_uploading")
        } else if sheet.audio_duration_ms >= AUDIO_DURATION_MINIMUM_MS {
            RiseFormat::duration_milliseconds(sheet.audio_duration_ms.max(AUDIO_DURATION_FLOOR_MS))
        } else {
            tr("music_upload_formats_hint")
        };

        let local_cover_file = Binding::entity(
            state.clone(),
            |state: &UploadTrackSheetState| state.local_cover_file.clone(),
            |state: &mut UploadTrackSheetState, value, _| state.local_cover_file = value,
        );
        let cover_preview = Binding::entity(
            state.clone(),
            |state: &UploadTrackSheetState| state.cover_preview.clone(),
            |state: &mut UploadTrackSheetState, value, _| state.cover_preview = value,
        );
        let track_title = Binding::entity(
            state.clone(),
            |state: &UploadTrackSheetState| state.track_title.clone(),
            |state: &mut UploadTrackSheetState, value, _| state.track_title = value,
        );

        let publish_state = state.clone();
        let on_dismiss = Rc::clone(&self.on_dismiss);
        let appear_state = state.clone();
        let preselected_album_id = self.preselected_album_id;
        let uploading_hint = (is_cover_uploading || is_audio_uploading).then(|| {
            let hint = if is_cover_uploading { tr("music_upload_cover_uploading") } else { tr("music_upload_file_uploading") };
            SecondaryText::new(hint).px(12.0).tac(TextAlignT::Center)
        });
        let button_title: SharedString =
            if is_publishing { tr("music_uploading").into() } else { tr("music_publish").into() };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .child(div().w_full().flex_none().child({
                let leading_dismiss = Rc::clone(&self.on_dismiss);
                SheetFloatingHeader::new((self.id.clone(), "header"), tr("music_upload_track"), move |_, cx| leading_dismiss(cx))
                    .shows_chrome(false)
            }))
            .child(
                scroll_view((self.id.clone(), "scroll")).flex_1().min_h_0().w_full().child(
                    vstack(16.0)
                        .w_full()
                        .pt(px(16.0))
                        .child(
                            MusicCoverPickerUi::new((self.id.clone(), "cover"), local_cover_file, cover_preview)
                                .placeholder_label(Some(tr("music_upload_cover")))
                                .cover_upload_progress(cover_upload_progress)
                                .show_upload_overlay(is_cover_uploading && has_local_cover),
                        )
                        .child(
                            vstack(12.0)
                                .w_full()
                                .px(px(16.0))
                                .child(Self::labeled_field(
                                    (self.id.clone(), "title").into(),
                                    tr("music_upload_title"),
                                    track_title,
                                    tr("music_track_title_placeholder"),
                                ))
                                .child(Self::album_picker(&self.id, &state, selected_album_id.as_deref(), &my_albums, &theme))
                                .child(Self::genre_chips(&self.id, &state, &genre, &theme))
                                .child(Self::exclusive_toggle(&self.id, &state, &theme))
                                .child(Self::audio_picker_row(
                                    &self.id,
                                    &state,
                                    audio_file_name,
                                    duration_label,
                                    is_audio_uploading,
                                    audio_upload_progress,
                                    &theme,
                                )),
                        )
                        .children(uploading_hint)
                        .child(
                            div().w_full().px(px(16.0)).pb(px(32.0)).child(
                                ButtonUi::new((self.id.clone(), "publish"))
                                    .disabled(!can_publish)
                                    .on_press(move |_, cx| Self::publish_track(&publish_state, &on_dismiss, cx))
                                    .child(MainText::new(button_title)),
                            ),
                        ),
                ),
            )
            .on_appear((self.id, "appear"), move |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_my_albums_if_needed(cx));
                if let Some(preselected_album_id) = preselected_album_id {
                    appear_state.update(cx, |state, cx| {
                        state.selected_album_id = Some(preselected_album_id);
                        cx.notify();
                    });
                }
            })
    }
}
