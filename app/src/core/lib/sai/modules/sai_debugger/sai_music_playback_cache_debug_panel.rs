#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, ListAlignment, ListState, SharedString, Subscription, Window, list,
    rgb, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::host_os::HostOs;
use rise_platform::media_keys::{Artwork, CommandOutcome, MediaCommand, MediaSnapshot};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text_selection::selectable_text;
use rise_ui::swiftui::transition::Presence;

use crate::core::lib::rise_call_audio_ownership::RiseCallAudioOwnership;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_debugger::sai_debug_storage_panel_views::SaiDebugBodyFormat;
use crate::core::lib::sai::modules::sai_debugger::sai_music_audio_interruption_probe::SaiMusicAudioInterruptionProbe;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::{
    MusicPlaybackDiskCache, MusicPlaybackDiskCacheListedEntry,
};
use crate::modules::music::shared::utils::music::music_player_state::{
    MusicPlayerState, MusicPlayerStateRemoteTransportCommand,
};

const IS_DEBUG_BUILD: bool = cfg!(debug_assertions);

const DESTRUCTIVE_RED: u32 = 0xe11d48;
const LINK_BLUE: u32 = 0x29b6f6;
const AUDIO_BADGE: u32 = 0x6a1b9a;
const FORMAT_BADGE: u32 = 0x00897b;
const SIZE_BADGE: u32 = 0x3949ab;
const UNAVAILABLE: &str = "unavailable";
const NOT_APPLICABLE: &str = "n/a";

type DeleteAction = Rc<dyn Fn(String, &mut Window, &mut App)>;
type ClearAllAction = Rc<dyn Fn(&mut Window, &mut App)>;

struct SaiMusicPlaybackCacheDebugPanelState {
    player: Entity<MusicPlayerState>,
    entries: Rc<Vec<MusicPlaybackDiskCacheListedEntry>>,
    entries_tick: Option<i64>,
    list: ListState,
    interruption_probe: SaiMusicAudioInterruptionProbe,
    remote_command_result: Option<String>,
    inspector_expanded: bool,
    inspector_presence: Presence<()>,
    inspector_chevron: Animated<f32>,
    _player_changes: Subscription,
}

impl SaiMusicPlaybackCacheDebugPanelState {
    fn new(cx: &mut Context<Self>) -> Self {
        let player = MusicPlayerState::shared(cx);
        let player_changes = cx.observe(&player, |_, _, cx| cx.notify());
        Self {
            player,
            entries: Rc::new(Vec::new()),
            entries_tick: None,
            list: ListState::new(0, ListAlignment::Top, px(200.)),
            interruption_probe: SaiMusicAudioInterruptionProbe::default(),
            remote_command_result: None,
            inspector_expanded: false,
            inspector_presence: Presence::new(None),
            inspector_chevron: Animated::new(0.),
            _player_changes: player_changes,
        }
    }

    fn sync_entries(&mut self, tick: i64) {
        if self.entries_tick == Some(tick) {
            return;
        }
        let previous_count = self.item_count();
        self.entries_tick = Some(tick);
        let user_id = SaiAccountCacheScope::active_user_id();
        self.entries = Rc::new(MusicPlaybackDiskCache::list_entries(user_id.as_deref()));
        self.list.splice(0..previous_count, self.item_count());
    }

    /// the cards or the empty line, then the inspector row of a debug build
    fn item_count(&self) -> usize {
        self.entries.len().max(1) + usize::from(IS_DEBUG_BUILD)
    }

    fn can_begin_interruption(&self, cx: &App) -> bool {
        let player = self.player.read(cx);
        !self.interruption_probe.is_active()
            && player.current_track.is_some()
            && player.is_playing
            && !player.is_preparing_playback
            && player.listening_session().is_none()
            && !player.is_listening_session_transitioning()
            && !RiseCallAudioOwnership::is_call_active()
    }
}

fn end_interruption(state: &Entity<SaiMusicPlaybackCacheDebugPanelState>, allow_resume: bool, cx: &mut App) {
    if !IS_DEBUG_BUILD {
        return;
    }
    let player = state.read(cx).player.clone();
    let (track_id, allow) = {
        let player = player.read(cx);
        (
            player.current_track.as_ref().map(|track| track.id.clone()),
            allow_resume
                && player.listening_session().is_none()
                && !player.is_listening_session_transitioning()
                && !RiseCallAudioOwnership::is_call_active(),
        )
    };
    let user_id = SaiAccountCacheScope::active_user_id();
    let epoch = SaiAccountCacheScope::active_epoch();
    state.update(cx, |state, cx| {
        state.interruption_probe.end(user_id.as_deref(), epoch.as_deref(), track_id.as_deref(), allow);
        cx.notify();
    });
}

fn element_id(base: &ElementId, part: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("{base}-{part}")))
}

fn now_playing_row(label: &'static str, value: String) -> Div {
    hstack(8.)
        .align(VerticalAlignment::FirstTextBaseline)
        .w_full()
        .child(div().child(label))
        .child(spacer_min_w(8.))
        .child(div().child(value).swiftui_font(Font::caption2().monospaced()))
}

fn seconds_value(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64()
}

#[derive(IntoElement)]
pub struct SaiMusicPlaybackCacheDebugPanel {
    id: ElementId,
    theme: AppTheme,
    snapshot_tick: Binding<i64>,
    on_request_delete: DeleteAction,
    on_request_clear_all: ClearAllAction,
}

impl SaiMusicPlaybackCacheDebugPanel {
    pub fn new(
        id: impl Into<ElementId>,
        theme: AppTheme,
        snapshot_tick: Binding<i64>,
        on_request_delete: impl Fn(String, &mut Window, &mut App) + 'static,
        on_request_clear_all: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            theme,
            snapshot_tick,
            on_request_delete: Rc::new(on_request_delete),
            on_request_clear_all: Rc::new(on_request_clear_all),
        }
    }

    fn header(&self, entries_count: usize) -> Div {
        let theme = &self.theme;
        let on_clear_all = self.on_request_clear_all.clone();
        let clear_all = (entries_count > 0).then(|| {
            plain(element_id(&self.id, "clear-all"))
                .on_press(move |window, cx| on_clear_all(window, cx))
                .child(
                    div()
                        .child("Clear all")
                        .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                        .text_color(rgb(DESTRUCTIVE_RED)),
                )
        });
        let tick = self.snapshot_tick.clone();
        hstack(None)
            .w_full()
            .child(
                div()
                    .child("Music")
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                    .text_color(theme.text.secondary),
            )
            .child(
                div()
                    .child(format!("({entries_count})"))
                    .swiftui_font(Font::caption2().monospaced_digit())
                    .text_color(theme.text.secondary.opacity(0.75)),
            )
            .child(spacer_min_w(0.))
            .children(clear_all)
            .child(
                plain(element_id(&self.id, "refresh"))
                    .on_press(move |window, cx| {
                        let next = tick.get(cx).wrapping_add(1);
                        tick.set(next, window, cx);
                    })
                    .child(
                        div()
                            .child("Refresh")
                            .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD))
                            .text_color(rgb(LINK_BLUE)),
                    ),
            )
            .px(px(14.))
            .py(px(8.))
            .bg(theme.bg._300.opacity(0.35))
    }

    fn path_line(&self) -> Div {
        let user_id = SaiAccountCacheScope::active_user_id();
        div()
            .child(MusicPlaybackDiskCache::folder_path_for_inspector(user_id.as_deref()))
            .swiftui_font(Font::caption2())
            .text_color(self.theme.text.secondary.opacity(0.78))
            .text_center()
            .line_clamp(3)
            .w_full()
            .px(px(12.))
            .pb(px(6.))
    }

    fn interruption_controls(&self, state: &Entity<SaiMusicPlaybackCacheDebugPanelState>, cx: &App) -> Div {
        let theme = &self.theme;
        let current = state.read(cx);
        let can_begin = current.can_begin_interruption(cx);
        let is_active = current.interruption_probe.is_active();
        let tint = theme.text.secondary;
        let begin_state = state.clone();
        let paused_state = state.clone();
        let resume_state = state.clone();

        vstack(8.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .text_color(theme.text.secondary)
            .child(
                div()
                    .child("Audio interruption probe")
                    .swiftui_font(Font::caption().weight(FontWeight::SEMIBOLD)),
            )
            .child(
                div()
                    .child(format!("Session: {NOT_APPLICABLE} \u{b7} mixing: {NOT_APPLICABLE}"))
                    .swiftui_font(Font::caption2().monospaced()),
            )
            .child(
                div()
                    .child(if is_active {
                        "Synthetic begin dispatched"
                    } else {
                        "Local playback only; closing ends without resume"
                    })
                    .swiftui_font(Font::caption2()),
            )
            .child(
                system_button(element_id(&self.id, "interruption-begin"), BorderedStyle::Bordered)
                    .tint(tint)
                    .disabled(!can_begin)
                    .on_press(move |_window, cx| {
                        let isolated = begin_state.read(cx).can_begin_interruption(cx);
                        let track_id = begin_state
                            .read(cx)
                            .player
                            .read(cx)
                            .current_track
                            .as_ref()
                            .map(|track| track.id.clone());
                        let user_id = SaiAccountCacheScope::active_user_id();
                        let epoch = SaiAccountCacheScope::active_epoch();
                        begin_state.update(cx, |state, cx| {
                            state.interruption_probe.begin(
                                user_id.as_deref(),
                                epoch.as_deref(),
                                track_id.as_deref(),
                                isolated,
                            );
                            cx.notify();
                        });
                    })
                    .child("Begin interruption"),
            )
            .child(
                hstack(12.)
                    .child(
                        system_button(element_id(&self.id, "interruption-end-paused"), BorderedStyle::Bordered)
                            .tint(tint)
                            .disabled(!is_active)
                            .on_press(move |_window, cx| end_interruption(&paused_state, false, cx))
                            .child("End \u{b7} keep paused"),
                    )
                    .child(
                        system_button(element_id(&self.id, "interruption-end-resume"), BorderedStyle::Bordered)
                            .tint(tint)
                            .disabled(!is_active)
                            .on_press(move |_window, cx| end_interruption(&resume_state, true, cx))
                            .child("End \u{b7} allow resume"),
                    ),
            )
            .p(px(12.))
            .bg(theme.bg._300.opacity(0.35))
    }
}

#[derive(Clone)]
struct NowPlayingInspector {
    id: ElementId,
    theme: AppTheme,
    snapshot_tick: Binding<i64>,
    state: Entity<SaiMusicPlaybackCacheDebugPanelState>,
}

impl NowPlayingInspector {
    fn render(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let current = self.state.read(cx);
        let player = current.player.read(cx);
        let track = player.current_track.clone();
        let can_inspect = SaiAccountCacheScope::active_user_id().is_some()
            && SaiAccountCacheScope::active_epoch().is_some()
            && track.is_some()
            && !RiseCallAudioOwnership::is_call_active();
        let snapshot = if can_inspect { player.published_now_playing() } else { None };
        let result = current.remote_command_result.clone();
        let presence = current.inspector_presence.clone();
        let chevron = current.inspector_chevron.clone();
        let theme = &self.theme;

        let now = RiseClock::monotonic();
        let chevron_degrees = chevron.frame_value(now, window);
        let toggle_state = self.state.clone();
        let label = plain(element_id(&self.id, "inspector-toggle"))
            .w_full()
            .on_press(move |_window, cx| {
                toggle_state.update(cx, |state, cx| {
                    state.inspector_expanded = !state.inspector_expanded;
                    let now = RiseClock::monotonic();
                    let is_open = state.inspector_expanded;
                    state.inspector_presence.set_in(is_open.then_some(()), ease_in_out(0.25), now, cx);
                    state.inspector_chevron.set_in(if is_open { 90. } else { 0. }, ease_in_out(0.25), now, cx);
                    cx.notify();
                });
            })
            .child(
                hstack(8.)
                    .w_full()
                    .child(div().child("Published Now Playing metadata"))
                    .child(spacer_min_w(0.))
                    .child(
                        div()
                            .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(11.), theme.text.secondary, false))
                            .rotation_effect(chevron_degrees),
                    ),
            );

        let content = presence.render_transitioned(now, window, |_| {
            self.metadata_rows(snapshot, track.as_ref().map(|track| (track.title.as_str(), track.artist.as_str())), result, cx)
        });

        vstack(8.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .swiftui_font(Font::caption())
            .text_color(theme.text.secondary)
            .child(label)
            .children(content)
            .p(px(12.))
            .bg(theme.bg._300.opacity(0.35))
            .into_any_element()
    }

    fn metadata_rows(
        &self,
        snapshot: Option<MediaSnapshot>,
        track: Option<(&str, &str)>,
        result: Option<String>,
        cx: &App,
    ) -> Div {
        let host = HostOs::current();
        let enabled = |command: MediaCommand| if command.is_deliverable_on(host) { "on" } else { "off" };
        let published = snapshot.as_ref().and_then(|snapshot| snapshot.now.as_ref());
        let state = snapshot.as_ref().map(|snapshot| snapshot.state);
        let duration = published.map(|info| seconds_value(info.duration)).filter(|seconds| *seconds > 0.);
        let artwork = match published.and_then(|info| info.artwork.as_ref()) {
            Some(Artwork::File(path)) => match image::image_dimensions(path) {
                Ok((width, height)) => format!(
                    "{} \u{d7} {}",
                    SaiMusicNowPlayingDebugValue::number(Some(f64::from(width))),
                    SaiMusicNowPlayingDebugValue::number(Some(f64::from(height)))
                ),
                Err(_) => format!("unreadable file {}", path.display()),
            },
            Some(Artwork::Url(_)) => NOT_APPLICABLE.to_owned(),
            None => UNAVAILABLE.to_owned(),
        };
        let player = self.state.read(cx).player.read(cx);
        let commands_disabled = player.current_track.is_none() || RiseCallAudioOwnership::is_call_active();

        vstack(6.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(
                div()
                    .child(if published.is_none() {
                        "No active metadata"
                    } else {
                        "Refresh reads raw published values; elapsed is not extrapolated"
                    })
                    .swiftui_font(Font::caption2()),
            )
            .child(now_playing_row(
                "Title",
                SaiMusicNowPlayingDebugValue::match_value(
                    published.map(|info| info.title.as_str()),
                    track.map(|track| track.0),
                ),
            ))
            .child(now_playing_row(
                "Artist",
                SaiMusicNowPlayingDebugValue::match_value(
                    published.map(|info| info.artist.as_str()),
                    track.map(|track| track.1),
                ),
            ))
            .child(now_playing_row(
                "Elapsed \u{b7} seconds",
                SaiMusicNowPlayingDebugValue::number(published.map(|info| seconds_value(info.elapsed))),
            ))
            .child(now_playing_row("Duration \u{b7} seconds", SaiMusicNowPlayingDebugValue::number(duration)))
            .child(now_playing_row(
                "Playback rate",
                SaiMusicNowPlayingDebugValue::number(
                    published.zip(state).map(|(info, state)| state.effective_rate(info.rate)),
                ),
            ))
            .child(now_playing_row(
                "Default rate",
                SaiMusicNowPlayingDebugValue::number(published.map(|info| info.rate)),
            ))
            .child(now_playing_row("Artwork \u{b7} width \u{d7} height", artwork))
            .child(
                div()
                    .child(format!(
                        "Command availability \u{b7} play: {} \u{b7} pause: {} \u{b7} seek: {}",
                        enabled(MediaCommand::Play),
                        enabled(MediaCommand::Pause),
                        enabled(MediaCommand::Seek),
                    ))
                    .swiftui_font(Font::caption2()),
            )
            .child(
                div()
                    .child("Remote commands run the same handler the media keys call.")
                    .swiftui_font(Font::caption2()),
            )
            .child(
                hstack(8.)
                    .child(self.remote_command_button("Play", MusicPlayerStateRemoteTransportCommand::Play, "play", commands_disabled))
                    .child(self.remote_command_button("Pause", MusicPlayerStateRemoteTransportCommand::Pause, "pause", commands_disabled))
                    .child(self.remote_command_button(
                        "Toggle",
                        MusicPlayerStateRemoteTransportCommand::TogglePlayPause,
                        "toggle",
                        commands_disabled,
                    )),
            )
            .child(
                hstack(8.)
                    .child(self.remote_command_button(
                        "Seek 0:10",
                        MusicPlayerStateRemoteTransportCommand::ChangePosition(10.),
                        "seek10",
                        commands_disabled,
                    ))
                    .child(self.remote_command_button(
                        "Seek 0:30",
                        MusicPlayerStateRemoteTransportCommand::ChangePosition(30.),
                        "seek30",
                        commands_disabled,
                    ))
                    .child(self.remote_command_button("Next", MusicPlayerStateRemoteTransportCommand::Next, "next", commands_disabled)),
            )
            .children(result.map(|result| {
                div()
                    .child(format!("Last command: {result}"))
                    .swiftui_font(Font::caption2().monospaced())
            }))
    }

    fn remote_command_button(
        &self,
        title: &'static str,
        command: MusicPlayerStateRemoteTransportCommand,
        id: &str,
        disabled: bool,
    ) -> SystemButton {
        let state = self.state.clone();
        let tick = self.snapshot_tick.clone();
        system_button(element_id(&self.id, &format!("command-{id}")), BorderedStyle::Bordered)
            .tint(self.theme.text.secondary)
            .disabled(disabled)
            .on_press(move |window, cx| {
                let player = state.read(cx).player.clone();
                let outcome = player.update(cx, |player, cx| player.handle_remote_command(command, cx));
                let status = if outcome == CommandOutcome::Handled { "success" } else { "failed" };
                state.update(cx, |state, cx| {
                    state.remote_command_result = Some(format!("{title} \u{b7} {status}"));
                    cx.notify();
                });
                let next = tick.get(cx).wrapping_add(1);
                tick.set(next, window, cx);
            })
            .child(title)
    }
}

impl RenderOnce for SaiMusicPlaybackCacheDebugPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(element_id(&self.id, "state"), cx, |_, cx| {
            SaiMusicPlaybackCacheDebugPanelState::new(cx)
        });
        let tick = self.snapshot_tick.get(cx);
        state.update(cx, |state, _| state.sync_entries(tick));

        let current_track_id = {
            let player = state.read(cx).player.read(cx);
            player.current_track.as_ref().map(|track| track.id.clone())
        };
        if IS_DEBUG_BUILD {
            let changed_state = state.clone();
            on_change(element_id(&self.id, "track"), current_track_id, false, window, cx, move |_, _, _, cx| {
                end_interruption(&changed_state, false, cx)
            });
        }

        let (entries, list_state) = {
            let current = state.read(cx);
            (current.entries.clone(), current.list.clone())
        };
        let header = self.header(entries.len());
        let interruption = IS_DEBUG_BUILD.then(|| self.interruption_controls(&state, cx));
        let path = self.path_line();

        let theme = self.theme.clone();
        let inspector = NowPlayingInspector {
            id: self.id.clone(),
            theme: theme.clone(),
            snapshot_tick: self.snapshot_tick.clone(),
            state: state.clone(),
        };
        let on_delete = self.on_request_delete.clone();
        let base_id = self.id.clone();
        let row_theme = theme.clone();
        let rows = list(list_state, move |index, window, cx| {
            let row_count = entries.len().max(1) + usize::from(IS_DEBUG_BUILD);
            let row = if index < entries.len() {
                let entry = entries[index].clone();
                let cache_key = entry.cache_key().to_owned();
                let track_id = entry.track_id.clone();
                let on_delete = on_delete.clone();
                SaiMusicPlaybackCacheDebugCardView {
                    id: element_id(&base_id, &format!("card-{}", entry.id)),
                    entry,
                    theme: row_theme.clone(),
                    on_copy: Rc::new(move |_window: &mut Window, cx: &mut App| UIPasteboard::general().set_string(&cache_key, cx)),
                    on_delete: Rc::new(move |window: &mut Window, cx: &mut App| on_delete(track_id.clone(), window, cx)),
                }
                .into_any_element()
            } else if index == 0 && entries.is_empty() {
                div()
                    .child("No cached tracks yet. Play a track with a remote audio URL to populate this folder.")
                    .swiftui_font(Font::caption())
                    .text_color(row_theme.text.secondary)
                    .text_center()
                    .w_full()
                    .pt(px(28.))
                    .into_any_element()
            } else {
                inspector.render(window, cx)
            };
            div()
                .w_full()
                .px(px(14.))
                .pt(px(14.))
                .when(index + 1 == row_count, |row| row.pb(px(14.)))
                .child(row)
                .into_any_element()
        })
        .size_full();

        let disappear_state = state.clone();
        vstack(0.)
            .w_full()
            .child(header)
            .children(interruption)
            .child(path)
            .child(div().w_full().h(px(500.)).bg(theme.bg._100).child(rows))
            .on_disappear(element_id(&self.id, "disappear"), move |_window, cx| {
                end_interruption(&disappear_state, false, cx)
            })
    }
}

#[derive(IntoElement)]
struct SaiMusicPlaybackCacheDebugCardView {
    id: ElementId,
    entry: MusicPlaybackDiskCacheListedEntry,
    theme: AppTheme,
    on_copy: ClearAllAction,
    on_delete: ClearAllAction,
}

impl SaiMusicPlaybackCacheDebugCardView {
    fn badge(text: impl Into<SharedString>, fill: u32) -> Div {
        div()
            .child(text.into())
            .swiftui_font(Font::caption2().weight(FontWeight::EXTRA_BOLD))
            .text_color(white())
            .px(px(7.))
            .py(px(3.))
            .bg(rgb(fill))
            .capsule()
    }

    fn icon_button(&self, part: &str, symbol: &str, color: gpui::Hsla, action: ClearAllAction) -> PlainButton {
        plain(element_id(&self.id, part)).on_press(move |window, cx| action(window, cx)).child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .frame_size(36., 32.)
                .bg(self.theme.bg._300.opacity(0.9))
                .clip_shape(Shape::continuous(8.))
                .children(IconUi::sized(symbol, px(15.), color, false)),
        )
    }
}

impl RenderOnce for SaiMusicPlaybackCacheDebugCardView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = &self.theme;
        let entry = &self.entry;
        let body = vstack(0.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .bg(theme.bg._200.opacity(0.92))
            .clip_shape(Shape::continuous(12.))
            .child(
                hstack(6.)
                    .w_full()
                    .child(Self::badge("AUDIO", AUDIO_BADGE))
                    .child(Self::badge(entry.format_label.to_uppercase(), FORMAT_BADGE))
                    .child(Self::badge(SaiDebugBodyFormat::size_label_byte_count(entry.file_bytes), SIZE_BADGE))
                    .child(spacer_min_w(0.))
                    .child(self.icon_button("delete", "trash", rgb(DESTRUCTIVE_RED).into(), self.on_delete.clone()))
                    .child(self.icon_button("copy", "doc.on.doc", theme.text.primary, self.on_copy.clone()))
                    .px(px(10.))
                    .py(px(8.))
                    .bg(theme.bg._300.opacity(0.45)),
            )
            .child(div().w_full().h(px(1.)).bg(theme.border._200))
            .child(
                vstack(6.)
                    .align(HorizontalAlignment::Leading)
                    .w_full()
                    .child(
                        div()
                            .child("TRACK ID")
                            .swiftui_font(Font::system(10., FontWeight::EXTRA_BOLD, FontDesign::Default))
                            .text_color(theme.text.secondary.opacity(0.85)),
                    )
                    .child(
                        div()
                            .w_full()
                            .swiftui_font(Font::system(11., FontWeight::NORMAL, FontDesign::Monospaced))
                            .text_color(theme.text.primary)
                            .line_clamp(3)
                            .child(selectable_text(element_id(&self.id, "track-id"), entry.track_id.clone())),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.))
                            .child(entry.file_name.clone())
                            .swiftui_font(Font::caption2())
                            .text_color(theme.text.secondary.opacity(0.75))
                            .line_clamp(1)
                            .text_ellipsis_middle(),
                    )
                    .p(px(10.))
                    .bg(theme.bg._100),
            );

        div()
            .relative()
            .w_full()
            .child(body)
            .child(centred_stroke(theme.border._200.opacity(0.65), 1., Shape::continuous(12.)))
    }
}

/// now playing debug value, the seam hands typed values so the any and number check become an option
pub enum SaiMusicNowPlayingDebugValue {}

impl SaiMusicNowPlayingDebugValue {
    pub fn number(value: Option<f64>) -> String {
        match value {
            Some(value) if value.is_finite() => format!("{value:.3}"),
            _ => UNAVAILABLE.to_owned(),
        }
    }

    /// match for a value and expected, match is a rust keyword
    pub fn match_value(value: Option<&str>, expected: Option<&str>) -> String {
        match (value, expected) {
            (Some(value), Some(expected)) if !value.is_empty() && !expected.is_empty() => {
                if value == expected {
                    "matches current track".to_owned()
                } else {
                    "differs from current track".to_owned()
                }
            }
            _ => UNAVAILABLE.to_owned(),
        }
    }
}
