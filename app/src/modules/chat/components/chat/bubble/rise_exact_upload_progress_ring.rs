use std::cell::Cell;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, ElementId, Entity, EntityId, FontWeight, Hsla, IntoElement, ParentElement, PathBuilder, Pixels,
    Point, RenderOnce, SharedString, Size, Styled, Task, WeakEntity, Window, canvas, div, point, px,
    transparent_black, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::rise_upload::rise_upload_progress::{RiseUploadKey, RiseUploadProgress};
use rise_i18n::{FormatArg, format_printf, tr};
use rise_theme::ui_color_system::UIColor;
use rise_theme::{Appearance, RiseThemeAppearance, ThemeStore};
use rise_ui::error_text_ui::system_red;
use rise_ui::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::linear;
use rise_ui::swiftui::text::TruncationMode;

use crate::core::lib::byte_format::ByteFormat;
use crate::core::rise_engine_bridge::rise_engine_upload_phase::RiseEngineUploadPhase;
use crate::modules::chat::components::chat::bubble::rise_exact_upload_progress_ring_hooks::rise_exact_upload_progress_ring_hooks;
use crate::modules::chat::stores::message::message_actions::message_types::MessageMediaItem;

/// maps upload phase to the visible arc of upload rings
/// both renderers read the same hub value so the mapping lives here once
pub enum RiseUploadRingPresentation {}

impl RiseUploadRingPresentation {
    const PREPARING_BAND: f64 = 0.25;

    /// all bytes sent but server hasnt acked, arc is full and only the rotation moves
    pub fn is_awaiting_server(progress: &RiseUploadProgress) -> bool {
        if *progress.phase() != RiseEngineUploadPhase::Uploading || progress.total_bytes() <= 0 {
            return false;
        }
        progress.completed_bytes() >= progress.total_bytes()
    }

    pub fn minimum_visible_fraction(phase: &RiseEngineUploadPhase) -> f64 {
        match phase {
            RiseEngineUploadPhase::Preparing | RiseEngineUploadPhase::Processing => 0.01,
            RiseEngineUploadPhase::Uploading
            | RiseEngineUploadPhase::Completed
            | RiseEngineUploadPhase::Failed { .. } => 0.08,
        }
    }

    pub fn visible_fraction(progress: &RiseUploadProgress) -> f64 {
        if Self::is_awaiting_server(progress) {
            return 1.0;
        }
        let fraction = progress.fraction().max(0.0).min(1.0);
        let banded = match progress.phase() {
            RiseEngineUploadPhase::Preparing => fraction * Self::PREPARING_BAND,
            RiseEngineUploadPhase::Uploading | RiseEngineUploadPhase::Processing => {
                Self::PREPARING_BAND + fraction * (1.0 - Self::PREPARING_BAND)
            }
            RiseEngineUploadPhase::Completed | RiseEngineUploadPhase::Failed { .. } => fraction,
        };
        Self::minimum_visible_fraction(progress.phase()).max(banded)
    }

    pub fn status_text(progress: &RiseUploadProgress) -> String {
        if Self::is_awaiting_server(progress) {
            return tr("chat_media_upload_sending");
        }
        match progress.phase() {
            RiseEngineUploadPhase::Preparing | RiseEngineUploadPhase::Processing => {
                let percent = (progress.fraction().min(1.0).max(0.0) * 100.0).round() as i64;
                format!("{} {}%", tr("chat_media_upload_processing"), percent)
            }
            RiseEngineUploadPhase::Uploading => {
                if progress.total_bytes() <= 0 {
                    return tr("chat_media_upload_loading");
                }
                format_printf(
                    &tr("chat_media_upload_progress"),
                    &[
                        FormatArg::Text(
                            ByteFormat::telegram_transferred(progress.completed_bytes(), progress.total_bytes()).into(),
                        ),
                        FormatArg::Text(ByteFormat::telegram(progress.total_bytes()).into()),
                    ],
                )
            }
            RiseEngineUploadPhase::Completed | RiseEngineUploadPhase::Failed { .. } => {
                tr("chat_media_upload_loading")
            }
        }
    }
}

/// upload key a leaf follows, only an active slot measured for real with an owner
fn upload_key(owner_local_id: &str, slot_index: i64, is_active: bool, is_layout_measurement: bool) -> Option<RiseUploadKey> {
    let normalized_owner_id = trimming_whitespaces_and_newlines(owner_local_id);
    if is_active && !is_layout_measurement && !normalized_owner_id.is_empty() {
        Some(RiseUploadKey::new(normalized_owner_id, slot_index.max(0)))
    } else {
        None
    }
}

/// leaf only upload ring renderer
/// progress lives in the leaf own state so ticks never invalidate the message row
#[derive(IntoElement)]
pub struct RiseExactUploadProgressRing {
    id: ElementId,
    owner_local_id: String,
    item: MessageMediaItem,
    is_layout_measurement: bool,
    progress_color: Hsla,
    track_color: Hsla,
    line_width: f32,
    ring_inset: f32,
}

impl RiseExactUploadProgressRing {
    pub fn new(
        id: impl Into<ElementId>,
        owner_local_id: impl Into<String>,
        item: MessageMediaItem,
        is_layout_measurement: bool,
    ) -> Self {
        Self {
            id: id.into(),
            owner_local_id: owner_local_id.into(),
            item,
            is_layout_measurement,
            progress_color: white(),
            track_color: transparent_black(),
            line_width: 2.5,
            ring_inset: 0.0,
        }
    }

    pub fn progress_color(mut self, progress_color: Hsla) -> Self {
        self.progress_color = progress_color;
        self
    }

    pub fn track_color(mut self, track_color: Hsla) -> Self {
        self.track_color = track_color;
        self
    }

    pub fn line_width(mut self, line_width: f32) -> Self {
        self.line_width = line_width;
        self
    }

    pub fn ring_inset(mut self, ring_inset: f32) -> Self {
        self.ring_inset = ring_inset;
        self
    }
}

impl RenderOnce for RiseExactUploadProgressRing {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view: Entity<RiseExactUploadProgressRingView> = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("upload-ring")),
            cx,
            |_, _| RiseExactUploadProgressRingView::new(),
        );
        let failure_color = system_red(cx);
        let reduce_motion = reduce_motion(cx);
        view.update(cx, |view, cx| {
            view.failure_color = failure_color;
            view.reduce_motion = reduce_motion;
            view.configure(
                &self.owner_local_id,
                self.item.local_slot_index.max(0),
                self.item.is_uploading || self.item.is_compressing,
                self.item.is_compressing,
                self.item.normalized_transfer_progress(),
                self.item.clamped_uploaded_bytes(),
                self.item.effective_total_bytes(),
                self.is_layout_measurement,
                self.progress_color,
                self.track_color,
                self.line_width,
                self.ring_inset,
                cx,
            );
        });

        let snapshot = view.read(cx).frame(RiseClock::monotonic());
        let painted = view.read(cx).painted.clone();
        let keys = ring_swap_keys(view.entity_id());
        if snapshot.needs_next_frame && !view.read(cx).is_ticking {
            view.update(cx, |view, _| view.is_ticking = true);
            let weak = view.downgrade();
            window.on_next_frame(move |window, cx| ring_tick(weak, window, cx));
        }
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                if snapshot.is_hidden {
                    painted.set(None);
                    return;
                }
                for ((path, color), key) in ring_paths(bounds, &snapshot).into_iter().zip(keys) {
                    if let Some(path) = path {
                        window.paint_swappable_path(path, color, key, bounds.origin);
                    }
                }
                painted.set(Some(bounds.size));
            },
        )
        .size_full()
    }
}

#[derive(Clone, Copy, Debug)]
struct RingFrame {
    is_hidden: bool,
    needs_next_frame: bool,
    line_width: f32,
    ring_inset: f32,
    track_color: Hsla,
    progress_color: Hsla,
    stroke_end: f32,
    rotation: f32,
}

/// points along the oval of bounds inset by inset, from start through sweep radians
/// clockwise on a y down screen starting at three oclock
fn oval_points(bounds: Bounds<Pixels>, inset: f32, start: f32, sweep: f32) -> Vec<Point<Pixels>> {
    let center = bounds.center();
    let radius_x = f32::from(bounds.size.width) / 2.0 - inset;
    let radius_y = f32::from(bounds.size.height) / 2.0 - inset;
    let steps = ((sweep.abs() / (2.0 * PI)) * 96.0).ceil().max(2.0) as usize;
    (0..=steps)
        .map(|step| {
            let angle = start + sweep * step as f32 / steps as f32;
            point(center.x + px(radius_x * angle.cos()), center.y + px(radius_y * angle.sin()))
        })
        .collect()
}

fn stroke_path(points: &[Point<Pixels>], line_width: f32, closed: bool) -> Option<gpui::Path<Pixels>> {
    let mut builder = PathBuilder::stroke(px(line_width));
    builder.add_polygon(points, closed);
    builder.build().ok()
}

/// round cap for the arc since gpui strokes have butt ends
/// each end gets a dot of the line width
fn round_cap(at: Point<Pixels>, line_width: f32) -> Option<gpui::Path<Pixels>> {
    let radius = line_width / 2.0;
    let points: Vec<Point<Pixels>> = (0..16)
        .map(|step| {
            let angle = 2.0 * PI * step as f32 / 16.0;
            point(at.x + px(radius * angle.cos()), at.y + px(radius * angle.sin()))
        })
        .collect();
    let mut builder = PathBuilder::fill();
    builder.add_polygon(&points, true);
    builder.build().ok()
}

fn empty_path(bounds: Bounds<Pixels>) -> Option<gpui::Path<Pixels>> {
    let at = bounds.origin;
    let mut builder = PathBuilder::fill();
    builder.add_polygon(&[at, point(at.x + px(1.0), at.y), point(at.x, at.y + px(1.0))], true);
    builder.build().ok()
}

/// the track oval, the progress arc and its two caps
/// always four paths so a path swap finds every slot from frame to frame
fn ring_paths(bounds: Bounds<Pixels>, frame: &RingFrame) -> [(Option<gpui::Path<Pixels>>, Hsla); 4] {
    let hidden = transparent_black();
    let empty = || (empty_path(bounds), hidden);
    let inset = frame.ring_inset.max(0.0) + frame.line_width / 2.0;
    if f32::from(bounds.size.width) - 2.0 * inset <= 0.0 || f32::from(bounds.size.height) - 2.0 * inset <= 0.0 {
        return [empty(), empty(), empty(), empty()];
    }
    let track = if frame.track_color.a > 0.0 {
        let points = oval_points(bounds, inset, 0.0, 2.0 * PI);
        (stroke_path(&points, frame.line_width, true), frame.track_color)
    } else {
        empty()
    };
    let sweep = 2.0 * PI * frame.stroke_end.clamp(0.0, 1.0);
    if sweep <= 0.0 {
        return [track, empty(), empty(), empty()];
    }
    let arc = oval_points(bounds, inset, frame.rotation, sweep);
    let color = frame.progress_color;
    let first = arc.first().copied().unwrap_or(bounds.origin);
    let last = arc.last().copied().unwrap_or(bounds.origin);
    [
        track,
        (stroke_path(&arc, frame.line_width, false), color),
        (round_cap(first, frame.line_width), color),
        (round_cap(last, frame.line_width), color),
    ]
}

fn ring_swap_keys(entity: EntityId) -> [u64; 4] {
    let base = entity.as_u64() & 0x00ff_ffff_ffff_ffff;
    [0x52u64, 0x53, 0x54, 0x55].map(|tag| base | (tag << 56))
}

/// desktop only, the moving ring swaps its paths on screen each frame
/// instead of asking the row to render
/// that dirtied the cell, the chat screen and the route
fn ring_tick(view: WeakEntity<RiseExactUploadProgressRingView>, window: &mut Window, cx: &mut App) {
    let Some(view) = view.upgrade() else {
        return;
    };
    let (snapshot, painted) = {
        let ring = view.read(cx);
        (ring.frame(RiseClock::monotonic()), ring.painted.get())
    };
    let swapped = !snapshot.is_hidden
        && painted.is_some_and(|painted_size| {
            let local = Bounds::new(Point::default(), painted_size);
            let paths = ring_paths(local, &snapshot);
            let mut swaps = Vec::with_capacity(4);
            for ((path, color), key) in paths.into_iter().zip(ring_swap_keys(view.entity_id())) {
                let Some(path) = path else {
                    return false;
                };
                swaps.push((key, path, color.into()));
            }
            window.swap_paths(swaps)
        });
    if swapped && snapshot.needs_next_frame {
        let weak = view.downgrade();
        window.on_next_frame(move |window, cx| ring_tick(weak, window, cx));
        return;
    }
    view.update(cx, |ring, cx| {
        ring.is_ticking = false;
        if !swapped {
            ring.painted.set(None);
            cx.notify();
        }
    });
}

pub struct RiseExactUploadProgressRingView {
    upload_key: Option<RiseUploadKey>,
    latest_progress: Option<RiseUploadProgress>,
    upload_progress_task: Option<Task<()>>,
    subscription_generation: u64,
    has_received_live_progress: bool,
    live_progress_was_removed: bool,

    seed_fraction: f64,
    seed_completed_bytes: i64,
    seed_total_bytes: i64,
    seed_phase: RiseEngineUploadPhase,
    ring_inset: f32,
    line_width: f32,
    rendered_fraction: f64,
    active_progress_color: Hsla,
    progress_stroke_color: Hsla,
    track_color: Hsla,
    failure_color: Hsla,
    is_showing_failure: bool,
    highest_rendered_fraction: f64,

    is_hidden: bool,
    stroke_end: Animated<f32>,
    rotation: Animated<f32>,
    reduce_motion: bool,
    painted: Rc<Cell<Option<Size<Pixels>>>>,
    is_ticking: bool,
}

impl RiseExactUploadProgressRingView {
    fn new() -> Self {
        Self {
            upload_key: None,
            latest_progress: None,
            upload_progress_task: None,
            subscription_generation: 0,
            has_received_live_progress: false,
            live_progress_was_removed: false,
            seed_fraction: 0.0,
            seed_completed_bytes: 0,
            seed_total_bytes: 0,
            seed_phase: RiseEngineUploadPhase::Preparing,
            ring_inset: 0.0,
            line_width: 1.0,
            rendered_fraction: -1.0,
            active_progress_color: white(),
            progress_stroke_color: white(),
            track_color: transparent_black(),
            failure_color: transparent_black(),
            is_showing_failure: false,
            highest_rendered_fraction: 0.0,
            is_hidden: true,
            stroke_end: Animated::new(0.0),
            rotation: Animated::new(0.0),
            reduce_motion: false,
            painted: Rc::new(Cell::new(None)),
            is_ticking: false,
        }
    }

    fn frame(&self, now: Duration) -> RingFrame {
        let is_moving = self.stroke_end.is_running(now) || self.rotation.is_running(now);
        RingFrame {
            is_hidden: self.is_hidden,
            needs_next_frame: !self.is_hidden && is_moving,
            line_width: self.line_width,
            ring_inset: self.ring_inset,
            track_color: self.track_color,
            progress_color: self.progress_stroke_color,
            stroke_end: self.stroke_end.value(now),
            rotation: self.rotation.value(now),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        owner_local_id: &str,
        slot_index: i64,
        is_active: bool,
        is_compressing: bool,
        seed_fraction: f64,
        completed_bytes: i64,
        total_bytes: i64,
        is_layout_measurement: bool,
        progress_color: Hsla,
        track_color: Hsla,
        line_width: f32,
        ring_inset: f32,
        cx: &mut Context<Self>,
    ) {
        let next_key = upload_key(owner_local_id, slot_index, is_active, is_layout_measurement);
        if self.upload_key != next_key {
            self.cancel_subscription();
            self.upload_key = next_key.clone();
            self.latest_progress = None;
            self.has_received_live_progress = false;
            self.live_progress_was_removed = false;
            self.rendered_fraction = -1.0;
            self.highest_rendered_fraction = 0.0;
            self.is_showing_failure = false;
        }

        self.seed_fraction = seed_fraction.max(0.0).min(1.0);
        self.seed_completed_bytes = completed_bytes.max(0);
        self.seed_total_bytes = total_bytes.max(0);
        self.seed_phase = if is_compressing || (seed_fraction <= 0.0 && completed_bytes <= 0) {
            RiseEngineUploadPhase::Preparing
        } else {
            RiseEngineUploadPhase::Uploading
        };

        self.ring_inset = ring_inset.max(0.0);
        self.line_width = line_width.max(1.0);
        self.track_color = track_color;
        self.active_progress_color = progress_color;
        if !self.is_showing_failure {
            self.progress_stroke_color = progress_color;
        }

        if next_key.is_none() {
            self.hide();
            return;
        }
        if self.live_progress_was_removed {
            self.hide();
        } else if let Some(latest_progress) = self.latest_progress.clone() {
            self.apply(&latest_progress);
        } else {
            self.apply_seed();
        }
        self.begin_subscription_if_needed(cx);
    }

    pub fn prepare_for_reuse(&mut self) {
        self.cancel_subscription();
        self.upload_key = None;
        self.latest_progress = None;
        self.has_received_live_progress = false;
        self.live_progress_was_removed = false;
        self.rendered_fraction = -1.0;
        self.highest_rendered_fraction = 0.0;
        self.is_showing_failure = false;
        self.hide();
    }

    fn apply_seed(&mut self) {
        let Some(upload_key) = self.upload_key.clone() else {
            return;
        };
        let seed = RiseUploadProgress::new(
            upload_key,
            self.seed_fraction,
            self.seed_completed_bytes,
            self.seed_total_bytes,
            self.seed_phase.clone(),
        );
        self.apply(&seed);
    }

    fn begin_subscription_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.upload_progress_task.is_some() {
            return;
        }
        let Some(upload_key) = self.upload_key.clone() else {
            return;
        };
        let Some(updates) = rise_exact_upload_progress_ring_hooks()
            .and_then(|hooks| (hooks.upload_progress_updates)(cx, upload_key.clone()))
        else {
            return;
        };
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        let generation = self.subscription_generation;
        self.upload_progress_task = Some(cx.spawn(async move |this, cx| {
            let mut updates = updates.await;
            while let Some(progress) = updates.next().await {
                let delivered = this.update(cx, |view, cx| {
                    if view.upload_key.as_ref() != Some(&upload_key) || view.subscription_generation != generation {
                        return false;
                    }
                    if let Some(progress) = progress {
                        view.has_received_live_progress = true;
                        view.live_progress_was_removed = false;
                        view.latest_progress = Some(progress.clone());
                        view.apply(&progress);
                    } else if view.has_received_live_progress {
                        view.latest_progress = None;
                        view.live_progress_was_removed = true;
                        view.hide();
                    }
                    if !view.is_ticking {
                        cx.notify();
                    }
                    true
                });
                if !matches!(delivered, Ok(true)) {
                    return;
                }
            }
            let _ = this.update(cx, |view, _| {
                if view.upload_key.as_ref() == Some(&upload_key) && view.subscription_generation == generation {
                    view.upload_progress_task = None;
                }
            });
        }));
    }

    fn cancel_subscription(&mut self) {
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        self.upload_progress_task = None;
    }

    fn apply(&mut self, progress: &RiseUploadProgress) {
        if Some(progress.key()) != self.upload_key.as_ref() {
            return;
        }
        match progress.phase() {
            RiseEngineUploadPhase::Completed => {
                self.hide();
                return;
            }
            RiseEngineUploadPhase::Failed { .. } => {
                self.is_hidden = false;
                self.is_showing_failure = true;
                self.progress_stroke_color = self.failure_color;
                self.stop_spinner();
            }
            RiseEngineUploadPhase::Preparing | RiseEngineUploadPhase::Uploading | RiseEngineUploadPhase::Processing => {
                self.is_hidden = false;
                self.is_showing_failure = false;
                self.progress_stroke_color = self.active_progress_color;
                self.ensure_spinner();
            }
        }

        let fraction = self
            .highest_rendered_fraction
            .max(RiseUploadRingPresentation::visible_fraction(progress));
        self.highest_rendered_fraction = fraction;
        if (fraction - self.rendered_fraction).abs() < 0.002 {
            return;
        }
        let previous_fraction = self.rendered_fraction;
        self.rendered_fraction = fraction;
        self.render(fraction, previous_fraction);
    }

    fn render(&mut self, fraction: f64, previous_fraction: f64) {
        let now = RiseClock::monotonic();
        let target = fraction.max(0.0).min(1.0) as f32;
        let current = self.stroke_end.value(now);

        if previous_fraction < 0.0 || self.reduce_motion || (target - current).abs() < 0.002 {
            self.stroke_end.jump(target);
            return;
        }
        let duration = if target < current { 0.20 } else { 0.14 };
        self.stroke_end.set(target, linear(duration), now, self.reduce_motion);
    }

    fn hide(&mut self) {
        self.is_hidden = true;
        self.stop_spinner();
    }

    fn ensure_spinner(&mut self) {
        let now = RiseClock::monotonic();
        if self.is_hidden || self.reduce_motion || self.rotation.is_running(now) {
            return;
        }
        self.rotation.jump(0.0);
        self.rotation
            .set(std::f32::consts::TAU, linear(0.82).repeat_forever(false), now, self.reduce_motion);
    }

    fn stop_spinner(&mut self) {
        self.rotation.jump(0.0);
    }
}

/// sent of total line for an uploading file
/// the row never gets byte ticks, only the upload hub does
/// so this leaf reads the hub value like the ring
#[derive(IntoElement)]
pub struct RiseExactUploadStatusLabel {
    id: ElementId,
    owner_local_id: String,
    item: MessageMediaItem,
    seed_text: String,
    is_layout_measurement: bool,
    font_size: f32,
    text_color: Option<Hsla>,
}

impl RiseExactUploadStatusLabel {
    pub fn new(
        id: impl Into<ElementId>,
        owner_local_id: impl Into<String>,
        item: MessageMediaItem,
        seed_text: impl Into<String>,
        is_layout_measurement: bool,
    ) -> Self {
        Self {
            id: id.into(),
            owner_local_id: owner_local_id.into(),
            item,
            seed_text: seed_text.into(),
            is_layout_measurement,
            font_size: 13.0,
            text_color: None,
        }
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    pub fn text_color(mut self, text_color: Hsla) -> Self {
        self.text_color = Some(text_color);
        self
    }
}

impl RenderOnce for RiseExactUploadStatusLabel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view: Entity<RiseExactUploadStatusLabelView> = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("upload-label")),
            cx,
            |_, _| RiseExactUploadStatusLabelView::new(),
        );
        let text_color = match self.text_color {
            Some(color) => color,
            None => {
                let appearance = match ThemeStore::shared(cx).read(cx).resolved_appearance() {
                    RiseThemeAppearance::Light => Appearance::Light,
                    RiseThemeAppearance::Dark => Appearance::Dark,
                };
                UIColor::secondary_label().resolved_color(appearance)
            }
        };
        view.update(cx, |view, cx| {
            view.configure(
                &self.owner_local_id,
                self.item.local_slot_index.max(0),
                self.item.is_uploading || self.item.is_compressing,
                &self.seed_text,
                self.is_layout_measurement,
                cx,
            );
        });
        let text = view.read(cx).text.clone().unwrap_or_default();
        div().w_full().child(
            MainText::new(text)
                .px(self.font_size)
                .font_weight(FontWeight::NORMAL)
                .color(text_color)
                .monospaced_digits(true)
                .line_limit(1)
                .truncation(TruncationMode::Middle),
        )
    }
}

pub struct RiseExactUploadStatusLabelView {
    text: Option<String>,
    upload_key: Option<RiseUploadKey>,
    upload_progress_task: Option<Task<()>>,
    subscription_generation: u64,
    has_received_live_progress: bool,
    last_label_update_time: Option<Duration>,
    last_phase: Option<RiseEngineUploadPhase>,
    last_was_awaiting_server: bool,
}

impl RiseExactUploadStatusLabelView {
    const LABEL_INTERVAL: Duration = Duration::from_millis(80);

    fn new() -> Self {
        Self {
            text: None,
            upload_key: None,
            upload_progress_task: None,
            subscription_generation: 0,
            has_received_live_progress: false,
            last_label_update_time: None,
            last_phase: None,
            last_was_awaiting_server: false,
        }
    }

    pub fn configure(
        &mut self,
        owner_local_id: &str,
        slot_index: i64,
        is_active: bool,
        seed_text: &str,
        is_layout_measurement: bool,
        cx: &mut Context<Self>,
    ) {
        let next_key = upload_key(owner_local_id, slot_index, is_active, is_layout_measurement);
        if self.upload_key != next_key {
            self.cancel_subscription();
            self.upload_key = next_key;
            self.has_received_live_progress = false;
            self.last_phase = None;
            self.last_was_awaiting_server = false;
            self.last_label_update_time = None;
        }
        if !self.has_received_live_progress && self.text.as_deref() != Some(seed_text) {
            self.text = Some(seed_text.to_owned());
        }
        self.begin_subscription_if_needed(cx);
    }

    pub fn prepare_for_reuse(&mut self) {
        self.cancel_subscription();
        self.upload_key = None;
        self.has_received_live_progress = false;
        self.last_phase = None;
        self.last_was_awaiting_server = false;
        self.last_label_update_time = None;
        self.text = None;
    }

    fn begin_subscription_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.upload_progress_task.is_some() {
            return;
        }
        let Some(upload_key) = self.upload_key.clone() else {
            return;
        };
        let Some(updates) = rise_exact_upload_progress_ring_hooks()
            .and_then(|hooks| (hooks.upload_progress_updates)(cx, upload_key.clone()))
        else {
            return;
        };
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        let generation = self.subscription_generation;
        self.upload_progress_task = Some(cx.spawn(async move |this, cx| {
            let mut updates = updates.await;
            while let Some(progress) = updates.next().await {
                let delivered = this.update(cx, |view, cx| {
                    if view.upload_key.as_ref() != Some(&upload_key) || view.subscription_generation != generation {
                        return false;
                    }
                    if let Some(progress) = progress {
                        view.has_received_live_progress = true;
                        if view.apply(&progress) {
                            cx.notify();
                        }
                    }
                    true
                });
                if !matches!(delivered, Ok(true)) {
                    return;
                }
            }
            let _ = this.update(cx, |view, _| {
                if view.upload_key.as_ref() == Some(&upload_key) && view.subscription_generation == generation {
                    view.upload_progress_task = None;
                }
            });
        }));
    }

    fn cancel_subscription(&mut self) {
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        self.upload_progress_task = None;
    }

    /// returns whether the label text changed
    fn apply(&mut self, progress: &RiseUploadProgress) -> bool {
        if Some(progress.key()) != self.upload_key.as_ref() {
            return false;
        }
        match progress.phase() {
            RiseEngineUploadPhase::Completed | RiseEngineUploadPhase::Failed { .. } => return false,
            RiseEngineUploadPhase::Preparing | RiseEngineUploadPhase::Uploading | RiseEngineUploadPhase::Processing => {}
        }
        let is_awaiting_server = RiseUploadRingPresentation::is_awaiting_server(progress);
        let presentation_changed = self.last_phase.as_ref() != Some(progress.phase())
            || self.last_was_awaiting_server != is_awaiting_server;
        let now = RiseClock::monotonic();
        let interval_passed = match self.last_label_update_time {
            Some(last) => now.saturating_sub(last) >= Self::LABEL_INTERVAL,
            None => true,
        };
        if !presentation_changed && !interval_passed {
            return false;
        }
        self.last_phase = Some(progress.phase().clone());
        self.last_was_awaiting_server = is_awaiting_server;
        self.last_label_update_time = Some(now);
        let text = RiseUploadRingPresentation::status_text(progress);
        if self.text.as_deref() != Some(text.as_str()) {
            self.text = Some(text);
            return true;
        }
        false
    }
}
