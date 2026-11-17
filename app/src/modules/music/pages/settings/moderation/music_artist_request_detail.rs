use gpui::{App, FontWeight, SharedString, Window};
use rise_canon::rise_proto::music::ProtoArtistRequestRecord;
use rise_canon::rise_service_text::music_service;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::wrappers::list_shell_ui::SettingsWrapperUi;
use crate::modules::music::pages::settings::moderation::music_request_shared::{
    MISSING_VALUE, RequestDetailState, RequestScrollBody, header_card, labeled_card, moderation_actions,
    music_status_title, pop_screen, reject_sheet,
};
use crate::modules::music::shared::utils::music::music_models::display_genre;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;

fn value_or_missing(value: String) -> String {
    if value.is_empty() { MISSING_VALUE.to_owned() } else { value }
}

#[derive(IntoElement)]
pub struct MusicArtistRequestDetail {
    request_id: String,
}

impl MusicArtistRequestDetail {
    pub fn new(request_id: impl Into<String>) -> Self {
        Self { request_id: request_id.into() }
    }

    fn req(request_id: &str, cx: &mut App) -> Option<ProtoArtistRequestRecord> {
        let actions = MusicActionsStore::shared(cx);
        let store = actions.read(cx);
        if let Some(request) = store.get_artist_request_sai.data().and_then(|data| data.request)
            && request.id == request_id
        {
            return Some(request);
        }
        store.list_artist_requests_sai.data()?.requests.into_iter().find(|request| request.id == request_id)
    }

    fn header_card(r: &ProtoArtistRequestRecord, theme: &AppTheme) -> Div {
        let shown_name = if r.stage_name.is_empty() {
            r.user.as_ref().map(|user| user.name.clone())
        } else {
            Some(r.stage_name.clone())
        };
        let tag = r.user.as_ref().map(|user| user.tag.clone()).filter(|tag| !tag.is_empty());
        let status = if r.status.is_empty() { music_service::ARTIST_STATUS_PENDING } else { r.status.as_str() };
        header_card(theme)
            .child(
                UserLogoUi::new(SharedString::from(format!("music-artist-request-detail-logo-{}", r.id)))
                    .source(r.user.as_ref().map(|user| SharedString::from(user.logo.clone())))
                    .name(shown_name.map(SharedString::from))
                    .user_id(r.user.as_ref().map(|user| SharedString::from(user.id.clone())))
                    .size(56.0),
            )
            .child(MainText::new(value_or_missing(r.stage_name.clone())).px(18.0).font_weight(FontWeight::SEMIBOLD))
            .when_some(tag, |card, tag| card.child(SecondaryText::new(format!("@{tag}")).px(13.0)))
            .child(MainText::new(music_status_title(status)).px(13.0).color(theme.primary._100))
    }
}

impl RenderOnce for MusicArtistRequestDetail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = format!("music-artist-request-detail-{}", self.request_id);
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(SharedString::from(format!("{id}-state")), cx, |_, cx| RequestDetailState::new(cx));
        let review_slice = MusicActionsStore::slices(cx).review_artist_request_sai.clone();
        let is_review_pending = review_slice.is_pending();

        let content = match Self::req(&self.request_id, cx) {
            Some(r) => {
                let genre = if r.genre.is_empty() { MISSING_VALUE.to_owned() } else { display_genre(&r.genre) };
                let approve_id = self.request_id.clone();
                vstack(14.0)
                    .w_full()
                    .px(px(16.0))
                    .pt(px(16.0))
                    .child(Self::header_card(&r, &theme))
                    .child(labeled_card(tr("music_artist_genre"), genre, &theme))
                    .child(labeled_card(tr("music_artist_bio"), value_or_missing(r.bio.clone()), &theme))
                    .when_some(r.instagram.clone().filter(|value| !value.is_empty()), |column, value| {
                        column.child(labeled_card(tr("music_artist_instagram"), value, &theme))
                    })
                    .when_some(r.vk.clone().filter(|value| !value.is_empty()), |column, value| {
                        column.child(labeled_card(tr("music_artist_vk"), value, &theme))
                    })
                    .when_some(r.rejection_reason.clone().filter(|value| !value.is_empty()), |column, value| {
                        column.child(labeled_card(tr("music_rejection_reason"), value, &theme))
                    })
                    .when(r.status == music_service::ARTIST_STATUS_PENDING, |column| {
                        column.child(moderation_actions(
                            &id,
                            is_review_pending,
                            &state,
                            nav_store.clone(),
                            move |cx| {
                                MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                                    store.review_moderation_artist_request(&approve_id, true, None, cx);
                                });
                            },
                            &theme,
                        ))
                    })
                    .child(div().h(px(80.0)))
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
                    store.review_moderation_artist_request(&reject_id, false, Some(reason), cx);
                });
            },
            &theme,
            cx,
        );
        let appear_id = self.request_id;

        div().size_full().child(
            SettingsWrapperUi::new(tr("music_artist_request_detail"), move |_, cx| pop_screen(nav_store.as_ref(), cx)).content(
                div().size_full().bg(theme.bg._100).child(RequestScrollBody::new(SharedString::from(format!("{id}-scroll")), content)),
            ),
        )
        .child(sheet)
        .on_appear(SharedString::from(format!("{id}-appear")), move |_, cx| {
            MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.open_moderation_artist_request(&appear_id, cx));
        })
    }
}
