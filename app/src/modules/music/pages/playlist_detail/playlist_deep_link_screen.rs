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
use crate::modules::music::stores::music_actions::music_types::MusicPlaylistTrackLinkError;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicPlaylist;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;

#[derive(Clone, Copy, PartialEq)]
enum ViewState {
    Loading,
    Failed(MusicPlaylistTrackLinkError),
}

struct PlaylistDeepLinkScreenState {
    state: ViewState,
}

impl PlaylistDeepLinkScreenState {
    fn set_state(&mut self, state: ViewState, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct PlaylistDeepLinkScreen {
    id: ElementId,
    playlist_id: String,
}

impl PlaylistDeepLinkScreen {
    pub fn new(playlist_id: impl Into<String>) -> Self {
        let playlist_id = playlist_id.into();
        Self { id: ElementId::Name(SharedString::from(format!("playlist-deep-link-{playlist_id}"))), playlist_id }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

#[derive(Clone)]
struct PlaylistDeepLinkResolver {
    playlist_id: String,
    nav_store: Option<Entity<NavigationStore>>,
    state: WeakEntity<PlaylistDeepLinkScreenState>,
}

impl PlaylistDeepLinkResolver {
    fn resolve(&self, cx: &mut App) {
        let _ = self.state.update(cx, |state, cx| state.set_state(ViewState::Loading, cx));
        let resolver = self.clone();
        let completion: Rc<dyn Fn(Result<RiseStoredMusicPlaylist, MusicPlaylistTrackLinkError>, &mut App)> =
            Rc::new(move |result: Result<RiseStoredMusicPlaylist, MusicPlaylistTrackLinkError>, cx: &mut App| match result {
                Ok(_) => resolver.open_playlist(cx),
                Err(error) => {
                    let _ = resolver.state.update(cx, |state, cx| state.set_state(ViewState::Failed(error), cx));
                }
            });
        let playlist_id = self.playlist_id.clone();
        MusicInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.resolve_playlist_link(&playlist_id, completion, cx));
    }

    fn open_playlist(&self, cx: &mut App) {
        let Some(nav_store) = self.nav_store.as_ref() else {
            return;
        };
        let source_route = RootRoute::MusicPlaylistLink { playlist_id: self.playlist_id.clone() };
        if nav_store.read(cx).path.last() != Some(&source_route) {
            return;
        }
        let collection_id = self.playlist_id.clone();
        nav_store.update(cx, |nav_store, cx| {
            nav_store.replace_top(RootRoute::MusicPlaylistDetail { collection_id }, cx);
        });
    }
}

impl RenderOnce for PlaylistDeepLinkScreen {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| PlaylistDeepLinkScreenState {
            state: ViewState::Loading,
        });
        let resolver = PlaylistDeepLinkResolver {
            playlist_id: self.playlist_id.clone(),
            nav_store: nav_store.clone(),
            state: state.downgrade(),
        };

        let task_resolver = resolver.clone();
        use_task(self.child_id("task"), self.playlist_id.clone(), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| task_resolver.resolve(cx));
        });

        let theme = rise_ui::theme(cx).clone();
        let body = match state.read(cx).state {
            ViewState::Loading => PlaylistCollaborationLoadingState::new().into_any_element(),
            ViewState::Failed(error) => {
                let retry: Option<ButtonAction> = error.is_retryable().then(|| {
                    let resolver = resolver.clone();
                    Rc::new(move |_: &mut Window, cx: &mut App| resolver.resolve(cx)) as ButtonAction
                });
                PlaylistCollaborationErrorState::new(self.child_id("error"), tr(error.localization_key()), retry)
                    .into_any_element()
            }
        };

        SettingsWrapperUi::new(tr("music_playlist_link_title"), move |_, cx| {
            if let Some(nav_store) = nav_store.as_ref() {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(div().size_full().bg(theme.bg._100).child(body))
    }
}
