use gpui::{
    AnyElement, AnyView, App, AppContext, AsyncApp, Bounds, Context, Div, ElementId, Entity, FontWeight, Hsla, IntoElement,
    ParentElement, Pixels, Render, SharedString, Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Subscription,
    Task, WeakEntity, Window, div, px,
};
use std::time::Duration;
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChrome;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::app::aside::aside_navigator;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::modules::music::pages::music_tab::mini_player_bar::MiniPlayerBar;
use crate::modules::music::pages::now_playing::music_live_playback_seek_bar::MusicLivePlaybackSeekBar;
use crate::modules::music::shared::utils::music::music_models::{MusicRepeatMode, MusicRepeatModeExt, TrackModel};
use crate::modules::music::shared::utils::music::music_output_volume::MusicOutputVolume;
use crate::modules::music::shared::utils::music::music_player_state::{MusicListeningBlockedControl, MusicPlayerState};
use crate::modules::music::stores::music_actions::music_types::MusicPlaylistListener;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverMedia;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

const COVER_SIDE: f32 = 52.0;
const COVER_RADIUS: f32 = 6.0;
const BUTTON_SIDE: f32 = 32.0;
const PLAY_SIDE: f32 = 34.0;
const CENTRE_MAX_WIDTH: f32 = 620.0;
const CENTRE_MIN_WIDTH: f32 = 300.0;
const LINE_ROW_HEIGHT: f32 = 16.0;
const LINE_HEIGHT: f32 = 4.0;
const THUMB_SIDE: f32 = 12.0;
const TIME_LABEL_WIDTH: f32 = 40.0;
const LINE_GAP: f32 = 6.0;
const VOLUME_WIDTH: f32 = 96.0;
const LIKED_COLOR: &str = "#E84393";
const MARKER_STEP_POINTS: f64 = 0.5;
const MARKER_MIN_TICK: TimeInterval = 0.4;

fn glyph(name: &str, side: f32, tint: Hsla) -> Option<gpui::Svg> {
    IconUi::sized(name, px(side), tint, name.ends_with(".fill"))
}

fn icon_button(
    id: &'static str,
    label: String,
    content: impl IntoElement,
    enabled: bool,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(SharedString::new_static(id))
        .flex_none()
        .tooltip_label(label.clone())
        .aria_label(label)
        .when(!enabled, |slot| slot.opacity(0.4))
        .child(
            plain(ElementId::Name(SharedString::from(format!("{id}-press"))))
                .flex()
                .items_center()
                .justify_center()
                .size(px(BUTTON_SIDE))
                .on_press(move |window, cx| {
                    if enabled {
                        action(window, cx);
                    }
                })
                .child(content),
        )
}

fn open_full_player(cx: &mut App) {
    MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_show_now_playing(true, cx));
}

/// whether a track is loaded, the root lays out the bar only when this flips
pub struct MusicGlobalPlayerBarPresence {
    is_present: bool,
    _player: Subscription,
}

impl MusicGlobalPlayerBarPresence {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let player = MusicPlayerState::shared(cx);
        Self {
            is_present: player.read(cx).current_track.is_some(),
            _player: cx.observe(&player, |presence: &mut Self, player, cx| {
                let is_present = player.read(cx).current_track.is_some();
                if is_present != presence.is_present {
                    presence.is_present = is_present;
                    cx.notify();
                }
            }),
        }
    }

    pub fn is_present(&self) -> bool {
        self.is_present
    }
}

/// what the bar draws from the player, a publication that keeps it changes nothing on screen
#[derive(Clone, PartialEq)]
struct MusicGlobalPlayerBarKey {
    track_id: Option<String>,
    title: String,
    artist: String,
    cover: String,
    is_playing: bool,
    is_preparing: bool,
    is_shuffled: bool,
    repeat_mode: MusicRepeatMode,
    has_active_effect: bool,
    is_music_playback: bool,
}

impl MusicGlobalPlayerBarKey {
    fn read(player: &MusicPlayerState, cx: &App) -> Self {
        let track = player.current_track.as_ref();
        Self {
            track_id: track.map(|track| track.id.clone()),
            title: track.map(|track| track.title.clone()).unwrap_or_default(),
            artist: track.map(|track| track.artist.clone()).unwrap_or_default(),
            cover: track.map(|track| track.cover_image_url.clone()).unwrap_or_default(),
            is_playing: player.is_playing,
            is_preparing: player.is_preparing_playback,
            is_shuffled: player.is_shuffled(cx),
            repeat_mode: player.repeat_mode(cx),
            has_active_effect: player.has_active_audio_effect(),
            is_music_playback: player.is_music_playback(),
        }
    }
}

pub struct MusicGlobalPlayerBar {
    shown: Option<MusicGlobalPlayerBarKey>,
    centre_width: f32,
    line: Entity<MusicGlobalPlayerLine>,
    listeners: Entity<MusicGlobalPlayerListeners>,
    volume: Entity<MusicGlobalPlayerVolume>,
    _player: Subscription,
    _services: Subscription,
}

impl MusicGlobalPlayerBar {
    pub const HEIGHT: f32 = 76.0;

    pub fn new(cx: &mut Context<Self>) -> Self {
        let player = MusicPlayerState::shared(cx);
        let services = MusicServicesStore::shared(cx);
        Self {
            shown: None,
            centre_width: 0.0,
            line: cx.new(MusicGlobalPlayerLine::new),
            listeners: cx.new(MusicGlobalPlayerListeners::new),
            volume: cx.new(|_| MusicGlobalPlayerVolume),
            _player: cx.observe(&player, |bar: &mut Self, player, cx| {
                if player.read(cx).show_now_playing() {
                    return;
                }
                let key = MusicGlobalPlayerBarKey::read(player.read(cx), cx);
                if bar.shown.as_ref() != Some(&key) {
                    cx.notify();
                }
            }),
            _services: cx.observe(&services, |_, _, cx| cx.notify()),
        }
    }

    /// listener markers layer laid over the bar and the band above it
    pub fn listeners(&self) -> Entity<MusicGlobalPlayerListeners> {
        self.listeners.clone()
    }

    fn set_centre(&mut self, centre: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.listeners.update(cx, |listeners, cx| listeners.set_centre(centre, cx));
        let width = f32::from(centre.size.width);
        if (self.centre_width - width).abs() < 0.5 {
            return;
        }
        self.centre_width = width;
        self.line.update(cx, |line, cx| line.set_row_width(width, cx));
        cx.notify();
    }

    fn cover(track: &TrackModel, theme: &rise_theme::AppTheme) -> AnyElement {
        if MusicCoverMedia::has_remote_cover(&track.cover_image_url) {
            return MusicCoverImage::new("global-player-cover", track.cover_image_url.clone())
                .fallback_colors(track.cover_colors.clone())
                .size(COVER_SIDE)
                .corner_radius(COVER_RADIUS)
                .into_any_element();
        }
        div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(COVER_SIDE))
            .rounded(px(COVER_RADIUS))
            .bg(LiquidChrome::surface_fill(theme, 0))
            .children(glyph("music.note", 20.0, theme.text.secondary))
            .into_any_element()
    }

    fn leading(track: &TrackModel, is_liked: bool, nav_store: Option<Entity<NavigationStore>>, theme: &rise_theme::AppTheme) -> Div {
        let artist_track = track.clone();
        let artist = plain(ElementId::Name(SharedString::new_static("global-player-artist")))
            .on_press(move |_, cx| {
                if artist_track.artist_is_registered && !artist_track.artist_id.is_empty() {
                    let route = RootRoute::MusicArtistProfile { artist_id: artist_track.artist_id.clone() };
                    if let Some(nav_store) = &nav_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                    }
                } else {
                    open_full_player(cx);
                }
            })
            .child(MainText::new(track.artist.clone()).px(12.0).color(theme.text.secondary).line_limit(1));
        let titles = vstack(2.0)
            .align(HorizontalAlignment::Leading)
            .flex_shrink()
            .min_w_0()
            .child(
                plain(ElementId::Name(SharedString::new_static("global-player-title")))
                    .on_press(|_, cx| open_full_player(cx))
                    .child(
                        MainText::new(track.title.clone())
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.primary)
                            .line_limit(1),
                    ),
            )
            .child(artist);

        let profile_track = track.clone();
        let add = icon_button(
            "global-player-add",
            tr("music_add_to_profile"),
            div().children(glyph("plus.circle", 18.0, theme.text.secondary)),
            track.transfer_phase.is_none(),
            move |_, cx| {
                let track = profile_track.clone();
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.add_track_to_profile(track, cx));
            },
        );
        let like_track_id = track.id.clone();
        let like = icon_button(
            "global-player-like",
            tr(if is_liked { "ctx_unlike" } else { "ctx_like" }),
            div().children(glyph(
                if is_liked { "heart.fill" } else { "heart" },
                18.0,
                if is_liked { rise_theme::color(LIKED_COLOR) } else { theme.text.secondary },
            )),
            true,
            move |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.toggle_like(&like_track_id, cx));
            },
        );

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .id("global-player-cover-press")
                    .flex_none()
                    .cursor_pointer()
                    .on_click(|_, _, cx| open_full_player(cx))
                    .child(Self::cover(track, theme)),
            )
            .child(titles)
            .child(add)
            .child(like)
    }

    fn transport(key: &MusicGlobalPlayerBarKey, theme: &rise_theme::AppTheme) -> Div {
        let inactive = theme.text.secondary;
        let accent = theme.primary._100;
        let play_content: AnyElement = if key.is_preparing {
            progress_view().tint(theme.text.primary).control_size(ControlSize::Small).into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(PLAY_SIDE))
                .rounded_full()
                .bg(LiquidChrome::surface_fill(theme, 0))
                .children(glyph(if key.is_playing { "pause.fill" } else { "play.fill" }, 16.0, theme.text.primary))
                .into_any_element()
        };
        let player_action = |action: fn(&mut MusicPlayerState, &mut Context<MusicPlayerState>)| {
            move |_: &mut Window, cx: &mut App| MusicPlayerState::shared(cx).update(cx, |player, cx| action(player, cx))
        };
        hstack(8.0)
            .child(icon_button(
                "global-player-shuffle",
                tr("music_shuffle"),
                div().children(glyph("shuffle", 16.0, if key.is_shuffled { accent } else { inactive })),
                true,
                player_action(|player, cx| player.toggle_shuffle(cx)),
            ))
            .child(icon_button(
                "global-player-previous",
                tr("music_player_previous"),
                div().children(glyph("backward.fill", 16.0, theme.text.primary)),
                true,
                player_action(|player, cx| player.previous(cx)),
            ))
            .child(icon_button(
                "global-player-play",
                tr(if key.is_playing { "media_viewer_pause" } else { "media_viewer_play" }),
                play_content,
                true,
                player_action(|player, cx| player.toggle_play_pause(cx)),
            ))
            .child(icon_button(
                "global-player-next",
                tr("music_player_next"),
                div().children(glyph("forward.fill", 16.0, theme.text.primary)),
                true,
                player_action(|player, cx| player.next(cx)),
            ))
            .child(icon_button(
                "global-player-repeat",
                format!("{}: {}", tr("music_player_repeat"), key.repeat_mode.accessibility_value()),
                div().children(glyph(
                    key.repeat_mode.icon(),
                    16.0,
                    if key.repeat_mode != MusicRepeatMode::Off { accent } else { inactive },
                )),
                true,
                player_action(|player, cx| player.cycle_repeat_mode(cx)),
            ))
    }

    fn trailing(&self, key: &MusicGlobalPlayerBarKey, theme: &rise_theme::AppTheme) -> Div {
        let effects = key.is_music_playback.then(|| {
            icon_button(
                "global-player-effects",
                tr("music_effects_short"),
                div().children(glyph(
                    if key.has_active_effect { "waveform.path.ecg" } else { "waveform" },
                    18.0,
                    if key.has_active_effect { theme.primary._100 } else { theme.text.secondary },
                )),
                true,
                |_, cx| MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.open_audio_effects(cx)),
            )
        });
        let queue = icon_button(
            "global-player-queue",
            tr("music_queue_title"),
            div().children(glyph("list.bullet", 18.0, theme.text.secondary)),
            true,
            |_, cx| {
                if aside_navigator::has_frontmost(cx) {
                    aside_navigator::open(RootRoute::MusicNowPlaying, cx);
                } else {
                    MusicPlayerState::shared(cx).update(cx, |player, cx| {
                        player.show_queue_sheet = true;
                        cx.notify();
                    });
                }
            },
        );
        let expand = icon_button(
            "global-player-expand",
            tr("desktop_menu_toggle_full_screen"),
            div().children(glyph("arrow.up.left.and.arrow.down.right", 16.0, theme.text.secondary)),
            true,
            |_, cx| open_full_player(cx),
        );
        let close = icon_button(
            "global-player-close",
            tr("close"),
            div().children(glyph("xmark", 14.0, theme.text.secondary)),
            true,
            |_, cx| MiniPlayerBar::dismiss(cx),
        );
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(2.0))
            .flex_1()
            .min_w_0()
            .children(effects)
            .child(queue)
            .child(expand)
            .child(
                AnyView::from(self.volume.clone())
                    .cached(StyleRefinement::default().flex_none().w(px(BUTTON_SIDE + VOLUME_WIDTH + 4.0)).h(px(BUTTON_SIDE))),
            )
            .child(close)
    }
}

impl Render for MusicGlobalPlayerBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let player = MusicPlayerState::shared(cx);
        let (key, track) = {
            let player = player.read(cx);
            (MusicGlobalPlayerBarKey::read(player, cx), player.current_track.clone())
        };
        self.shown = Some(key.clone());
        let Some(track) = track else {
            return div().size_full();
        };
        let theme = rise_ui::theme(cx).clone();
        let like_track = track.clone();
        let is_liked = MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.is_liked(&like_track, cx));
        let nav_store = env::<NavigationStoreKey>(cx);

        let bar = cx.entity();
        let line_style = if self.centre_width > 0.0 {
            StyleRefinement::default().flex_none().w(px(self.centre_width)).h(px(LINE_ROW_HEIGHT))
        } else {
            StyleRefinement::default().w_full().h(px(LINE_ROW_HEIGHT)).flex_none()
        };
        let centre = vstack(2.0)
            .relative()
            .flex_1()
            .min_w(px(CENTRE_MIN_WIDTH))
            .max_w(px(CENTRE_MAX_WIDTH))
            .items_center()
            .child(measure("global-player-centre-width", move |bounds, _, cx| {
                bar.update(cx, |bar, cx| bar.set_centre(bounds, cx));
            }))
            .child(Self::transport(&key, &theme))
            .child(AnyView::from(self.line.clone()).cached(line_style));

        div()
            .size_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.0))
            .px(px(12.0))
            .child(Self::leading(&track, is_liked, nav_store, &theme))
            .child(centre)
            .child(self.trailing(&key, &theme))
    }
}

/// position line with elapsed and total times beside it, drag or click seeks
pub struct MusicGlobalPlayerLine {
    row_width: f32,
    width: f32,
    scrubbed: Option<f64>,
    is_hovered: bool,
    shown: (String, i64),
    _player: Subscription,
    _position: Subscription,
}

impl MusicGlobalPlayerLine {
    fn new(cx: &mut Context<Self>) -> Self {
        let player = MusicPlayerState::shared(cx);
        let publication = player.read(cx).position_publication().clone();
        let positioned = player.clone();
        Self {
            row_width: 0.0,
            width: 0.0,
            scrubbed: None,
            is_hovered: false,
            shown: (String::new(), -1),
            _player: cx.observe(&player, |line: &mut Self, player, cx| {
                if player.read(cx).show_now_playing() {
                    return;
                }
                if line.shown != line.drawn(player.read(cx)) {
                    cx.notify();
                }
            }),
            _position: cx.observe(&publication, move |line: &mut Self, _, cx| {
                let player = positioned.read(cx);
                if player.show_now_playing() || line.scrubbed.is_some() {
                    return;
                }
                if line.shown != line.drawn(player) {
                    cx.notify();
                }
            }),
        }
    }

    fn set_row_width(&mut self, row_width: f32, cx: &mut Context<Self>) {
        if self.row_width == row_width {
            return;
        }
        self.row_width = row_width;
        self.width = (row_width - TIME_LABEL_WIDTH * 2.0 - LINE_GAP * 2.0).max(0.0);
        cx.notify();
    }

    fn duration(player: &MusicPlayerState) -> TimeInterval {
        if player.duration > 0.0 {
            return player.duration;
        }
        player.current_track.as_ref().map_or(0.0, |track| track.duration_seconds.max(0) as f64)
    }

    fn fraction(&self, player: &MusicPlayerState) -> f64 {
        if player.is_preparing_playback {
            return 0.0;
        }
        self.scrubbed.unwrap_or(player.progress).clamp(0.0, 1.0)
    }

    fn current_time(&self, player: &MusicPlayerState) -> TimeInterval {
        match self.scrubbed {
            Some(fraction) => fraction * Self::duration(player),
            None if player.is_preparing_playback => 0.0,
            None => player.current_time,
        }
    }

    /// what a render would put on screen, the elapsed label and fill end in half points
    fn drawn(&self, player: &MusicPlayerState) -> (String, i64) {
        let label = RiseFormat::duration_time_interval(self.current_time(player));
        let end = (self.fraction(player) * f64::from(self.width) * 2.0).round() as i64;
        (label, end)
    }

    fn is_interactive(player: &MusicPlayerState) -> bool {
        !player.is_preparing_playback && !player.is_listening_control_locked(MusicListeningBlockedControl::Seek)
    }

    fn scrub_gesture(line: Entity<Self>) -> DragGesture {
        let changed = line.clone();
        DragGesture::new(0.0)
            .on_changed(move |value, _, cx| {
                if !Self::is_interactive(MusicPlayerState::shared(cx).read(cx)) {
                    return;
                }
                let x = f32::from(value.location.x);
                changed.update(cx, |line, cx| {
                    if line.width <= 0.0 {
                        return;
                    }
                    line.scrubbed = Some(f64::from(x / line.width).clamp(0.0, 1.0));
                    cx.notify();
                });
            })
            .on_ended(move |_, _, cx| {
                let player = MusicPlayerState::shared(cx);
                let (is_interactive, is_preparing) = {
                    let player = player.read(cx);
                    (Self::is_interactive(player), player.is_preparing_playback)
                };
                let scrubbed = line.update(cx, |line, cx| {
                    let scrubbed = line.scrubbed.take();
                    cx.notify();
                    scrubbed
                });
                if !is_interactive {
                    if !is_preparing {
                        player.update(cx, |player, cx| {
                            player.report_blocked_listening_control(MusicListeningBlockedControl::Seek, cx);
                        });
                    }
                    return;
                }
                if let Some(fraction) = scrubbed {
                    player.update(cx, |player, cx| player.seek(fraction, cx));
                }
            })
    }
}

impl Render for MusicGlobalPlayerLine {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let player = MusicPlayerState::shared(cx);
        let (fraction, current_label, duration_label, is_interactive) = {
            let player = player.read(cx);
            self.shown = self.drawn(player);
            (
                self.fraction(player) as f32,
                RiseFormat::duration_time_interval(self.current_time(player)),
                RiseFormat::duration_time_interval(Self::duration(player)),
                Self::is_interactive(player),
            )
        };
        let theme = rise_ui::theme(cx).clone();
        let line = cx.entity();
        let hovered = line.clone();
        let active = self.is_hovered || self.scrubbed.is_some();
        let fill_tint = if active { theme.primary._100 } else { theme.text.primary };
        let filled = (self.width * fraction).max(0.0);
        let time = |label: String| {
            div().flex_none().w(px(TIME_LABEL_WIDTH)).flex().justify_center().child(
                MainText::new(label).px(11.0).color(theme.text.secondary).monospaced_digits(true).line_limit(1),
            )
        };

        let track = div()
            .id("global-player-line")
            .relative()
            .flex_none()
            .w(px(self.width))
            .h(px(LINE_ROW_HEIGHT))
            .role(gpui::Role::Slider)
            .aria_label(SharedString::from(tr("music_now_playing_progress")))
            .aria_value(SharedString::from(format!("{current_label} / {duration_label}")))
            .when(is_interactive, |track| track.cursor_pointer())
            .on_hover(move |is_hovered, _, cx| {
                let is_hovered = *is_hovered;
                hovered.update(cx, |line, cx| {
                    if line.is_hovered != is_hovered {
                        line.is_hovered = is_hovered;
                        cx.notify();
                    }
                });
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px((LINE_ROW_HEIGHT - LINE_HEIGHT) / 2.0))
                    .h(px(LINE_HEIGHT))
                    .rounded_full()
                    .bg(LiquidChrome::surface_fill(&theme, 1)),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px((LINE_ROW_HEIGHT - LINE_HEIGHT) / 2.0))
                    .w(px(filled))
                    .h(px(LINE_HEIGHT))
                    .rounded_full()
                    .bg(fill_tint),
            )
            .when(active && is_interactive, |track| {
                track.child(
                    div()
                        .absolute()
                        .left(px((filled - THUMB_SIDE / 2.0).max(0.0)))
                        .top(px((LINE_ROW_HEIGHT - THUMB_SIDE) / 2.0))
                        .size(px(THUMB_SIDE))
                        .rounded_full()
                        .bg(theme.text.primary),
                )
            })
            .gesture("global-player-line-scrub", Self::scrub_gesture(line));
        let row = if self.row_width > 0.0 { div().flex_none().w(px(self.row_width)) } else { div().w_full() };

        row.h(px(LINE_ROW_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(LINE_GAP))
            .child(time(current_label))
            .child(track)
            .child(time(duration_label))
    }
}

/// what the listener markers draw, the listeners and the track length they sit on
#[derive(Clone, PartialEq)]
struct MusicGlobalPlayerListenersKey {
    listeners: Vec<MusicPlaylistListener>,
    duration: TimeInterval,
}

impl MusicGlobalPlayerListenersKey {
    fn read(cx: &mut App) -> Self {
        let player = MusicPlayerState::shared(cx);
        let playlist_id = player.read(cx).listening_playlist_id().map(str::to_owned);
        let listeners = MusicLivePlaybackSeekBar::visible_playlist_listeners(&player, playlist_id.as_deref(), cx);
        let duration = MusicGlobalPlayerLine::duration(player.read(cx));
        Self { listeners, duration }
    }
}

/// up to three listeners of the playing playlist at their own projected positions
/// green while that listener plays, muted while paused
/// a dot on the line plus the avatar
pub struct MusicGlobalPlayerListeners {
    centre: Option<Bounds<Pixels>>,
    frame: Option<Bounds<Pixels>>,
    track_width: f32,
    shown: Option<MusicGlobalPlayerListenersKey>,
    is_armed: bool,
    is_stalled: bool,
    _tick: Option<Task<()>>,
    _player: Subscription,
    _services: Subscription,
}

impl MusicGlobalPlayerListeners {
    pub const AVATAR_BAND: f32 = MusicLivePlaybackSeekBar::LISTENER_AVATAR_SIDE + 4.0;

    fn new(cx: &mut Context<Self>) -> Self {
        let player = MusicPlayerState::shared(cx);
        let services = MusicServicesStore::shared(cx);
        Self {
            centre: None,
            frame: None,
            track_width: 0.0,
            shown: None,
            is_armed: false,
            is_stalled: false,
            _tick: None,
            _player: cx.observe(&player, |listeners: &mut Self, player, cx| {
                if player.read(cx).show_now_playing() {
                    return;
                }
                if listeners.is_stalled {
                    listeners.is_stalled = false;
                    cx.notify();
                    return;
                }
                listeners.refresh(cx);
            }),
            _services: cx.observe(&services, |listeners: &mut Self, _, cx| {
                if MusicPlayerState::shared(cx).read(cx).show_now_playing() {
                    return;
                }
                listeners.refresh(cx);
            }),
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let key = MusicGlobalPlayerListenersKey::read(cx);
        if self.shown.as_ref() != Some(&key) {
            cx.notify();
        }
    }

    fn set_centre(&mut self, centre: Bounds<Pixels>, cx: &mut Context<Self>) {
        if self.centre == Some(centre) {
            return;
        }
        self.centre = Some(centre);
        self.track_width = (f32::from(centre.size.width) - TIME_LABEL_WIDTH * 2.0 - LINE_GAP * 2.0).max(0.0);
        cx.notify();
    }

    fn set_frame(&mut self, frame: Bounds<Pixels>, cx: &mut Context<Self>) {
        if self.frame == Some(frame) {
            return;
        }
        self.frame = Some(frame);
        cx.notify();
    }

    /// arms one redraw for the moment a playing listeners marker moves a device pixel
    fn arm(&mut self, key: &MusicGlobalPlayerListenersKey, cx: &mut Context<Self>) {
        let rate = key
            .listeners
            .iter()
            .filter(|listener| listener.is_playing)
            .map(|listener| listener.playback_rate_multiplier().max(0.1))
            .fold(0.0_f64, f64::max);
        if rate <= 0.0 || self.track_width <= 0.0 || key.duration <= 0.0 {
            self.is_armed = false;
            self._tick = None;
            return;
        }
        if self.is_armed {
            return;
        }
        let delay = (MARKER_STEP_POINTS * key.duration / (f64::from(self.track_width) * rate)).clamp(MARKER_MIN_TICK, 1.0);
        self.is_armed = true;
        self._tick = Some(cx.spawn(async move |view: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            let _ = view.update(cx, |listeners, cx| {
                listeners.is_armed = false;
                if MusicPlayerState::shared(cx).read(cx).show_now_playing() {
                    listeners.is_stalled = true;
                    return;
                }
                cx.notify();
            });
        }));
    }
}

impl Render for MusicGlobalPlayerListeners {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key = MusicGlobalPlayerListenersKey::read(cx);
        let this = cx.entity();
        let frame_probe = measure("global-player-listeners-frame", move |bounds, _, cx| {
            this.update(cx, |listeners, cx| listeners.set_frame(bounds, cx));
        });
        let mut markers: Vec<Div> = Vec::new();
        if let (Some(centre), Some(frame)) = (self.centre, self.frame) {
            let id = ElementId::Name(SharedString::new_static("global-player-listeners"));
            let playing_accent = MusicLivePlaybackSeekBar::listener_playing_accent(cx);
            let paused_accent = rise_ui::theme(cx).text.primary.opacity(0.45);
            let now_ms = RiseClock::now().ms();
            let track_left = f32::from(centre.origin.x - frame.origin.x) + TIME_LABEL_WIDTH + LINE_GAP;
            let line_y = f32::from(centre.bottom() - frame.origin.y) - LINE_ROW_HEIGHT / 2.0;
            let fractions: Vec<f64> = {
                let services = MusicServicesStore::shared(cx);
                let services = services.read(cx);
                key.listeners
                    .iter()
                    .map(|listener| MusicLivePlaybackSeekBar::listener_fraction(services, listener, key.duration, now_ms))
                    .collect()
            };
            let avatar_side = MusicLivePlaybackSeekBar::LISTENER_AVATAR_SIDE;
            let dot_side = MusicLivePlaybackSeekBar::LISTENER_DOT_SIDE;
            for (index, (listener, fraction)) in key.listeners.iter().zip(fractions).enumerate().rev() {
                let x = MusicLivePlaybackSeekBar::listener_marker_x(index, track_left + self.track_width * fraction as f32);
                let accent = if listener.is_playing { playing_accent } else { paused_accent };
                markers.push(
                    div()
                        .absolute()
                        .left(px(x - avatar_side / 2.0))
                        .top(px(0.0))
                        .child(MusicLivePlaybackSeekBar::listener_avatar(&id, listener, accent)),
                );
                markers.push(
                    div()
                        .absolute()
                        .left(px(x - dot_side / 2.0))
                        .top(px(line_y - dot_side / 2.0))
                        .child(MusicLivePlaybackSeekBar::listener_dot(accent)),
                );
            }
        }
        self.arm(&key, cx);
        self.shown = Some(key);
        div().relative().size_full().child(frame_probe).children(markers)
    }
}

/// speaker (mute and back) and volume line, redraws only itself
pub struct MusicGlobalPlayerVolume;

impl MusicGlobalPlayerVolume {
    fn set_from(x: f32, view: &Entity<Self>, cx: &mut App) {
        MusicOutputVolume::set_level(x / VOLUME_WIDTH, cx);
        view.update(cx, |_, cx| cx.notify());
    }
}

impl Render for MusicGlobalPlayerVolume {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let level = MusicOutputVolume::level();
        let is_muted = MusicOutputVolume::is_muted();
        let speaker = if is_muted {
            "speaker.slash.fill"
        } else if level < 0.5 {
            "speaker.wave.2.fill"
        } else {
            "speaker.wave.3.fill"
        };
        let view = cx.entity();
        let muted_view = view.clone();
        let dragged_view = view.clone();
        let mute = icon_button(
            "global-player-mute",
            tr(if is_muted { "feed_sound_unmute" } else { "feed_sound_mute" }),
            div().children(glyph(speaker, 16.0, theme.text.secondary)),
            true,
            move |_, cx| {
                MusicOutputVolume::toggle_mute(cx);
                muted_view.update(cx, |_, cx| cx.notify());
            },
        );
        let line = div()
            .id("global-player-volume")
            .relative()
            .flex_none()
            .w(px(VOLUME_WIDTH))
            .h(px(LINE_ROW_HEIGHT))
            .cursor_pointer()
            .role(gpui::Role::Slider)
            .aria_label(SharedString::from(tr("editor_volume_music")))
            .aria_value(SharedString::from(format!("{}%", (level * 100.0).round() as i32)))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px((LINE_ROW_HEIGHT - LINE_HEIGHT) / 2.0))
                    .h(px(LINE_HEIGHT))
                    .rounded_full()
                    .bg(LiquidChrome::surface_fill(&theme, 1)),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px((LINE_ROW_HEIGHT - LINE_HEIGHT) / 2.0))
                    .w(px(VOLUME_WIDTH * level))
                    .h(px(LINE_HEIGHT))
                    .rounded_full()
                    .bg(theme.text.primary),
            )
            .gesture(
                "global-player-volume-drag",
                DragGesture::new(0.0)
                    .on_changed(move |value, _, cx| Self::set_from(f32::from(value.location.x), &dragged_view, cx))
                    .on_ended(move |value, _, cx| Self::set_from(f32::from(value.location.x), &view, cx)),
            );
        div().size_full().flex().flex_row().items_center().gap(px(4.0)).child(mute).child(line)
    }
}
