use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, App, Background, Bounds, ContentMask, Context, CursorStyle, ElementId, Entity, FocusHandle, FontWeight,
    Hsla, KeyDownEvent, KeyUpEvent, PathBuilder, Pixels, Rgba, SharedString, Task, Window, black, canvas, fill,
    linear_color_stop, linear_gradient, point, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use crate::core::lib::canon_format::RiseFormat;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::editor::editor_image_layout::EditorImageLayout;
use crate::modules::media_picker::editor::editor_music_plate::EditorMusicPlatePreview;
use crate::modules::music::shared::utils::music::music_track_waveform::MusicTrackWaveform;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::components::post::post_music_playback_controller::PostMusicPlaybackController;

const PLAYBACK_ID: &str = "trim_overlay_preview";
const EDGE_REACH: f32 = 26.0;
const MAX_WINDOW_WIDTH_FRACTION: f32 = 1.0 / 3.0;
const WAVEFORM_HEIGHT: f32 = 66.0;
const OUTSIDE_HEIGHT_FRACTION: f32 = 0.6;
const WINDOW_CORNER_RADIUS: f32 = 10.0;
const PLACEHOLDER_PEAK: f32 = 0.30;
const KEYBOARD_STEP_MS: i64 = 1_000;

/// plate decision made inside the trim overlay
/// transform belongs to the editor canvas entity not here
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackTrimPlateChoice {
    pub is_dark: bool,
}

pub type TrackTrimOverlayConfirm = Rc<dyn Fn(i64, i64, Option<TrackTrimPlateChoice>, &mut Window, &mut App)>;
pub type TrackTrimOverlayCancel = Rc<dyn Fn(&mut Window, &mut App)>;

/// full window clip picker, not a sheet
/// selection window is fixed and the full waveform scrolls under it
/// edge grips clamp the clip to 5 to 30 s (30 default)
#[derive(IntoElement)]
pub struct TrackTrimOverlay {
    id: SharedString,
    track: RiseStoredMusicTrack,
    initial_start_ms: i64,
    initial_end_ms: i64,
    allows_plate: bool,
    initial_plate: Option<TrackTrimPlateChoice>,
    opaque_backdrop: bool,
    backdrop: Option<AnyElement>,
    on_confirm: TrackTrimOverlayConfirm,
    on_cancel: TrackTrimOverlayCancel,
}

impl TrackTrimOverlay {
    pub const MIN_LENGTH_MS: i64 = 5_000;
    pub const MAX_LENGTH_MS: i64 = 30_000;

    pub fn new(
        id: impl Into<SharedString>,
        track: RiseStoredMusicTrack,
        initial_start_ms: i64,
        initial_end_ms: i64,
        on_confirm: impl Fn(i64, i64, Option<TrackTrimPlateChoice>, &mut Window, &mut App) + 'static,
        on_cancel: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            track,
            initial_start_ms,
            initial_end_ms,
            allows_plate: false,
            initial_plate: None,
            opaque_backdrop: false,
            backdrop: None,
            on_confirm: Rc::new(on_confirm),
            on_cancel: Rc::new(on_cancel),
        }
    }

    pub fn allows_plate(mut self, allows_plate: bool) -> Self {
        self.allows_plate = allows_plate;
        self
    }

    pub fn initial_plate(mut self, initial_plate: Option<TrackTrimPlateChoice>) -> Self {
        self.initial_plate = initial_plate;
        self
    }

    pub fn opaque_backdrop(mut self, opaque_backdrop: bool) -> Self {
        self.opaque_backdrop = opaque_backdrop;
        self
    }

    pub fn backdrop(mut self, backdrop: Option<AnyElement>) -> Self {
        self.backdrop = backdrop;
        self
    }

    /// segment clamped for sending
    /// a clip from another post may be cut by another client or against a longer track
    /// server only accepts bounds inside the duration, empty input defaults to the start
    pub fn normalized_segment(start_ms: Option<i64>, end_ms: Option<i64>, duration_ms: Option<i64>) -> (i64, i64) {
        let duration = duration_ms.unwrap_or(0).max(0);
        let ceiling = if duration > 0 { duration } else { Self::MAX_LENGTH_MS };
        let (Some(start_ms), Some(end_ms)) = (start_ms, end_ms) else {
            return (0, Self::MAX_LENGTH_MS.min(ceiling));
        };
        if end_ms <= start_ms {
            return (0, Self::MAX_LENGTH_MS.min(ceiling));
        }
        let length = (end_ms - start_ms).clamp(Self::MIN_LENGTH_MS, Self::MAX_LENGTH_MS);
        if length > ceiling {
            return (0, ceiling);
        }
        let start = start_ms.clamp(0, ceiling - length);
        (start, start + length)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DragMode {
    Scroll,
    ResizeLeading,
    ResizeTrailing,
}

/// screen geometry of the fixed window
/// origin x, width and px per ms waveform scale
#[derive(Clone, Copy, Debug)]
struct TrackTrimWindowLayout {
    x: f32,
    width: f32,
    scale: f32,
}

/// state of the trim overlay, lives as long as the overlay is drawn
struct TrackTrimOverlayState {
    track: RiseStoredMusicTrack,
    initial_start_ms: i64,
    initial_end_ms: i64,
    initial_plate: Option<TrackTrimPlateChoice>,
    start_ms: i64,
    length_ms: i64,
    peaks: Option<Rc<Vec<f32>>>,
    peaks_generation: Uuid,
    is_paused: bool,
    playback_anchor: Duration,
    paused_fraction: f64,
    drag_mode: Option<DragMode>,
    drag_anchor_start_ms: Option<i64>,
    drag_anchor_length_ms: Option<i64>,
    resize_anchor_leading_x: Option<f32>,
    resize_anchor_trailing_x: Option<f32>,
    plate_choice: Option<TrackTrimPlateChoice>,
    did_apply_initial_state: bool,
    strip_width: Rc<Cell<f32>>,
    selected_handle: Option<DragMode>,
    keyboard_moved_segment: bool,
    focus_handle: FocusHandle,
    plate_presence: Presence<()>,
    plate_dark: Animated<f32>,
    plate_selected: Animated<f32>,
    peaks_reveal: Animated<f32>,
    peaks_task: Option<Task<()>>,
    _background_observer: RiseSubscription,
    _background_signals: Task<()>,
}

impl TrackTrimOverlayState {
    fn new(
        track: RiseStoredMusicTrack,
        initial_start_ms: i64,
        initial_end_ms: i64,
        initial_plate: Option<TrackTrimPlateChoice>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let background_observer =
            RiseAppEvents::default_center().observe(UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, move |_| {
                let _ = sender.unbounded_send(());
            });
        let background_signals = cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                let updated = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                    this.handle_background(cx);
                });
                if updated.is_err() {
                    return;
                }
            }
        });
        Self {
            track,
            initial_start_ms,
            initial_end_ms,
            initial_plate,
            start_ms: 0,
            length_ms: TrackTrimOverlay::MAX_LENGTH_MS,
            peaks: None,
            peaks_generation: Uuid::new_v4(),
            is_paused: false,
            playback_anchor: RiseClock::monotonic(),
            paused_fraction: 0.0,
            drag_mode: None,
            drag_anchor_start_ms: None,
            drag_anchor_length_ms: None,
            resize_anchor_leading_x: None,
            resize_anchor_trailing_x: None,
            plate_choice: None,
            did_apply_initial_state: false,
            strip_width: Rc::new(Cell::new(0.0)),
            selected_handle: None,
            keyboard_moved_segment: false,
            focus_handle: cx.focus_handle(),
            plate_presence: Presence::new(None)
                .with_transition(Transition::scale(0.92).combined(Transition::opacity())),
            plate_dark: Animated::new(0.0),
            plate_selected: Animated::new(0.0),
            peaks_reveal: Animated::new(0.0),
            peaks_task: None,
            _background_observer: background_observer,
            _background_signals: background_signals,
        }
    }

    fn duration_ms(&self) -> i64 {
        self.track.duration_ms.unwrap_or(0).max(TrackTrimOverlay::MIN_LENGTH_MS)
    }

    fn end_ms(&self) -> i64 {
        (self.start_ms + self.length_ms).min(self.duration_ms())
    }

    fn on_appear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        if !self.did_apply_initial_state {
            self.did_apply_initial_state = true;
            self.set_plate_choice(self.initial_plate, cx);
        }
        self.apply_initial_segment();
        self.load_waveform(cx);
        self.start_playback(cx);
        cx.notify();
    }

    fn handle_background(&mut self, cx: &mut Context<Self>) {
        if self.is_paused {
            return;
        }
        self.paused_fraction = self.playback_fraction(RiseClock::monotonic());
        self.stop_playback(cx);
        self.is_paused = true;
        cx.notify();
    }

    fn set_plate_choice(&mut self, choice: Option<TrackTrimPlateChoice>, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        let was_shown = self.plate_choice.is_some();
        self.plate_choice = choice;
        let shows = choice.is_some();
        self.plate_presence.set_in(shows.then_some(()), ease_in_out(0.2), now, cx);
        self.plate_selected.set_in(if shows { 1.0 } else { 0.0 }, ease_in_out(0.15), now, cx);
        if let Some(choice) = choice {
            let dark = if choice.is_dark { 1.0 } else { 0.0 };
            if was_shown {
                self.plate_dark.set_in(dark, ease_in_out(0.18), now, cx);
            } else {
                self.plate_dark.jump(dark);
            }
        }
        cx.notify();
    }

    fn flip_plate(&mut self, cx: &mut Context<Self>) {
        let Some(mut choice) = self.plate_choice else {
            return;
        };
        choice.is_dark = !choice.is_dark;
        self.set_plate_choice(Some(choice), cx);
    }

    /// constant waveform scale, px per millisecond
    fn px_per_ms(width: f32) -> f32 {
        width * MAX_WINDOW_WIDTH_FRACTION / TrackTrimOverlay::MAX_LENGTH_MS as f32
    }

    /// screen geometry of the fixed window for the current or drag anchored state
    fn window_layout(&self, width: f32) -> TrackTrimWindowLayout {
        let scale = Self::px_per_ms(width);
        let window_width = self.length_ms as f32 * scale;
        if let Some(leading_x) = self.resize_anchor_leading_x {
            return TrackTrimWindowLayout { x: leading_x, width: window_width, scale };
        }
        if let Some(trailing_x) = self.resize_anchor_trailing_x {
            return TrackTrimWindowLayout { x: trailing_x - window_width, width: window_width, scale };
        }
        TrackTrimWindowLayout { x: (width - window_width) / 2.0, width: window_width, scale }
    }

    fn min_length(&self) -> i64 {
        TrackTrimOverlay::MIN_LENGTH_MS.min(self.duration_ms())
    }

    fn max_length(&self) -> i64 {
        TrackTrimOverlay::MAX_LENGTH_MS.min(self.duration_ms())
    }

    fn clamp_start(&self, candidate: i64) -> i64 {
        candidate.min(self.duration_ms() - self.length_ms).max(0)
    }

    fn waveform_sample_count(&self) -> usize {
        let usable = self.strip_width.get().max(200.0);
        let total_width = self.duration_ms() as f64 * f64::from(Self::px_per_ms(usable));
        ((total_width / 8.0) as usize).clamp(24, 700)
    }

    fn begin_drag(&mut self, location_x: f32, width: f32) {
        let layout = self.window_layout(width);
        let window_min_x = layout.x;
        let window_max_x = layout.x + layout.width;

        let mode = if (location_x - window_min_x).abs() <= EDGE_REACH {
            self.resize_anchor_trailing_x = Some(window_max_x);
            DragMode::ResizeLeading
        } else if (location_x - window_max_x).abs() <= EDGE_REACH {
            self.resize_anchor_leading_x = Some(window_min_x);
            DragMode::ResizeTrailing
        } else {
            DragMode::Scroll
        };
        self.drag_mode = Some(mode);
        self.selected_handle = (mode != DragMode::Scroll).then_some(mode);
        self.drag_anchor_start_ms = Some(self.start_ms);
        self.drag_anchor_length_ms = Some(self.length_ms);
    }

    fn apply_drag(&mut self, translation: f32, width: f32, cx: &mut Context<Self>) {
        let (Some(mode), Some(anchor_start), Some(anchor_length)) =
            (self.drag_mode, self.drag_anchor_start_ms, self.drag_anchor_length_ms)
        else {
            return;
        };
        let scale = Self::px_per_ms(width);
        if scale <= 0.0 {
            return;
        }
        let delta_ms = (f64::from(translation) / f64::from(scale)).round() as i64;
        self.apply_delta(mode, anchor_start, anchor_length, delta_ms, cx);
    }

    fn apply_delta(
        &mut self,
        mode: DragMode,
        anchor_start: i64,
        anchor_length: i64,
        delta_ms: i64,
        cx: &mut Context<Self>,
    ) {
        match mode {
            DragMode::Scroll => {
                let next = self.clamp_start(anchor_start - delta_ms);
                if next != self.start_ms {
                    self.start_ms = next;
                    cx.notify();
                }
            }
            DragMode::ResizeLeading => {
                let anchor_end = anchor_start + anchor_length;
                let next_start = (anchor_start + delta_ms)
                    .min(anchor_end - self.min_length())
                    .max(0)
                    .max(anchor_end - self.max_length());
                let next_length = anchor_end - next_start;
                if next_start != self.start_ms || next_length != self.length_ms {
                    self.start_ms = next_start;
                    self.length_ms = next_length;
                    cx.notify();
                }
            }
            DragMode::ResizeTrailing => {
                let next_end = (anchor_start + anchor_length + delta_ms)
                    .max(anchor_start + self.min_length())
                    .min(self.duration_ms())
                    .min(anchor_start + self.max_length());
                let next_length = next_end - anchor_start;
                if next_length != self.length_ms {
                    self.length_ms = next_length;
                    cx.notify();
                }
            }
        }
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        self.drag_mode = None;
        self.drag_anchor_start_ms = None;
        self.drag_anchor_length_ms = None;
        self.resize_anchor_leading_x = None;
        self.resize_anchor_trailing_x = None;
        self.start_playback(cx);
        cx.notify();
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        match event.keystroke.key.as_str() {
            "space" => {
                if !event.is_held {
                    self.toggle_pause(cx);
                }
                true
            }
            "left" => {
                self.nudge_selection(-KEYBOARD_STEP_MS, cx);
                true
            }
            "right" => {
                self.nudge_selection(KEYBOARD_STEP_MS, cx);
                true
            }
            _ => false,
        }
    }

    fn handle_key_up(&mut self, event: &KeyUpEvent, cx: &mut Context<Self>) {
        if !matches!(event.keystroke.key.as_str(), "left" | "right") || !self.keyboard_moved_segment {
            return;
        }
        self.keyboard_moved_segment = false;
        self.start_playback(cx);
        cx.notify();
    }

    fn nudge_selection(&mut self, delta_ms: i64, cx: &mut Context<Self>) {
        let (start_ms, length_ms) = (self.start_ms, self.length_ms);
        self.keyboard_moved_segment = true;
        match self.selected_handle {
            Some(mode) => self.apply_delta(mode, start_ms, length_ms, delta_ms, cx),
            None => self.apply_delta(DragMode::Scroll, start_ms, length_ms, -delta_ms, cx),
        }
    }

    fn playback_fraction(&self, now: MotionTime) -> f64 {
        if self.length_ms <= 0 {
            return 0.0;
        }
        if self.is_paused {
            return self.paused_fraction;
        }
        let elapsed = now.saturating_sub(self.playback_anchor).as_secs_f64();
        let segment = self.length_ms as f64 / 1000.0;
        if segment <= 0.0 {
            return 0.0;
        }
        (elapsed % segment) / segment
    }

    fn start_playback(&mut self, cx: &mut Context<Self>) {
        let local_url = MusicTrackWaveform::local_file_url(&self.track);
        let remote_url = RemoteMediaURL::optional_normalized(self.track.audio_url.as_deref())
            .and_then(|normalized| Url::from_string(&normalized));
        let Some(url) = local_url.or(remote_url) else {
            return;
        };
        let start_ms = self.start_ms;
        let end_ms = self.end_ms();
        let track_id = self.track.id.clone();
        let started = PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| {
            controller.play_ambient(
                PLAYBACK_ID,
                &url,
                track_id.as_deref(),
                start_ms,
                end_ms,
                true,
                false,
                false,
                cx,
            )
        });
        if !started {
            self.is_paused = true;
            return;
        }
        self.is_paused = false;
        self.playback_anchor = RiseClock::monotonic();
    }

    fn stop_playback(&mut self, cx: &mut Context<Self>) {
        PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(PLAYBACK_ID, cx));
    }

    fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        if self.is_paused {
            self.start_playback(cx);
        } else {
            self.paused_fraction = self.playback_fraction(RiseClock::monotonic());
            self.stop_playback(cx);
            self.is_paused = true;
        }
        cx.notify();
    }

    fn apply_initial_segment(&mut self) {
        let initial_length = self.initial_end_ms - self.initial_start_ms;
        if initial_length >= self.min_length() {
            self.length_ms = initial_length.min(self.max_length());
            self.start_ms = self.clamp_start(self.initial_start_ms);
        } else {
            self.length_ms = self.max_length();
            self.start_ms = 0;
        }
    }

    fn load_waveform(&mut self, cx: &mut Context<Self>) {
        let generation = Uuid::new_v4();
        self.peaks_generation = generation;
        let track = self.track.clone();
        let sample_count = self.waveform_sample_count() as i64;
        self.peaks_task = Some(cx.spawn(async move |this, cx| {
            let result = MusicTrackWaveform::peaks(&track, sample_count, cx).await;
            let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                if this.peaks_generation != generation {
                    return;
                }
                if let Some(result) = result {
                    this.peaks = Some(Rc::new(result));
                    this.peaks_reveal.set_in(1.0, ease_in_out(0.2), RiseClock::monotonic(), cx);
                    cx.notify();
                }
            });
        }));
    }

    fn paint_track_timeline(&self, bounds: Bounds<Pixels>, now: MotionTime, accent: Hsla, window: &mut Window) {
        let fraction = self.playback_fraction(now).clamp(0.0, 1.0) as f32;
        let total = f32::from(bounds.size.width).max(1.0);
        let height = f32::from(bounds.size.height);
        let radius = gpui::px(height / 2.0);
        let duration = self.duration_ms() as f64;
        let segment_x = total * (self.start_ms as f64 / duration) as f32;
        let segment_width = (total * (self.length_ms as f64 / duration) as f32).max(3.0);

        window.paint_quad(fill(bounds, white().opacity(0.22)).corner_radii(radius));
        let segment = local_rect(bounds, segment_x, 0.0, segment_width, height);
        window.paint_quad(fill(segment, playback_gradient(accent, 0.3)).corner_radii(radius));
        let played = local_rect(bounds, segment_x, 0.0, (segment_width * fraction).max(2.0), height);
        window.paint_quad(fill(played, playback_gradient(accent, 1.0)).corner_radii(radius));
    }

    fn played_sweep(&self, window_rect: Bounds<Pixels>, now: MotionTime, accent: Hsla, window: &mut Window) {
        let fraction = self.playback_fraction(now).clamp(0.0, 1.0) as f32;
        let radius = gpui::px(WINDOW_CORNER_RADIUS);
        window.paint_quad(fill(window_rect, white().opacity(0.07)).corner_radii(radius));
        let played_width = f32::from(window_rect.size.width) * fraction;
        if played_width <= 0.5 {
            return;
        }
        let played = Bounds::new(window_rect.origin, size(gpui::px(played_width), window_rect.size.height));
        window.with_content_mask(Some(ContentMask { bounds: played }), |window| {
            window.paint_quad(fill(window_rect, playback_gradient(accent, 1.0)).corner_radii(radius));
        });
    }

    fn waveform_canvas(
        &self,
        bounds: Bounds<Pixels>,
        layout: TrackTrimWindowLayout,
        window_rect: Bounds<Pixels>,
        now: MotionTime,
        accent: Hsla,
        window: &mut Window,
    ) {
        let reveal = self.peaks_reveal.frame_value(now, window).clamp(0.0, 1.0);
        match self.peaks.as_deref().map(Vec::as_slice) {
            Some(values) if reveal >= 1.0 => self.paint_bars(bounds, layout, Some(values), 1.0, window),
            Some(values) => {
                self.paint_bars(bounds, layout, None, 1.0 - reveal, window);
                self.paint_bars(bounds, layout, Some(values), reveal, window);
            }
            None => self.paint_bars(bounds, layout, None, 1.0, window),
        }

        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if let Some(border) = rounded_rect_stroke(window_rect, WINDOW_CORNER_RADIUS, 3.0) {
            window.paint_path(border, playback_gradient(accent, 1.0));
        }

        let grip_height = height * 0.4;
        let grip_width = 5.0;
        let window_width = layout.width.min(width);
        for border_x in [layout.x + 1.5, layout.x + window_width - 1.5] {
            let (x, y) = (border_x - grip_width / 2.0, (height - grip_height) / 2.0);
            let outline = local_rect(bounds, x - 1.0, y - 1.0, grip_width + 2.0, grip_height + 2.0);
            window.paint_quad(fill(outline, black().opacity(0.55)).corner_radii(gpui::px(grip_width / 2.0 + 1.0)));
            let grip = local_rect(bounds, x, y, grip_width, grip_height);
            window.paint_quad(fill(grip, white()).corner_radii(gpui::px(grip_width / 2.0)));
        }
    }

    /// values is none for the placeholder strip, that one needs no array of equal numbers
    fn paint_bars(
        &self,
        bounds: Bounds<Pixels>,
        layout: TrackTrimWindowLayout,
        values: Option<&[f32]>,
        opacity: f32,
        window: &mut Window,
    ) {
        let canvas_width = f32::from(bounds.size.width);
        let canvas_height = f32::from(bounds.size.height);
        let bar_count = values.map_or_else(|| self.waveform_sample_count(), <[f32]>::len).max(1);
        let total_width = self.duration_ms() as f32 * layout.scale;
        let step = total_width / bar_count as f32;
        let bar_width = (step * 0.55).clamp(3.0, 5.0);
        let origin_x = layout.x - self.start_ms as f32 * layout.scale;
        let window_min_x = layout.x;
        let window_max_x = layout.x + layout.width.min(canvas_width);

        let outside_available = canvas_height * OUTSIDE_HEIGHT_FRACTION - 8.0;
        let inside_available = canvas_height - 16.0;
        let inside_color = white().opacity(opacity);
        let outside_color = white().opacity(0.34 * opacity);
        for index in 0..bar_count {
            let x = origin_x + index as f32 * step + (step - bar_width) / 2.0;
            if x + bar_width < -6.0 {
                continue;
            }
            if x > canvas_width + 6.0 {
                break;
            }
            let peak = values.and_then(|values| values.get(index)).copied().unwrap_or(PLACEHOLDER_PEAK);
            let value = peak.clamp(0.12, 1.0);
            let center_x = x + bar_width / 2.0;
            let in_window = center_x >= window_min_x && center_x <= window_max_x;
            let bar_height = (value * if in_window { inside_available } else { outside_available }).max(3.0);
            let rect = local_rect(bounds, x, (canvas_height - bar_height) / 2.0, bar_width, bar_height);
            let color = if in_window { inside_color } else { outside_color };
            window.paint_quad(fill(rect, color).corner_radii(gpui::px(bar_width / 2.0)));
        }
    }

    fn paint_waveform_window(&self, bounds: Bounds<Pixels>, now: MotionTime, accent: Hsla, window: &mut Window) {
        let width = f32::from(bounds.size.width).max(1.0);
        let layout = self.window_layout(width);
        let window_rect = local_rect(
            bounds,
            layout.x,
            0.0,
            layout.width.min(f32::from(bounds.size.width)),
            f32::from(bounds.size.height),
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            self.played_sweep(window_rect, now, accent, window);
            self.waveform_canvas(bounds, layout, window_rect, now, accent, window);
        });
    }
}

fn local_rect(bounds: Bounds<Pixels>, x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(
        point(bounds.origin.x + gpui::px(x), bounds.origin.y + gpui::px(y)),
        size(gpui::px(width), gpui::px(height)),
    )
}

fn rounded_rect_stroke(rect: Bounds<Pixels>, radius: f32, line_width: f32) -> Option<gpui::Path<Pixels>> {
    let left = f32::from(rect.origin.x);
    let top = f32::from(rect.origin.y);
    let right = left + f32::from(rect.size.width);
    let bottom = top + f32::from(rect.size.height);
    let radius = radius.min((right - left) / 2.0).min((bottom - top) / 2.0);
    let radii = point(gpui::px(radius), gpui::px(radius));
    let at = |x: f32, y: f32| point(gpui::px(x), gpui::px(y));
    let mut builder = PathBuilder::stroke(gpui::px(line_width));
    builder.move_to(at(left + radius, top));
    builder.line_to(at(right - radius, top));
    builder.arc_to(radii, gpui::px(0.0), false, true, at(right, top + radius));
    builder.line_to(at(right, bottom - radius));
    builder.arc_to(radii, gpui::px(0.0), false, true, at(right - radius, bottom));
    builder.line_to(at(left + radius, bottom));
    builder.arc_to(radii, gpui::px(0.0), false, true, at(left, bottom - radius));
    builder.line_to(at(left, top + radius));
    builder.arc_to(radii, gpui::px(0.0), false, true, at(left + radius, top));
    builder.close();
    builder.build().ok()
}

fn playback_trailing_color() -> Hsla {
    Rgba { r: 1.0, g: 0.62, b: 0.26, a: 1.0 }.into()
}

/// playback gradient from leading to trailing with the opacity over it
fn playback_gradient(accent: Hsla, opacity: f32) -> Background {
    linear_gradient(
        90.0,
        linear_color_stop(accent.opacity(opacity), 0.0),
        linear_color_stop(playback_trailing_color().opacity(opacity), 1.0),
    )
}

fn icon(name: &str, side: f32, color: Hsla, is_filled: bool) -> impl IntoElement {
    div().flex().items_center().justify_center().children(IconUi::sized(name, gpui::px(side), color, is_filled))
}

fn with_tooltip(id: &'static str, label: String, content: impl IntoElement) -> impl IntoElement {
    let label = SharedString::from(label);
    div()
        .id(id)
        .tooltip_label(label)
        .child(content)
}

impl TrackTrimOverlay {
    fn top_bar(&self, state: &Entity<TrackTrimOverlayState>, accent: Hsla, cx: &App) -> impl IntoElement {
        let this = state.read(cx);
        div()
            .relative()
            .w_full()
            .pt(px(6.0))
            .flex()
            .flex_col()
            .justify_center()
            .child(
                div()
                    .px(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE + 8.0))
                    .min_h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(2.0))
                    .child(
                        MainText::new(tr("music_trim_title"))
                            .px(17.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white())
                            .number_of_lines(1),
                    )
                    .child(
                        MainText::new(RiseFormat::duration_milliseconds(this.end_ms() - this.start_ms))
                            .px(13.0)
                            .tac(TextAlignT::Center)
                            .color(white().opacity(0.7))
                            .number_of_lines(1)
                            .monospaced_digits(true),
                    ),
            )
            .child(self.top_bar_buttons(state, accent))
    }

    fn top_bar_buttons(&self, state: &Entity<TrackTrimOverlayState>, accent: Hsla) -> impl IntoElement {
        let cancel_state = state.clone();
        let on_cancel = self.on_cancel.clone();
        let confirm_state = state.clone();
        let on_confirm = self.on_confirm.clone();
        let allows_plate = self.allows_plate;
        div()
            .absolute()
            .top(px(6.0))
            .left_0()
            .right_0()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                LiquidGlassButton::new("track-trim-cancel", LiquidGlassButtonShape::Circle)
                    .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
                    .accessibility_label(Some(tr("cancel")))
                    .on_press(move |window, cx| {
                        Self::cancel(&cancel_state, &on_cancel, window, cx);
                    })
                    .child(icon("xmark", 17.0, white(), false)),
            )
            .child(
                LiquidGlassButton::new("track-trim-done", LiquidGlassButtonShape::Circle)
                    .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
                    .tint(Some(accent))
                    .accessibility_label(Some(tr("music_trim_done")))
                    .on_press(move |window, cx| {
                        Self::confirm(&confirm_state, allows_plate, &on_confirm, window, cx);
                    })
                    .child(icon("checkmark", 17.0, white(), false)),
            )
    }

    fn cancel(
        state: &Entity<TrackTrimOverlayState>,
        on_cancel: &TrackTrimOverlayCancel,
        window: &mut Window,
        cx: &mut App,
    ) {
        state.update(cx, |this, cx| this.stop_playback(cx));
        on_cancel(window, cx);
    }

    fn confirm(
        state: &Entity<TrackTrimOverlayState>,
        allows_plate: bool,
        on_confirm: &TrackTrimOverlayConfirm,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (start_ms, end_ms, plate) = state.update(cx, |this, cx| {
            this.stop_playback(cx);
            (this.start_ms, this.end_ms(), if allows_plate { this.plate_choice } else { None })
        });
        on_confirm(start_ms, end_ms, plate, window, cx);
    }

    fn plate_preview(
        &self,
        state: &Entity<TrackTrimOverlayState>,
        now: MotionTime,
        window: &Window,
        cx: &App,
    ) -> Option<impl IntoElement> {
        let this = state.read(cx);
        let track = &this.track;
        let title = track.title.clone().unwrap_or_default();
        let artist = track.artist.as_ref().map(|artist| artist.stage_name.clone()).unwrap_or_default();
        let cover_url = RemoteMediaURL::optional_normalized(track.cover_url.as_deref()).unwrap_or_default();
        let track_id = track.id.clone().unwrap_or_default();
        let dark = this.plate_dark.frame_value(now, window).clamp(0.0, 1.0);
        let preview = move |id: &'static str, is_dark: bool| {
            EditorMusicPlatePreview::new(id, title.clone(), artist.clone(), cover_url.clone(), track_id.clone(), is_dark)
                .base_width(206.0)
        };
        let base = preview("track-trim-plate", dark >= 1.0);
        let fade = (dark > 0.0 && dark < 1.0).then(|| preview("track-trim-plate-dark", true));
        let tapped = state.clone();
        this.plate_presence.render_transitioned(now, window, move |_| {
            plain("track-trim-plate-flip")
                .on_press(move |_, cx| tapped.update(cx, |this, cx| this.flip_plate(cx)))
                .child(
                    div()
                        .relative()
                        .child(base)
                        .children(fade.map(|fade| div().absolute().inset_0().opacity(dark).child(fade))),
                )
        })
    }

    fn plate_mode_icons(
        &self,
        state: &Entity<TrackTrimOverlayState>,
        now: MotionTime,
        accent: Hsla,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let selected = state.read(cx).plate_selected.frame_value(now, window).clamp(0.0, 1.0);
        let off_state = state.clone();
        let on_state = state.clone();
        div()
            .flex()
            .flex_row()
            .justify_center()
            .gap(px(12.0))
            .child(Self::plate_mode_icon(
                "track-trim-plate-off",
                "nosign",
                1.0 - selected,
                tr("music_plate_off"),
                accent,
                move |_, cx| off_state.update(cx, |this, cx| this.set_plate_choice(None, cx)),
            ))
            .child(Self::plate_mode_icon(
                "track-trim-plate-on",
                "music.note",
                selected,
                tr("music_plate_on"),
                accent,
                move |_, cx| {
                    on_state.update(cx, |this, cx| {
                        if this.plate_choice.is_none() {
                            this.set_plate_choice(Some(TrackTrimPlateChoice { is_dark: false }), cx);
                        }
                    });
                },
            ))
    }

    fn plate_mode_icon(
        id: &'static str,
        system_image: &str,
        selected: f32,
        label: String,
        accent: Hsla,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        with_tooltip(
            id,
            label,
            plain(SharedString::from(format!("{id}-button"))).on_press(action).child(
                div()
                    .relative()
                    .size(px(38.0))
                    .rounded_full()
                    .bg(white().opacity(0.16))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded_full()
                            .opacity(selected)
                            .bg(playback_gradient(accent, 1.0)),
                    )
                    .child(icon(system_image, 15.0, white(), false)),
            ),
        )
    }

    fn controls_block(
        &self,
        state: &Entity<TrackTrimOverlayState>,
        now: MotionTime,
        accent: Hsla,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let plate_icons = self.allows_plate.then(|| self.plate_mode_icons(state, now, accent, window, cx));
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .children(plate_icons)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .child(div().flex_1().min_w(px(0.0)).h(px(4.0)).child(self.track_timeline(state, accent)))
                    .child(self.pause_button(state, cx)),
            )
            .child(div().w_full().h(px(WAVEFORM_HEIGHT)).child(self.waveform_window(state, accent, cx)))
    }

    fn track_timeline(&self, state: &Entity<TrackTrimOverlayState>, accent: Hsla) -> impl IntoElement {
        let state = state.clone();
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                state.read(cx).paint_track_timeline(bounds, RiseClock::monotonic(), accent, window);
            },
        )
        .size_full()
    }

    fn pause_button(&self, state: &Entity<TrackTrimOverlayState>, cx: &App) -> impl IntoElement {
        let is_paused = state.read(cx).is_paused;
        let tapped = state.clone();
        with_tooltip(
            "track-trim-pause-hint",
            tr(if is_paused { "music_trim_preview" } else { "music_trim_stop" }),
            plain("track-trim-pause")
                .on_press(move |_, cx| tapped.update(cx, |this, cx| this.toggle_pause(cx)))
                .child(
                    div()
                        .size(px(40.0))
                        .rounded_full()
                        .bg(white())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(if is_paused { "play.fill" } else { "pause.fill" }, 15.0, black(), true)),
                ),
        )
    }

    fn waveform_window(&self, state: &Entity<TrackTrimOverlayState>, accent: Hsla, cx: &App) -> impl IntoElement {
        let this = state.read(cx);
        let strip_width = this.strip_width.clone();
        let width = strip_width.get();
        let grips = (width > 0.0).then(|| {
            let layout = this.window_layout(width);
            [layout.x, layout.x + layout.width].map(|border_x| {
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(border_x - EDGE_REACH))
                    .w(px(EDGE_REACH * 2.0))
                    .cursor(CursorStyle::ResizeLeftRight)
            })
        });
        let painted = state.clone();
        let strip = canvas(
            move |bounds, window, _| {
                let measured = f32::from(bounds.size.width);
                if strip_width.replace(measured) != measured {
                    window.request_animation_frame();
                }
            },
            move |bounds, _, window, cx| {
                let this = painted.read(cx);
                this.paint_waveform_window(bounds, RiseClock::monotonic(), accent, window);
                if matches!(this.drag_mode, Some(DragMode::ResizeLeading | DragMode::ResizeTrailing)) {
                    window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
                }
                if !this.is_paused {
                    window.request_animation_frame();
                }
            },
        )
        .absolute()
        .inset_0();
        div()
            .relative()
            .w_full()
            .h(px(WAVEFORM_HEIGHT))
            .overflow_hidden()
            .child(strip)
            .children(grips.into_iter().flatten())
            .gesture("track-trim-window-gesture", Self::window_gesture(state))
    }

    fn window_gesture(state: &Entity<TrackTrimOverlayState>) -> DragGesture {
        let changed = state.clone();
        let ended = state.clone();
        DragGesture::new(0.0)
            .on_changed(move |value, _, cx| {
                changed.update(cx, |this, cx| {
                    let width = this.strip_width.get().max(1.0);
                    if this.drag_mode.is_none() {
                        this.begin_drag(f32::from(value.start_location.x), width);
                    }
                    this.apply_drag(f32::from(value.translation.width), width, cx);
                });
            })
            .on_ended(move |_, _, cx| {
                ended.update(cx, |this, cx| this.end_drag(cx));
            })
    }
}

impl RenderOnce for TrackTrimOverlay {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let track_id = self.track.id.clone().unwrap_or_default();
        let key: SharedString = format!("{}-{track_id}", self.id).into();
        let state = window.use_keyed_state(ElementId::Name(key.clone()), cx, |_, cx| {
            TrackTrimOverlayState::new(
                self.track.clone(),
                self.initial_start_ms,
                self.initial_end_ms,
                self.initial_plate,
                cx,
            )
        });
        let accent = rise_ui::theme(cx).primary._100;
        let now = RiseClock::monotonic();
        let safe = EditorImageLayout::resolved_safe_area_insets(EdgeInsets::new(0.0, 0.0, 0.0, 0.0));
        let backdrop = self.backdrop.take();
        let dim = if self.opaque_backdrop && backdrop.is_none() { 1.0 } else { 0.55 };

        let plate = self.plate_preview(&state, now, window, cx);
        let content = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .px(px(16.0))
            .pt(px(safe.top.max(12.0)))
            .pb(px(safe.bottom.max(16.0) + 22.0))
            .child(self.top_bar(&state, accent, cx))
            .child(div().flex_1().min_h(px(0.0)))
            .child(div().flex().justify_center().children(plate))
            .child(div().flex_1().min_h(px(0.0)))
            .child(self.controls_block(&state, now, accent, window, cx));

        let focus_handle = state.read(cx).focus_handle.clone();
        let refocus = focus_handle.clone();
        let content = content.capture_any_mouse_down(move |_, window, cx| {
            let refocus = refocus.clone();
            window.defer(cx, move |window, cx| {
                if !refocus.contains_focused(window, cx) {
                    window.focus(&refocus, cx);
                }
            });
        });
        let keys = state.clone();
        let (key_cancel, key_confirm) = (self.on_cancel.clone(), self.on_confirm.clone());
        let allows_plate = self.allows_plate;
        let keys_up = state.clone();
        let appear = state.clone();
        let disappear = state.clone();
        let lifecycle_key: SharedString = format!("{key}-lifecycle").into();
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .track_focus(&focus_handle)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => Self::cancel(&keys, &key_cancel, window, cx),
                    "enter" | "space" if !focus_handle.is_focused(window) => return,
                    "enter" if event.keystroke.modifiers.modified() => return,
                    "enter" => Self::confirm(&keys, allows_plate, &key_confirm, window, cx),
                    _ => {
                        if !keys.update(cx, |this, cx| this.handle_key_down(event, cx)) {
                            return;
                        }
                    }
                }
                cx.stop_propagation();
            })
            .on_key_up(move |event: &KeyUpEvent, _, cx| {
                keys_up.update(cx, |this, cx| this.handle_key_up(event, cx));
            })
            .children(backdrop.map(|backdrop| {
                div().absolute().inset_0().bg(black()).overflow_hidden().child(backdrop)
            }))
            .child(div().absolute().inset_0().bg(black().opacity(dim)).occlude())
            .child(content)
            .on_appear(ElementId::Name(lifecycle_key), move |window, cx| {
                appear.update(cx, |this, cx| this.on_appear(window, cx));
            })
            .and_on_disappear(move |_, cx| {
                disappear.update(cx, |this, cx| this.stop_playback(cx));
            })
    }
}
