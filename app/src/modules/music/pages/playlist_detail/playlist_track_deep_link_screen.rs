use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, SharedString, WeakEntity, Window};
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RootRoute;
use crate::core::widgets::wrappers::list_shell_ui::SettingsWrapperUi;
use crate::modules::music::pages::playlist_collaboration::playlist_collaboration_components::{
    PlaylistCollaborationErrorState, PlaylistCollaborationLoadingState,
};
use crate::modules::music::pages::playlist_detail::track_deep_link_screen::MusicTrackDeepLinkReadyState;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_types::{
    MusicPlaylistTrackLinkError, MusicResolvedPlaylistTrackLink,
};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;

#[derive(Clone)]
enum ViewState {
    Loading,
    Ready(TrackModel),
    Failed(MusicPlaylistTrackLinkError),
}

struct PlaylistTrackDeepLinkScreenState {
    state: ViewState,
}

impl PlaylistTrackDeepLinkScreenState {
    fn set_state(&mut self, state: ViewState, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct PlaylistTrackDeepLinkScreen {
    id: ElementId,
    playlist_id: String,
    track_id: String,
}

impl PlaylistTrackDeepLinkScreen {
    pub fn new(playlist_id: impl Into<String>, track_id: impl Into<String>) -> Self {
        let playlist_id = playlist_id.into();
        let track_id = track_id.into();
        Self {
            id: ElementId::Name(SharedString::from(format!("playlist-track-deep-link-{playlist_id}-{track_id}"))),
            playlist_id,
            track_id,
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

fn show_now_playing(cx: &mut App) {
    MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_show_now_playing(true, cx));
}

fn play_direct(track: &TrackModel, cx: &mut App) {
    let track = track.clone();
    let scope_id = format!("track:{}", track.id);
    let queue = vec![track.clone()];
    MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
        store.play(track, queue, Some(scope_id.as_str()), true, Some(Box::new(show_now_playing)), cx);
    });
}

#[derive(Clone)]
struct PlaylistTrackDeepLinkResolver {
    playlist_id: String,
    track_id: String,
    nav_store: Option<Entity<NavigationStore>>,
    state: WeakEntity<PlaylistTrackDeepLinkScreenState>,
}

impl PlaylistTrackDeepLinkResolver {
    fn set_state(&self, state: ViewState, cx: &mut App) {
        let _ = self.state.update(cx, |screen, cx| screen.set_state(state, cx));
    }

    fn resolve(&self, cx: &mut App) {
        self.set_state(ViewState::Loading, cx);
        let resolver = self.clone();
        let completion: Rc<dyn Fn(Result<MusicResolvedPlaylistTrackLink, MusicPlaylistTrackLinkError>, &mut App)> =
            Rc::new(move |result: Result<MusicResolvedPlaylistTrackLink, MusicPlaylistTrackLinkError>, cx: &mut App| match result {
                Ok(resolved) => resolver.open(resolved, cx),
                Err(error) => resolver.set_state(ViewState::Failed(error), cx),
            });
        let playlist_id = self.playlist_id.clone();
        let track_id = self.track_id.clone();
        MusicInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.resolve_playlist_track_link(&playlist_id, &track_id, completion, cx));
    }

    fn open(&self, resolved: MusicResolvedPlaylistTrackLink, cx: &mut App) {
        let source_route =
            RootRoute::MusicPlaylistTrackLink { playlist_id: self.playlist_id.clone(), track_id: self.track_id.clone() };
        let is_top = self
            .nav_store
            .as_ref()
            .is_some_and(|nav_store| nav_store.read(cx).path.last() == Some(&source_route));
        let track = resolved.track.as_track_model().filter(|track| !track.stable_audio_url().is_empty());
        let (true, Some(track)) = (is_top, track) else {
            self.set_state(ViewState::Failed(MusicPlaylistTrackLinkError::TrackNotFound), cx);
            return;
        };

        let Some(playlist) = resolved.playlist else {
            self.set_state(ViewState::Ready(track.clone()), cx);
            play_direct(&track, cx);
            return;
        };

        let queue: Vec<TrackModel> =
            playlist.tracks.unwrap_or_default().iter().filter_map(|track| track.as_track_model()).collect();
        if !queue.iter().any(|candidate| candidate.id == track.id) {
            self.set_state(ViewState::Failed(MusicPlaylistTrackLinkError::TrackNotFound), cx);
            return;
        }

        if let Some(nav_store) = self.nav_store.as_ref() {
            let collection_id = self.playlist_id.clone();
            nav_store.update(cx, |nav_store, cx| {
                nav_store.replace_top(RootRoute::MusicPlaylistDetail { collection_id }, cx);
            });
        }
        let scope_id = format!("playlist:{}", self.playlist_id);
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.play(track, queue, Some(scope_id.as_str()), true, Some(Box::new(show_now_playing)), cx);
        });
    }
}

impl RenderOnce for PlaylistTrackDeepLinkScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| PlaylistTrackDeepLinkScreenState {
            state: ViewState::Loading,
        });
        let resolver = PlaylistTrackDeepLinkResolver {
            playlist_id: self.playlist_id.clone(),
            track_id: self.track_id.clone(),
            nav_store: nav_store.clone(),
            state: state.downgrade(),
        };

        let task_resolver = resolver.clone();
        let task_id = format!("{}:{}", self.playlist_id, self.track_id);
        use_task(self.child_id("task"), task_id, window, cx, async move |cx| {
            let _ = cx.update(|_, cx| task_resolver.resolve(cx));
        });

        let theme = rise_ui::theme(cx).clone();
        let body = match state.read(cx).state.clone() {
            ViewState::Loading => PlaylistCollaborationLoadingState::new().into_any_element(),
            ViewState::Ready(track) => {
                let played = track.clone();
                MusicTrackDeepLinkReadyState::new(self.child_id("ready"), track, move |_, cx| play_direct(&played, cx))
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
