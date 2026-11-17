use gpui::{
    App, Bounds, ContentMask, Corners, ElementId, Entity, Hsla, IntoElement, RenderOnce, SharedString, Subscription, Window, canvas, div,
    fill, point, px, size, white,
};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_ui::frame_request::request_frame_if_on_screen;
use rise_ui::swiftui::environment::layout_direction;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::slider::slider;
use rise_ui::swiftui::transition::UnitPoint;

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

struct MusicPlaybackSeekBarState {
    is_scrubbing: bool,
    scrubbed_progress: f64,
    scrub_start_progress: f64,
    did_move_scrubber: bool,
    _player_observation: Subscription,
    _position_observation: Subscription,
}

impl MusicPlaybackSeekBarState {
    fn set_scrub_value(&mut self, value: f64, player_progress: f64) {
        let clamped = value.clamp(0.0, 1.0);
        if !self.is_scrubbing {
            self.scrub_start_progress = player_progress;
            self.is_scrubbing = true;
        }
        if (clamped - self.scrub_start_progress).abs() > 0.001 {
            self.did_move_scrubber = true;
        }
        self.scrubbed_progress = clamped;
    }
}

#[derive(IntoElement)]
pub struct MusicPlaybackSeekBar {
    id: ElementId,
    player: Entity<MusicPlayerState>,
    track_duration_seconds: i64,
    tint: Hsla,
    time_color: Hsla,
    is_current: bool,
}

impl MusicPlaybackSeekBar {
    pub fn new(id: impl Into<ElementId>, player: Entity<MusicPlayerState>, track_duration_seconds: i64) -> Self {
        Self {
            id: id.into(),
            player,
            track_duration_seconds,
            tint: white(),
            time_color: white().opacity(0.6),
            is_current: true,
        }
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = tint;
        self
    }

    pub fn time_color(mut self, time_color: Hsla) -> Self {
        self.time_color = time_color;
        self
    }

    pub fn is_current(mut self, is_current: bool) -> Self {
        self.is_current = is_current;
        self
    }

    fn display_duration(&self, player: &MusicPlayerState) -> TimeInterval {
        if !self.is_current || player.is_preparing_playback {
            return if self.track_duration_seconds > 0 { self.track_duration_seconds as f64 } else { 0.0 };
        }
        let declared = self.track_duration_seconds.max(0) as f64;
        if player.duration > 0.0 && (declared <= 0.0 || (player.duration - declared).abs() >= 2.0) {
            return player.duration;
        }
        declared
    }

    fn display_current_time(&self, player: &MusicPlayerState, state: &MusicPlaybackSeekBarState) -> TimeInterval {
        if !self.is_current || player.is_preparing_playback {
            return 0.0;
        }
        if state.is_scrubbing { state.scrubbed_progress * self.display_duration(player) } else { player.current_time }
    }

    fn format_time(t: TimeInterval) -> String {
        RiseFormat::duration_time_interval(t)
    }

    fn handle_scrubbing_changed(
        state: &Entity<MusicPlaybackSeekBarState>,
        player: &Entity<MusicPlayerState>,
        is_current: bool,
        editing: bool,
        cx: &mut App,
    ) {
        let progress = player.read(cx).progress;
        if editing {
            state.update(cx, |state, cx| {
                if !state.is_scrubbing {
                    state.scrub_start_progress = progress;
                    state.scrubbed_progress = progress;
                    state.did_move_scrubber = false;
                    state.is_scrubbing = true;
                    cx.notify();
                }
            });
            return;
        }

        let target = state.update(cx, |state, cx| {
            if !state.is_scrubbing {
                return None;
            }
            let target = state.scrubbed_progress.clamp(0.0, 1.0);
            let did_move = state.did_move_scrubber && (target - state.scrub_start_progress).abs() > 0.001;
            state.is_scrubbing = false;
            state.did_move_scrubber = false;
            cx.notify();
            (is_current && did_move).then_some(target)
        });
        if let Some(target) = target {
            player.update(cx, |player, cx| player.seek(target, cx));
        }
    }
}

impl RenderOnce for MusicPlaybackSeekBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        let player = self.player.clone();
        let observed = player.clone();
        let initial_progress = player.read(cx).progress;
        let publication = player.read(cx).position_publication().clone();
        let state = window.use_keyed_state(id.clone(), cx, move |_, cx| MusicPlaybackSeekBarState {
            is_scrubbing: false,
            scrubbed_progress: initial_progress,
            scrub_start_progress: initial_progress,
            did_move_scrubber: false,
            _player_observation: cx.observe(&observed, |_, _, cx| cx.notify()),
            _position_observation: cx.observe(&publication, |_, _, cx| cx.notify()),
        });

        let changed_state = state.clone();
        let changed_player = player.clone();
        on_change(
            SharedString::from(format!("{id}-track")),
            player.read(cx).current_track.as_ref().map(|track| track.id.clone()),
            false,
            window,
            cx,
            move |_, _, _, cx| {
                let progress = changed_player.read(cx).progress;
                changed_state.update(cx, |state, cx| {
                    state.is_scrubbing = false;
                    state.scrubbed_progress = progress;
                    state.scrub_start_progress = progress;
                    state.did_move_scrubber = false;
                    cx.notify();
                });
            },
        );

        let is_current = self.is_current;
        let is_preparing_playback = player.read(cx).is_preparing_playback;
        let is_disabled = !is_current || is_preparing_playback;

        let (current_label, duration_label) = {
            let player_state = player.read(cx);
            let bar_state = state.read(cx);
            (
                Self::format_time(self.display_current_time(player_state, bar_state)),
                Self::format_time(self.display_duration(player_state)),
            )
        };

        let value_state = state.clone();
        let value_player = player.clone();
        let write_state = state.clone();
        let write_player = player.clone();
        let value = Binding::new(
            move |cx| {
                let player = value_player.read(cx);
                if !is_current || player.is_preparing_playback {
                    return 0.0;
                }
                let state = value_state.read(cx);
                if state.is_scrubbing { state.scrubbed_progress } else { player.progress }
            },
            move |value, _, cx| {
                if is_disabled {
                    return;
                }
                let progress = write_player.read(cx).progress;
                write_state.update(cx, |state, cx| {
                    state.set_scrub_value(value, progress);
                    cx.notify();
                });
            },
        );

        let editing_state = state;
        let editing_player = player;
        let tint = self.tint;

        vstack(6.0)
            .w_full()
            .px(px(32.0))
            .child(
                zstack(Alignment::Center)
                    .relative()
                    .w_full()
                    .h(px(31.0))
                    .child(
                        div().w_full().opacity(if is_current && is_preparing_playback { 0.28 } else { 1.0 }).child(
                            slider(SharedString::from(format!("{id}-slider")), value, 0.0..=1.0)
                                .tint(tint)
                                .on_editing_changed(move |editing, _, cx| {
                                    if !is_disabled {
                                        Self::handle_scrubbing_changed(
                                            &editing_state,
                                            &editing_player,
                                            is_current,
                                            editing,
                                            cx,
                                        );
                                    }
                                }),
                        ),
                    )
                    .when(is_disabled, |stack| {
                        stack.child(div().id(SharedString::from(format!("{id}-disabled"))).absolute().inset_0().occlude())
                    })
                    .when(is_current && is_preparing_playback, |stack| {
                        stack.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .child(MusicSeekBarShimmer { id: SharedString::from(format!("{id}-shimmer")), color: tint }),
                        )
                    }),
            )
            .child(
                hstack(None)
                    .w_full()
                    .swiftui_font(Font::system_size(12.0))
                    .text_color(self.time_color)
                    .child(current_label)
                    .child(spacer())
                    .child(duration_label),
            )
    }
}

struct MusicSeekBarShimmerState {
    started_at: f64,
}

#[derive(IntoElement)]
struct MusicSeekBarShimmer {
    id: SharedString,
    color: Hsla,
}

impl MusicSeekBarShimmer {
    const START_PHASE: f64 = -0.6;
    const END_PHASE: f64 = 1.0;
    const DURATION: f64 = 1.15;

    fn phase(elapsed: f64) -> f32 {
        let cycle = (elapsed / Self::DURATION).rem_euclid(1.0);
        (Self::START_PHASE + (Self::END_PHASE - Self::START_PHASE) * cycle) as f32
    }
}

impl RenderOnce for MusicSeekBarShimmer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id, cx, |_, _| MusicSeekBarShimmerState {
            started_at: RiseClock::monotonic().as_secs_f64(),
        });
        let phase = Self::phase(RiseClock::monotonic().as_secs_f64() - state.read(cx).started_at);

        let color = self.color;
        let gradient = LinearGradient::colors(
            [color.opacity(0.05), color.opacity(0.55), color.opacity(0.05)],
            UnitPoint::LEADING,
            UnitPoint::TRAILING,
        );

        div().w_full().h(px(4.0)).px(px(6.0)).child(
            canvas(
                |bounds, window, cx: &mut App| {
                    request_frame_if_on_screen(bounds, window);
                    layout_direction(cx)
                },
                move |bounds, direction, window, _| {
                    let width = f32::from(bounds.size.width);
                    window.paint_quad(fill(bounds, color.opacity(0.12)).corner_radii(Corners::all(px(2.0))));
                    let band = Bounds::new(
                        point(bounds.origin.x + px(phase * (width + 80.0)), bounds.origin.y),
                        size(px((width * 0.35).max(48.0)), bounds.size.height),
                    );
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        gradient.paint(band, direction, window);
                    });
                },
            )
            .size_full(),
        )
    }
}
