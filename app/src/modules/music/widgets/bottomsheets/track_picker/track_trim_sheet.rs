use std::rc::Rc;

use gpui::{
    App, Context, CursorStyle, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, KeyUpEvent, SharedString, Window,
};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::prelude::*;

use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::widgets::music_track_cover_image::MusicTrackCoverImage;
use crate::modules::post::components::post::post_music_playback_controller::PostMusicPlaybackController;

const PLAYBACK_ID: &str = "trim_preview";
const MIN_LENGTH_MS: i64 = 1_000;
const MAX_LENGTH_MS: i64 = 90_000;
const DEFAULT_END_MS: i64 = 30_000;
const HANDLE_WIDTH: f32 = 22.0;
const BAR_HEIGHT: f32 = 56.0;
const KEYBOARD_STEP_MS: i64 = 1_000;

pub type TrackTrimSheetConfirm = Rc<dyn Fn(i64, i64, &mut Window, &mut App)>;
pub type TrackTrimSheetDismiss = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct TrackTrimSheet {
    id: SharedString,
    track: RiseStoredMusicTrack,
    initial_start_ms: i64,
    initial_end_ms: i64,
    on_confirm: TrackTrimSheetConfirm,
    on_dismiss: Option<TrackTrimSheetDismiss>,
}

impl TrackTrimSheet {
    pub fn new(
        id: impl Into<SharedString>,
        track: RiseStoredMusicTrack,
        initial_start_ms: i64,
        initial_end_ms: i64,
        on_confirm: impl Fn(i64, i64, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            track,
            initial_start_ms,
            initial_end_ms,
            on_confirm: Rc::new(on_confirm),
            on_dismiss: None,
        }
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(on_dismiss));
        self
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TrackTrimSheetHandle {
    Start,
    End,
}

/// state of the track trim sheet, lives as long as the sheet is drawn
struct TrackTrimSheetState {
    track: RiseStoredMusicTrack,
    initial_start_ms: i64,
    initial_end_ms: i64,
    start_ms: i64,
    end_ms: i64,
    is_previewing: bool,
    start_drag_anchor: Option<i64>,
    end_drag_anchor: Option<i64>,
    bar_width: f32,
    active_handle: Option<TrackTrimSheetHandle>,
    is_press_tracked: bool,
    selected_handle: TrackTrimSheetHandle,
    keyboard_moved_segment: bool,
    focus_handle: FocusHandle,
}

impl TrackTrimSheetState {
    fn new(track: RiseStoredMusicTrack, initial_start_ms: i64, initial_end_ms: i64, cx: &mut Context<Self>) -> Self {
        Self {
            track,
            initial_start_ms,
            initial_end_ms,
            start_ms: 0,
            end_ms: DEFAULT_END_MS,
            is_previewing: false,
            start_drag_anchor: None,
            end_drag_anchor: None,
            bar_width: 0.0,
            active_handle: None,
            is_press_tracked: false,
            selected_handle: TrackTrimSheetHandle::End,
            keyboard_moved_segment: false,
            focus_handle: cx.focus_handle(),
        }
    }

    fn duration_ms(&self) -> i64 {
        self.track.duration_ms.unwrap_or(0).max(MIN_LENGTH_MS)
    }

    fn segment_length_ms(&self) -> i64 {
        self.end_ms - self.start_ms
    }

    fn fraction(&self, ms: i64) -> f32 {
        (ms as f64 / self.duration_ms() as f64) as f32
    }

    fn start_handle_x(&self, width: f32) -> f32 {
        (self.fraction(self.start_ms) * width - HANDLE_WIDTH / 2.0).max(0.0)
    }

    fn end_handle_x(&self, width: f32) -> f32 {
        (self.fraction(self.end_ms) * width - HANDLE_WIDTH / 2.0).min(width - HANDLE_WIDTH)
    }

    fn apply_initial_segment(&mut self) {
        let duration = self.duration_ms();
        let default_end = DEFAULT_END_MS.min(duration);
        let mut start = self.initial_start_ms.max(0);
        let mut end = if self.initial_end_ms > start { self.initial_end_ms } else { default_end };
        end = end.min(duration);
        end = end.max((start + MIN_LENGTH_MS).min(duration));
        start = start.min((end - MIN_LENGTH_MS).max(0));
        if end - start > MAX_LENGTH_MS {
            end = start + MAX_LENGTH_MS;
        }
        self.start_ms = start;
        self.end_ms = end;
    }

    fn move_start(&mut self, anchor: i64, delta_ms: i64, cx: &mut Context<Self>) {
        let next = (anchor + delta_ms).min(self.end_ms - MIN_LENGTH_MS).max(self.end_ms - MAX_LENGTH_MS).max(0);
        if next != self.start_ms {
            self.start_ms = next;
            cx.notify();
        }
    }

    fn move_end(&mut self, anchor: i64, delta_ms: i64, cx: &mut Context<Self>) {
        let next = (anchor + delta_ms)
            .max(self.start_ms + MIN_LENGTH_MS)
            .min(self.start_ms + MAX_LENGTH_MS)
            .min(self.duration_ms());
        if next != self.end_ms {
            self.end_ms = next;
            cx.notify();
        }
    }

    fn handle_drag_changed(&mut self, location_x: f32, translation: f32, cx: &mut Context<Self>) {
        let width = self.bar_width.max(1.0);
        if !self.is_press_tracked {
            self.is_press_tracked = true;
            let on_end = (self.end_handle_x(width)..=self.end_handle_x(width) + HANDLE_WIDTH).contains(&location_x);
            let on_start =
                (self.start_handle_x(width)..=self.start_handle_x(width) + HANDLE_WIDTH).contains(&location_x);
            self.active_handle = if on_end {
                Some(TrackTrimSheetHandle::End)
            } else if on_start {
                Some(TrackTrimSheetHandle::Start)
            } else {
                None
            };
            if let Some(handle) = self.active_handle {
                self.selected_handle = handle;
            }
        }
        let delta_ms = (f64::from(translation / width) * self.duration_ms() as f64) as i64;
        match self.active_handle {
            Some(TrackTrimSheetHandle::Start) => {
                let anchor = *self.start_drag_anchor.get_or_insert(self.start_ms);
                self.move_start(anchor, delta_ms, cx);
            }
            Some(TrackTrimSheetHandle::End) => {
                let anchor = *self.end_drag_anchor.get_or_insert(self.end_ms);
                self.move_end(anchor, delta_ms, cx);
            }
            None => {}
        }
    }

    fn handle_drag_ended(&mut self, cx: &mut Context<Self>) {
        self.is_press_tracked = false;
        let was_on_handle = self.active_handle.take().is_some();
        self.start_drag_anchor = None;
        self.end_drag_anchor = None;
        if was_on_handle {
            self.restart_preview_if_needed(cx);
        }
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let delta_ms = match event.keystroke.key.as_str() {
            "space" => {
                if !event.is_held {
                    self.toggle_preview(cx);
                }
                return true;
            }
            "left" => -KEYBOARD_STEP_MS,
            "right" => KEYBOARD_STEP_MS,
            _ => return false,
        };
        self.keyboard_moved_segment = true;
        match self.selected_handle {
            TrackTrimSheetHandle::Start => self.move_start(self.start_ms, delta_ms, cx),
            TrackTrimSheetHandle::End => self.move_end(self.end_ms, delta_ms, cx),
        }
        true
    }

    fn handle_key_up(&mut self, event: &KeyUpEvent, cx: &mut Context<Self>) {
        if !matches!(event.keystroke.key.as_str(), "left" | "right") || !self.keyboard_moved_segment {
            return;
        }
        self.keyboard_moved_segment = false;
        self.restart_preview_if_needed(cx);
    }

    fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        if self.is_previewing {
            self.stop_preview(cx);
        } else {
            self.start_preview(cx);
        }
    }

    fn start_preview(&mut self, cx: &mut Context<Self>) {
        let Some(url) = RemoteMediaURL::optional_normalized(self.track.audio_url.as_deref())
            .and_then(|normalized| Url::from_string(&normalized))
        else {
            return;
        };
        let (start_ms, end_ms) = (self.start_ms, self.end_ms);
        let track_id = self.track.id.clone();
        PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| {
            controller.play_ambient(PLAYBACK_ID, &url, track_id.as_deref(), start_ms, end_ms, true, true, false, cx);
        });
        self.is_previewing = true;
        cx.notify();
    }

    fn stop_preview(&mut self, cx: &mut Context<Self>) {
        if !self.is_previewing {
            return;
        }
        PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(PLAYBACK_ID, cx));
        self.is_previewing = false;
        cx.notify();
    }

    fn restart_preview_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.is_previewing {
            self.start_preview(cx);
        }
    }
}

fn format_ms(ms: i64) -> String {
    let total = ms / 1000;
    format!("{}:{:02}", total / 60, total % 60)
}

impl TrackTrimSheet {
    fn header(&self, state: &Entity<TrackTrimSheetState>, theme: &AppTheme) -> impl IntoElement {
        let tapped = state.clone();
        let on_dismiss = self.on_dismiss.clone();
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .px(px(16.0))
            .pt(px(8.0))
            .pb(px(4.0))
            .child(
                plain("track-trim-sheet-cancel")
                    .w(px(72.0))
                    .flex()
                    .justify_start()
                    .on_press(move |window, cx| {
                        tapped.update(cx, |this, cx| this.stop_preview(cx));
                        if let Some(on_dismiss) = &on_dismiss {
                            on_dismiss(window, cx);
                        }
                    })
                    .child(
                        MainText::new(tr("cancel"))
                            .px(15.0)
                            .font_weight(FontWeight::MEDIUM)
                            .color(theme.text.secondary),
                    ),
            )
            .child(div().flex_1().min_w(px(0.0)))
            .child(
                MainText::new(tr("music_trim_title"))
                    .px(16.0)
                    .font_weight(FontWeight::BOLD)
                    .color(theme.text.primary)
                    .line_limit(Some(1)),
            )
            .child(div().flex_1().min_w(px(0.0)))
            .child(div().w(px(72.0)))
    }

    fn track_row(&self, state: &Entity<TrackTrimSheetState>, theme: &AppTheme, cx: &App) -> impl IntoElement {
        let this = state.read(cx);
        let track = &this.track;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .child(
                MusicTrackCoverImage::new(
                    gpui::ElementId::Name(format!("track-trim-cover-{}", track.id.as_deref().unwrap_or_default()).into()),
                    RemoteMediaURL::optional_normalized(track.cover_url.as_deref()).unwrap_or_default(),
                )
                .size(Some(44.0))
                .corner_radius(8.0),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(3.0))
                    .child(
                        MainText::new(track.artist_title_line())
                            .px(15.0)
                            .font_weight(FontWeight::MEDIUM)
                            .color(theme.text.primary)
                            .number_of_lines(1),
                    )
                    .child(SecondaryText::new(format_ms(this.duration_ms())).px(12.0)),
            )
    }

    fn trim_bar(&self, state: &Entity<TrackTrimSheetState>, theme: &AppTheme, cx: &App) -> impl IntoElement {
        let this = state.read(cx);
        let width = this.bar_width.max(1.0);
        let start_x = this.fraction(this.start_ms) * width;
        let end_x = this.fraction(this.end_ms) * width;
        let handles = (this.bar_width > 0.0).then(|| {
            [this.start_handle_x(width), this.end_handle_x(width)].map(|x| Self::trim_handle(x, theme))
        });
        let measured = state.clone();
        let changed = state.clone();
        let ended = state.clone();
        div()
            .relative()
            .w_full()
            .h(px(BAR_HEIGHT))
            .child(div().absolute().inset_0().rounded(px(10.0)).bg(theme.bg._300))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(start_x))
                    .w(px((end_x - start_x).max(HANDLE_WIDTH)))
                    .rounded(px(10.0))
                    .bg(theme.primary._100.opacity(0.22))
                    .border_2()
                    .border_color(theme.primary._100),
            )
            .children(handles.into_iter().flatten())
            .child(measure("track-trim-sheet-bar-size", move |bounds, _, cx| {
                measured.update(cx, |this, cx| {
                    this.bar_width = f32::from(bounds.size.width);
                    cx.notify();
                });
            }))
            .gesture(
                "track-trim-sheet-bar-gesture",
                DragGesture::new(0.0)
                    .on_changed(move |value, _, cx| {
                        changed.update(cx, |this, cx| {
                            this.handle_drag_changed(
                                f32::from(value.start_location.x),
                                f32::from(value.translation.width),
                                cx,
                            );
                        });
                    })
                    .on_ended(move |_, _, cx| {
                        ended.update(cx, |this, cx| this.handle_drag_ended(cx));
                    }),
            )
    }

    fn trim_handle(x: f32, theme: &AppTheme) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .left(px(x))
            .w(px(HANDLE_WIDTH))
            .h(px(BAR_HEIGHT))
            .rounded(px(6.0))
            .bg(theme.primary._100)
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::ResizeLeftRight)
            .child(div().w(px(3.0)).h(px(22.0)).rounded_full().bg(theme.content_on(theme.primary._100).opacity(0.9)))
    }

    fn labels_row(&self, state: &Entity<TrackTrimSheetState>, theme: &AppTheme, cx: &App) -> impl IntoElement {
        let this = state.read(cx);
        div()
            .flex()
            .flex_row()
            .items_center()
            .child(
                MainText::new(format_ms(this.start_ms))
                    .px(13.0)
                    .font_weight(FontWeight::MEDIUM)
                    .color(theme.text.primary),
            )
            .child(div().flex_1().min_w(px(0.0)))
            .child(
                SecondaryText::new(format!("{}: {}", tr("music_trim_length_label"), format_ms(this.segment_length_ms())))
                    .px(12.0),
            )
            .child(div().flex_1().min_w(px(0.0)))
            .child(
                MainText::new(format_ms(this.end_ms))
                    .px(13.0)
                    .font_weight(FontWeight::MEDIUM)
                    .color(theme.text.primary),
            )
    }

    fn preview_button(&self, state: &Entity<TrackTrimSheetState>, theme: &AppTheme, cx: &App) -> impl IntoElement {
        let is_previewing = state.read(cx).is_previewing;
        let tapped = state.clone();
        plain("track-trim-sheet-preview")
            .w_full()
            .on_press(move |_, cx| tapped.update(cx, |this, cx| this.toggle_preview(cx)))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .py(px(12.0))
                    .bg(theme.bg._200)
                    .rounded_full()
                    .children(IconUi::sized(
                        if is_previewing { "stop.fill" } else { "play.fill" },
                        px(14.0),
                        theme.primary._100,
                        true,
                    ))
                    .child(
                        MainText::new(tr(if is_previewing { "music_trim_stop" } else { "music_trim_preview" }))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    ),
            )
    }
}

impl RenderOnce for TrackTrimSheet {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let track_id = self.track.id.clone().unwrap_or_default();
        let key: SharedString = format!("{}-{track_id}", self.id).into();
        let state = window.use_keyed_state(ElementId::Name(key.clone()), cx, |_, cx| {
            TrackTrimSheetState::new(self.track.clone(), self.initial_start_ms, self.initial_end_ms, cx)
        });
        let theme = rise_ui::theme(cx);
        let focus_handle = state.read(cx).focus_handle.clone();
        let confirm_state = state.clone();
        let on_confirm = self.on_confirm.clone();
        let keys = state.clone();
        let keys_up = state.clone();
        let appear = state.clone();
        let disappear = state.clone();
        let lifecycle_key: SharedString = format!("{key}-lifecycle").into();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg._100)
            .track_focus(&focus_handle)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if keys.update(cx, |this, cx| this.handle_key_down(event, cx)) {
                    cx.stop_propagation();
                }
            })
            .on_key_up(move |event: &KeyUpEvent, _, cx| {
                keys_up.update(cx, |this, cx| this.handle_key_up(event, cx));
            })
            .child(self.header(&state, theme))
            .child(div().px(px(16.0)).pt(px(14.0)).child(self.track_row(&state, theme, cx)))
            .child(div().px(px(16.0)).pt(px(22.0)).child(self.trim_bar(&state, theme, cx)))
            .child(div().px(px(16.0)).pt(px(10.0)).child(self.labels_row(&state, theme, cx)))
            .child(div().px(px(16.0)).pt(px(22.0)).child(self.preview_button(&state, theme, cx)))
            .child(
                div().px(px(16.0)).pt(px(12.0)).child(
                    ButtonUi::new("track-trim-sheet-confirm")
                        .disabled(false)
                        .is_pending(false)
                        .on_press(move |window, cx| {
                            let (start_ms, end_ms) = confirm_state.update(cx, |this, cx| {
                                this.stop_preview(cx);
                                (this.start_ms, this.end_ms)
                            });
                            on_confirm(start_ms, end_ms, window, cx);
                        })
                        .child(MainText::new(tr("music_trim_done")).px(15.0).font_weight(FontWeight::SEMIBOLD)),
                ),
            )
            .child(div().flex_1().min_h(px(0.0)))
            .on_appear(ElementId::Name(lifecycle_key), move |window, cx| {
                let focus_handle = appear.read(cx).focus_handle.clone();
                window.focus(&focus_handle, cx);
                appear.update(cx, |this, cx| {
                    this.apply_initial_segment();
                    cx.notify();
                });
            })
            .and_on_disappear(move |_, cx| {
                disappear.update(cx, |this, cx| this.stop_preview(cx));
            })
    }
}
