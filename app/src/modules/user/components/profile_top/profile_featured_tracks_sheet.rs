use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, IntoElement, MouseButton, RenderOnce, SharedString, Stateful,
    Svg, Window, black, deferred, div, prelude::*, px, white,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::music::shared::utils::music::featured_profile_track_store::FeaturedProfileTrackStore;
use crate::modules::music::shared::utils::music::music_models::{MusicRepeatMode, MusicRepeatModeExt, TrackModel};
use crate::modules::music::shared::utils::music::music_player_state::{MusicPlayOptions, MusicPlayerState};
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::widgets::music_marquee_title::MusicMarqueeTitle;
use crate::modules::music::widgets::music_playback_seek_bar::MusicPlaybackSeekBar;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

const COVER_SIZE: f32 = 44.0;
const ROW_HEIGHT: f32 = 64.0;
const MINIMUM_VISIBLE_ROWS: usize = 3;
const SWAP_SPRING_RESPONSE: f64 = 0.28;
const DROP_SPRING_RESPONSE: f64 = 0.3;
const REORDER_SPRING_DAMPING: f64 = 0.86;

#[derive(Default)]
struct ProfileFeaturedTracksSheetState {
    dragging_id: Option<String>,
    drag_offset_y: f32,
    accumulated_shift: f32,
    working_order: Vec<String>,
    list_content_height: f32,
    controls_height: f32,
    row_shifts: HashMap<String, Animated<f32>>,
    drag_swallows_click: bool,
}

impl ProfileFeaturedTracksSheetState {
    fn displayed_tracks(&self, base: Vec<TrackModel>) -> Vec<TrackModel> {
        if self.dragging_id.is_none() || self.working_order.is_empty() {
            return base;
        }
        self.working_order.iter().filter_map(|id| base.iter().find(|track| &track.id == id).cloned()).collect()
    }

    fn shift_row(&mut self, id: &str, from: f32, response: f64, cx: &App) {
        let shift = self.row_shifts.entry(id.to_owned()).or_insert_with(|| Animated::new(0.0));
        shift.jump(from);
        shift.set_in(0.0, spring(response, REORDER_SPRING_DAMPING), RiseClock::monotonic(), cx);
    }
}

#[derive(Clone)]
struct ProfileFeaturedTracksSheetActions {
    id: ElementId,
    user_id: String,
    is_owner: bool,
    selected_track_id: Binding<Option<String>>,
    on_empty: Rc<dyn Fn(&mut Window, &mut App)>,
    player: Entity<MusicPlayerState>,
    state: Entity<ProfileFeaturedTracksSheetState>,
    tracks: Rc<Vec<TrackModel>>,
}

impl ProfileFeaturedTracksSheetActions {
    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        let name: SharedString = name.into();
        ElementId::from((self.id.clone(), name))
    }

    fn is_currently_playing(&self, track: &TrackModel, cx: &App) -> bool {
        self.player.read(cx).current_track.as_ref().is_some_and(|current| {
            ProfileFeaturedTracksPlayback::identity(current) == ProfileFeaturedTracksPlayback::identity(track)
        })
    }

    fn select(&self, track: &TrackModel, window: &mut Window, cx: &mut App) {
        self.selected_track_id.set(Some(track.id.clone()), window, cx);
        if self.is_currently_playing(track, cx) {
            self.player.update(cx, |player, cx| player.toggle_play_pause(cx));
            return;
        }
        self.play(track, cx);
    }

    fn toggle_playback(&self, track: &TrackModel, cx: &mut App) {
        if self.is_currently_playing(track, cx) {
            self.player.update(cx, |player, cx| player.toggle_play_pause(cx));
        } else {
            self.play(track, cx);
        }
    }

    fn play(&self, track: &TrackModel, cx: &mut App) {
        let options = MusicPlayOptions {
            queue: self.tracks.as_ref().clone(),
            scope_id: Some(ProfileFeaturedTracksPlayback::scope_id(&self.user_id)),
            record_play_on_success: false,
            ..MusicPlayOptions::default()
        };
        let track = track.clone();
        self.player.update(cx, |player, cx| player.play(track, options, cx));
    }

    fn remove(&self, track: &TrackModel, window: &mut Window, cx: &mut App) {
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.remove_track_from_profile(&track.id, cx));
        let remaining = FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| store.tracks_for(&self.user_id, cx));
        self.selected_track_id.set(remaining.first().map(|track| track.id.clone()), window, cx);
        if remaining.is_empty() {
            (self.on_empty)(window, cx);
        }
    }

    fn update_drag(&self, track_id: &str, translation_y: f32, cx: &mut App) {
        let tracks = self.tracks.clone();
        self.state.update(cx, |state, cx| {
            if state.dragging_id.as_deref() != Some(track_id) {
                state.dragging_id = Some(track_id.to_owned());
                state.working_order = tracks.iter().map(|track| track.id.clone()).collect();
                state.accumulated_shift = 0.0;
            }
            state.drag_swallows_click = true;
            state.drag_offset_y = translation_y - state.accumulated_shift;
            if let Some(index) = state.working_order.iter().position(|id| id == track_id) {
                if state.drag_offset_y > ROW_HEIGHT * 0.6 && index + 1 < state.working_order.len() {
                    state.working_order.swap(index, index + 1);
                    let displaced = state.working_order[index].clone();
                    state.shift_row(&displaced, ROW_HEIGHT, SWAP_SPRING_RESPONSE, cx);
                    state.accumulated_shift += ROW_HEIGHT;
                    state.drag_offset_y = translation_y - state.accumulated_shift;
                } else if state.drag_offset_y < -ROW_HEIGHT * 0.6 && index > 0 {
                    state.working_order.swap(index, index - 1);
                    let displaced = state.working_order[index].clone();
                    state.shift_row(&displaced, -ROW_HEIGHT, SWAP_SPRING_RESPONSE, cx);
                    state.accumulated_shift -= ROW_HEIGHT;
                    state.drag_offset_y = translation_y - state.accumulated_shift;
                }
            }
            cx.notify();
        });
    }

    fn end_drag(&self, cx: &mut App) {
        let order = self.state.update(cx, |state, cx| {
            let order = std::mem::take(&mut state.working_order);
            if let Some(dragged) = state.dragging_id.take() {
                let from = state.drag_offset_y;
                state.shift_row(&dragged, from, DROP_SPRING_RESPONSE, cx);
            }
            state.drag_offset_y = 0.0;
            state.accumulated_shift = 0.0;
            cx.notify();
            order
        });
        if order.is_empty() {
            return;
        }
        let _ = selection_changed();
        MusicInteractionsStore::shared(cx).update(cx, |store, cx| store.reorder_profile_tracks(&order, cx));
        let reordered = FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| store.tracks_for(&self.user_id, cx));
        let scope_id = ProfileFeaturedTracksPlayback::scope_id(&self.user_id);
        self.player.update(cx, |player, cx| player.replace_ad_hoc_queue(&scope_id, reordered, cx));
    }

    fn track_list(
        &self,
        min_height: &Rc<dyn Fn(f32, &mut Window, &mut App)>,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        let is_dragging = self.state.read(cx).dragging_id.is_some();
        let items: Vec<GroupedBtn> = self
            .tracks
            .iter()
            .map(|track| {
                GroupedBtn::new(track.id.clone(), track.title.clone()).content_slot(self.row(track, theme, window, cx))
            })
            .collect();
        let measured_state = self.state.clone();
        let measured_min_height = min_height.clone();
        let row_count = self.tracks.len();
        scroll_view(self.child_id("scroll"))
            .shows_indicators(false)
            .scroll_disabled(is_dragging)
            .child(
                div()
                    .relative()
                    .w_full()
                    .pt(px(10.0))
                    .pb(px(12.0))
                    .child(
                        GroupedBtns::new(self.child_id("group"), items)
                            .title(tr("profile_featured_tracks_title"))
                            .corner_radius(20.0)
                            .horizontal_padding(12.0)
                            .separator_leading_padding(72.0),
                    )
                    .child(measure(self.child_id("list-height"), move |bounds, window, cx| {
                        measured_state.update(cx, |state, _| state.list_content_height = f32::from(bounds.size.height));
                        report_minimum_height(&measured_state, row_count, &measured_min_height, window, cx);
                    })),
            )
    }

    fn row(&self, track: &TrackModel, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let is_current = self.is_currently_playing(track, cx);
        let is_playing = self.player.read(cx).is_playing;
        let (is_dragged, offset_y) = {
            let state = self.state.read(cx);
            let is_dragged = state.dragging_id.as_deref() == Some(track.id.as_str());
            let offset_y = if is_dragged {
                state.drag_offset_y
            } else {
                state
                    .row_shifts
                    .get(&track.id)
                    .map_or(0.0, |shift| shift.frame_value(RiseClock::monotonic(), window))
            };
            (is_dragged, offset_y)
        };

        let cover = zstack(Alignment::Center)
            .relative()
            .size(px(COVER_SIZE))
            .clip_shape(Shape::continuous(10.0))
            .child(
                MusicCoverImage::new(self.child_id(format!("cover-{}", track.id)), track.cover_image_url.clone())
                    .fallback_colors(track.cover_colors.clone())
                    .size(COVER_SIZE)
                    .corner_radius(10.0),
            )
            .when(is_current, |cover| {
                cover.child(div().absolute().inset_0().bg(black().opacity(0.32))).children(IconUi::sized(
                    if is_playing { "waveform" } else { "play.fill" },
                    px(13.0),
                    white(),
                    !is_playing,
                ))
            });

        let reset_state = self.state.clone();
        let tap_actions = self.clone();
        let tap_track = track.clone();
        let row = hstack(12.0)
            .id(self.child_id(format!("row-{}", track.id)))
            .relative()
            .top(px(offset_y))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px(px(16.0))
            .py(px(10.0))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                reset_state.update(cx, |state, _| state.drag_swallows_click = false);
            })
            .on_click(move |_, window, cx| {
                let swallowed = tap_actions.state.update(cx, |state, _| std::mem::take(&mut state.drag_swallows_click));
                if !swallowed {
                    tap_actions.select(&tap_track, window, cx);
                }
            })
            .child(cover)
            .child(
                vstack(2.0)
                    .items_start()
                    .flex_1()
                    .min_w_0()
                    .child(
                        MainText::new(track.title.clone())
                            .px(15.0)
                            .color(if is_current { theme.primary._100 } else { theme.text.primary })
                            .number_of_lines(1),
                    )
                    .child(
                        MainText::new(format!("{} \u{b7} {}", track.formatted_duration(), track.artist))
                            .px(12.0)
                            .color(theme.text.secondary)
                            .number_of_lines(1),
                    ),
            )
            .child(div().flex_none().w(px(4.0)))
            .when(self.is_owner, |row| row.child(self.drag_handle(track, theme)));
        if is_dragged { deferred(row).with_priority(1).into_any_element() } else { row.into_any_element() }
    }

    fn drag_handle(&self, track: &TrackModel, theme: &AppTheme) -> impl IntoElement + use<> {
        let label = tr("profile_featured_tracks_title");
        let changed_actions = self.clone();
        let changed_id = track.id.clone();
        let ended_actions = self.clone();
        div()
            .id(self.child_id(format!("handle-{}", track.id)))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(34.0))
            .h(px(ROW_HEIGHT))
            .aria_label(label.clone())
            .tooltip_label(label)
            .children(IconUi::sized("line.3.horizontal", px(17.0), theme.text.secondary, false))
            .gesture(
                self.child_id(format!("handle-drag-{}", track.id)),
                DragGesture::new(2.0)
                    .on_changed(move |value: &DragGestureValue, _: &mut Window, cx: &mut App| {
                        changed_actions.update_drag(&changed_id, f32::from(value.translation.height), cx);
                    })
                    .on_ended(move |_: &DragGestureValue, _: &mut Window, cx: &mut App| {
                        ended_actions.end_drag(cx);
                    }),
            )
    }

    fn control_button(
        &self,
        name: &'static str,
        label: String,
        icon: Option<Svg>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(self.child_id(format!("{name}-hint")))
            .flex()
            .flex_1()
            .aria_label(label.clone())
            .tooltip_label(label)
            .child(
                plain(self.child_id(name))
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .min_h(px(44.0))
                    .on_press(action)
                    .children(icon),
            )
    }

    fn controls(&self, track: &TrackModel, theme: &AppTheme, cx: &mut App) -> impl IntoElement + use<> {
        let is_current = self.is_currently_playing(track, cx);
        let (is_playing, is_shuffled, repeat_mode) = {
            let player = self.player.read(cx);
            (player.is_playing, player.is_shuffled(cx), player.repeat_mode(cx))
        };
        let shows_pause = is_current && is_playing;
        let share_track = track.clone();

        let shuffle_player = self.player.clone();
        let previous_player = self.player.clone();
        let next_player = self.player.clone();
        let repeat_player = self.player.clone();
        let toggle_actions = self.clone();
        let toggle_track = track.clone();

        let transport = hstack(0.0)
            .w_full()
            .child(
                self.control_button(
                    "shuffle",
                    tr("music_shuffle"),
                    IconUi::sized(
                        "shuffle",
                        px(21.0),
                        if is_shuffled { theme.primary._100 } else { theme.text.secondary },
                        false,
                    ),
                    move |_, cx| {
                        shuffle_player.update(cx, |player, cx| player.toggle_shuffle(cx));
                    },
                )
                .aria_selected(is_shuffled),
            )
            .child(self.control_button(
                "previous",
                tr("music_player_previous"),
                IconUi::sized("backward.fill", px(21.0), theme.text.primary, true),
                move |_, cx| {
                    previous_player.update(cx, |player, cx| player.previous(cx));
                },
            ))
            .child(self.control_button(
                "toggle",
                tr(if shows_pause { "media_viewer_pause" } else { "media_viewer_play" }),
                IconUi::sized(
                    if shows_pause { "pause.fill" } else { "play.fill" },
                    px(30.0),
                    theme.text.primary,
                    true,
                ),
                move |_, cx| toggle_actions.toggle_playback(&toggle_track, cx),
            ))
            .child(self.control_button(
                "next",
                tr("music_player_next"),
                IconUi::sized("forward.fill", px(21.0), theme.text.primary, true),
                move |_, cx| {
                    next_player.update(cx, |player, cx| player.next(cx));
                },
            ))
            .child(
                self.control_button(
                    "repeat",
                    tr("music_player_repeat"),
                    IconUi::sized(
                        repeat_mode.icon(),
                        px(21.0),
                        if matches!(repeat_mode, MusicRepeatMode::Off) {
                            theme.text.secondary
                        } else {
                            theme.primary._100
                        },
                        false,
                    ),
                    move |_, cx| {
                        repeat_player.update(cx, |player, cx| player.cycle_repeat_mode(cx));
                    },
                )
                .aria_value(repeat_mode.accessibility_value()),
            );

        let remove = self.is_owner.then(|| {
            let remove_actions = self.clone();
            let remove_track = track.clone();
            let label = tr("music_remove_from_profile");
            div().w_full().child(
                LiquidGlassButton::new(self.child_id("remove"), LiquidGlassButtonShape::Capsule)
                    .horizontal_padding(22.0)
                    .vertical_padding(13.0)
                    .tint(Some(theme.bg._300.opacity(0.72)))
                    .accessibility_label(Some(label.clone()))
                    .on_press(move |window, cx| remove_actions.remove(&remove_track, window, cx))
                    .child(
                        hstack(8.0)
                            .w_full()
                            .justify_center()
                            .children(IconUi::sized("trash", px(15.0), theme.semantic.error_100, false))
                            .child(
                                MainText::new(label)
                                    .px(16.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(theme.semantic.error_100),
                            ),
                    ),
            )
        });

        vstack(14.0)
            .w_full()
            .px(px(18.0))
            .pt(px(12.0))
            .pb(px(10.0))
            .bg(theme.bg._100)
            .child(div().w_full().h(px(0.5)).bg(theme.border._100))
            .child(
                hstack(12.0)
                    .w_full()
                    .child(
                        MusicCoverImage::new(self.child_id("subject-cover"), track.cover_image_url.clone())
                            .fallback_colors(track.cover_colors.clone())
                            .size(56.0)
                            .corner_radius(13.0),
                    )
                    .child(
                        vstack(2.0)
                            .items_start()
                            .flex_1()
                            .min_w_0()
                            .child(
                                MusicMarqueeTitle::new(self.child_id("subject-title"), track.title.clone())
                                    .font(Font::system_weight(17.0, FontWeight::BOLD))
                                    .foreground(theme.text.primary)
                                    .line_height(22.0),
                            )
                            .child(
                                MusicMarqueeTitle::new(self.child_id("subject-artist"), track.artist.clone())
                                    .font(Font::system_size(14.0))
                                    .foreground(theme.text.secondary)
                                    .line_height(18.0),
                            ),
                    )
                    .child(div().flex_none().w(px(0.0)))
                    .child(
                        plain(self.child_id("share"))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(40.0))
                            .on_press(move |_, cx| {
                                let track = share_track.clone();
                                ShareSheetStore::shared(cx).update(cx, |sheet, cx| sheet.present_track(track, None, cx));
                            })
                            .children(IconUi::sized("square.and.arrow.up", px(19.0), theme.text.primary, false)),
                    ),
            )
            .child(
                MusicPlaybackSeekBar::new(self.child_id("seek"), self.player.clone(), track.duration_seconds)
                    .tint(theme.primary._100)
                    .time_color(theme.text.secondary)
                    .is_current(is_current),
            )
            .child(transport)
            .children(remove)
    }
}

fn report_minimum_height(
    state: &Entity<ProfileFeaturedTracksSheetState>,
    row_count: usize,
    on_minimum_height: &Rc<dyn Fn(f32, &mut Window, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) {
    let (list_content_height, controls_height) = {
        let state = state.read(cx);
        (state.list_content_height, state.controls_height)
    };
    if list_content_height <= 0.0 || controls_height <= 0.0 {
        return;
    }
    let hidden_rows = row_count.saturating_sub(MINIMUM_VISIBLE_ROWS);
    let list = (list_content_height - hidden_rows as f32 * ROW_HEIGHT).max(0.0);
    on_minimum_height(list + controls_height, window, cx);
}

#[derive(IntoElement)]
pub struct ProfileFeaturedTracksSheet {
    id: ElementId,
    user_id: String,
    is_owner: bool,
    selected_track_id: Binding<Option<String>>,
    on_empty: Rc<dyn Fn(&mut Window, &mut App)>,
    on_minimum_height: Rc<dyn Fn(f32, &mut Window, &mut App)>,
}

impl ProfileFeaturedTracksSheet {
    pub fn new(
        id: impl Into<ElementId>,
        user_id: impl Into<String>,
        selected_track_id: Binding<Option<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            user_id: user_id.into(),
            is_owner: true,
            selected_track_id,
            on_empty: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_minimum_height: Rc::new(|_: f32, _: &mut Window, _: &mut App| {}),
        }
    }

    pub fn is_owner(mut self, is_owner: bool) -> Self {
        self.is_owner = is_owner;
        self
    }

    pub fn on_empty(mut self, on_empty: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_empty = Rc::new(on_empty);
        self
    }

    pub fn on_minimum_height(mut self, on_minimum_height: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_minimum_height = Rc::new(on_minimum_height);
        self
    }
}

impl RenderOnce for ProfileFeaturedTracksSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state((self.id.clone(), "state"), cx, |_, _| {
            ProfileFeaturedTracksSheetState::default()
        });
        let player = MusicPlayerState::shared(cx);
        let base = FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| store.tracks_for(&self.user_id, cx));
        let tracks = Rc::new(state.read(cx).displayed_tracks(base));

        let empty_handler = self.on_empty.clone();
        on_change((self.id.clone(), "empty"), tracks.is_empty(), true, window, cx, move |_, is_empty, window, cx| {
            if *is_empty {
                empty_handler(window, cx);
            }
        });
        let current_track_id = player.read(cx).current_track.as_ref().map(|track| track.id.clone());
        let synced_tracks = tracks.clone();
        let synced_selection = self.selected_track_id.clone();
        on_change((self.id.clone(), "current-track"), current_track_id, false, window, cx, move |_, id, window, cx| {
            let Some(id) = id else { return };
            if synced_tracks.iter().any(|track| &track.id == id) {
                synced_selection.set(Some(id.clone()), window, cx);
            }
        });

        let selected_id = self.selected_track_id.get(cx);
        let subject = tracks
            .iter()
            .find(|track| selected_id.as_ref() == Some(&track.id))
            .or_else(|| tracks.first())
            .cloned();
        let row_count = tracks.len();
        let actions = ProfileFeaturedTracksSheetActions {
            id: self.id,
            user_id: self.user_id,
            is_owner: self.is_owner,
            selected_track_id: self.selected_track_id,
            on_empty: self.on_empty,
            player,
            state: state.clone(),
            tracks,
        };

        let content = subject.map(|subject| {
            let controls_state = state.clone();
            let controls_min_height = self.on_minimum_height.clone();
            vec![
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(actions.track_list(&self.on_minimum_height, &theme, window, cx))
                    .guide_anchor(RiseGuideAnchor::FeaturedTracksList)
                    .into_any_element(),
                div()
                    .relative()
                    .flex_none()
                    .w_full()
                    .child(actions.controls(&subject, &theme, cx))
                    .child(measure(actions.child_id("controls-height"), move |bounds, window, cx| {
                        controls_state.update(cx, |state, _| state.controls_height = f32::from(bounds.size.height));
                        report_minimum_height(&controls_state, row_count, &controls_min_height, window, cx);
                    }))
                    .into_any_element(),
            ]
        });

        vstack(0.0).size_full().bg(theme.bg._100).children(content.into_iter().flatten())
    }
}

pub enum ProfileFeaturedTracksPlayback {}

impl ProfileFeaturedTracksPlayback {
    pub fn scope_id(user_id: &str) -> String {
        format!("profile-featured:{user_id}")
    }

    /// track identity, a track pinned from a chat has no server id so it goes by the triple
    pub fn identity(track: &TrackModel) -> String {
        format!("{}|{}|{}", track.id, track.stable_audio_url(), track.source_message_id.as_deref().unwrap_or(""))
    }
}
