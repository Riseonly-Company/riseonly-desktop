use std::rc::Rc;

use gpui::{App, Entity, FontWeight, SharedString, Window};
use rise_canon::rise_service_text::music_service;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::main_text::MainText;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_floating_top_inset};
use crate::modules::music::pages::settings::moderation::moderation_status_filter_bar::ModerationStatusFilterBar;
use crate::modules::music::pages::settings::moderation::music_request_shared::{
    MISSING_VALUE, MUSIC_PREFETCH_THRESHOLD, RequestListState, RequestSkeletonBody, music_status_title, pop_screen,
    request_list_row,
};
use crate::modules::music::shared::utils::music::music_models::display_genre;
use crate::modules::music::shared::utils::music::music_pagination_prefetch::MusicPaginationPrefetchModifiers;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::WireArtistRequestDTO;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

#[derive(IntoElement, Default)]
pub struct MusicArtistRequestsList;

impl MusicArtistRequestsList {
    pub fn new() -> Self {
        Self
    }

    fn artist_request_row(req: &WireArtistRequestDTO, theme: &AppTheme) -> Div {
        let shown_name = if req.stage_name.is_empty() {
            req.user.as_ref().map(|user| user.name.clone())
        } else {
            Some(req.stage_name.clone())
        };
        let subtitle = req
            .user
            .as_ref()
            .filter(|user| !user.tag.is_empty())
            .map(|user| format!("@{}", user.tag))
            .unwrap_or_else(|| display_genre(&req.genre));
        hstack(12.0)
            .w_full()
            .p(px(14.0))
            .bg(theme.bg._200)
            .continuous_corners(f32::from(theme.radius._300))
            .child(
                UserLogoUi::new(SharedString::from(format!("music-artist-request-logo-{}", req.id)))
                    .source(req.user.as_ref().map(|user| SharedString::from(user.logo.clone())))
                    .name(shown_name.map(SharedString::from))
                    .user_id(req.user.as_ref().map(|user| SharedString::from(user.id.clone())))
                    .size(44.0),
            )
            .child(
                vstack(4.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        MainText::new(if req.stage_name.is_empty() { MISSING_VALUE.to_owned() } else { req.stage_name.clone() })
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(SecondaryText::new(subtitle).px(12.0)),
            )
            .child(Self::status_badge(
                if req.status.is_empty() { music_service::ARTIST_STATUS_PENDING } else { req.status.as_str() },
                theme,
            ))
    }

    fn status_badge(status: &str, theme: &AppTheme) -> Div {
        div()
            .flex_none()
            .px(px(10.0))
            .py(px(5.0))
            .bg(theme.bg._100)
            .capsule()
            .child(MainText::new(music_status_title(status)).px(11.0).color(theme.text.secondary))
    }

    fn open_request(nav_store: Option<&Entity<NavigationStore>>, request_id: String, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            let route = RootRoute::MusicArtistRequestDetail { request_id };
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }
}

impl RenderOnce for MusicArtistRequestsList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state("music-artist-requests-list-state", cx, |_, cx| RequestListState::new(cx));
        let status_filter = state.read(cx).status_filter.clone();
        let slice = MusicActionsStore::slices(cx).list_artist_requests_sai.clone();
        let requests: Rc<Vec<WireArtistRequestDTO>> = Rc::new(slice.data().map(|data| data.requests).unwrap_or_default());
        let ids: Vec<SharedString> = requests.iter().map(|req| SharedString::from(req.id.clone())).collect();
        state.update(cx, |state, _| state.list_model.sync(ids));

        let appear_status = status_filter.clone();
        let back_store = nav_store.clone();
        let is_empty = requests.is_empty();
        let list_state = state.clone();
        let row_theme = theme.clone();
        let cache_key = MusicServicesStore::artist_requests_cache_key(&status_filter);
        let body = SaiDataRender::slice(&*slice, move |_, cx| {
            let top_inset = list_shell_floating_top_inset(cx);
            list_state.read(cx).list_model.list().element(move |index, _, cx| {
                let Some(req) = requests.get(index).filter(|req| !req.id.is_empty()) else {
                    return div().into_any_element();
                };
                let request_id = req.id.clone();
                let row_store = nav_store.clone();
                let status_filter = status_filter.clone();
                let row = plain(SharedString::from(format!("music-artist-request-{}", req.id)))
                    .w_full()
                    .on_press(move |_, cx| Self::open_request(row_store.as_ref(), request_id.clone(), cx))
                    .child(Self::artist_request_row(req, &row_theme))
                    .music_pagination_prefetch(
                        SharedString::from(format!("music-artist-request-prefetch-{}", req.id)),
                        index,
                        requests.len(),
                        &cache_key,
                        true,
                        MUSIC_PREFETCH_THRESHOLD,
                        move |_, cx| {
                            MusicInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.load_more_moderation_artist_requests(&status_filter, cx));
                        },
                        cx,
                    );
                request_list_row(index, requests.len(), top_inset + 12.0, 10.0, 80.0)
                    .px(px(16.0))
                    .child(row)
                    .into_any_element()
            })
        })
        .is_empty(is_empty)
        .no_data_text(tr("music_moderation_empty"))
        .need_pending(is_empty)
        .sai_data_render_loading(|_, _| RequestSkeletonBody::new(12.0, UserListSkeleton::new().rows(10).shows_separators(false)));

        ListShellUi::new()
            .title(SharedString::from(tr("music_admin_artist_requests")))
            .on_back(move |_, cx| pop_screen(back_store.as_ref(), cx))
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .below_header(ModerationStatusFilterBar::new(
                RequestListState::status_filter_binding(&state),
                theme.clone(),
                |status, cx| {
                    MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_moderation_artist_requests(status, cx));
                },
            ))
            .content(div().size_full().bg(theme.bg._100).child(body))
            .on_appear("music-artist-requests-list-appear", move |_, cx| {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_moderation_artist_requests(&appear_status, true, cx));
            })
    }
}
