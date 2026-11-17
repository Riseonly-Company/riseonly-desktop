use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, FontWeight, Hsla, ScrollStrategy, SharedString, Svg, UniformListScrollHandle, Window,
    black, point, rgb, uniform_list, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::modifiers::ModifierExt;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::pages::now_playing::music_live_playback_seek_bar::MusicLivePlaybackSeekBar;
use crate::modules::music::pages::now_playing::now_playing::MusicListeningControlAlert;
use crate::modules::music::pages::playlist_detail::music_playlist_listener_cluster::MusicPlaylistListenerCluster;
use crate::modules::music::pages::track::track_row::PlayingWaveform;
use crate::modules::music::shared::utils::music::music_models::{MusicRepeatMode, MusicRepeatModeExt, TrackModel};
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_types::MusicPlaylistListener;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::music_mini_player_bottom_padding::observed_music_player;
use crate::modules::music::widgets::music_player_quick_actions::MusicPlayerQuickActions;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

const ACCENT_COLOR: u32 = 0xE84393;
const QUEUE_ROW_HEIGHT: f32 = 68.5;
const MINIMUM_VISIBLE_QUEUE_HEIGHT: f32 = 2. * QUEUE_ROW_HEIGHT;
const EMPTY_QUEUE_HEIGHT: f32 = 84.;
const QUEUE_COVER_SIDE: f32 = 48.;
const QUEUE_COVER_RADIUS: f32 = 8.;

type OnMinimumHeight = Rc<dyn Fn(f32, &mut Window, &mut App)>;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

fn icon(name: &str, size: f32, color: Hsla) -> Option<Svg> {
    IconUi::sized(name, px(size), color, name.ends_with(".fill"))
}

fn accent() -> Hsla {
    Hsla::from(rgb(ACCENT_COLOR))
}

/// music right panel follows the app theme
/// the sheet and panel inside the full player keep the dark player surface
#[derive(Clone, Copy)]
struct QueueColors {
    text: Hsla,
    secondary: Hsla,
    hairline: Hsla,
    background: Hsla,
}

impl QueueColors {
    fn resolve(follows_theme: bool, theme: &AppTheme) -> Self {
        if follows_theme {
            Self {
                text: theme.text.primary,
                secondary: theme.text.secondary,
                hairline: theme.border._100,
                background: theme.bg._100,
            }
        } else {
            Self { text: white(), secondary: white().opacity(0.7), hairline: white().opacity(0.12), background: black() }
        }
    }
}

struct MusicQueueSheetState {
    header_height: f32,
    controls_height: f32,
    available_height: f32,
    list_handle: UniformListScrollHandle,
    scroll_motion: Animated<f32>,
    is_scroll_moving: bool,
}

impl MusicQueueSheetState {
    fn new() -> Self {
        Self {
            header_height: 0.,
            controls_height: 0.,
            available_height: 0.,
            list_handle: UniformListScrollHandle::new(),
            scroll_motion: Animated::new(0.),
            is_scroll_moving: false,
        }
    }

    /// writes the running animation into the scroll, called from render before prepaint reads the offset
    fn scroll_frame(&mut self, window: &Window) {
        if !self.is_scroll_moving {
            return;
        }
        let now = RiseClock::monotonic();
        let top = self.scroll_motion.frame_value(now, window);
        self.list_handle.0.borrow().base_handle.set_offset(point(px(0.), px(-top)));
        if !self.scroll_motion.is_running(now) {
            self.is_scroll_moving = false;
        }
    }

    fn scroll_to(&mut self, index: usize, animated: bool, cx: &mut Context<Self>) {
        if !animated {
            self.is_scroll_moving = false;
            self.list_handle.scroll_to_item(index, ScrollStrategy::Top);
            cx.notify();
            return;
        }
        let (top, max_top) = {
            let state = self.list_handle.0.borrow();
            (-f32::from(state.base_handle.offset().y), f32::from(state.base_handle.max_offset().y))
        };
        self.scroll_motion.jump(top);
        let target = (index as f32 * QUEUE_ROW_HEIGHT).min(max_top);
        self.scroll_motion.set_in(target, ease_in_out(0.25), RiseClock::monotonic(), cx);
        self.is_scroll_moving = true;
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct MusicQueueSheet {
    id: ElementId,
    on_minimum_height: OnMinimumHeight,
    shows_controls: bool,
    follows_theme: bool,
}

#[derive(Clone)]
struct MusicQueueSheetParts {
    id: ElementId,
    player: Entity<MusicPlayerState>,
    state: Entity<MusicQueueSheetState>,
    on_minimum_height: OnMinimumHeight,
    shows_controls: bool,
    colors: QueueColors,
}

impl MusicQueueSheet {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), on_minimum_height: Rc::new(|_, _, _| {}), shows_controls: true, follows_theme: false }
    }

    /// desktop panel beside the center takes the app theme instead of the dark player one
    pub fn follows_theme(mut self) -> Self {
        self.follows_theme = true;
        self
    }

    /// queue list alone, for a panel next to a player that already has controls
    pub fn list_only(mut self) -> Self {
        self.shows_controls = false;
        self
    }

    pub fn on_minimum_height(mut self, on_minimum_height: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_minimum_height = Rc::new(on_minimum_height);
        self
    }
}

impl MusicQueueSheetParts {
    fn child(&self, name: &'static str) -> ElementId {
        child_id(&self.id, name)
    }

    fn report_minimum_height(&self, window: &mut Window, cx: &mut App) {
        let (header_height, controls_height) = {
            let state = self.state.read(cx);
            (state.header_height, state.controls_height)
        };
        if header_height <= 0. {
            return;
        }
        let (is_empty, has_track) = {
            let player = self.player.read(cx);
            (player.queue.is_empty(), player.current_track.is_some())
        };
        let list = if is_empty { EMPTY_QUEUE_HEIGHT } else { MINIMUM_VISIBLE_QUEUE_HEIGHT };
        let controls = if has_track && self.shows_controls { controls_height } else { 0. };
        (self.on_minimum_height)(header_height + list + controls, window, cx);
    }

    fn is_compact(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        let player = self.player.read(cx);
        if state.available_height <= 0. || state.header_height <= 0. || player.queue.is_empty() {
            return false;
        }
        let controls = if player.current_track.is_none() || !self.shows_controls { 0. } else { state.controls_height };
        state.available_height < state.header_height + MINIMUM_VISIBLE_QUEUE_HEIGHT + controls
    }

    fn scroll_to_current_track(player: &Entity<MusicPlayerState>, state: &Entity<MusicQueueSheetState>, animated: bool, cx: &mut App) {
        let index = {
            let player = player.read(cx);
            let Some(id) = player.current_track.as_ref().map(|track| track.id.as_str()) else {
                return;
            };
            player.queue.iter().position(|track| track.id == id)
        };
        let Some(index) = index else {
            return;
        };
        state.update(cx, |state, cx| state.scroll_to(index, animated, cx));
    }

    fn listeners_by_track(&self, cx: &mut App) -> HashMap<String, Vec<MusicPlaylistListener>> {
        let Some(playlist_id) = self.player.read(cx).listening_playlist_id().map(str::to_owned) else {
            return HashMap::new();
        };
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let listeners = MusicServicesStore::shared(cx)
            .update(cx, |services, cx| services.playlist_listening_snapshot(&playlist_id, None, cx))
            .map(|snapshot| snapshot.listeners)
            .unwrap_or_default();
        let mut grouped: HashMap<String, Vec<MusicPlaylistListener>> = HashMap::new();
        for listener in listeners {
            if Some(listener.user_id.as_str()) != current_user_id.as_deref() {
                grouped.entry(listener.track_id.clone()).or_default().push(listener);
            }
        }
        grouped
    }

    fn header(&self, is_shuffled: bool) -> impl IntoElement {
        let measured = self.clone();
        div()
            .relative()
            .w_full()
            .flex_none()
            .child(
                hstack(None::<f32>)
                    .w_full()
                    .px(px(20.))
                    .pt(px(8.))
                    .pb(px(12.))
                    .child(MainText::new(tr("music_queue_title")).px(18.).font_weight(FontWeight::BOLD).color(self.colors.text))
                    .child(spacer())
                    .when(is_shuffled, |row| {
                        row.child(
                            hstack(4.)
                                .children(icon("shuffle", 12., self.colors.secondary))
                                .child(SecondaryText::new(tr("music_queue_shuffled")).px(12.).color(self.colors.secondary)),
                        )
                    }),
            )
            .child(measure(self.child("header-height"), move |bounds, window, cx| {
                let height = f32::from(bounds.size.height);
                measured.state.update(cx, |state, cx| {
                    state.header_height = height;
                    cx.notify();
                });
                measured.report_minimum_height(window, cx);
            }))
    }

    fn queue_list(&self, compact: bool, listeners: Rc<HashMap<String, Vec<MusicPlaylistListener>>>, cx: &mut App) -> impl IntoElement {
        let queue_count = self.player.read(cx).queue.len();
        let list_handle = self.state.read(cx).list_handle.clone();
        let rows = self.clone();
        let appear_player = self.player.clone();
        let appear_state = self.state.clone();
        div()
            .w_full()
            .when(compact, |list| list.h(px(MINIMUM_VISIBLE_QUEUE_HEIGHT)).flex_none())
            .when(!compact, |list| list.flex_1().min_h_0())
            .child(
                uniform_list(self.child("rows"), queue_count, move |range, _, cx| {
                    range
                        .filter_map(|index| {
                            let track = rows.player.read(cx).queue.get(index).cloned()?;
                            let track_listeners = listeners.get(&track.id).cloned().unwrap_or_default();
                            Some(rows.queue_row(track, index, queue_count, track_listeners, cx))
                        })
                        .collect::<Vec<_>>()
                })
                .track_scroll(&list_handle)
                .size_full()
                .pb(px(24.)),
            )
            .on_appear(self.child("list-appear"), move |_, cx| {
                Self::scroll_to_current_track(&appear_player, &appear_state, false, cx);
            })
    }

    fn queue_row(
        &self,
        track: TrackModel,
        index: usize,
        queue_count: usize,
        listeners: Vec<MusicPlaylistListener>,
        cx: &App,
    ) -> AnyElement {
        let theme = rise_ui::theme(cx);
        let (is_current, is_active) = {
            let player = self.player.read(cx);
            let is_current = player.current_track.as_ref().is_some_and(|current| current.id == track.id);
            (is_current, is_current && (player.is_playing || player.is_preparing_playback))
        };
        let row_id = ElementId::NamedChild(Arc::new(self.child("row")), SharedString::from(track.id.clone()));
        let player = self.player.clone();

        vstack(0.)
            .w_full()
            .h(px(QUEUE_ROW_HEIGHT))
            .child(
                plain(row_id.clone())
                    .w_full()
                    .h(px(QUEUE_ROW_HEIGHT - 0.5))
                    .on_press(move |_, cx| {
                        player.update(cx, |player, cx| {
                            if is_current {
                                player.toggle_play_pause(cx);
                            } else {
                                player.play_queue_item(index, cx);
                            }
                        });
                    })
                    .child(self.queue_row_content(&track, &row_id, is_current, is_active, listeners, theme)),
            )
            .child(div().w_full().pl(px(76.)).child(div().w_full().h(px(0.5)).when(index + 1 < queue_count, |line| {
                line.bg(self.colors.hairline)
            })))
            .into_any_element()
    }

    fn queue_row_content(
        &self,
        track: &TrackModel,
        row_id: &ElementId,
        is_current: bool,
        is_active: bool,
        listeners: Vec<MusicPlaylistListener>,
        theme: &AppTheme,
    ) -> impl IntoElement {
        let primary = theme.primary._100;
        let cover = div()
            .relative()
            .size(px(QUEUE_COVER_SIDE))
            .flex_none()
            .child(
                div().absolute().inset_0().continuous_corners(QUEUE_COVER_RADIUS).overflow_hidden().child(
                    MusicCoverImage::new(child_id(row_id, "cover"), track.cover_image_url.clone())
                        .fallback_colors(track.cover_colors.clone())
                        .size(QUEUE_COVER_SIDE)
                        .corner_radius(QUEUE_COVER_RADIUS),
                ),
            )
            .when(is_active, |cover| {
                cover.child(
                    div()
                        .absolute()
                        .inset_0()
                        .continuous_corners(QUEUE_COVER_RADIUS)
                        .bg(black().opacity(0.35))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(PlayingWaveform::new(child_id(row_id, "waveform"))),
                )
            })
            .when(!listeners.is_empty(), |cover| {
                let total_count = listeners.len() as i64;
                cover.child(
                    div().absolute().top(px(-7.)).right(px(-7.)).child(
                        MusicPlaylistListenerCluster::new(child_id(row_id, "listeners"), listeners, total_count).size(25.),
                    ),
                )
            });

        hstack(12.)
            .w_full()
            .h_full()
            .px(px(16.))
            .py(px(10.))
            .child(cover)
            .child(
                vstack(2.)
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .child(
                        MainText::new(track.title.clone())
                            .px(15.)
                            .font_weight(if is_current { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                            .color(if is_current { primary } else { self.colors.text })
                            .line_limit(1),
                    )
                    .child(SecondaryText::new(track.artist.clone()).px(13.).color(self.colors.secondary).line_limit(1)),
            )
            .child(if is_current {
                SecondaryText::new(tr("music_queue_now")).px(12.).color(primary)
            } else {
                SecondaryText::new(track.formatted_duration()).px(12.).color(self.colors.secondary)
            })
    }

    fn transport_button(
        &self,
        name: &'static str,
        label: String,
        content: impl IntoElement,
        action: impl Fn(&mut MusicPlayerState, &mut Context<MusicPlayerState>) + 'static,
    ) -> impl IntoElement {
        let player = self.player.clone();
        div()
            .id(self.child(name))
            .flex_1()
            .flex()
            .justify_center()
            .aria_label(label.clone())
            .tooltip_label(label)
            .child(
                plain(child_id(&self.child(name), "press"))
                    .on_press(move |_, cx| player.update(cx, |player, cx| action(player, cx)))
                    .child(content),
            )
    }

    fn transport_row(&self, cx: &App) -> impl IntoElement {
        let (is_shuffled, is_playing, repeat_mode) = {
            let player = self.player.read(cx);
            (player.is_shuffled(cx), player.is_playing, player.repeat_mode(cx))
        };
        let inactive = white().opacity(0.7);

        hstack(0.)
            .w_full()
            .child(self.transport_button(
                "shuffle",
                tr("music_shuffle"),
                div().children(icon("shuffle", 19., if is_shuffled { accent() } else { inactive })),
                |player, cx| player.toggle_shuffle(cx),
            ))
            .child(self.transport_button(
                "previous",
                tr("music_player_previous"),
                div().children(icon("backward.fill", 21., white())),
                |player, cx| player.previous(cx),
            ))
            .child(self.transport_button(
                "play",
                tr(if is_playing { "media_viewer_pause" } else { "media_viewer_play" }),
                div()
                    .w(px(58.))
                    .h(px(46.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(icon(if is_playing { "pause.fill" } else { "play.fill" }, 27., white())),
                |player, cx| player.toggle_play_pause(cx),
            ))
            .child(self.transport_button(
                "next",
                tr("music_player_next"),
                div().children(icon("forward.fill", 21., white())),
                |player, cx| player.next(cx),
            ))
            .child(self.transport_button(
                "repeat",
                format!("{}: {}", tr("music_player_repeat"), repeat_mode.accessibility_value()),
                div().children(icon(
                    repeat_mode.icon(),
                    19.,
                    if repeat_mode != MusicRepeatMode::Off { accent() } else { inactive },
                )),
                |player, cx| player.cycle_repeat_mode(cx),
            ))
    }

    fn current_track_controls(&self, track: TrackModel, cx: &App) -> Div {
        let measured = self.clone();
        let share_player = self.player.clone();
        let share_track = track.clone();
        let profile_track = track.clone();
        let share_label = tr("music_now_playing_share");
        let playlist_id = self.player.read(cx).listening_playlist_id().map(str::to_owned);

        div()
            .relative()
            .w_full()
            .flex_none()
            .child(
                vstack(14.)
                    .w_full()
                    .px(px(18.))
                    .pb(px(16.))
                    .bg(black())
                    .child(div().w_full().h(px(0.5)).bg(white().opacity(0.12)))
                    .child(
                        hstack(12.)
                            .w_full()
                            .child(
                                MusicCoverImage::new(self.child("controls-cover"), track.cover_image_url.clone())
                                    .fallback_colors(track.cover_colors.clone())
                                    .size(54.)
                                    .corner_radius(12.),
                            )
                            .child(
                                vstack(2.)
                                    .flex_1()
                                    .min_w_0()
                                    .items_start()
                                    .child(
                                        MainText::new(track.title.clone())
                                            .px(16.)
                                            .font_weight(FontWeight::BOLD)
                                            .color(white())
                                            .number_of_lines(1),
                                    )
                                    .child(
                                        SecondaryText::new(track.artist.clone())
                                            .px(13.)
                                            .color(white().opacity(0.7))
                                            .line_limit(1),
                                    ),
                            )
                            .child(
                                div()
                                    .id(self.child("share-hint"))
                                    .aria_label(share_label.clone())
                                    .tooltip_label(share_label)
                                    .child(
                                        plain(self.child("share"))
                                            .size(px(38.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .on_press(move |_, cx| {
                                                let playlist_id =
                                                    share_player.read(cx).listening_playlist_id().map(str::to_owned);
                                                let track = share_track.clone();
                                                ShareSheetStore::shared(cx)
                                                    .update(cx, |store, cx| store.present_track(track, playlist_id, cx));
                                            })
                                            .children(icon("square.and.arrow.up", 19., white())),
                                    ),
                            ),
                    )
                    .child(
                        MusicLivePlaybackSeekBar::new(
                            self.child("seek-bar"),
                            self.player.clone(),
                            track.duration_seconds,
                        )
                        .playlist_id(playlist_id)
                        .horizontal_padding(14.),
                    )
                    .child(self.transport_row(cx))
                    .child(MusicPlayerQuickActions::new(self.child("quick-actions")).on_dark_background(true))
                    .when(track.transfer_phase.is_none(), |controls| {
                        controls.child(
                            ButtonUi::new(self.child("add-to-profile"))
                                .on_press(move |_, cx| {
                                    let track = profile_track.clone();
                                    MusicInteractionsStore::shared(cx)
                                        .update(cx, |interactions, cx| interactions.add_track_to_profile(track, cx));
                                })
                                .child(
                                    hstack(8.)
                                        .children(icon("person.crop.circle.badge.plus", 17., white()))
                                        .child(
                                            MainText::new(tr("music_add_to_profile"))
                                                .px(15.)
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .color(white()),
                                        ),
                                ),
                        )
                    }),
            )
            .child(measure(self.child("controls-height"), move |bounds, window, cx| {
                let height = f32::from(bounds.size.height);
                measured.state.update(cx, |state, cx| {
                    state.controls_height = height;
                    cx.notify();
                });
                measured.report_minimum_height(window, cx);
            }))
    }

    fn sheet_content(&self, compact: bool, cx: &mut App) -> impl IntoElement {
        let listeners = Rc::new(self.listeners_by_track(cx));
        let (is_shuffled, is_empty, current_track) = {
            let player = self.player.read(cx);
            (player.is_shuffled(cx), player.queue.is_empty(), player.current_track.clone())
        };

        let body = if is_empty {
            div()
                .w_full()
                .py(px(32.))
                .flex()
                .justify_center()
                .child(
                    SecondaryText::new(tr("music_queue_empty"))
                        .px(14.)
                        .tac(TextAlignT::Center)
                        .color(self.colors.secondary),
                )
                .into_any_element()
        } else {
            self.queue_list(compact, listeners, cx).into_any_element()
        };

        vstack(0.)
            .w_full()
            .when(!compact, |content| content.h_full())
            .items_start()
            .child(self.header(is_shuffled))
            .child(body)
            .children(current_track.filter(|_| self.shows_controls).map(|track| self.current_track_controls(track, cx)))
    }
}

impl RenderOnce for MusicQueueSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = observed_music_player(child_id(&self.id, "player"), window, cx);
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| MusicQueueSheetState::new());
        state.update(cx, |state, _| state.scroll_frame(window));
        let parts = MusicQueueSheetParts {
            id: self.id.clone(),
            player: player.clone(),
            state: state.clone(),
            on_minimum_height: self.on_minimum_height.clone(),
            shows_controls: self.shows_controls,
            colors: QueueColors::resolve(self.follows_theme, rise_ui::theme(cx)),
        };
        let background = parts.colors.background;

        let (is_queue_empty, current_track_id, show_audio_effects_sheet) = {
            let player = player.read(cx);
            (
                player.queue.is_empty(),
                player.current_track.as_ref().map(|track| track.id.clone()),
                player.show_audio_effects_sheet(),
            )
        };

        let reporting = parts.clone();
        on_change(child_id(&self.id, "queue-empty"), is_queue_empty, false, window, cx, move |_, _, window, cx| {
            reporting.report_minimum_height(window, cx);
        });
        let scrolled_player = player.clone();
        let scrolled_state = state.clone();
        on_change(child_id(&self.id, "current-track"), current_track_id, false, window, cx, move |_, _, _, cx| {
            MusicQueueSheetParts::scroll_to_current_track(&scrolled_player, &scrolled_state, true, cx);
        });

        let compact = parts.is_compact(cx);
        let content = if compact {
            scroll_view(child_id(&self.id, "compact-scroll"))
                .size_full()
                .shows_indicators(false)
                .child(parts.sheet_content(true, cx))
                .into_any_element()
        } else {
            parts.sheet_content(false, cx).into_any_element()
        };

        let measured = state.clone();
        div()
            .relative()
            .size_full()
            .bg(background)
            .child(content)
            .child(measure(child_id(&self.id, "available-height"), move |bounds, _, cx| {
                let height = f32::from(bounds.size.height);
                measured.update(cx, |state, cx| {
                    if state.available_height != height {
                        state.available_height = height;
                        cx.notify();
                    }
                });
            }))
            .modifier(
                MusicListeningControlAlert::new(child_id(&self.id, "control-alert"))
                    .is_active(!show_audio_effects_sheet),
            )
    }
}
