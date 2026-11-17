use std::rc::Rc;

use gpui::{App, ElementId, Entity, FontWeight, IntoElement, RenderOnce, Window, black, div, prelude::*, px, white};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::prelude::*;

use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::{MusicPlayOptions, MusicPlayerState};
use crate::modules::user::components::profile_top::profile_featured_tracks_sheet::ProfileFeaturedTracksPlayback;

#[derive(IntoElement)]
pub struct ProfileFeaturedTrackRow {
    id: ElementId,
    track: TrackModel,
    queue: Vec<TrackModel>,
    scope_id: String,
    on_open: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl ProfileFeaturedTrackRow {
    pub fn new(id: impl Into<ElementId>, track: TrackModel) -> Self {
        Self {
            id: id.into(),
            track,
            queue: Vec::new(),
            scope_id: "profile-featured".to_owned(),
            on_open: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn queue(mut self, queue: Vec<TrackModel>) -> Self {
        self.queue = queue;
        self
    }

    pub fn scope_id(mut self, scope_id: impl Into<String>) -> Self {
        self.scope_id = scope_id.into();
        self
    }

    pub fn on_open(mut self, on_open: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open = Rc::new(on_open);
        self
    }

    fn start_playback(
        player: &Entity<MusicPlayerState>,
        track: &TrackModel,
        queue: &[TrackModel],
        scope_id: &str,
        cx: &mut App,
    ) {
        let options = MusicPlayOptions {
            queue: queue.to_vec(),
            scope_id: Some(scope_id.to_owned()),
            record_play_on_success: false,
            ..MusicPlayOptions::default()
        };
        let track = track.clone();
        player.update(cx, |player, cx| player.play(track, options, cx));
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::from((self.id.clone(), name))
    }
}

impl RenderOnce for ProfileFeaturedTrackRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = MusicPlayerState::shared(cx);
        let (is_current, is_playing) = {
            let player = player.read(cx);
            let is_current = player.current_track.as_ref().is_some_and(|current| {
                ProfileFeaturedTracksPlayback::identity(current) == ProfileFeaturedTracksPlayback::identity(&self.track)
            });
            (is_current, player.is_playing)
        };
        let shows_pause = is_current && is_playing;
        let title = if self.track.artist.is_empty() {
            self.track.title.clone()
        } else {
            format!("{} - {}", self.track.artist, self.track.title)
        };
        let play_id = self.child_id("play");
        let open_id = self.child_id("open");
        let on_open = self.on_open;
        let track = self.track;
        let queue = if self.queue.is_empty() { vec![track.clone()] } else { self.queue };
        let scope_id = self.scope_id;
        let open_player = player.clone();
        let open_track = track.clone();
        let open_queue = queue.clone();
        let open_scope_id = scope_id.clone();

        hstack(9.0)
            .w_full()
            .child(
                plain(play_id)
                    .on_press(move |_, cx| {
                        if is_current {
                            player.update(cx, |player, cx| player.toggle_play_pause(cx));
                        } else {
                            Self::start_playback(&player, &track, &queue, &scope_id, cx);
                        }
                    })
                    .py(px(6.0))
                    .child(
                        zstack(Alignment::Center)
                            .size(px(17.0))
                            .rounded_full()
                            .bg(white())
                            .child(
                                div().relative().left(px(if shows_pause { 0.0 } else { 1.0 })).flex().children(
                                    IconUi::sized(
                                        if shows_pause { "pause.fill" } else { "play.fill" },
                                        px(7.0),
                                        black(),
                                        true,
                                    ),
                                ),
                            ),
                    ),
            )
            .child(
                plain(open_id)
                    .on_press(move |window, cx| {
                        if !is_current {
                            Self::start_playback(&open_player, &open_track, &open_queue, &open_scope_id, cx);
                        } else if !is_playing {
                            open_player.update(cx, |player, cx| player.toggle_play_pause(cx));
                        }
                        on_open(window, cx);
                    })
                    .flex_1()
                    .min_w_0()
                    .child(
                        hstack(0.0).w_full().child(
                            MainText::new(title)
                                .px(13.0)
                                .tac(TextAlignT::Left)
                                .font_weight(FontWeight::BOLD)
                                .number_of_lines(1),
                        )
                        .child(spacer_min_w(0.0)),
                    ),
            )
    }
}
