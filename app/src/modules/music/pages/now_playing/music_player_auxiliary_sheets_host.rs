use std::sync::Arc;

use gpui::{App, Context, ElementId, Empty, Render, SharedString, Window};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::music::pages::playlist_detail::music_playlist_listeners_screen::MusicPlaylistListenersScreen;
use crate::modules::music::widgets::bottomsheets::now_playing::music_audio_effects_sheet::MusicAudioEffectsSheet;
use crate::modules::music::widgets::music_mini_player_bottom_padding::observed_music_player;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

#[derive(IntoElement)]
pub struct MusicPlayerAuxiliarySheetsHost {
    id: ElementId,
}

impl MusicPlayerAuxiliarySheetsHost {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for MusicPlayerAuxiliarySheetsHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = observed_music_player(child_id(&self.id, "player"), window, cx);
        let opens_session_after_listeners =
            window.use_keyed_state(child_id(&self.id, "opens-session-after-listeners"), cx, |_, _| false);
        let background = rise_ui::theme(cx).bg._100;
        let (listening_playlist_id, is_music_playback, track_id) = {
            let player = player.read(cx);
            (
                player.listening_playlist_id().map(str::to_owned),
                player.is_music_playback(),
                player.current_track.as_ref().map(|track| track.id.clone()),
            )
        };

        let reset_flag = opens_session_after_listeners.clone();
        let reset_player = player.clone();
        on_change(
            child_id(&self.id, "listening-playlist"),
            listening_playlist_id.clone(),
            false,
            window,
            cx,
            move |_, _, _, cx| {
                reset_flag.update(cx, |flag, cx| {
                    *flag = false;
                    cx.notify();
                });
                reset_player.update(cx, |player, cx| {
                    player.show_playlist_listeners_sheet = false;
                    cx.notify();
                });
            },
        );

        let effects_player = player.clone();
        on_change(
            child_id(&self.id, "is-music-playback"),
            is_music_playback,
            false,
            window,
            cx,
            move |_, is_music, _, cx| {
                if !*is_music {
                    effects_player.update(cx, |player, cx| player.set_show_audio_effects_sheet(false, cx));
                }
            },
        );

        let content_id = child_id(&self.id, "effects-content");
        let closed_player = player.clone();
        let refused_player = player.clone();
        let effects_sheet = BottomSheetUi::new(
            child_id(&self.id, "effects"),
            Binding::entity(player.clone(), |player| player.show_audio_effects_sheet(), |player, value, cx| {
                player.set_show_audio_effects_sheet(value, cx);
            }),
            move |_, _| {
                let closed_player = closed_player.clone();
                MusicAudioEffectsSheet::new(content_id.clone(), move |_, cx| {
                    closed_player.update(cx, |player, cx| player.set_show_audio_effects_sheet(false, cx));
                })
            },
            |_, _| Empty,
        )
        .snap_fractions(vec![0.93])
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .sheet_background(Some(background))
        .footer_background(Some(background))
        .on_presentation_refused(move |_, cx| {
            refused_player.update(cx, |player, cx| player.set_show_audio_effects_sheet(false, cx));
        });

        let listeners = listening_playlist_id.map(|playlist_id| {
            let session_player = player.clone();
            let session_flag = opens_session_after_listeners.clone();
            let dismissed_player = player.clone();
            let dismissed_flag = opens_session_after_listeners.clone();
            MusicPlaylistListenersScreen::new(
                child_id(&self.id, "listeners"),
                Binding::entity(player.clone(), |player| player.show_playlist_listeners_sheet, |player, value, _| {
                    player.show_playlist_listeners_sheet = value;
                }),
                playlist_id,
            )
            .track_id(track_id)
            .owns_observation(false)
            .on_open_session(move |cx: &mut App| {
                session_player.update(cx, |player, cx| {
                    if player.show_playlist_listeners_sheet {
                        session_flag.update(cx, |flag, cx| {
                            *flag = true;
                            cx.notify();
                        });
                        player.show_playlist_listeners_sheet = false;
                    } else {
                        player.show_listening_session_sheet = true;
                    }
                    cx.notify();
                });
            })
            .on_did_dismiss(move |_, cx| {
                if !*dismissed_flag.read(cx) {
                    return;
                }
                dismissed_flag.update(cx, |flag, cx| {
                    *flag = false;
                    cx.notify();
                });
                dismissed_player.update(cx, |player, cx| {
                    if player.is_music_playback() && player.listening_session().is_some() {
                        player.show_listening_session_sheet = true;
                        cx.notify();
                    }
                });
            })
        });

        div().absolute().inset_0().child(effects_sheet).children(listeners)
    }
}

/// desktop only, root draws this layer as its own cached view
/// host observes the player through keyed state which notifies the view drawing it
/// drawn by the root every position tick laid out the whole window again
pub struct MusicPlayerAuxiliarySheetsLayer;

impl Render for MusicPlayerAuxiliarySheetsLayer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(MusicPlayerAuxiliarySheetsHost::new("root-music-player-auxiliary-sheets"))
    }
}
