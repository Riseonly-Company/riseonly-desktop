use std::rc::Rc;

use gpui::{App, SharedString, Window};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_i18n::tr;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::music::pages::settings::moderation::music_request_shared::{
    MUSIC_PREFETCH_THRESHOLD, RequestListState, RequestSkeletonBody, pop_screen, request_list_row,
};
use crate::modules::music::pages::track::track_row::TrackRow;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::ProtoTrackRecordExt;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;
use crate::modules::music::widgets::music_skeletons::TrackListSkeleton;

struct MyMusicTrackSubmission {
    track: TrackModel,
    rejection_reason: Option<String>,
}

#[derive(IntoElement, Default)]
pub struct MyMusicTrackRequests;

impl MyMusicTrackRequests {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for MyMusicTrackRequests {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("my-music-track-requests-state", cx, |_, cx| RequestListState::new(cx));
        let slice = MusicActionsStore::slices(cx).list_my_tracks_sai.clone();
        let rejected = RiseMusicTrackStatus::Rejected.wire();
        let pending = RiseMusicTrackStatus::Pending.wire();
        let submission_tracks: Rc<Vec<MyMusicTrackSubmission>> = Rc::new(
            slice
                .data()
                .map(|data| data.tracks)
                .unwrap_or_default()
                .iter()
                .filter(|wire| wire.status == pending || wire.status == rejected)
                .filter_map(|wire| {
                    wire.as_track_model().map(|track| MyMusicTrackSubmission {
                        track,
                        rejection_reason: wire
                            .rejection_reason
                            .clone()
                            .filter(|reason| wire.status == rejected && !reason.is_empty()),
                    })
                })
                .collect(),
        );
        let ids: Vec<SharedString> =
            submission_tracks.iter().map(|submission| SharedString::from(submission.track.id.clone())).collect();
        let submission_count = ids.len();
        state.update(cx, |state, _| state.list_model.sync(ids));

        let is_empty = submission_tracks.is_empty();
        let list_state = state.clone();
        let error_color = theme.semantic.error_100;
        let cache_key = MusicServicesStore::my_tracks_cache_key();
        let row_cache_key = cache_key.clone();
        let body = SaiDataRender::slice(&*slice, move |_, cx| {
            let top_inset = list_shell_floating_top_inset(cx);
            list_state.read(cx).list_model.list().element(move |index, _, cx| {
                let Some(submission) = submission_tracks.get(index) else {
                    return div().into_any_element();
                };
                let track = &submission.track;
                let row = vstack(6.0)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(
                        TrackRow::new(SharedString::from(format!("my-music-track-request-{}", track.id)), track.clone())
                            .on_tap(|_, _| {}),
                    )
                    .when_some(submission.rejection_reason.clone(), |column, reason| {
                        column.child(
                            div().px(px(4.0)).child(
                                SecondaryText::new(format!("{}: {reason}", tr("music_rejection_reason")))
                                    .px(12.0)
                                    .color(error_color),
                            ),
                        )
                    })
                    .music_pagination_prefetch(
                        SharedString::from(format!("my-music-track-request-prefetch-{}", track.id)),
                        index,
                        submission_tracks.len(),
                        &row_cache_key,
                        true,
                        MUSIC_PREFETCH_THRESHOLD,
                        |_, cx| {
                            MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_my_tracks(cx));
                        },
                        cx,
                    );
                let entry =
                    request_list_row(index, submission_tracks.len(), top_inset + 12.0, 8.0, 0.0).px(px(16.0)).child(row);
                if index + 1 == submission_tracks.len() {
                    return entry.music_mini_player_bottom_padding(80.0, 0.0).into_any_element();
                }
                entry.into_any_element()
            })
        })
        .is_empty(is_empty)
        .no_data_text(tr("music_moderation_empty"))
        .need_pending(is_empty)
        .sai_data_render_loading(|_, _| {
            RequestSkeletonBody::new(12.0, TrackListSkeleton::new().rows(8).horizontal_padding(16.0))
        })
        .music_pagination_drain_filtered_empty(
            "my-music-track-requests-drain",
            submission_count,
            &cache_key,
            true,
            |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_my_tracks(cx));
            },
            cx,
        );

        SettingsWrapperUi::new(tr("music_my_track_requests"), move |_, cx| pop_screen(nav_store.as_ref(), cx))
            .content(div().size_full().bg(theme.bg._100).child(body))
            .on_appear("my-music-track-requests-appear", |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_my_tracks_if_needed(cx));
            })
    }
}
