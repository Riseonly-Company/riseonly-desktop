use std::rc::Rc;

use gpui::{App, IntoElement, RenderOnce, SharedString, Window, div, px};
use rise_ui::IconUi;
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::swiftui::prelude::*;

use super::music_mini_player_bottom_padding::observed_music_player;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;

#[derive(IntoElement)]
pub struct MusicScopePlaybackButton {
    scope_id: SharedString,
    tracks: Vec<TrackModel>,
    size: f32,
    on_play_started: Option<Rc<dyn Fn(&mut App)>>,
}

impl MusicScopePlaybackButton {
    pub fn new(scope_id: impl Into<SharedString>, tracks: Vec<TrackModel>) -> Self {
        Self {
            scope_id: scope_id.into(),
            tracks,
            size: PageHeaderMetrics::TOOLBAR_BUTTON_SIZE,
            on_play_started: None,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn on_play_started(mut self, on_play_started: impl Fn(&mut App) + 'static) -> Self {
        self.on_play_started = Some(Rc::new(on_play_started));
        self
    }

    fn is_active_in_scope(&self, player: &MusicPlayerState) -> bool {
        if player.playback_scope_id() != Some(&*self.scope_id) {
            return false;
        }
        let Some(current) = player.current_track.as_ref() else {
            return false;
        };
        self.tracks.iter().any(|track| track.id == current.id)
    }
}

impl RenderOnce for MusicScopePlaybackButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = SharedString::from(format!("music-scope-playback-{}", self.scope_id));
        let player = observed_music_player(SharedString::from(format!("{id}-player")), window, cx);
        let (is_active_in_scope, is_playing) = {
            let state = player.read(cx);
            (self.is_active_in_scope(state), state.is_playing)
        };
        let icon = if is_active_in_scope && is_playing { "pause.fill" } else { "play.fill" };
        let primary = rise_ui::theme(cx).primary._100;
        let on_primary = rise_ui::theme(cx).content_on(primary);
        let disabled = self.tracks.is_empty();
        let size = self.size;
        let scope_id = self.scope_id;
        let tracks = Rc::new(self.tracks);
        let on_play_started = self.on_play_started;

        plain(id)
            .disabled(disabled)
            .opacity(if disabled { 0.5 } else { 1.0 })
            .on_press(move |_, cx| {
                if is_active_in_scope {
                    player.update(cx, |player, cx| player.toggle_play_pause(cx));
                } else {
                    let tracks = (*tracks).clone();
                    let scope_id = scope_id.clone();
                    let on_playback_accepted = on_play_started.clone().map(|on_play_started| {
                        Box::new(move |cx: &mut App| on_play_started(cx)) as Box<dyn FnOnce(&mut App)>
                    });
                    MusicInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.play_collection(tracks, Some(&*scope_id), on_playback_accepted, cx);
                    });
                }
            })
            .child(
                zstack(Alignment::Center)
                    .relative()
                    .size(px(size))
                    .child(div().absolute().inset_0().circle().bg(primary))
                    .children(IconUi::sized(icon, px((size * 0.38).max(14.0)), on_primary, true)),
            )
    }
}
