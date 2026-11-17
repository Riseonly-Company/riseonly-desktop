use gpui::{App, FontWeight, SharedString, Window};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::widgets::wrappers::list_shell_ui::SettingsWrapperUi;
use crate::modules::music::pages::settings::moderation::music_request_shared::{
    RequestDetailState, RequestScrollBody, labeled_card, moderation_actions, music_status_title, pop_screen, reject_sheet,
};
use crate::modules::music::shared::utils::music::music_models::{TrackModel, display_genre};
use crate::modules::music::shared::utils::music::music_playback_url::MusicPlaybackUrl;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::{ProtoTrackRecordExt, RiseStoredMusicTrack};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::music_exclusive_badge::MusicExclusiveBadge;
use crate::modules::music::widgets::music_mini_player_bottom_padding::observed_music_player;
use crate::modules::music::widgets::music_track_cover_image::MusicTrackCoverImage;

const PREVIEW_ICON_SIDE: f32 = 17.0;

#[derive(IntoElement)]
pub struct MusicTrackRequestDetail {
    track_id: String,
}

impl MusicTrackRequestDetail {
    pub fn new(track_id: impl Into<String>) -> Self {
        Self { track_id: track_id.into() }
    }

    fn wire(track_id: &str, cx: &mut App) -> Option<RiseStoredMusicTrack> {
        let actions = MusicActionsStore::shared(cx);
        let store = actions.read(cx);
        if let Some(track) = store.get_track_request_sai.data().and_then(|data| data.track)
            && track.id == track_id
        {
            return Some(RiseStoredMusicTrack::from_canon(&track));
        }
        store.list_track_requests_sai.data()?.tracks.iter().find(|track| track.id == track_id).map(|track| track.stored())
    }

    fn can_preview_audio(wire: &RiseStoredMusicTrack) -> bool {
        let Some(raw) = wire.audio_url.as_deref().map(trimming_whitespaces_and_newlines).filter(|raw| !raw.is_empty())
        else {
            return false;
        };
        !MusicPlaybackUrl::playback_url_candidates_from(raw).is_empty()
    }

    fn is_pending_request(wire: &RiseStoredMusicTrack) -> bool {
        wire.status.as_deref().unwrap_or_default().to_lowercase() == RiseMusicTrackStatus::Pending.wire()
    }

    fn start_preview(track: TrackModel, cx: &mut App) {
        let preview = MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.preview_track(track, cx));
        cx.spawn(async move |cx| {
            if preview.await {
                cx.update(|cx| {
                    MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_show_now_playing(true, cx));
                });
            }
        })
        .detach();
    }

    fn preview_button(id: &str, wire: &RiseStoredMusicTrack, track: &TrackModel, player: &MusicPlayerState, theme: &AppTheme) -> ButtonUi {
        let can_preview_audio = Self::can_preview_audio(wire);
        let is_current = player.current_track.as_ref().is_some_and(|current| current.id == track.id);
        let is_preparing_current = player.is_preparing_playback && is_current;
        let ink = theme.content_on(theme.primary._100);
        let track = track.clone();
        ButtonUi::new(SharedString::from(format!("{id}-preview")))
            .disabled(!can_preview_audio || player.is_preparing_playback)
            .on_press(move |_, cx| {
                if !can_preview_audio {
                    NativeConfirmAlert::present_notice(
                        &tr("music_upload_error_title"),
                        &tr("music_preview_no_audio"),
                        None,
                        None,
                        |_: &mut App| {},
                        cx,
                    );
                    return;
                }
                Self::start_preview(track.clone(), cx);
            })
            .child(
                hstack(8.0)
                    .when(is_preparing_current, |row| row.child(progress_view().tint(ink)))
                    .when(!is_preparing_current, |row| {
                        let symbol = if player.is_playing && is_current { "pause.fill" } else { "play.fill" };
                        row.children(IconUi::sized(symbol, px(PREVIEW_ICON_SIDE), ink, symbol.ends_with(".fill")))
                    })
                    .child(
                        MainText::new(if is_preparing_current { tr("music_preview_loading") } else { tr("music_preview_track") })
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD),
                    ),
            )
    }
}

impl RenderOnce for MusicTrackRequestDetail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = format!("music-track-request-detail-{}", self.track_id);
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(SharedString::from(format!("{id}-state")), cx, |_, cx| RequestDetailState::new(cx));
        let player = observed_music_player(SharedString::from(format!("{id}-player")), window, cx);
        let actions = MusicActionsStore::shared(cx);
        let review_slice = actions.read(cx).review_track_request_sai.clone();
        let is_review_pending = review_slice.is_pending();
        let is_get_pending = actions.read(cx).get_track_request_sai.is_pending();
        let wire = Self::wire(&self.track_id, cx);
        let track = wire.as_ref().and_then(|wire| wire.as_track_model());

        let content = match (wire, track) {
            (Some(w), Some(t)) => {
                let approve_id = self.track_id.clone();
                let preview_button = Self::preview_button(&id, &w, &t, player.read(cx), &theme);
                let status = w.status.clone().unwrap_or_else(|| RiseMusicTrackStatus::Pending.wire().to_owned());
                vstack(14.0)
                    .w_full()
                    .px(px(16.0))
                    .pt(px(16.0))
                    .when(!t.cover_image_url.is_empty(), |column| {
                        column.child(
                            MusicTrackCoverImage::new(SharedString::from(format!("{id}-cover")), t.cover_image_url.clone())
                                .size(180.0)
                                .corner_radius(12.0),
                        )
                    })
                    .child(MainText::new(t.title.clone()).px(20.0).tac(TextAlignT::Center).font_weight(FontWeight::BOLD))
                    .when(w.is_exclusive == Some(true), |column| column.child(MusicExclusiveBadge::new()))
                    .child(SecondaryText::new(format!("{} \u{00B7} {}", t.artist, t.formatted_duration())).px(13.0).tac(TextAlignT::Center))
                    .when_some(w.genre.clone().filter(|genre| !genre.is_empty()), |column, genre| {
                        column.child(labeled_card(tr("music_artist_genre"), display_genre(&genre), &theme))
                    })
                    .when_some(
                        w.album.as_ref().map(|album| album.title.clone()).filter(|title| !title.is_empty()),
                        |column, title| column.child(labeled_card(tr("music_track_album"), title, &theme)),
                    )
                    .child(labeled_card(tr("music_status_label"), music_status_title(&status), &theme))
                    .when_some(w.rejection_reason.clone().filter(|reason| !reason.is_empty()), |column, reason| {
                        column.child(labeled_card(tr("music_rejection_reason"), reason, &theme))
                    })
                    .child(preview_button)
                    .when(Self::is_pending_request(&w), |column| {
                        column.child(moderation_actions(
                            &id,
                            is_review_pending,
                            &state,
                            nav_store.clone(),
                            move |cx| {
                                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                                    store.review_moderation_track_request(&approve_id, true, None, cx);
                                });
                            },
                            &theme,
                        ))
                    })
                    .child(div().h(px(80.0)))
            }
            _ if is_get_pending => {
                div().w_full().flex().justify_center().pt(px(80.0)).child(progress_view().tint(theme.primary._100))
            }
            _ => div().w_full().pt(px(80.0)).child(
                SecondaryText::new(tr("music_request_not_found")).px(13.0).tac(TextAlignT::Center),
            ),
        };

        let reject_id = self.track_id.clone();
        let sheet = reject_sheet(
            &id,
            &state,
            nav_store.clone(),
            move || review_slice.is_pending(),
            move |reason, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.review_moderation_track_request(&reject_id, false, Some(reason), cx);
                });
            },
            &theme,
            cx,
        );
        let appear_id = self.track_id;

        div()
            .size_full()
            .child(
                SettingsWrapperUi::new(tr("music_track_request_detail"), move |_, cx| pop_screen(nav_store.as_ref(), cx))
                    .content(
                        div()
                            .size_full()
                            .bg(theme.bg._100)
                            .child(RequestScrollBody::new(SharedString::from(format!("{id}-scroll")), content)),
                    ),
            )
            .child(sheet)
            .on_appear(SharedString::from(format!("{id}-appear")), move |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.open_moderation_track_request(&appear_id, cx));
            })
    }
}
