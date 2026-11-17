use gpui::{
    App, AsyncWindowContext, Background, BoxShadow, ElementId, Entity, FocusHandle, Hsla, KeyDownEvent, Path, Pixels, Point,
    SharedString, Subscription, Task, WeakEntity, Window, black, canvas, point, white,
};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_i18n::tr;
use rise_theme::ui_color_system::UIColor;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::font::NATURAL_LINE_HEIGHT;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::media_viewer::media_viewer_video_scrubber::rounded_rect_path;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::shared::utils::music::music_player_state::{MusicListeningBlockedControl, MusicPlayerState};
use crate::modules::music::stores::music_actions::music_types::MusicPlaylistListener;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_mini_player_bottom_padding::{
    MusicPlayerHiddenUnderFullPlayerKey, is_hidden_under_full_player,
};

const THUMB_DIAMETER: f32 = 22.;
const TRACK_HEIGHT: f32 = 6.;
const ROW_HEIGHT: f32 = 58.;
const MARKER_AVATAR: f32 = 24.;
const MARKER_STEM: f32 = 5.;
const MARKER_DOT: f32 = 7.;
const MARKER_HEIGHT: f32 = MARKER_AVATAR + MARKER_STEM + MARKER_DOT;
const ADJUSTABLE_STEP_SECONDS: TimeInterval = 15.;
const PROJECTION_STEP_POINTS: f64 = 0.5;
const PROJECTION_MIN_TICK: TimeInterval = 1. / 120.;
const THUMB_STROKE: f32 = 0.5;
const THUMB_FILL_OPACITY: f32 = 0.85;
const THUMB_STROKE_OPACITY: f32 = 0.55;
const THUMB_BODY_OPACITY: f32 = 1. - (1. - THUMB_FILL_OPACITY) / (1. - THUMB_STROKE_OPACITY);

/// next redraw of a projecting bar, armed for when the thumb or a label changes
struct MusicLivePlaybackSeekBarTick {
    deadline: TimeInterval,
    _task: Task<()>,
}

/// what a projection step needs to move the painted fill and thumb without a render
#[derive(Clone, Copy)]
struct MusicLivePlaybackSeekBarSwap {
    duration: TimeInterval,
    rate_multiplier: f64,
    projection_rate: f64,
}

/// fill and thumb as painted, in the rows own coordinates
struct MusicLivePlaybackSeekBarShapes {
    fill: Option<Path<Pixels>>,
    ring: Option<Path<Pixels>>,
    body: Option<Path<Pixels>>,
}

struct MusicLivePlaybackSeekBarState {
    is_scrubbing: bool,
    scrubbed_progress: f64,
    scrub_start_progress: f64,
    did_move_scrubber: bool,
    anchor_time: TimeInterval,
    anchor_at: TimeInterval,
    width: f32,
    thumb_scale: Animated<f32>,
    focus: FocusHandle,
    tick: Option<MusicLivePlaybackSeekBarTick>,
    hides_under_full_player: bool,
    swap: Option<MusicLivePlaybackSeekBarSwap>,
    painted: Rc<Cell<bool>>,
    shown_label: String,
    anchored_for: Option<(TimeInterval, bool)>,
    _player_observation: Subscription,
    _position_observation: Subscription,
    _services_observation: Subscription,
}

impl MusicLivePlaybackSeekBarState {
    fn reset_anchor(&mut self, current_time: TimeInterval) {
        self.anchor_time = current_time;
        self.anchor_at = RiseClock::monotonic().as_secs_f64();
    }

    /// true if the projection steps by path swaps, an armed tick repaints from the current anchor
    fn swaps_live(&self) -> bool {
        self.swap.is_some() && self.tick.is_some() && self.painted.get()
    }
}

#[derive(IntoElement)]
pub struct MusicLivePlaybackSeekBar {
    id: ElementId,
    player: Entity<MusicPlayerState>,
    track_duration_seconds: i64,
    playlist_id: Option<String>,
    horizontal_padding: f32,
    is_covered: bool,
}

impl MusicLivePlaybackSeekBar {
    pub fn new(id: impl Into<ElementId>, player: Entity<MusicPlayerState>, track_duration_seconds: i64) -> Self {
        Self { id: id.into(), player, track_duration_seconds, playlist_id: None, horizontal_padding: 32., is_covered: false }
    }

    /// bar row and its time labels, the definite box a cached view around the bar lays out
    pub fn height() -> f32 {
        ROW_HEIGHT + 6. + Font::caption().size * NATURAL_LINE_HEIGHT
    }

    pub fn playlist_id(mut self, playlist_id: Option<String>) -> Self {
        self.playlist_id = playlist_id;
        self
    }

    pub fn horizontal_padding(mut self, horizontal_padding: f32) -> Self {
        self.horizontal_padding = horizontal_padding;
        self
    }

    /// desktop only, a bar under the full player holds its thumb still
    pub fn is_covered(mut self, is_covered: bool) -> Self {
        self.is_covered = is_covered;
        self
    }

    const LISTENER_MARKER_REACH: f32 = MARKER_HEIGHT - MARKER_DOT / 2.;
    pub const LISTENER_AVATAR_SIDE: f32 = MARKER_AVATAR;
    pub const LISTENER_DOT_SIDE: f32 = MARKER_DOT;

    pub fn visible_playlist_listeners(
        player: &Entity<MusicPlayerState>,
        playlist_id: Option<&str>,
        cx: &mut App,
    ) -> Vec<MusicPlaylistListener> {
        let Some(track_id) = player.read(cx).current_track.as_ref().map(|track| track.id.clone()) else {
            return Vec::new();
        };
        let Some(playlist_id) = playlist_id else {
            return Vec::new();
        };
        let snapshot = MusicServicesStore::shared(cx)
            .update(cx, |services, cx| services.playlist_listening_snapshot(playlist_id, None, cx));
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let mut listeners: Vec<MusicPlaylistListener> = snapshot
            .map(|snapshot| snapshot.listeners)
            .unwrap_or_default()
            .into_iter()
            .filter(|listener| {
                listener.track_id == track_id && Some(listener.user_id.as_str()) != current_user_id.as_deref()
            })
            .collect();
        listeners.sort_by(|left, right| {
            right.is_playing.cmp(&left.is_playing).then_with(|| left.user_id.cmp(&right.user_id))
        });
        listeners.truncate(3);
        listeners
    }

    fn visible_listeners(&self, cx: &mut App) -> Vec<MusicPlaylistListener> {
        Self::visible_playlist_listeners(&self.player, self.playlist_id.as_deref(), cx)
    }

    /// where the listener is in its track as a fraction of duration seconds
    pub fn listener_fraction(
        services: &MusicServicesStore,
        listener: &MusicPlaylistListener,
        duration: TimeInterval,
        now_ms: i64,
    ) -> f64 {
        let position_ms = services.projected_position_ms_for_listener(listener, now_ms, listener.playback_rate_multiplier());
        if duration > 0. { (position_ms as f64 / (duration * 1_000.)).clamp(0., 1.) } else { 0. }
    }

    /// center x of the index-th marker pointing at x
    pub fn listener_marker_x(index: usize, x: f32) -> f32 {
        x + index as f32 * 3.
    }

    fn listener_marker_origin(index: usize, x: f32, centre_y: f32) -> (f32, f32) {
        (Self::listener_marker_x(index, x) - MARKER_AVATAR / 2., centre_y - Self::LISTENER_MARKER_REACH)
    }

    /// accent of a playing listeners marker
    pub fn listener_playing_accent(cx: &App) -> Hsla {
        UIColor::system_green().resolved_color(rise_ui::theme(cx).appearance)
    }

    fn is_seek_locked(player: &MusicPlayerState) -> bool {
        player.is_listening_control_locked(MusicListeningBlockedControl::Seek)
    }

    fn is_interactive(player: &MusicPlayerState) -> bool {
        !player.is_preparing_playback && !Self::is_seek_locked(player)
    }

    /// store republishes position 2.5 times a second which is a visible step at this width
    /// nothing else knows the position more often so the bar projects between publications
    /// same trick the listener markers use
    fn is_projecting(player: &MusicPlayerState, state: &MusicLivePlaybackSeekBarState) -> bool {
        player.is_playing && !state.is_scrubbing && !player.is_preparing_playback
    }

    /// thumb is inset by its own radius at both ends so this is the
    /// single definition of where the playhead is
    /// markers reuse it so they line up
    fn thumb_centre(width: f32, progress: f64) -> f32 {
        let travel = (width - THUMB_DIAMETER).max(0.);
        THUMB_DIAMETER / 2. + travel * progress.clamp(0., 1.) as f32
    }

    fn display_duration(&self, player: &MusicPlayerState) -> TimeInterval {
        let declared = if self.track_duration_seconds > 0 { self.track_duration_seconds as f64 } else { 0. };
        if player.is_preparing_playback {
            return declared;
        }
        if player.duration > 0. { player.duration } else { declared }
    }

    fn display_current_time(&self, player: &MusicPlayerState, state: &MusicLivePlaybackSeekBarState) -> TimeInterval {
        if player.is_preparing_playback {
            return 0.;
        }
        if state.is_scrubbing { state.scrubbed_progress * self.display_duration(player) } else { player.current_time }
    }

    fn projected_time(
        player: &MusicPlayerState,
        state: &MusicLivePlaybackSeekBarState,
        now_seconds: TimeInterval,
        rate_multiplier: f64,
    ) -> TimeInterval {
        if !Self::is_projecting(player, state) {
            return player.current_time;
        }
        let elapsed = (now_seconds - state.anchor_at).max(0.);
        let bounded = elapsed.min(1.);
        state.anchor_time + bounded * rate_multiplier
    }

    fn display_progress(
        &self,
        player: &MusicPlayerState,
        state: &MusicLivePlaybackSeekBarState,
        now_seconds: TimeInterval,
        rate_multiplier: f64,
    ) -> f64 {
        if player.is_preparing_playback {
            return 0.;
        }
        if state.is_scrubbing {
            return state.scrubbed_progress;
        }
        let duration = self.display_duration(player);
        if duration <= 0. {
            return player.progress;
        }
        (Self::projected_time(player, state, now_seconds, rate_multiplier) / duration).clamp(0., 1.)
    }

    /// seconds until the projection shows something new
    /// a device pixel of thumb travel or the next whole second of the label, whichever is first
    fn tick_delay(width: f32, duration: TimeInterval, projected_seconds: TimeInterval, rate: f64) -> TimeInterval {
        let rate = if rate > 0. { rate } else { 1. };
        let label_delay = (projected_seconds.floor() + 1. - projected_seconds).max(0.) / rate;
        let travel = f64::from((width - THUMB_DIAMETER).max(0.));
        let step_delay = if travel > 0. && duration > 0. {
            PROJECTION_STEP_POINTS * duration / (travel * rate)
        } else {
            label_delay
        };
        step_delay.min(label_delay).clamp(PROJECTION_MIN_TICK, 1.)
    }

    fn schedule_tick(
        state: &Entity<MusicLivePlaybackSeekBarState>,
        player: &Entity<MusicPlayerState>,
        delay: Option<TimeInterval>,
        now_seconds: TimeInterval,
        window: &Window,
        cx: &mut App,
    ) {
        let Some(delay) = delay else {
            if state.read(cx).tick.is_some() {
                state.update(cx, |state, _| state.tick = None);
            }
            return;
        };
        let deadline = now_seconds + delay;
        let armed = state.read(cx).tick.as_ref().map(|tick| tick.deadline);
        if armed.is_some_and(|armed| armed > now_seconds && armed <= deadline) {
            return;
        }
        let weak = state.downgrade();
        let weak_player = player.downgrade();
        let task = window.spawn(cx, async move |cx: &mut AsyncWindowContext| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            let _ = cx.update(move |window, _| {
                window.on_next_frame(move |window, cx| Self::tick(weak, weak_player, deadline, window, cx));
            });
        });
        state.update(cx, |state, _| state.tick = Some(MusicLivePlaybackSeekBarTick { deadline, _task: task }));
    }

    fn tick(
        weak: WeakEntity<MusicLivePlaybackSeekBarState>,
        weak_player: WeakEntity<MusicPlayerState>,
        deadline: TimeInterval,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (Some(state), Some(player)) = (weak.upgrade(), weak_player.upgrade()) else {
            return;
        };
        if state.read(cx).tick.as_ref().map(|tick| tick.deadline) != Some(deadline) {
            return;
        }
        let now = RiseClock::monotonic();
        let now_seconds = now.as_secs_f64();
        let step = {
            let bar = state.read(cx);
            let player = player.read(cx);
            bar.swap.filter(|_| {
                bar.painted.get() && Self::is_projecting(player, bar) && !bar.thumb_scale.is_running(now)
            })
            .map(|swap| {
                let projected = Self::projected_time(player, bar, now_seconds, swap.rate_multiplier);
                let progress = if swap.duration > 0. { (projected / swap.duration).clamp(0., 1.) } else { player.progress };
                let projection_left = bar.anchor_at + 1. - now_seconds;
                let next = (projection_left > 0.).then(|| {
                    Self::tick_delay(bar.width, swap.duration, projected, swap.projection_rate)
                        .min(projection_left.max(PROJECTION_MIN_TICK))
                });
                (Self::shapes(bar.width, progress, 1., Point::default()), next)
            })
        };
        let Some((shapes, next)) = step else {
            state.update(cx, |state, cx| {
                state.tick = None;
                cx.notify();
            });
            return;
        };
        let swapped = Self::swaps(state.entity_id().as_u64(), shapes).is_some_and(|swaps| window.swap_paths(swaps));
        if !swapped {
            state.update(cx, |state, cx| {
                state.painted.set(false);
                state.tick = None;
                cx.notify();
            });
            return;
        }
        state.update(cx, |state, _| state.tick = None);
        Self::schedule_tick(&state, &player, next, now_seconds, window, cx);
    }

    fn swap_key(base: u64, shape: u64) -> u64 {
        base ^ (shape << 56)
    }

    fn shapes(width: f32, progress: f64, scale: f32, origin: Point<Pixels>) -> MusicLivePlaybackSeekBarShapes {
        let travel = (width - THUMB_DIAMETER).max(0.);
        let filled = (THUMB_DIAMETER / 2. + travel * progress.clamp(0., 1.) as f32).max(0.5);
        let centre = Self::thumb_centre(width, progress);
        let radius = THUMB_DIAMETER * scale / 2.;
        let disc = |radius: f32| {
            rounded_rect_path(centre - radius, ROW_HEIGHT / 2. - radius, radius * 2., radius * 2., origin)
        };
        MusicLivePlaybackSeekBarShapes {
            fill: rounded_rect_path(0., (ROW_HEIGHT - TRACK_HEIGHT) / 2., filled, TRACK_HEIGHT, origin),
            ring: disc(radius + THUMB_STROKE / 2.),
            body: disc((radius - THUMB_STROKE / 2.).max(0.)),
        }
    }

    fn swaps(base: u64, shapes: MusicLivePlaybackSeekBarShapes) -> Option<Vec<(u64, Path<Pixels>, Background)>> {
        Some(vec![
            (Self::swap_key(base, 1), shapes.fill?, white().into()),
            (Self::swap_key(base, 2), shapes.ring?, white().opacity(THUMB_STROKE_OPACITY).into()),
            (Self::swap_key(base, 3), shapes.body?, white().opacity(THUMB_BODY_OPACITY).into()),
        ])
    }

    fn format_time(time: TimeInterval) -> String {
        RiseFormat::duration_time_interval(time)
    }

    fn finish_scrub(state: &Entity<MusicLivePlaybackSeekBarState>, player: &Entity<MusicPlayerState>, cx: &mut App) {
        let target = state.update(cx, |state, cx| {
            if !state.is_scrubbing {
                return None;
            }
            let target = state.scrubbed_progress.clamp(0., 1.);
            let did_move = state.did_move_scrubber && (target - state.scrub_start_progress).abs() > 0.001;
            state.is_scrubbing = false;
            state.did_move_scrubber = false;
            cx.notify();
            did_move.then_some(target)
        });
        if let Some(target) = target {
            player.update(cx, |player, cx| player.seek(target, cx));
        }
    }

    /// desktop version of the adjustable action, the arrow keys of the focused bar
    pub fn adjust(duration: TimeInterval, increment: bool, player: &Entity<MusicPlayerState>, cx: &mut App) {
        let (is_interactive, is_locked, base) = {
            let player = player.read(cx);
            (Self::is_interactive(player), Self::is_seek_locked(player), player.progress)
        };
        if !is_interactive || duration <= 0. {
            if is_locked {
                player.update(cx, |player, cx| {
                    player.report_blocked_listening_control(MusicListeningBlockedControl::Seek, cx);
                });
            }
            return;
        }
        let delta = ADJUSTABLE_STEP_SECONDS / duration;
        let target = if increment { base + delta } else { base - delta };
        player.update(cx, |player, cx| player.seek(target.clamp(0., 1.), cx));
    }

    fn scrub_gesture(state: &Entity<MusicLivePlaybackSeekBarState>, player: &Entity<MusicPlayerState>) -> DragGesture {
        let changed_state = state.clone();
        let changed_player = player.clone();
        let ended_state = state.clone();
        let ended_player = player.clone();
        DragGesture::new(0.)
            .on_changed(move |value, _, cx| {
                let (is_interactive, progress) = {
                    let player = changed_player.read(cx);
                    (Self::is_interactive(player), player.progress)
                };
                if !is_interactive {
                    return;
                }
                changed_state.update(cx, |state, cx| {
                    if !state.is_scrubbing {
                        state.scrub_start_progress = progress;
                        state.scrubbed_progress = progress;
                        state.did_move_scrubber = false;
                        state.is_scrubbing = true;
                    }
                    let travel = (state.width - THUMB_DIAMETER).max(1.);
                    let raw = (f32::from(value.location.x) - THUMB_DIAMETER / 2.) / travel;
                    let clamped = f64::from(raw).clamp(0., 1.);
                    if (clamped - state.scrub_start_progress).abs() > 0.001 {
                        state.did_move_scrubber = true;
                    }
                    state.scrubbed_progress = clamped;
                    cx.notify();
                });
            })
            .on_ended(move |_, _, cx| {
                let (is_interactive, is_preparing, is_locked) = {
                    let player = ended_player.read(cx);
                    (Self::is_interactive(player), player.is_preparing_playback, Self::is_seek_locked(player))
                };
                if !is_interactive {
                    if is_locked && !is_preparing {
                        ended_player.update(cx, |player, cx| {
                            player.report_blocked_listening_control(MusicListeningBlockedControl::Seek, cx);
                        });
                    }
                    return;
                }
                Self::finish_scrub(&ended_state, &ended_player, cx);
            })
    }

    /// ringed avatar of one listener in accent, its playing or paused tint
    pub fn listener_avatar(id: &ElementId, listener: &MusicPlaylistListener, accent: Hsla) -> Div {
        let logo_id = ElementId::Name(format!("{id}-marker-{}", listener.user_id).into());
        div()
            .relative()
            .size(px(MARKER_AVATAR))
            .opacity(if listener.is_playing { 1. } else { 0.65 })
            .circle()
            .shadow(vec![BoxShadow {
                color: black().opacity(0.35),
                offset: point(px(0.), px(1.)),
                blur_radius: px(3.),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(
                UserLogoUi::new(logo_id)
                    .source(SharedString::from(listener.user.logo.clone()))
                    .name(SharedString::from(listener.user.name.clone()))
                    .user_id(SharedString::from(listener.user_id.clone()))
                    .size(MARKER_AVATAR)
                    .show_border(false)
                    .show_story_ring(false),
            )
            .child(centred_stroke(accent, 1.5, Shape::Circle))
    }

    /// dot in accent that marks a listeners position on the track
    pub fn listener_dot(accent: Hsla) -> Div {
        div()
            .relative()
            .size(px(MARKER_DOT))
            .circle()
            .bg(accent)
            .child(centred_stroke(black().opacity(0.35), 1., Shape::Circle))
    }

    fn listener_marker(id: &ElementId, listener: &MusicPlaylistListener, playing_accent: Hsla, paused_accent: Hsla) -> Div {
        let accent = if listener.is_playing { playing_accent } else { paused_accent };
        vstack(0.)
            .child(Self::listener_avatar(id, listener, accent))
            .child(div().w(px(1.5)).h(px(MARKER_STEM)).bg(accent))
            .child(Self::listener_dot(accent))
    }

    fn marker_layer(&self, listeners: &[MusicPlaylistListener], duration: TimeInterval, width: f32, cx: &mut App) -> Div {
        let now_ms = RiseClock::now().ms();
        let playing_accent = Self::listener_playing_accent(cx);
        let services = MusicServicesStore::shared(cx);
        let services = services.read(cx);
        let mut layer = div().absolute().inset_0();
        for (index, listener) in listeners.iter().enumerate().rev() {
            let fraction = Self::listener_fraction(services, listener, duration, now_ms);
            let (left, top) = Self::listener_marker_origin(index, Self::thumb_centre(width, fraction), ROW_HEIGHT / 2.);
            layer = layer.child(div().absolute().left(px(left)).top(px(top)).child(Self::listener_marker(
                &self.id,
                listener,
                playing_accent,
                white().opacity(0.45),
            )));
        }
        layer
    }
}

impl RenderOnce for MusicLivePlaybackSeekBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        let player = self.player.clone();
        let services = MusicServicesStore::shared(cx);
        let observed_player = player.clone();
        let positioned_player = player.clone();
        let publication = player.read(cx).position_publication().clone();
        let (initial_progress, initial_time) = {
            let player = player.read(cx);
            (player.progress, player.current_time)
        };
        let hides_under_full_player = env::<MusicPlayerHiddenUnderFullPlayerKey>(cx);
        let is_covered = self.is_covered || is_hidden_under_full_player(player.read(cx), hides_under_full_player);
        if is_covered {
            gpui::hear_entity_through_events_only(player.entity_id());
            gpui::hear_entity_through_events_only(publication.entity_id());
        }
        let state = window.use_keyed_state(id.clone(), cx, move |_, cx| MusicLivePlaybackSeekBarState {
            is_scrubbing: false,
            scrubbed_progress: initial_progress,
            scrub_start_progress: initial_progress,
            did_move_scrubber: false,
            anchor_time: initial_time,
            anchor_at: RiseClock::monotonic().as_secs_f64(),
            width: 0.,
            thumb_scale: Animated::new(1.),
            focus: cx.focus_handle().tab_stop(true),
            tick: None,
            hides_under_full_player,
            swap: None,
            painted: Rc::new(Cell::new(false)),
            shown_label: String::new(),
            anchored_for: None,
            _player_observation: cx.observe(&observed_player, |state: &mut MusicLivePlaybackSeekBarState, player, cx| {
                if is_hidden_under_full_player(player.read(cx), state.hides_under_full_player) {
                    return;
                }
                cx.notify();
            }),
            _position_observation: cx.observe(&publication, move |state: &mut MusicLivePlaybackSeekBarState, _, cx| {
                let player = positioned_player.read(cx);
                if is_hidden_under_full_player(player, state.hides_under_full_player) {
                    return;
                }
                let current_time = player.current_time;
                if state.swaps_live() && Self::format_time(current_time) == state.shown_label {
                    state.reset_anchor(current_time);
                    return;
                }
                state.reset_anchor(current_time);
                state.anchored_for = Some((current_time, player.is_playing));
                cx.notify();
            }),
            _services_observation: cx.observe(&services, |_, _, cx| cx.notify()),
        });

        let (current_time, is_playing, current_track_id) = {
            let player = player.read(cx);
            (player.current_time, player.is_playing, player.current_track.as_ref().map(|track| track.id.clone()))
        };
        let anchored_state = state.clone();
        on_change(
            SharedString::from(format!("{id}-anchor")),
            (current_time, is_playing),
            false,
            window,
            cx,
            move |_, (current_time, is_playing), _, cx| {
                let (current_time, is_playing) = (*current_time, *is_playing);
                anchored_state.update(cx, |state, cx| {
                    if state.anchored_for.take() == Some((current_time, is_playing)) {
                        return;
                    }
                    state.reset_anchor(current_time);
                    if !state.swaps_live() {
                        cx.notify();
                    }
                });
            },
        );
        let changed_state = state.clone();
        let changed_player = player.clone();
        on_change(
            SharedString::from(format!("{id}-track")),
            current_track_id,
            false,
            window,
            cx,
            move |_, _, _, cx| {
                let (progress, current_time) = {
                    let player = changed_player.read(cx);
                    (player.progress, player.current_time)
                };
                changed_state.update(cx, |state, cx| {
                    state.is_scrubbing = false;
                    state.scrubbed_progress = progress;
                    state.scrub_start_progress = progress;
                    state.did_move_scrubber = false;
                    state.reset_anchor(current_time);
                    cx.notify();
                });
            },
        );

        let visible_listeners = self.visible_listeners(cx);
        let rate_multiplier = player.read(cx).active_playback_rate_multiplier(cx);
        let now = RiseClock::monotonic();
        let reduce_motion = cx.reduce_motion();
        let is_scrubbing = state.read(cx).is_scrubbing;
        state.update(cx, |state, _| {
            state.thumb_scale.set(if is_scrubbing { 1.18 } else { 1. }, spring(0.25, 0.8), now, reduce_motion);
        });
        let thumb_scale = state.read(cx).thumb_scale.frame_value(now, window);

        let (progress, duration, current_label, duration_label, is_preparing, is_locked, is_interactive, is_animating, focus, width) = {
            let player = player.read(cx);
            let bar = state.read(cx);
            let duration = self.display_duration(player);
            (
                self.display_progress(player, bar, now.as_secs_f64(), rate_multiplier),
                duration,
                Self::format_time(self.display_current_time(player, bar)),
                Self::format_time(duration),
                player.is_preparing_playback,
                Self::is_seek_locked(player),
                Self::is_interactive(player),
                Self::is_projecting(player, bar) || visible_listeners.iter().any(|listener| listener.is_playing),
                bar.focus.clone(),
                bar.width,
            )
        };

        let projection_rate = if visible_listeners.iter().any(|listener| listener.is_playing) {
            rate_multiplier.max(1.)
        } else {
            rate_multiplier
        };
        let listeners_move = visible_listeners.iter().any(|listener| listener.is_playing);
        let (projected_seconds, projection_left) = {
            let player = player.read(cx);
            let bar = state.read(cx);
            (
                Self::projected_time(player, bar, now.as_secs_f64(), rate_multiplier),
                bar.anchor_at + 1. - now.as_secs_f64(),
            )
        };
        let tick_delay = (is_animating && !is_covered)
            .then(|| Self::tick_delay(width, duration, projected_seconds, projection_rate))
            .and_then(|delay| {
                if listeners_move {
                    Some(delay)
                } else if projection_left > 0. {
                    Some(delay.min(projection_left.max(PROJECTION_MIN_TICK)))
                } else {
                    None
                }
            });
        let swap = (tick_delay.is_some() && !listeners_move && !is_scrubbing).then_some(MusicLivePlaybackSeekBarSwap {
            duration,
            rate_multiplier,
            projection_rate,
        });
        let shown_label = current_label.clone();
        state.update(cx, |state, _| {
            state.swap = swap;
            state.shown_label = shown_label;
        });
        Self::schedule_tick(&state, &player, tick_delay, now.as_secs_f64(), window, cx);

        let track_opacity = if is_preparing { 0.28 } else if is_locked { 0.45 } else { 1. };
        let thumb_opacity = if is_preparing { 0. } else if is_locked { 0.45 } else { 1. };

        let markers = self.marker_layer(&visible_listeners, duration, width, cx);
        let swap_base = state.entity_id().as_u64();
        let fill_painted = state.read(cx).painted.clone();
        let thumb_painted = fill_painted.clone();
        let measured = state.clone();
        let key_player = player.clone();
        let accessibility_value = format!("{current_label} / {duration_label}");

        let row = div()
            .id(SharedString::from(format!("{id}-row")))
            .relative()
            .w_full()
            .h(px(ROW_HEIGHT))
            .role(gpui::Role::Slider)
            .aria_label(SharedString::from(tr("music_now_playing_progress")))
            .aria_value(SharedString::from(accessibility_value))
            .track_focus(&focus)
            .when(is_interactive, |row| row.cursor_pointer())
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                let increment = match event.keystroke.key.as_str() {
                    "left" | "down" => false,
                    "right" | "up" => true,
                    _ => return,
                };
                cx.stop_propagation();
                Self::adjust(duration, increment, &key_player, cx);
            })
            .child(measure(SharedString::from(format!("{id}-width")), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                measured.update(cx, |state, cx| {
                    if state.width != width {
                        state.width = width;
                        cx.notify();
                    }
                });
            }))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px((ROW_HEIGHT - TRACK_HEIGHT) / 2.))
                    .w_full()
                    .h(px(TRACK_HEIGHT))
                    .opacity(track_opacity)
                    .rounded_full()
                    .bg(white().opacity(0.22)),
            )
            .child(div().absolute().inset_0().opacity(track_opacity).child(
                canvas(|_, _, _| (), move |bounds, (), window, _| {
                    let fill = Self::shapes(width, progress, thumb_scale, bounds.origin).fill;
                    fill_painted.set(fill.is_some());
                    if let Some(fill) = fill {
                        window.paint_swappable_path(fill, white(), Self::swap_key(swap_base, 1), bounds.origin);
                    }
                })
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            ))
            .child(markers)
            .child(div().absolute().inset_0().opacity(thumb_opacity).child(
                canvas(|_, _, _| (), move |bounds, (), window, _| {
                    let shapes = Self::shapes(width, progress, thumb_scale, bounds.origin);
                    let mut complete = thumb_painted.get();
                    for (key, path, opacity) in [(2, shapes.ring, THUMB_STROKE_OPACITY), (3, shapes.body, THUMB_BODY_OPACITY)] {
                        match path {
                            Some(path) => window.paint_swappable_path(
                                path,
                                white().opacity(opacity),
                                Self::swap_key(swap_base, key),
                                bounds.origin,
                            ),
                            None => complete = false,
                        }
                    }
                    thumb_painted.set(complete);
                })
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            ))
            .gesture(SharedString::from(format!("{id}-scrub")), Self::scrub_gesture(&state, &player));

        vstack(6.)
            .w_full()
            .px(px(self.horizontal_padding))
            .child(div().relative().w_full().h(px(ROW_HEIGHT)).child(row).when(is_preparing, |host| {
                host.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(progress_view().control_size(ControlSize::Small).tint(white())),
                )
            }))
            .child(
                hstack(None)
                    .w_full()
                    .swiftui_font(Font::caption())
                    .text_color(white().opacity(0.6))
                    .child(current_label)
                    .child(spacer())
                    .child(duration_label),
            )
    }
}
