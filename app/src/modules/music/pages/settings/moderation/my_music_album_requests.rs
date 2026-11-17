use std::rc::Rc;

use gpui::{App, FontWeight, SharedString, Window};
use rise_canon::rise_service_text::music_service;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::widgets::wrappers::list_shell_ui::{SettingsWrapperUi, list_shell_floating_top_inset};
use crate::modules::music::pages::settings::moderation::music_request_shared::{
    MISSING_VALUE, MUSIC_PREFETCH_THRESHOLD, RequestListState, RequestSkeletonBody, album_status_badge, pop_screen,
    request_list_row,
};
use crate::modules::music::shared::utils::music::music_models::display_genre;
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::{WireAlbumRequestDTO, WireAlbumRequestDTOExt};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_skeletons::MusicLibraryListSkeleton;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

#[derive(IntoElement, Default)]
pub struct MyMusicAlbumRequests;

impl MyMusicAlbumRequests {
    pub fn new() -> Self {
        Self
    }

    fn album_request_row(request: &WireAlbumRequestDTO, theme: &AppTheme) -> Div {
        let rejection_reason = request
            .rejection_reason
            .as_ref()
            .filter(|reason| {
                request.status == music_service::ALBUM_REQUEST_STATUS_REJECTED && !reason.is_empty()
            })
            .cloned();
        hstack(14.0)
            .w_full()
            .px(px(16.0))
            .py(px(12.0))
            .child(
                MusicCoverImage::new(
                    SharedString::from(format!("my-music-album-request-cover-{}", request.id)),
                    request.cover_image_url(),
                )
                .size(56.0)
                .corner_radius(10.0),
            )
            .child(
                vstack(4.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        MainText::new(if request.title.is_empty() { MISSING_VALUE.to_owned() } else { request.title.clone() })
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .line_limit(1),
                    )
                    .child(
                        hstack(6.0)
                            .when(!request.genre.is_empty(), |row| {
                                row.child(SecondaryText::new(display_genre(&request.genre)).px(12.0))
                            })
                            .children(album_status_badge(&request.status, theme)),
                    )
                    .when_some(rejection_reason, |column, reason| {
                        column.child(
                            SecondaryText::new(format!("{}: {reason}", tr("music_rejection_reason")))
                                .px(12.0)
                                .color(theme.semantic.error_100),
                        )
                    }),
            )
    }
}

impl RenderOnce for MyMusicAlbumRequests {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("my-music-album-requests-state", cx, |_, cx| RequestListState::new(cx));
        let slice = MusicActionsStore::slices(cx).get_my_album_requests_sai.clone();
        let my_album_requests: Rc<Vec<WireAlbumRequestDTO>> =
            Rc::new(slice.data().map(|data| data.requests).unwrap_or_default());
        let ids: Vec<SharedString> =
            my_album_requests.iter().map(|request| SharedString::from(request.id.clone())).collect();
        state.update(cx, |state, _| state.list_model.sync(ids));

        let is_empty = my_album_requests.is_empty();
        let list_state = state.clone();
        let row_theme = theme.clone();
        let cache_key = MusicServicesStore::my_album_requests_cache_key();
        let body = SaiDataRender::slice(&*slice, move |_, cx| {
            let top_inset = list_shell_floating_top_inset(cx);
            let border = row_theme.border._100;
            list_state.read(cx).list_model.list().element(move |index, _, cx| {
                let Some(request) = my_album_requests.get(index) else {
                    return div().into_any_element();
                };
                let row = Self::album_request_row(request, &row_theme).music_pagination_prefetch(
                    SharedString::from(format!("my-music-album-request-prefetch-{}", request.id)),
                    index,
                    my_album_requests.len(),
                    &cache_key,
                    true,
                    MUSIC_PREFETCH_THRESHOLD,
                    |_, cx| {
                        MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_my_album_requests(cx));
                    },
                    cx,
                );
                request_list_row(index, my_album_requests.len(), top_inset + 8.0, 0.0, 80.0)
                    .child(row)
                    .child(div().pl(px(78.0)).w_full().child(div().w_full().h(px(0.5)).bg(border)))
                    .into_any_element()
            })
        })
        .is_empty(is_empty)
        .no_data_text(tr("music_moderation_empty"))
        .need_pending(is_empty)
        .sai_data_render_loading(|_, _| RequestSkeletonBody::new(8.0, MusicLibraryListSkeleton::new().rows(8)));

        SettingsWrapperUi::new(tr("music_album_requests"), move |_, cx| pop_screen(nav_store.as_ref(), cx))
            .content(div().size_full().bg(theme.bg._100).child(body))
            .on_appear("my-music-album-requests-appear", |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_my_album_requests_if_needed(cx));
            })
    }
}
