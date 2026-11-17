use gpui::{App, FontWeight, SharedString, Subscription, Window};
use rise_canon::rise_service_text::music_service;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::widgets::wrappers::list_shell_ui::SettingsWrapperUi;
use crate::modules::music::pages::settings::moderation::music_request_shared::{
    MISSING_VALUE, RequestScrollBody, labeled_card, music_status_title, pop_screen,
};
use crate::modules::music::shared::utils::music::music_models::display_genre;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;

struct MyMusicArtistRequestState {
    _subscriptions: [Subscription; 2],
}

impl MyMusicArtistRequestState {
    fn new(cx: &mut Context<Self>) -> Self {
        let actions = MusicActionsStore::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            _subscriptions: [
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }
}

fn value_or_missing(value: &str) -> String {
    if value.is_empty() { MISSING_VALUE.to_owned() } else { value.to_owned() }
}

#[derive(IntoElement, Default)]
pub struct MyMusicArtistRequest;

impl MyMusicArtistRequest {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for MyMusicArtistRequest {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        window.use_keyed_state("my-music-artist-request-state", cx, |_, cx| MyMusicArtistRequestState::new(cx));
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let slice = MusicActionsStore::slices(cx).get_my_artist_request_sai.clone();
        let request = slice.data().and_then(|data| data.request);
        let is_empty = request.is_none();
        let body_theme = theme.clone();

        let body = SaiDataRender::slice(&*slice, move |_, _| {
            let theme = body_theme;
            let Some(r) = request else {
                return div().into_any_element();
            };
            let status = if r.status.is_empty() { music_service::ARTIST_STATUS_NONE } else { r.status.as_str() };
            let genre = if r.genre.is_empty() { MISSING_VALUE.to_owned() } else { display_genre(&r.genre) };
            let reason = r.rejection_reason.clone().filter(|reason| !reason.is_empty());
            RequestScrollBody::new(
                SharedString::new_static("my-music-artist-request-scroll"),
                vstack(14.0)
                    .w_full()
                    .px(px(16.0))
                    .pt(px(16.0))
                    .child(
                        MainText::new(music_status_title(status))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    )
                    .child(labeled_card(tr("music_artist_stage_name"), value_or_missing(&r.stage_name), &theme))
                    .child(labeled_card(tr("music_artist_genre"), genre, &theme))
                    .child(labeled_card(tr("music_artist_bio"), value_or_missing(&r.bio), &theme))
                    .when_some(reason, |column, reason| {
                        column.child(labeled_card(tr("music_rejection_reason"), reason, &theme))
                    })
                    .child(div().h(px(80.0))),
            )
            .into_any_element()
        })
        .is_empty(is_empty)
        .no_data_text(tr("music_no_artist_request"))
        .need_pending(is_empty);

        SettingsWrapperUi::new(tr("music_my_artist_request"), move |_, cx| pop_screen(nav_store.as_ref(), cx))
            .content(div().size_full().bg(theme.bg._100).child(body))
            .on_appear("my-music-artist-request-appear", |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.load_my_artist_request_if_needed(true, cx));
            })
    }
}
