use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, FontWeight, SharedString, WeakEntity, Window, white};
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RootRoute;
use crate::core::widgets::wrappers::list_shell_ui::SettingsWrapperUi;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState,
};
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_types::MusicTrackLinkError;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

#[derive(Clone)]
enum ViewState {
    Loading,
    Ready(TrackModel),
    Failed(MusicTrackLinkError),
}

struct TrackDeepLinkScreenState {
    state: ViewState,
}

impl TrackDeepLinkScreenState {
    fn set_state(&mut self, state: ViewState, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct TrackDeepLinkScreen {
    id: ElementId,
    track_id: String,
}

impl TrackDeepLinkScreen {
    pub fn new(track_id: impl Into<String>) -> Self {
        let track_id = track_id.into();
        Self { id: ElementId::Name(SharedString::from(format!("track-deep-link-{track_id}"))), track_id }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

fn play(track: &TrackModel, cx: &mut App) {
    let track = track.clone();
    let scope_id = format!("track:{}", track.id);
    let queue = vec![track.clone()];
    MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
        store.play(
            track,
            queue,
            Some(scope_id.as_str()),
            true,
            Some(Box::new(|cx: &mut App| {
                MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_show_now_playing(true, cx));
            })),
            cx,
        );
    });
}

#[derive(Clone)]
struct TrackDeepLinkResolver {
    track_id: String,
    nav_store: Option<Entity<NavigationStore>>,
    state: WeakEntity<TrackDeepLinkScreenState>,
}

impl TrackDeepLinkResolver {
    fn set_state(&self, state: ViewState, cx: &mut App) {
        let _ = self.state.update(cx, |screen, cx| screen.set_state(state, cx));
    }

    fn resolve(&self, cx: &mut App) {
        self.set_state(ViewState::Loading, cx);
        let resolver = self.clone();
        let completion: Rc<dyn Fn(Result<RiseStoredMusicTrack, MusicTrackLinkError>, &mut App)> =
            Rc::new(move |result: Result<RiseStoredMusicTrack, MusicTrackLinkError>, cx: &mut App| {
                let source_route = RootRoute::MusicTrackLink { track_id: resolver.track_id.clone() };
                let is_top = resolver
                    .nav_store
                    .as_ref()
                    .is_some_and(|nav_store| nav_store.read(cx).path.last() == Some(&source_route));
                if !is_top {
                    return;
                }
                match result {
                    Ok(wire) => {
                        let Some(track) = wire.as_track_model().filter(|track| !track.stable_audio_url().is_empty())
                        else {
                            resolver.set_state(ViewState::Failed(MusicTrackLinkError::LoadFailed), cx);
                            return;
                        };
                        resolver.set_state(ViewState::Ready(track.clone()), cx);
                        play(&track, cx);
                    }
                    Err(error) => resolver.set_state(ViewState::Failed(error), cx),
                }
            });
        let track_id = self.track_id.clone();
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.resolve_track_link(&track_id, completion, cx));
    }
}

impl RenderOnce for TrackDeepLinkScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window
            .use_keyed_state(self.child_id("state"), cx, |_, _| TrackDeepLinkScreenState { state: ViewState::Loading });
        let resolver = TrackDeepLinkResolver {
            track_id: self.track_id.clone(),
            nav_store: nav_store.clone(),
            state: state.downgrade(),
        };

        let task_resolver = resolver.clone();
        use_task(self.child_id("task"), self.track_id.clone(), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| task_resolver.resolve(cx));
        });

        let theme = rise_ui::theme(cx).clone();
        let body = match state.read(cx).state.clone() {
            ViewState::Loading => PlaylistCollaborationLoadingState::new().into_any_element(),
            ViewState::Ready(track) => {
                let played = track.clone();
                MusicTrackDeepLinkReadyState::new(self.child_id("ready"), track, move |_, cx| play(&played, cx))
                    .into_any_element()
            }
            ViewState::Failed(error) => {
                let retry: Option<ButtonAction> = error.is_retryable().then(|| {
                    let resolver = resolver.clone();
                    Rc::new(move |_: &mut Window, cx: &mut App| resolver.resolve(cx)) as ButtonAction
                });
                PlaylistCollaborationErrorState::new(self.child_id("error"), tr(error.localization_key()), retry)
                    .into_any_element()
            }
        };

        SettingsWrapperUi::new(tr("music_track_link_title"), move |_, cx| {
            if let Some(nav_store) = nav_store.as_ref() {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(div().size_full().bg(theme.bg._100).child(body))
    }
}

#[derive(IntoElement)]
pub struct MusicTrackDeepLinkReadyState {
    id: ElementId,
    track: TrackModel,
    on_play: ButtonAction,
}

impl MusicTrackDeepLinkReadyState {
    pub fn new(
        id: impl Into<ElementId>,
        track: TrackModel,
        on_play: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), track, on_play: Rc::new(on_play) }
    }
}

impl RenderOnce for MusicTrackDeepLinkReadyState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let on_play = self.on_play;
        let mut titles = vstack(5.0).px(px(24.0)).child(
            MainText::new(self.track.title)
                .px(21.0)
                .tac(TextAlignT::Center)
                .font_weight(FontWeight::BOLD)
                .color(theme.text.primary)
                .number_of_lines(2),
        );
        if !self.track.artist.is_empty() {
            titles = titles.child(
                MainText::new(self.track.artist)
                    .px(15.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .number_of_lines(1),
            );
        }

        vstack(18.0)
            .size_full()
            .justify_center()
            .px(px(24.0))
            .py(px(40.0))
            .child(
                MusicCoverImage::new(ElementId::NamedChild(Arc::new(self.id.clone()), "cover".into()), self.track.cover_image_url)
                    .fallback_colors(self.track.cover_colors)
                    .size(180.0_f32)
                    .corner_radius(18.0),
            )
            .child(titles)
            .child(
                div().w_full().max_w(px(220.0)).child(
                    ButtonUi::new(ElementId::NamedChild(Arc::new(self.id), "play".into()))
                        .on_press(move |window, cx| on_play(window, cx))
                        .child(
                            hstack(8.0)
                                .children(IconUi::sized("play.fill", px(15.0), white(), true))
                                .child(
                                    MainText::new(tr("music_play"))
                                        .px(15.0)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .color(white()),
                                ),
                        ),
                ),
            )
    }
}
