use std::collections::HashMap;

use gpui::{App, Context, Entity, ParentElement, Render, Styled, Subscription, Window};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::sheet_header_bar::SheetHeaderBar;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::input_ui::InputUi;
use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    MusicAudioEffectKind, MusicAudioEffectKindExt,
};
use crate::modules::music::shared::utils::music::music_track_download_store::{
    MusicTrackDownloadPhase, MusicTrackDownloadStore,
};
use crate::modules::music::widgets::music_audio_effect_section::MusicAudioEffectSection;
use crate::modules::music::widgets::music_cover_picker_ui::MusicCoverPickerUi;

/// global host for the track download flow next to the other root sheets
/// both steps share one sheet since a nested one put the save dialog on a dying controller
pub struct MusicTrackDownloadSheet {
    store: Entity<MusicTrackDownloadStore>,
    _store_observation: Subscription,
}

impl MusicTrackDownloadSheet {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = MusicTrackDownloadStore::shared(cx);
        let store_observation = cx.observe(&store, |_, _, cx| cx.notify());
        Self { store, _store_observation: store_observation }
    }

    fn content(store: &Entity<MusicTrackDownloadStore>, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let phase = store.read(cx).phase();
        let is_options = phase == MusicTrackDownloadPhase::Options;
        let on_left = store.clone();
        let page = if is_options {
            Self::options_content(store, cx)
        } else {
            Self::metadata_content(store, cx)
        };
        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(
                SheetHeaderBar::new(
                    "music-track-download-header",
                    if is_options { tr("music_download_title") } else { tr("music_download_metadata_title") },
                    move |_, cx| {
                        on_left.update(cx, |store, cx| {
                            if store.phase() == MusicTrackDownloadPhase::Options {
                                store.dismiss_sheet(cx);
                            } else {
                                store.back_to_options(cx);
                            }
                        });
                    },
                )
                .left_system_image(if is_options { "xmark" } else { "chevron.backward" })
                .right_system_image(None),
            )
            .child(
                div().flex_1().min_h_0().w_full().child(
                    scroll_view("music-track-download-scroll").shows_indicators(false).child(
                        vstack(16.0).w_full().pt(px(8.0)).child(page).child(spacer_min_h(96.0)),
                    ),
                ),
            )
            .into_any_element()
    }

    fn options_content(store: &Entity<MusicTrackDownloadStore>, cx: &mut App) -> AnyElement {
        let read = store.read(cx);
        let settings = read.draft_settings().clone();
        let is_locked = read.is_working();
        let track_line = read.sheet_track().map(|track| format!("{} - {}", track.artist, track.title));
        vstack(16.0)
            .w_full()
            .children(track_line.map(|line| {
                div().w_full().px(px(24.0)).child(SecondaryText::new(line).px(13.0).tac(TextAlignT::Center))
            }))
            .children(MusicAudioEffectKind::all_cases().iter().copied().map(|kind| {
                let toggle_store = store.clone();
                let value_store = store.clone();
                MusicAudioEffectSection::new(
                    kind,
                    settings.is_enabled(kind),
                    settings.value(kind),
                    is_locked,
                    move |value, _, cx| Self::toggle(&toggle_store, kind, value, cx),
                    move |value, _, cx| Self::update(&value_store, kind, value, cx),
                )
            }))
            .into_any_element()
    }

    fn metadata_content(store: &Entity<MusicTrackDownloadStore>, cx: &mut App) -> AnyElement {
        let read = store.read(cx);
        let existing_cover_url = read.sheet_track().map(|track| track.cover_image_url.clone());
        let draft_title = read.draft_title().to_owned();
        let draft_artist = read.draft_artist().to_owned();
        let title_store = store.clone();
        let artist_store = store.clone();
        vstack(18.0)
            .w_full()
            .px(px(16.0))
            .child(
                MusicCoverPickerUi::new(
                    "music-track-download-cover",
                    Binding::entity(
                        store.clone(),
                        |store| store.draft_cover_file().cloned(),
                        |store, value, cx| store.set_draft_cover_file(value, cx),
                    ),
                    Binding::entity(
                        store.clone(),
                        |store| store.draft_cover().cloned(),
                        |store, value, cx| store.set_draft_cover(value, cx),
                    ),
                )
                .existing_cover_url(existing_cover_url)
                .placeholder_label(Some(tr("music_download_cover"))),
            )
            .child(
                InputUi::new("music-track-download-title")
                    .values(HashMap::from([("title".to_owned(), draft_title)]))
                    .name("title")
                    .placeholder(tr("music_download_field_title"))
                    .set_value(move |_, value, _, cx| {
                        title_store.update(cx, |store, cx| store.set_draft_title(value.to_owned(), cx));
                    }),
            )
            .child(
                InputUi::new("music-track-download-artist")
                    .values(HashMap::from([("artist".to_owned(), draft_artist)]))
                    .name("artist")
                    .placeholder(tr("music_download_field_artist"))
                    .set_value(move |_, value, _, cx| {
                        artist_store.update(cx, |store, cx| store.set_draft_artist(value.to_owned(), cx));
                    }),
            )
            .into_any_element()
    }

    fn footer(store: &Entity<MusicTrackDownloadStore>, theme: &AppTheme, cx: &App) -> AnyElement {
        let read = store.read(cx);
        let is_busy = read.is_working();
        let is_options = read.phase() == MusicTrackDownloadPhase::Options;
        let on_press = store.clone();
        ButtonUi::new("music-track-download-confirm")
            .disabled(is_busy)
            .is_pending(is_busy)
            .on_press(move |_, cx| {
                on_press.update(cx, |store, cx| {
                    if store.phase() == MusicTrackDownloadPhase::Options {
                        store.continue_to_metadata(cx);
                    } else {
                        store.save(cx);
                    }
                });
            })
            .child(
                MainText::new(if is_options { tr("music_download_confirm") } else { tr("music_download_save") })
                    .px(16.0)
                    .color(theme.content_on(theme.primary._100)),
            )
            .into_any_element()
    }

    fn toggle(store: &Entity<MusicTrackDownloadStore>, kind: MusicAudioEffectKind, value: bool, cx: &mut App) {
        store.update(cx, |store, cx| {
            if store.is_working() {
                return;
            }
            let mut settings = store.draft_settings().clone();
            settings.set_enabled(kind, value);
            store.set_draft_settings(settings, cx);
        });
    }

    fn update(store: &Entity<MusicTrackDownloadStore>, kind: MusicAudioEffectKind, value: f64, cx: &mut App) {
        store.update(cx, |store, cx| {
            if store.is_working() {
                return;
            }
            let mut settings = store.draft_settings().clone();
            settings.set_value(kind, value);
            store.set_draft_settings(settings, cx);
        });
    }
}

impl Render for MusicTrackDownloadSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let is_presented = Binding::entity(
            self.store.clone(),
            |store| store.is_sheet_presented(),
            |store, value, cx| {
                if !value {
                    store.dismiss_sheet(cx);
                }
            },
        );
        let content_store = self.store.clone();
        let footer_store = self.store.clone();
        let did_dismiss_store = self.store.clone();
        BottomSheetUi::new(
            "music-track-download-sheet",
            is_presented,
            move |_: &mut Window, cx: &mut App| {
                let theme = rise_ui::theme(cx).clone();
                Self::content(&content_store, &theme, cx)
            },
            move |_: &mut Window, cx: &mut App| {
                let theme = rise_ui::theme(cx).clone();
                Self::footer(&footer_store, &theme, cx)
            },
        )
        .snap_fractions(vec![0.93])
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(true))
        .footer_horizontal_padding(Some(14.0))
        .footer_top_padding(Some(10.0))
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
        .on_did_dismiss(move |_, cx| {
            did_dismiss_store.update(cx, |store, cx| store.sheet_did_dismiss(cx));
        })
    }
}
