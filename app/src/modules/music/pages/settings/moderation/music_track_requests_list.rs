use std::rc::Rc;

use gpui::{App, SharedString, Window};
use rise_i18n::tr;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_floating_top_inset};
use crate::modules::music::pages::settings::moderation::moderation_status_filter_bar::ModerationStatusFilterBar;
use crate::modules::music::pages::settings::moderation::music_request_shared::{
    MUSIC_PREFETCH_THRESHOLD, RequestListState, RequestSkeletonBody, pop_screen, request_list_row,
};
use crate::modules::music::pages::track::track_row::TrackRow;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::{ProtoTrackRecordExt, RiseStoredMusicTrack};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_mini_player_bottom_padding::MusicMiniPlayerBottomPaddingModifiers;
use crate::modules::music::widgets::music_skeletons::TrackListSkeleton;

#[derive(IntoElement, Default)]
pub struct MusicTrackRequestsList;

impl MusicTrackRequestsList {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for MusicTrackRequestsList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("music-track-requests-list-state", cx, |_, cx| RequestListState::new(cx));
        let status_filter = state.read(cx).status_filter.clone();
        let slice = MusicActionsStore::slices(cx).list_track_requests_sai.clone();
        let requests: Vec<RiseStoredMusicTrack> =
            slice.data().map(|data| data.tracks).unwrap_or_default().iter().map(|track| track.stored()).collect();
        let tracks: Rc<Vec<TrackModel>> = Rc::new(requests.iter().filter_map(|track| track.as_track_model()).collect());
        let ids: Vec<SharedString> = tracks.iter().map(|track| SharedString::from(track.id.clone())).collect();
        state.update(cx, |state, _| state.list_model.sync(ids));

        let appear_status = status_filter.clone();
        let back_store = nav_store.clone();
        let is_empty = requests.is_empty();
        let list_state = state.clone();
        let cache_key = MusicServicesStore::track_requests_cache_key(&status_filter);
        let body = SaiDataRender::slice(&*slice, move |_, cx| {
            let top_inset = list_shell_floating_top_inset(cx);
            list_state.read(cx).list_model.list().element(move |index, _, cx| {
                let Some(track) = tracks.get(index) else {
                    return div().into_any_element();
                };
                let track_id = track.id.clone();
                let row_store = nav_store.clone();
                let status_filter = status_filter.clone();
                let row = TrackRow::new(SharedString::from(format!("music-track-request-{}", track.id)), track.clone())
                    .shows_like_button(false)
                    .on_tap(move |_, cx| {
                        if let Some(nav_store) = row_store.as_ref() {
                            let route = RootRoute::MusicTrackRequestDetail { track_id: track_id.clone() };
                            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                        }
                    })
                    .music_pagination_prefetch(
                        SharedString::from(format!("music-track-request-prefetch-{}", track.id)),
                        index,
                        tracks.len(),
                        &cache_key,
                        true,
                        MUSIC_PREFETCH_THRESHOLD,
                        move |_, cx| {
                            MusicInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.load_more_moderation_track_requests(&status_filter, cx));
                        },
                        cx,
                    );
                let entry = request_list_row(index, tracks.len(), top_inset + 12.0, 8.0, 0.0).px(px(16.0)).child(row);
                if index + 1 == tracks.len() {
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
        });

        ListShellUi::new()
            .title(SharedString::from(tr("music_admin_track_requests")))
            .on_back(move |_, cx| pop_screen(back_store.as_ref(), cx))
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .below_header(
                ModerationStatusFilterBar::new(RequestListState::status_filter_binding(&state), theme.clone(), |status, cx| {
                    MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_moderation_track_requests(status, cx));
                })
                .track_mode(true),
            )
            .content(div().size_full().bg(theme.bg._100).child(body))
            .on_appear("music-track-requests-list-appear", move |_, cx| {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_moderation_track_requests(&appear_status, true, cx));
            })
    }
}
