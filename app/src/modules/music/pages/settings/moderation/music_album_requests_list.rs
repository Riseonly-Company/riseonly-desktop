use std::rc::Rc;

use gpui::{App, Entity, FontWeight, SharedString, Window};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_floating_top_inset};
use crate::modules::music::pages::settings::moderation::moderation_status_filter_bar::ModerationStatusFilterBar;
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
pub struct MusicAlbumRequestsList;

impl MusicAlbumRequestsList {
    pub fn new() -> Self {
        Self
    }

    fn album_request_row(
        request: &WireAlbumRequestDTO,
        nav_store: Option<Entity<NavigationStore>>,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let request_id = request.id.clone();
        let artist_name = request.artist.as_ref().map(|artist| artist.stage_name.clone()).filter(|name| !name.is_empty());
        plain(SharedString::from(format!("music-album-request-{request_id}")))
            .w_full()
            .on_press(move |_, cx| {
                if !request_id.is_empty()
                    && let Some(nav_store) = nav_store.as_ref()
                {
                    let route = RootRoute::MusicAlbumRequestDetail { request_id: request_id.clone() };
                    nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            })
            .child(
                hstack(14.0)
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
                    .child(
                        MusicCoverImage::new(
                            SharedString::from(format!("music-album-request-cover-{}", request.id)),
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
                                    .when_some(artist_name, |row, name| row.child(SecondaryText::new(name).px(12.0)))
                                    .children(album_status_badge(&request.status, theme)),
                            )
                            .when(!request.genre.is_empty(), |column| {
                                column.child(SecondaryText::new(display_genre(&request.genre)).px(12.0))
                            }),
                    )
                    .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(13.0), theme.text.secondary, false)),
            )
    }
}

impl RenderOnce for MusicAlbumRequestsList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("music-album-requests-list-state", cx, |_, cx| RequestListState::new(cx));
        let status_filter = state.read(cx).status_filter.clone();
        let slice = MusicActionsStore::slices(cx).list_album_requests_sai.clone();
        let requests: Rc<Vec<WireAlbumRequestDTO>> = Rc::new(slice.data().map(|data| data.requests).unwrap_or_default());
        let ids: Vec<SharedString> = requests.iter().map(|request| SharedString::from(request.id.clone())).collect();
        state.update(cx, |state, _| state.list_model.sync(ids));

        let appear_status = status_filter.clone();
        let back_store = nav_store.clone();
        let is_empty = requests.is_empty();
        let list_state = state.clone();
        let row_theme = theme.clone();
        let cache_key = MusicServicesStore::album_requests_cache_key(&status_filter);
        let body = SaiDataRender::slice(&*slice, move |_, cx| {
            let top_inset = list_shell_floating_top_inset(cx);
            let border = row_theme.border._100;
            list_state.read(cx).list_model.list().element(move |index, _, cx| {
                let Some(request) = requests.get(index) else {
                    return div().into_any_element();
                };
                let status_filter = status_filter.clone();
                let row = Self::album_request_row(request, nav_store.clone(), &row_theme).music_pagination_prefetch(
                    SharedString::from(format!("music-album-request-prefetch-{}", request.id)),
                    index,
                    requests.len(),
                    &cache_key,
                    true,
                    MUSIC_PREFETCH_THRESHOLD,
                    move |_, cx| {
                        MusicInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.load_more_moderation_album_requests(&status_filter, cx));
                    },
                    cx,
                );
                request_list_row(index, requests.len(), top_inset + 8.0, 0.0, 80.0)
                    .child(row)
                    .child(div().pl(px(78.0)).w_full().child(div().w_full().h(px(0.5)).bg(border)))
                    .into_any_element()
            })
        })
        .is_empty(is_empty)
        .no_data_text(tr("music_moderation_empty"))
        .need_pending(is_empty)
        .sai_data_render_loading(|_, _| RequestSkeletonBody::new(8.0, MusicLibraryListSkeleton::new().rows(8)));

        ListShellUi::new()
            .title(SharedString::from(tr("music_admin_album_requests")))
            .on_back(move |_, cx| pop_screen(back_store.as_ref(), cx))
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .below_header(
                ModerationStatusFilterBar::new(RequestListState::status_filter_binding(&state), theme.clone(), |status, cx| {
                    MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_moderation_album_requests(status, cx));
                })
                .track_mode(false),
            )
            .content(div().size_full().bg(theme.bg._100).child(body))
            .on_appear("music-album-requests-list-appear", move |_, cx| {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_moderation_album_requests(&appear_status, true, cx));
            })
    }
}
