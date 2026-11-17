use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::LocalBoxFuture;
use gpui::{
    AnyElement, App, AsyncWindowContext, BorderStyle, Bounds, Context, Corners, DispatchPhase, Entity, FontWeight,
    Hitbox, HitboxBehavior, Hsla, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, Render, RenderOnce, SharedString, Size, Styled, Task, WeakEntity, Window, canvas,
    div, fill, hsla, point, px, quad, transparent_black,
};
use rise_i18n::tr;
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::lifecycle::{on_change, use_task};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::slider::slider;
use rise_ui::swiftui::stacks::{hstack, spacer_min_w, vstack};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use crate::modules::media_picker::editor::editor_video_playback_controller::EditorVideoPlaybackController;
use crate::modules::music::shared::utils::music::music_track_waveform::MusicTrackWaveform;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;

type EditorMusicLaneAction = Rc<dyn Fn(&mut Window, &mut App)>;
type EditorMusicLaneClipAction = Rc<dyn Fn(i64, i64, i64, &mut Window, &mut App)>;

/// second lane under the filmstrip, whole track as waveform with the clip window and two grips
/// shorter than the filmstrip on purpose, its just a backing track
#[derive(IntoElement)]
pub struct EditorMusicLaneRepresentable {
    id: SharedString,
    track: RiseStoredMusicTrack,
    video_duration_seconds: f64,
    offset_seconds: Binding<f64>,
    start_ms: Binding<i64>,
    end_ms: Binding<i64>,
    accent: Hsla,
    playback: Rc<EditorVideoPlaybackController>,
    minimum_length_ms: i64,
    on_interaction_started: EditorMusicLaneAction,
    on_clip_committed: EditorMusicLaneAction,
    on_request_volume: EditorMusicLaneAction,
}

impl EditorMusicLaneRepresentable {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<SharedString>,
        track: RiseStoredMusicTrack,
        video_duration_seconds: f64,
        offset_seconds: Binding<f64>,
        start_ms: Binding<i64>,
        end_ms: Binding<i64>,
        accent: Hsla,
        playback: Rc<EditorVideoPlaybackController>,
        minimum_length_ms: i64,
        on_interaction_started: impl Fn(&mut Window, &mut App) + 'static,
        on_clip_committed: impl Fn(&mut Window, &mut App) + 'static,
        on_request_volume: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            track,
            video_duration_seconds,
            offset_seconds,
            start_ms,
            end_ms,
            accent,
            playback,
            minimum_length_ms,
            on_interaction_started: Rc::new(on_interaction_started),
            on_clip_committed: Rc::new(on_clip_committed),
            on_request_volume: Rc::new(on_request_volume),
        }
    }

    fn video_duration_ms(&self) -> i64 {
        (self.video_duration_seconds * 1000.0).max(0.0).round() as i64
    }

    fn bind(
        &self,
        view: &Entity<EditorMusicLaneUIView>,
        coordinator: &Entity<EditorMusicLaneRepresentableCoordinator>,
        cx: &mut App,
    ) {
        let weak_view = view.downgrade();
        let weak_coordinator = coordinator.downgrade();
        let track = self.track.clone();
        let on_waveform_resolution_changed: Rc<dyn Fn(i64, &mut App)> = Rc::new(move |count, cx| {
            let _ = weak_coordinator.update(cx, |coordinator, cx| {
                coordinator.load_peaks(&track, weak_view.clone(), count, cx);
            });
        });
        let offset_seconds = self.offset_seconds.clone();
        let start_ms = self.start_ms.clone();
        let end_ms = self.end_ms.clone();
        let on_clip_changed: EditorMusicLaneClipAction = Rc::new(move |offset, start, end, window, cx| {
            offset_seconds.set(offset as f64 / 1000.0, window, cx);
            start_ms.set(start, window, cx);
            end_ms.set(end, window, cx);
        });
        let on_clip_committed = self.on_clip_committed.clone();
        let on_clip_committed: EditorMusicLaneClipAction =
            Rc::new(move |_, _, _, window, cx| on_clip_committed(window, cx));
        let on_interaction_started = self.on_interaction_started.clone();
        let on_long_press = self.on_request_volume.clone();
        view.update(cx, |view, _| {
            view.on_waveform_resolution_changed = Some(on_waveform_resolution_changed);
            view.on_interaction_started = Some(on_interaction_started);
            view.on_clip_changed = Some(on_clip_changed);
            view.on_clip_committed = Some(on_clip_committed);
            view.on_long_press = Some(on_long_press);
        });
    }
}

impl RenderOnce for EditorMusicLaneRepresentable {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let coordinator = window.use_keyed_state(SharedString::from(format!("{}-coordinator", self.id)), cx, |_, _| {
            EditorMusicLaneRepresentableCoordinator::default()
        });
        let mut made = false;
        let view = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            made = true;
            EditorMusicLaneUIView::new()
        });
        let video_duration_ms = self.video_duration_ms();
        let offset_ms = (self.offset_seconds.get(cx) * 1000.0).max(0.0).round() as i64;
        let start_ms = self.start_ms.get(cx);
        let end_ms = self.end_ms.get(cx);
        self.bind(&view, &coordinator, cx);
        if made {
            self.playback.attach_music_lane(&view, cx);
            let track_duration_ms = self.track.duration_ms.unwrap_or(0);
            let (accent, minimum_length_ms) = (self.accent, self.minimum_length_ms);
            view.update(cx, |view, _| {
                view.configure(
                    video_duration_ms,
                    track_duration_ms,
                    offset_ms,
                    start_ms,
                    end_ms,
                    accent,
                    minimum_length_ms,
                );
            });
        } else {
            let accent = self.accent;
            view.update(cx, |view, _| view.apply_clip(video_duration_ms, offset_ms, start_ms, end_ms, accent));
        }
        div().size_full().child(view)
    }
}

#[derive(Default)]
pub struct EditorMusicLaneRepresentableCoordinator {
    loaded_track_id: Option<String>,
    loaded_sample_count: i64,
    load_task: Option<Task<()>>,
}

impl EditorMusicLaneRepresentableCoordinator {
    pub fn load_peaks(
        &mut self,
        track: &RiseStoredMusicTrack,
        view: WeakEntity<EditorMusicLaneUIView>,
        desired: i64,
        cx: &mut Context<Self>,
    ) {
        let id = track.id.clone().unwrap_or_default();
        if id.is_empty() {
            return;
        }
        if self.loaded_track_id.as_deref() == Some(id.as_str()) && desired <= self.loaded_sample_count {
            return;
        }
        self.loaded_track_id = Some(id.clone());
        self.loaded_sample_count = desired;
        let track = track.clone();
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let Some(peaks) = MusicTrackWaveform::peaks(&track, desired, cx).await else {
                return;
            };
            let is_current = this
                .read_with(cx, |this, _| {
                    this.loaded_track_id.as_deref() == Some(id.as_str()) && this.loaded_sample_count == desired
                })
                .unwrap_or(false);
            if is_current {
                let _ = view.update(cx, |view, cx| view.apply_peaks(peaks, cx));
            }
        }));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorMusicLanePanKind {
    Left,
    Right,
    Slide,
}

struct EditorMusicLanePan {
    kind: EditorMusicLanePanKind,
    pressed_x: Pixels,
    last_x: Pixels,
    began: bool,
}

/// everything the canvas needs to paint one frame
#[derive(Clone)]
struct EditorMusicLaneFrame {
    waveform: EditorMusicWaveformView,
    waveform_frame: Bounds<f32>,
    selection_frame: Bounds<f32>,
    left_handle: Bounds<f32>,
    right_handle: Bounds<f32>,
    left_grip: Bounds<f32>,
    right_grip: Bounds<f32>,
    playhead: Bounds<f32>,
    accent: Hsla,
    has_chrome: bool,
}

pub struct EditorMusicLaneUIView {
    waveform: EditorMusicWaveformView,

    video_duration_ms: i64,
    track_duration_ms: i64,
    offset_ms: i64,
    start_ms: i64,
    end_ms: i64,
    minimum_length_ms: i64,
    accent: Hsla,
    playhead_fraction: f64,

    bounds_size: Size<f32>,
    selection_haptic: SelectionFeedbackGenerator,
    display_link: Option<Task<()>>,
    playhead_provider: Option<Rc<dyn Fn(&App) -> Option<f64>>>,
    pan: Option<EditorMusicLanePan>,

    pub on_interaction_started: Option<EditorMusicLaneAction>,
    pub on_waveform_resolution_changed: Option<Rc<dyn Fn(i64, &mut App)>>,
    reported_sample_count: i64,
    pub on_clip_changed: Option<EditorMusicLaneClipAction>,
    pub on_clip_committed: Option<EditorMusicLaneClipAction>,
    pub on_long_press: Option<EditorMusicLaneAction>,
}

impl Default for EditorMusicLaneUIView {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorMusicLaneUIView {
    const LANE_INSET: f32 = 9.0;
    const LANE_VERTICAL_INSET: f32 = 3.0;
    const CORNER_RADIUS: f32 = 6.0;
    const HANDLE_WIDTH: f32 = 11.0;
    const HANDLE_HIT_X: f32 = 12.0;
    const HANDLE_HIT_Y: f32 = 8.0;
    const PAN_SLOP: f32 = 3.0;
    const PLAYHEAD_STEP: Duration = Duration::from_micros(8_333);

    pub fn new() -> Self {
        Self {
            waveform: EditorMusicWaveformView::default(),
            video_duration_ms: 0,
            track_duration_ms: 0,
            offset_ms: 0,
            start_ms: 0,
            end_ms: 0,
            minimum_length_ms: 1_000,
            accent: hsla(0.0, 0.0, 1.0, 1.0),
            playhead_fraction: 0.0,
            bounds_size: Size { width: 0.0, height: 0.0 },
            selection_haptic: SelectionFeedbackGenerator::new(),
            display_link: None,
            playhead_provider: None,
            pan: None,
            on_interaction_started: None,
            on_waveform_resolution_changed: None,
            reported_sample_count: 0,
            on_clip_changed: None,
            on_clip_committed: None,
            on_long_press: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        video_duration_ms: i64,
        track_duration_ms: i64,
        offset_ms: i64,
        start_ms: i64,
        end_ms: i64,
        accent: Hsla,
        minimum_length_ms: i64,
    ) {
        self.video_duration_ms = video_duration_ms.max(0);
        if self.track_duration_ms != track_duration_ms.max(0) {
            self.reported_sample_count = 0;
        }
        self.track_duration_ms = track_duration_ms.max(0);
        self.accent = accent;
        self.minimum_length_ms = minimum_length_ms.max(1);
        self.apply_clamped(offset_ms, start_ms, end_ms);
    }

    pub fn apply_clip(&mut self, video_duration_ms: i64, offset_ms: i64, start_ms: i64, end_ms: i64, accent: Hsla) {
        self.video_duration_ms = video_duration_ms.max(0);
        self.accent = accent;
        self.apply_clamped(offset_ms, start_ms, end_ms);
    }

    pub fn apply_peaks(&mut self, peaks: Vec<f32>, cx: &mut Context<Self>) {
        self.waveform.peaks = Some(Rc::new(peaks));
        cx.notify();
    }

    pub fn waveform_sample_count(&self, track_duration_ms: i64) -> i64 {
        if self.video_duration_ms <= 0 || track_duration_ms <= 0 || self.bounds_size.width <= 1.0 {
            return 0;
        }
        let px_per_ms = self.usable_width() / self.video_duration_ms as f32;
        let total_width = track_duration_ms as f32 * px_per_ms;
        ((total_width / 4.0) as i64).clamp(24, 2_048)
    }

    pub fn follow_playhead(&mut self, provider: Rc<dyn Fn(&App) -> Option<f64>>, cx: &mut Context<Self>) {
        self.playhead_provider = Some(provider);
        if self.display_link.is_some() {
            return;
        }
        self.display_link = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Self::PLAYHEAD_STEP).await;
                if this.update(cx, |this, cx| this.step_playhead(cx)).is_err() {
                    return;
                }
            }
        }));
    }

    pub fn stop_following_playhead(&mut self) {
        self.display_link = None;
        self.playhead_provider = None;
    }

    pub fn update_playhead(&mut self, fraction: f64, cx: &mut Context<Self>) {
        self.playhead_fraction = fraction.clamp(0.0, 1.0);
        cx.notify();
    }

    fn step_playhead(&mut self, cx: &mut Context<Self>) {
        let Some(provider) = self.playhead_provider.clone() else { return };
        let Some(value) = provider(cx) else { return };
        let clamped = value.clamp(0.0, 1.0);
        if (clamped - self.playhead_fraction).abs() <= 0.0005 {
            return;
        }
        self.playhead_fraction = clamped;
        cx.notify();
    }

    fn layout_subviews(&mut self, size: Size<f32>, cx: &mut Context<Self>) {
        if self.bounds_size != size {
            self.bounds_size = size;
            cx.notify();
        }
        let count = self.waveform_sample_count(self.track_duration_ms);
        if count <= self.reported_sample_count {
            return;
        }
        self.reported_sample_count = count;
        if let Some(on_waveform_resolution_changed) = self.on_waveform_resolution_changed.clone() {
            on_waveform_resolution_changed(count, cx);
        }
    }

    fn usable_width(&self) -> f32 {
        (self.bounds_size.width - Self::LANE_INSET * 2.0).max(1.0)
    }

    fn x_for_ms(&self, ms: i64) -> f32 {
        if self.video_duration_ms <= 0 {
            return Self::LANE_INSET;
        }
        Self::LANE_INSET + (ms as f64 / self.video_duration_ms as f64) as f32 * self.usable_width()
    }

    /// computes the chrome and playhead layout for the frame being painted
    fn layout_frame(&self) -> EditorMusicLaneFrame {
        let lane_height = (self.bounds_size.height - Self::LANE_VERTICAL_INSET * 2.0).max(0.0);
        let lane_y = Self::LANE_VERTICAL_INSET;
        let waveform_frame = Bounds {
            origin: point(Self::LANE_INSET, lane_y),
            size: Size { width: (self.bounds_size.width - Self::LANE_INSET * 2.0).max(0.0), height: lane_height },
        };
        let lx = self.x_for_ms(self.offset_ms);
        let rx = self.x_for_ms(self.offset_ms + (self.end_ms - self.start_ms));

        let px_per_ms = if self.video_duration_ms > 0 { self.usable_width() / self.video_duration_ms as f32 } else { 0.0 };
        let mut waveform = self.waveform.clone();
        waveform.track_origin_x = (lx - Self::LANE_INSET) - self.start_ms as f32 * px_per_ms;
        waveform.track_width = self.track_duration_ms.max(1) as f32 * px_per_ms;
        waveform.clip_range = ((lx - Self::LANE_INSET).max(0.0), (rx - Self::LANE_INSET).max(0.0));

        let clip_width = (rx - lx).max(0.0);
        let selection_frame = Bounds { origin: point(lx, lane_y), size: Size { width: clip_width, height: lane_height } };
        let hw = Self::HANDLE_WIDTH.min((clip_width / 2.0).max(1.0));
        let left_handle = Bounds { origin: point(lx, lane_y), size: Size { width: hw, height: lane_height } };
        let right_handle =
            Bounds { origin: point(lx + clip_width - hw, lane_y), size: Size { width: hw, height: lane_height } };
        let grip_size = Size { width: 2.0, height: (lane_height * 0.4).clamp(6.0, 12.0) };
        let grip_at = |handle: Bounds<f32>| Bounds {
            origin: point(
                handle.origin.x + (hw - grip_size.width) / 2.0,
                lane_y + (lane_height - grip_size.height) / 2.0,
            ),
            size: grip_size,
        };
        let playhead_x = Self::LANE_INSET + self.playhead_fraction as f32 * self.usable_width();
        EditorMusicLaneFrame {
            waveform,
            waveform_frame,
            selection_frame,
            left_handle,
            right_handle,
            left_grip: grip_at(left_handle),
            right_grip: grip_at(right_handle),
            playhead: Bounds {
                origin: point(playhead_x - 1.0, lane_y),
                size: Size { width: 2.0, height: lane_height },
            },
            accent: self.accent,
            has_chrome: self.video_duration_ms > 0 && self.bounds_size.width > 1.0,
        }
    }

    /// keeps the clip inside the track and inside the video
    fn apply_clamped(&mut self, offset_ms: i64, start_ms: i64, end_ms: i64) {
        if self.video_duration_ms <= 0 || self.track_duration_ms <= 0 {
            self.offset_ms = 0;
            self.start_ms = 0;
            self.end_ms = 0;
            return;
        }
        let mut start = start_ms.min(self.track_duration_ms - 1).max(0);
        let mut length = self.minimum_length_ms.max(end_ms - start);
        length = length.min(self.track_duration_ms - start);
        length = length.min(self.video_duration_ms);
        if length < self.minimum_length_ms {
            start = (self.track_duration_ms - self.minimum_length_ms).max(0);
            length = self.minimum_length_ms.min(self.track_duration_ms - start);
        }
        let offset = offset_ms.min(self.video_duration_ms - length).max(0);
        self.start_ms = start;
        self.end_ms = start + length;
        self.offset_ms = offset;
    }

    fn ms_per_point(&self) -> f64 {
        self.video_duration_ms as f64 / f64::from(self.usable_width())
    }

    fn pan_delta_ms(&self, translation: Pixels) -> i64 {
        (f64::from(f32::from(translation)) * self.ms_per_point()).round() as i64
    }

    fn fire_action(action: Option<EditorMusicLaneAction>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(action) = action {
            window.defer(cx, move |window, cx| action(window, cx));
        }
    }

    fn fire_clip(&self, action: Option<EditorMusicLaneClipAction>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(action) = action {
            let (offset, start, end) = (self.offset_ms, self.start_ms, self.end_ms);
            window.defer(cx, move |window, cx| action(offset, start, end, window, cx));
        }
    }

    fn begin_gesture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.selection_haptic.prepare();
        let _ = self.selection_haptic.selection_changed();
        Self::fire_action(self.on_interaction_started.clone(), window, cx);
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, origin: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let location = point(f32::from(event.position.x - origin.x), f32::from(event.position.y - origin.y));
        if event.button == MouseButton::Right {
            self.handle_hold(location, window, cx);
            return;
        }
        if event.button != MouseButton::Left || self.video_duration_ms <= 0 || self.track_duration_ms <= 0 {
            return;
        }
        let frame = self.layout_frame();
        let hit_handle = |handle: Bounds<f32>| {
            location.x >= handle.origin.x - Self::HANDLE_HIT_X
                && location.x <= handle.origin.x + handle.size.width + Self::HANDLE_HIT_X
                && location.y >= handle.origin.y - Self::HANDLE_HIT_Y
                && location.y <= handle.origin.y + handle.size.height + Self::HANDLE_HIT_Y
        };
        let lx = self.x_for_ms(self.offset_ms);
        let rx = self.x_for_ms(self.offset_ms + (self.end_ms - self.start_ms));
        let kind = if hit_handle(frame.right_handle) {
            EditorMusicLanePanKind::Right
        } else if hit_handle(frame.left_handle) {
            EditorMusicLanePanKind::Left
        } else if location.x >= lx
            && location.x <= rx
            && location.y >= 0.0
            && location.y <= self.bounds_size.height
        {
            EditorMusicLanePanKind::Slide
        } else {
            return;
        };
        cx.stop_propagation();
        self.pan = Some(EditorMusicLanePan {
            kind,
            pressed_x: event.position.x,
            last_x: event.position.x,
            began: false,
        });
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.pan.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_gesture(window, cx);
            return;
        }
        let Some(pan) = self.pan.as_mut() else { return };
        let is_first_change = !pan.began;
        if is_first_change && f32::from(event.position.x - pan.pressed_x).abs() < Self::PAN_SLOP {
            return;
        }
        pan.began = true;
        let translation = event.position.x - pan.last_x;
        pan.last_x = event.position.x;
        let kind = pan.kind;
        if is_first_change {
            self.begin_gesture(window, cx);
        }
        let delta = self.pan_delta_ms(translation);
        match kind {
            EditorMusicLanePanKind::Left => self.pan_left(delta),
            EditorMusicLanePanKind::Right => self.apply_clamped(self.offset_ms, self.start_ms, self.end_ms + delta),
            EditorMusicLanePanKind::Slide => self.apply_clamped(self.offset_ms + delta, self.start_ms, self.end_ms),
        }
        self.fire_clip(self.on_clip_changed.clone(), window, cx);
        cx.notify();
    }

    fn mouse_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_gesture(window, cx);
    }

    fn finish_gesture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pan) = self.pan.take() else { return };
        if pan.began {
            self.fire_clip(self.on_clip_committed.clone(), window, cx);
        }
    }

    fn pan_left(&mut self, delta: i64) {
        let right_edge = self.offset_ms + (self.end_ms - self.start_ms);
        let new_offset = (self.offset_ms + delta).min(right_edge - self.minimum_length_ms).max(0);
        let applied = new_offset - self.offset_ms;
        self.apply_clamped(new_offset, self.start_ms + applied, self.end_ms);
    }

    fn handle_hold(&mut self, location: Point<f32>, window: &mut Window, cx: &mut Context<Self>) {
        if location.x < 0.0
            || location.x > self.bounds_size.width
            || location.y < 0.0
            || location.y > self.bounds_size.height
        {
            return;
        }
        cx.stop_propagation();
        let _ = self.selection_haptic.prepare();
        let _ = self.selection_haptic.selection_changed();
        Self::fire_action(self.on_long_press.clone(), window, cx);
    }

    fn paint_frame(frame: &EditorMusicLaneFrame, origin: Point<Pixels>, window: &mut Window) {
        let at = |bounds: Bounds<f32>| Bounds {
            origin: point(origin.x + px(bounds.origin.x), origin.y + px(bounds.origin.y)),
            size: Size { width: px(bounds.size.width), height: px(bounds.size.height) },
        };
        window.paint_quad(
            fill(at(frame.waveform_frame), hsla(0.0, 0.0, 0.12, 1.0)).corner_radii(px(Self::CORNER_RADIUS)),
        );
        let waveform_bounds = at(frame.waveform_frame);
        window.with_content_mask(Some(gpui::ContentMask { bounds: waveform_bounds }), |window| {
            frame.waveform.draw(waveform_bounds, window);
        });
        if !frame.has_chrome {
            return;
        }
        let selection_bounds = at(frame.selection_frame);
        window.with_content_mask(Some(gpui::ContentMask { bounds: selection_bounds }), |window| {
            window.paint_quad(
                fill(at(frame.left_handle), frame.accent).corner_radii(Corners {
                    top_left: px(Self::CORNER_RADIUS),
                    bottom_left: px(Self::CORNER_RADIUS),
                    top_right: px(0.0),
                    bottom_right: px(0.0),
                }),
            );
            window.paint_quad(
                fill(at(frame.right_handle), frame.accent).corner_radii(Corners {
                    top_left: px(0.0),
                    bottom_left: px(0.0),
                    top_right: px(Self::CORNER_RADIUS),
                    bottom_right: px(Self::CORNER_RADIUS),
                }),
            );
            let grip_color = hsla(0.0, 0.0, 0.12, 0.9);
            window.paint_quad(fill(at(frame.left_grip), grip_color).corner_radii(px(1.0)));
            window.paint_quad(fill(at(frame.right_grip), grip_color).corner_radii(px(1.0)));
        });
        window.paint_quad(quad(
            selection_bounds,
            px(Self::CORNER_RADIUS),
            transparent_black(),
            px(2.0),
            frame.accent,
            BorderStyle::Solid,
        ));
        window.paint_quad(fill(at(frame.playhead), hsla(0.0, 0.0, 1.0, 1.0)).corner_radii(px(1.0)));
    }
}

impl Render for EditorMusicLaneUIView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = self.layout_frame();
        let view = cx.entity().downgrade();
        let measured = view.clone();
        let surface = canvas(
            |bounds, window, _| {
                let reach = Bounds {
                    origin: point(bounds.origin.x - px(Self::HANDLE_HIT_X), bounds.origin.y - px(Self::HANDLE_HIT_Y)),
                    size: Size {
                        width: bounds.size.width + px(Self::HANDLE_HIT_X * 2.0),
                        height: bounds.size.height + px(Self::HANDLE_HIT_Y * 2.0),
                    },
                };
                window.insert_hitbox(reach, HitboxBehavior::Normal)
            },
            move |bounds, hitbox: Hitbox, window, _| {
                let origin = bounds.origin;
                Self::paint_frame(&frame, origin, window);
                let down_view = view.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                        return;
                    }
                    let _ = down_view.update(cx, |this, cx| this.mouse_down(event, origin, window, cx));
                });
                let move_view = view.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let _ = move_view.update(cx, |this, cx| this.mouse_move(event, window, cx));
                });
                let up_view = view.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                        return;
                    }
                    let _ = up_view.update(cx, |this, cx| this.mouse_up(window, cx));
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let measure_id = SharedString::from(format!("editor-music-lane-{}", cx.entity_id()));
        div()
            .relative()
            .size_full()
            .child(surface)
            .child(measure(measure_id, move |bounds, _, cx| {
                let size = Size { width: f32::from(bounds.size.width), height: f32::from(bounds.size.height) };
                let _ = measured.update(cx, |this, cx| this.layout_subviews(size, cx));
            }))
    }
}

/// waveform of the whole track at video scale, same points per second as the filmstrip
/// clip part is full strength and the rest dimmed
/// runs off the right edge if the track is longer
#[derive(Clone)]
pub struct EditorMusicWaveformView {
    pub peaks: Option<Rc<Vec<f32>>>,
    pub track_origin_x: f32,
    pub track_width: f32,
    pub clip_range: (f32, f32),
}

impl Default for EditorMusicWaveformView {
    fn default() -> Self {
        Self { peaks: None, track_origin_x: 0.0, track_width: 0.0, clip_range: (0.0, 0.0) }
    }
}

impl EditorMusicWaveformView {
    const PLACEHOLDER_PEAK: f32 = 0.3;
    const PLACEHOLDER_COUNT: usize = 96;

    fn draw(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if width <= 1.0 || self.track_width <= 1.0 {
            return;
        }
        let count = self.peaks.as_ref().map_or(Self::PLACEHOLDER_COUNT, |peaks| peaks.len()).max(1);
        let step = self.track_width / count as f32;
        let bar_width = (step * 0.55).clamp(1.5, 3.0);
        let available = height - 6.0;

        for index in Self::visible_bars(count, self.track_origin_x, step, bar_width, width) {
            let x = self.track_origin_x + index as f32 * step + (step - bar_width) / 2.0;
            let center = x + bar_width / 2.0;
            let in_clip = center >= self.clip_range.0 && center <= self.clip_range.1;
            let peak = self.peaks.as_ref().and_then(|peaks| peaks.get(index).copied()).unwrap_or(Self::PLACEHOLDER_PEAK);
            let value = peak.clamp(0.12, 1.0);
            let bar_height = (value * available * if in_clip { 1.0 } else { 0.62 }).max(2.0);
            let bar = Bounds {
                origin: point(bounds.origin.x + px(x), bounds.origin.y + px((height - bar_height) / 2.0)),
                size: Size { width: px(bar_width), height: px(bar_height) },
            };
            let color = hsla(0.0, 0.0, 1.0, if in_clip { 1.0 } else { 0.34 });
            window.paint_quad(fill(bar, color).corner_radii(px(bar_width / 2.0)));
        }
    }

    fn visible_bars(count: usize, origin_x: f32, step: f32, bar_width: f32, width: f32) -> Range<usize> {
        let offset = (step - bar_width) / 2.0;
        let first = ((-bar_width - origin_x - offset) / step).ceil().max(0.0) as usize;
        let last = ((width - origin_x - offset) / step).floor();
        if last < 0.0 {
            return 0..0;
        }
        first.min(count)..(last as usize + 1).min(count)
    }
}

/// bottom sound sheet with both lanes together since you tune the balance between them
#[derive(IntoElement)]
pub struct EditorSoundSheet {
    video_volume: Binding<f64>,
    music_volume: Binding<f64>,
    music_title: Option<String>,
    has_music: bool,
    on_close: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl EditorSoundSheet {
    pub fn new(
        video_volume: Binding<f64>,
        music_volume: Binding<f64>,
        music_title: Option<String>,
        has_music: bool,
    ) -> Self {
        Self { video_volume, music_volume, music_title, has_music, on_close: None }
    }

    pub fn on_close(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(action));
        self
    }

    fn row(
        id: &'static str,
        icon: &'static str,
        title: String,
        subtitle: Option<String>,
        value: Binding<f64>,
        theme: &AppTheme,
        cx: &App,
    ) -> AnyElement {
        let percent = (value.get(cx) * 100.0).round() as i64;
        let badge = div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(29.0))
            .h(px(29.0))
            .rounded(px(8.0))
            .bg(theme.bg._300)
            .children(IconUi::sized(icon, px(14.0), theme.text.primary, false));
        let mut titles = vstack(2.0)
            .items_start()
            .child(MainText::new(title).px(15.0).font_weight(FontWeight::SEMIBOLD));
        if let Some(subtitle) = subtitle.filter(|subtitle| !subtitle.is_empty()) {
            titles = titles.child(SecondaryText::new(subtitle).px(12.0).line_limit(1));
        }
        vstack(8.0)
            .w_full()
            .items_start()
            .child(
                hstack(10.0)
                    .w_full()
                    .child(badge)
                    .child(titles)
                    .child(spacer_min_w(0.0))
                    .child(
                        MainText::new(format!("{percent}%"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.secondary),
                    ),
            )
            .child(div().w_full().child(slider(id, value, 0.0..=1.0).tint(theme.primary._100)))
            .into_any_element()
    }
}

impl RenderOnce for EditorSoundSheet {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme();
        let mut header = hstack(10.0)
            .w_full()
            .pt(px(4.0))
            .child(MainText::new(tr("editor_sound_title")).px(17.0).font_weight(FontWeight::BOLD))
            .child(spacer_min_w(0.0));
        if let Some(on_close) = self.on_close.clone() {
            header = header.child(
                plain("editor-sound-close")
                    .on_press(move |window, cx| on_close(window, cx))
                    .flex_none()
                    .size(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.bg._300)
                    .rounded_full()
                    .children(IconUi::sized("xmark", px(12.0), theme.text.secondary, false)),
            );
        }

        let mut column = vstack(22.0)
            .w_full()
            .items_start()
            .px(px(16.0))
            .child(header)
            .child(Self::row(
                "editor-sound-video",
                "video.fill",
                tr("editor_volume_video"),
                None,
                self.video_volume,
                &theme,
                cx,
            ));
        if self.has_music {
            column = column.child(Self::row(
                "editor-sound-music",
                "music.note",
                tr("editor_volume_music"),
                self.music_title,
                self.music_volume,
                &theme,
                cx,
            ));
        }
        column.pb(px(20.0))
    }
}

type EditorMusicPreviewAction = Rc<dyn Fn(&mut Window, &mut App)>;

struct EditorMusicPreviewBackgroundObserver {
    handler: Rc<RefCell<EditorMusicPreviewAction>>,
    _subscription: RiseSubscription,
    _task: Task<()>,
}

#[derive(IntoElement)]
pub struct EditorMusicPreviewObservers {
    id: SharedString,
    signature: String,
    volume: f64,
    is_playing: bool,
    clip_picker_open: bool,
    on_track_changed: Rc<dyn Fn(AsyncWindowContext) -> LocalBoxFuture<'static, ()>>,
    on_volume_changed: EditorMusicPreviewAction,
    on_playback_changed: EditorMusicPreviewAction,
    on_background: EditorMusicPreviewAction,
    content: AnyElement,
}

impl EditorMusicPreviewObservers {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<SharedString>,
        signature: String,
        volume: f64,
        is_playing: bool,
        clip_picker_open: bool,
        on_track_changed: impl Fn(AsyncWindowContext) -> LocalBoxFuture<'static, ()> + 'static,
        on_volume_changed: impl Fn(&mut Window, &mut App) + 'static,
        on_playback_changed: impl Fn(&mut Window, &mut App) + 'static,
        on_background: impl Fn(&mut Window, &mut App) + 'static,
        content: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            signature,
            volume,
            is_playing,
            clip_picker_open,
            on_track_changed: Rc::new(on_track_changed),
            on_volume_changed: Rc::new(on_volume_changed),
            on_playback_changed: Rc::new(on_playback_changed),
            on_background: Rc::new(on_background),
            content: content.into_any_element(),
        }
    }
}

impl RenderOnce for EditorMusicPreviewObservers {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let child_id = |suffix: &str| SharedString::from(format!("{}-{suffix}", self.id));

        let on_track_changed = self.on_track_changed.clone();
        use_task(child_id("track"), self.signature.clone(), window, cx, async move |cx| {
            on_track_changed(cx.clone()).await;
        });
        let on_volume_changed = self.on_volume_changed.clone();
        on_change(child_id("volume"), self.volume.to_bits(), false, window, cx, move |_, _, window, cx| {
            on_volume_changed(window, cx);
        });
        let on_playback_changed = self.on_playback_changed.clone();
        on_change(child_id("playing"), self.is_playing, false, window, cx, move |_, _, window, cx| {
            on_playback_changed(window, cx);
        });
        let on_playback_changed = self.on_playback_changed.clone();
        on_change(child_id("clip-picker"), self.clip_picker_open, false, window, cx, move |_, _, window, cx| {
            on_playback_changed(window, cx);
        });
        let on_background = self.on_background.clone();
        let observer = window.use_keyed_state(child_id("background"), cx, |window, cx| {
            let handler = Rc::new(RefCell::new(on_background.clone()));
            let (sender, mut inbox) = mpsc::unbounded::<()>();
            let subscription = RiseAppEvents::default_center().observe(
                UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
                move |_| {
                    let _ = sender.unbounded_send(());
                },
            );
            let current = handler.clone();
            let task = window.spawn(cx, async move |cx| {
                while inbox.next().await.is_some() {
                    let action = current.borrow().clone();
                    let _ = cx.update(|window, cx| action(window, cx));
                }
            });
            EditorMusicPreviewBackgroundObserver { handler, _subscription: subscription, _task: task }
        });
        *observer.read(cx).handler.borrow_mut() = self.on_background.clone();

        self.content
    }
}
