use gpui::{App, FontWeight, SharedString, Window};
use rise_canon::rise_service_text::music_service;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::widgets::wrappers::list_shell_ui::SettingsWrapperUi;
use crate::modules::music::pages::settings::moderation::music_request_shared::{
    MISSING_VALUE, RequestDetailState, RequestScrollBody, header_card, labeled_card, moderation_actions,
    music_status_title, pop_screen, reject_sheet,
};
use crate::modules::music::shared::utils::music::music_models::display_genre;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::{WireAlbumRequestDTO, WireAlbumRequestDTOExt};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

fn shown_status(r: &WireAlbumRequestDTO) -> String {
    music_status_title(if r.status.is_empty() { music_service::ALBUM_REQUEST_STATUS_PENDING } else { r.status.as_str() })
}

#[derive(IntoElement)]
pub struct MusicAlbumRequestDetail {
    request_id: String,
}

impl MusicAlbumRequestDetail {
    pub fn new(request_id: impl Into<String>) -> Self {
        Self { request_id: request_id.into() }
    }

    fn req(request_id: &str, cx: &mut App) -> Option<WireAlbumRequestDTO> {
        let actions = MusicActionsStore::shared(cx);
        let store = actions.read(cx);
        if let Some(request) = store.get_album_request_sai.data().and_then(|data| data.request)
            && request.id == request_id
        {
            return Some(request);
        }
        store.list_album_requests_sai.data()?.requests.into_iter().find(|request| request.id == request_id)
    }

    fn is_pending_request(r: &WireAlbumRequestDTO) -> bool {
        r.status.to_lowercase() == music_service::ALBUM_REQUEST_STATUS_PENDING
    }

    fn header_card(r: &WireAlbumRequestDTO, theme: &AppTheme) -> Div {
        header_card(theme)
            .child(
                MusicCoverImage::new(SharedString::from(format!("music-album-request-detail-cover-{}", r.id)), r.cover_image_url())
                    .size(140.0)
                    .corner_radius(12.0),
            )
            .child(
                MainText::new(if r.title.is_empty() { MISSING_VALUE.to_owned() } else { r.title.clone() })
                    .px(18.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(MainText::new(shown_status(r)).px(13.0).color(theme.primary._100))
    }
}

impl RenderOnce for MusicAlbumRequestDetail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = format!("music-album-request-detail-{}", self.request_id);
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(SharedString::from(format!("{id}-state")), cx, |_, cx| RequestDetailState::new(cx));
        let actions = MusicActionsStore::shared(cx);
        let review_slice = actions.read(cx).review_album_request_sai.clone();
        let is_review_pending = review_slice.is_pending();
        let is_get_pending = actions.read(cx).get_album_request_sai.is_pending();

        let content = match Self::req(&self.request_id, cx) {
            Some(r) => {
                let approve_id = self.request_id.clone();
                let artist_name = r.artist.as_ref().map(|artist| artist.stage_name.clone()).filter(|name| !name.is_empty());
                vstack(14.0)
                    .w_full()
                    .px(px(16.0))
                    .pt(px(16.0))
                    .child(Self::header_card(&r, &theme))
                    .when(!r.genre.is_empty(), |column| {
                        column.child(labeled_card(tr("music_artist_genre"), display_genre(&r.genre), &theme))
                    })
                    .when_some(artist_name, |column, name| {
                        column.child(labeled_card(tr("music_artist_stage_name"), name, &theme))
                    })
                    .child(labeled_card(tr("music_status_label"), shown_status(&r), &theme))
                    .when_some(r.rejection_reason.clone().filter(|reason| !reason.is_empty()), |column, reason| {
                        column.child(labeled_card(tr("music_rejection_reason"), reason, &theme))
                    })
                    .when(Self::is_pending_request(&r), |column| {
                        column.child(moderation_actions(
                            &id,
                            is_review_pending,
                            &state,
                            nav_store.clone(),
                            move |cx| {
                                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                                    store.review_moderation_album_request(&approve_id, true, None, cx);
                                });
                            },
                            &theme,
                        ))
                    })
                    .child(div().h(px(80.0)))
            }
            None if is_get_pending => {
                div().w_full().flex().justify_center().pt(px(80.0)).child(progress_view().tint(theme.primary._100))
            }
            None => div().w_full().pt(px(80.0)).child(
                SecondaryText::new(tr("music_request_not_found")).px(13.0).tac(TextAlignT::Center),
            ),
        };

        let reject_id = self.request_id.clone();
        let sheet = reject_sheet(
            &id,
            &state,
            nav_store.clone(),
            move || review_slice.is_pending(),
            move |reason, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.review_moderation_album_request(&reject_id, false, Some(reason), cx);
                });
            },
            &theme,
            cx,
        );
        let appear_id = self.request_id;

        div()
            .size_full()
            .child(
                SettingsWrapperUi::new(tr("music_album_request_detail"), move |_, cx| pop_screen(nav_store.as_ref(), cx))
                    .content(
                        div()
                            .size_full()
                            .bg(theme.bg._100)
                            .child(RequestScrollBody::new(SharedString::from(format!("{id}-scroll")), content)),
                    ),
            )
            .child(sheet)
            .on_appear(SharedString::from(format!("{id}-appear")), move |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.open_moderation_album_request(&appear_id, cx));
            })
    }
}
