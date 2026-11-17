use std::f32::consts::PI;
use std::time::Duration;

use futures::StreamExt;
use gpui::{
    App, AsyncApp, Bounds, Context, ElementId, Entity, FontWeight, Hsla, Pixels, Point, SharedString, Task, TextRun,
    Window, canvas, fill, hsla, point, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_ui::IconUi;
use rise_ui::loader_ui::ring_spin_degrees;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::{Animated, linear};

use crate::core::lib::byte_format::ByteFormat;
use crate::core::media::rise_media_transfer_hub::{
    RiseMediaTransferHub, RiseMediaTransferKey, RiseMediaTransferPhase, RiseMediaTransferRequest,
    RiseMediaTransferState,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseMediaTransferChipViewMetrics {
    pub height: f32,
    pub ring_side: f32,
    pub line_width: f32,
    pub glyph_side: f32,
    pub leading_inset: f32,
    pub spacing: f32,
    pub trailing_inset: f32,
    pub font_size: f32,
    pub shows_background: bool,
    pub shows_ring: bool,
    pub ring_color: Hsla,
    pub track_color: Hsla,
    pub label_color: Hsla,
    pub glyph_color: Hsla,
    pub uses_reserved_width: bool,
}

impl Default for RiseMediaTransferChipViewMetrics {
    fn default() -> Self {
        Self {
            height: 22.,
            ring_side: 16.,
            line_width: 1.6,
            glyph_side: 9.,
            leading_inset: 3.,
            spacing: 5.,
            trailing_inset: 8.,
            font_size: 11.,
            shows_background: true,
            shows_ring: true,
            ring_color: white(),
            track_color: white().opacity(0.28),
            label_color: white().opacity(0.96),
            glyph_color: white(),
            uses_reserved_width: false,
        }
    }
}

impl RiseMediaTransferChipViewMetrics {
    pub fn standard() -> Self {
        Self::default()
    }

    /// ring only with no capsule, for laying over a round control like a document or audio icon
    pub fn ring(side: f32, line_width: f32, tint: Option<Hsla>, track: Option<Hsla>) -> Self {
        let tint = tint.unwrap_or_else(white);
        let mut metrics = Self::standard();
        metrics.shows_background = false;
        metrics.height = side;
        metrics.ring_side = side;
        metrics.line_width = line_width;
        metrics.glyph_side = (side * 0.34).max(9.);
        metrics.leading_inset = 0.;
        metrics.spacing = 0.;
        metrics.trailing_inset = 0.;
        metrics.ring_color = tint;
        metrics.track_color = track.unwrap_or_else(|| white().opacity(0.28));
        metrics.glyph_color = tint;
        metrics
    }

    /// text only, for a subtitle line in a bubble that just needs the transferred pair
    pub fn inline_label(font_size: f32, color: Hsla) -> Self {
        let mut metrics = Self::standard();
        metrics.shows_background = false;
        metrics.shows_ring = false;
        metrics.height = (font_size * 1.35).ceil();
        metrics.ring_side = 0.;
        metrics.leading_inset = 0.;
        metrics.spacing = 0.;
        metrics.trailing_inset = 0.;
        metrics.font_size = font_size;
        metrics.label_color = color;
        metrics.uses_reserved_width = true;
        metrics
    }

    /// under about 110 pt the byte counts turn into noise so the chip falls back to ring only
    pub fn fitting(side: f32) -> Self {
        let mut metrics = Self::standard();
        let scale = (side / 130.).max(0.62).min(1.);
        metrics.height = (22. * scale).max(16.);
        metrics.ring_side = (16. * scale).max(12.);
        metrics.line_width = (1.6 * scale).max(1.2);
        metrics.glyph_side = (9. * scale).max(7.);
        metrics.font_size = (11. * scale).max(9.);
        metrics.spacing = (5. * scale).max(3.);
        metrics.trailing_inset = (8. * scale).max(5.);
        metrics.leading_inset = (3. * scale).max(2.);
        metrics
    }

    fn label_font(&self) -> Font {
        Font::system(self.font_size, FontWeight::SEMIBOLD, FontDesign::Default).monospaced_digit()
    }
}

/// download button for one remote object, a leaf on purpose
/// progress goes straight into this entity so a byte tick never reaches the message row
pub struct RiseMediaTransferChipView {
    key: Option<RiseMediaTransferKey>,
    declared_byte_count: i64,
    shows_byte_counts: bool,
    shows_idle_affordance: bool,
    shows_size_when_cached: bool,
    is_interactive: bool,
    hides_small_transfers: bool,
    metrics: RiseMediaTransferChipViewMetrics,
    updates_task: Option<Task<()>>,
    subscription_generation: u64,
    latest: Option<RiseMediaTransferState>,
    rendered_fraction: f64,
    rendered_phase: Option<RiseMediaTransferPhase>,
    last_label_update: Duration,
    is_paused: bool,
    is_in_window: bool,
    is_actionable: bool,
    is_hidden: bool,
    is_ring_progress_hidden: bool,
    is_glyph_hidden: bool,
    glyph: &'static str,
    label_text: Option<String>,
    accessibility_label: Option<String>,
    progress: Animated<f32>,
    rotation_started_at: Option<Duration>,
}

impl RiseMediaTransferChipView {
    const LABEL_THROTTLE: Duration = Duration::from_millis(80);
    const FRACTION_EPSILON: f64 = 0.002;
    const PROGRESS_DURATION: f64 = 0.14;
    const ROTATION_PERIOD: Duration = Duration::from_millis(820);
    const MINIMUM_READOUT_BYTES: i64 = 256 * 1_024;

    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            key: None,
            declared_byte_count: 0,
            shows_byte_counts: true,
            shows_idle_affordance: true,
            shows_size_when_cached: false,
            is_interactive: true,
            hides_small_transfers: true,
            metrics: RiseMediaTransferChipViewMetrics::standard(),
            updates_task: None,
            subscription_generation: 0,
            latest: None,
            rendered_fraction: -1.,
            rendered_phase: None,
            last_label_update: Duration::ZERO,
            is_paused: false,
            is_in_window: true,
            is_actionable: false,
            is_hidden: true,
            is_ring_progress_hidden: true,
            is_glyph_hidden: true,
            glyph: "arrow.down",
            label_text: None,
            accessibility_label: None,
            progress: Animated::new(0.),
            rotation_started_at: None,
        }
    }

    pub fn is_actionable(&self) -> bool {
        self.is_actionable
    }

    /// true if the chip draws something, a hidden chip lays out as nothing
    pub fn is_drawn(&self) -> bool {
        !self.is_hidden
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        key: Option<RiseMediaTransferKey>,
        declared_byte_count: i64,
        metrics: RiseMediaTransferChipViewMetrics,
        shows_byte_counts: bool,
        shows_idle_affordance: bool,
        shows_size_when_cached: bool,
        is_interactive: bool,
        hides_small_transfers: bool,
        cx: &mut Context<Self>,
    ) {
        if self.key == key
            && (key.is_some() || self.is_hidden)
            && self.declared_byte_count == declared_byte_count.max(0)
            && self.metrics == metrics
            && self.shows_byte_counts == shows_byte_counts
            && self.shows_idle_affordance == shows_idle_affordance
            && self.shows_size_when_cached == shows_size_when_cached
            && self.is_interactive == is_interactive
            && self.hides_small_transfers == hides_small_transfers
        {
            self.begin_subscription_if_needed(cx);
            return;
        }
        tracing::warn!(target: "riseonly::X590", "chip configure changed key_some={}", key.is_some());
        let drawn = self.drawn_state();
        let metrics_changed = self.metrics != metrics;
        if self.key != key {
            self.cancel_subscription();
            self.key = key.clone();
            self.latest = None;
            self.rendered_fraction = -1.;
            self.rendered_phase = None;
            self.last_label_update = Duration::ZERO;
        }
        self.declared_byte_count = declared_byte_count.max(0);
        self.shows_byte_counts = shows_byte_counts;
        self.shows_idle_affordance = shows_idle_affordance;
        self.shows_size_when_cached = shows_size_when_cached;
        self.is_interactive = is_interactive;
        self.hides_small_transfers = hides_small_transfers;
        self.metrics = metrics;

        if key.is_none() {
            self.retire();
            if metrics_changed || self.drawn_state() != drawn {
                cx.notify();
            }
            return;
        }
        if let Some(latest) = self.latest.clone() {
            self.apply(&latest, true, cx);
        } else {
            self.retire();
        }
        self.begin_subscription_if_needed(cx);
        if metrics_changed || self.drawn_state() != drawn {
            cx.notify();
        }
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        let drawn = self.drawn_state();
        self.cancel_subscription();
        self.key = None;
        self.latest = None;
        self.declared_byte_count = 0;
        self.rendered_fraction = -1.;
        self.rendered_phase = None;
        self.last_label_update = Duration::ZERO;
        self.retire();
        if self.drawn_state() != drawn {
            cx.notify();
        }
    }

    pub fn handle_tap(&self, request: RiseMediaTransferRequest, cx: &mut App) {
        if !self.is_actionable {
            return;
        }
        RiseMediaTransferHub::shared().toggle(request, cx);
    }

    /// paused chips stop their stream and spin, only a visible ring should cost a frame
    pub fn set_paused(&mut self, paused: bool, cx: &mut Context<Self>) {
        if self.is_paused == paused {
            return;
        }
        let drawn = self.drawn_state();
        self.is_paused = paused;
        if paused {
            self.cancel_subscription();
            self.stop_rotation();
        } else {
            self.begin_subscription_if_needed(cx);
            self.ensure_rotation(cx);
        }
        if self.drawn_state() != drawn {
            cx.notify();
        }
    }

    pub fn did_move_to_window(&mut self, is_in_window: bool, cx: &mut Context<Self>) {
        if self.is_in_window == is_in_window {
            if is_in_window {
                let drawn = self.drawn_state();
                self.begin_subscription_if_needed(cx);
                self.ensure_rotation(cx);
                if self.drawn_state() != drawn {
                    cx.notify();
                }
            }
            return;
        }
        let drawn = self.drawn_state();
        self.is_in_window = is_in_window;
        if is_in_window {
            self.begin_subscription_if_needed(cx);
            self.ensure_rotation(cx);
        } else {
            self.cancel_subscription();
            self.stop_rotation();
        }
        if self.drawn_state() != drawn {
            cx.notify();
        }
    }

    pub fn preferred_width(&self, limited_to: f32, window: &Window) -> f32 {
        let metrics = &self.metrics;
        let ring_block = if metrics.shows_ring { metrics.leading_inset + metrics.ring_side } else { 0. };
        let measured = self.measured_text();
        let Some(measured) = measured.filter(|text| self.shows_byte_counts && !text.is_empty()) else {
            let trailing = if metrics.shows_ring { metrics.leading_inset } else { 0. };
            return (ring_block + trailing).max(0.).min(limited_to);
        };
        let text_width = self.text_width(&measured, window).ceil();
        let spacing = if metrics.shows_ring { metrics.spacing } else { 0. };
        let full = ring_block + spacing + text_width + metrics.trailing_inset;
        full.max(ring_block).min(limited_to)
    }

    fn text_width(&self, text: &str, window: &Window) -> f32 {
        let font = self.metrics.label_font();
        let run = TextRun {
            len: text.len(),
            font: gpui::Font {
                family: font.family(),
                features: font.features(),
                fallbacks: None,
                weight: font.weight,
                style: font.style(),
            },
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(
            window
                .text_system()
                .shape_line(SharedString::from(text.to_string()), px(font.size), &[run], None)
                .width(),
        )
    }

    fn reserved_text(&self) -> Option<String> {
        if !self.metrics.uses_reserved_width || self.declared_byte_count <= 0 {
            return None;
        }
        let formatted = ByteFormat::telegram(self.declared_byte_count);
        Some(tr_args(
            "chat_media_download_progress",
            &[FormatArg::Text(formatted.as_str().into()), FormatArg::Text(formatted.as_str().into())],
        ))
    }

    fn measured_text(&self) -> Option<String> {
        self.reserved_text().or_else(|| self.label_text.clone())
    }

    fn begin_subscription_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.is_in_window || self.is_paused || self.updates_task.is_some() {
            return;
        }
        let Some(key) = self.key.clone() else {
            return;
        };
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        let generation = self.subscription_generation;
        self.updates_task = Some(cx.spawn(async move |this, cx| {
            let mut updates = RiseMediaTransferHub::shared().updates(&key);
            while let Some(state) = updates.next().await {
                let is_current = this
                    .update(cx, |this, cx| {
                        if this.key.as_ref() != Some(&key) || this.subscription_generation != generation {
                            return false;
                        }
                        let drawn = this.drawn_state();
                        this.latest = Some(state.clone());
                        this.apply(&state, false, cx);
                        if this.drawn_state() != drawn {
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !is_current {
                    return;
                }
            }
            let _ = this.update(cx, |this, _| {
                if this.key.as_ref() == Some(&key) && this.subscription_generation == generation {
                    if let Some(task) = this.updates_task.take() {
                        task.detach();
                    }
                }
            });
        }));
    }

    fn cancel_subscription(&mut self) {
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        self.updates_task = None;
    }

    fn apply(&mut self, state: &RiseMediaTransferState, force_label: bool, cx: &mut Context<Self>) {
        if self.key.as_ref() != Some(&state.key) {
            return;
        }
        let phase_changed = self.rendered_phase.as_ref() != Some(&state.phase);
        self.rendered_phase = Some(state.phase.clone());
        if phase_changed {
            tracing::warn!(
                target: "riseonly::X1922",
                "chip phase key={} phase={:?} idle_affordance={} total={}",
                state.key.cache_key,
                state.phase,
                self.shows_idle_affordance,
                state.total_bytes
            );
        }

        match state.phase {
            RiseMediaTransferPhase::Cached => {
                self.is_actionable = false;
                if !self.shows_size_when_cached {
                    self.retire();
                    return;
                }
                self.is_hidden = false;
                self.is_ring_progress_hidden = true;
                self.is_glyph_hidden = true;
                self.stop_rotation();
            }
            RiseMediaTransferPhase::Idle => {
                if !self.shows_idle_affordance {
                    self.is_actionable = false;
                    self.retire();
                    return;
                }
                self.is_actionable = self.is_interactive;
                self.is_hidden = false;
                self.is_ring_progress_hidden = true;
                self.is_glyph_hidden = !self.metrics.shows_ring;
                self.glyph = "arrow.down";
                self.stop_rotation();
            }
            RiseMediaTransferPhase::Failed { .. } => {
                if !self.shows_idle_affordance {
                    self.is_actionable = false;
                    self.retire();
                    return;
                }
                self.is_actionable = self.is_interactive;
                self.is_hidden = false;
                self.is_ring_progress_hidden = true;
                self.is_glyph_hidden = !self.metrics.shows_ring;
                self.glyph = "arrow.clockwise";
                self.stop_rotation();
            }
            RiseMediaTransferPhase::Queued | RiseMediaTransferPhase::Downloading => {
                if !(self.shows_idle_affordance
                    || !self.hides_small_transfers
                    || state.total_bytes == 0
                    || state.total_bytes >= Self::MINIMUM_READOUT_BYTES)
                {
                    self.is_actionable = false;
                    self.retire();
                    return;
                }
                self.is_actionable = self.is_interactive;
                self.is_hidden = false;
                self.is_ring_progress_hidden = false;
                self.is_glyph_hidden = !self.is_interactive || !self.metrics.shows_ring;
                if self.is_interactive {
                    self.glyph = "xmark";
                }
                self.ensure_rotation(cx);
            }
        }

        if state.is_active() {
            let fraction = if state.total_bytes > 0 { state.fraction().clamp(0.05, 1.) } else { 0.16 };
            if phase_changed || (fraction - self.rendered_fraction).abs() >= Self::FRACTION_EPSILON {
                self.render_progress(fraction, !phase_changed, cx);
                self.rendered_fraction = fraction;
            }
        } else {
            self.rendered_fraction = -1.;
        }

        let now = RiseClock::monotonic();
        if force_label
            || phase_changed
            || now.saturating_sub(self.last_label_update) >= Self::LABEL_THROTTLE
        {
            self.last_label_update = now;
            self.update_label(state);
        }
    }

    fn update_label(&mut self, state: &RiseMediaTransferState) {
        let total = self.declared_byte_count.max(state.total_bytes);
        self.label_text = match state.phase {
            RiseMediaTransferPhase::Queued | RiseMediaTransferPhase::Downloading => {
                if total > 0 {
                    let transferred = ByteFormat::telegram_transferred(state.downloaded_bytes, total);
                    let total = ByteFormat::telegram(total);
                    Some(tr_args(
                        "chat_media_download_progress",
                        &[FormatArg::Text(transferred.as_str().into()), FormatArg::Text(total.as_str().into())],
                    ))
                } else if state.downloaded_bytes > 0 {
                    Some(ByteFormat::telegram(state.downloaded_bytes))
                } else {
                    None
                }
            }
            RiseMediaTransferPhase::Idle | RiseMediaTransferPhase::Failed { .. } => {
                (total > 0).then(|| ByteFormat::telegram(total))
            }
            RiseMediaTransferPhase::Cached => {
                (self.shows_size_when_cached && total > 0).then(|| ByteFormat::telegram(total))
            }
        };
        self.accessibility_label = self.accessibility_text(state);
    }

    fn accessibility_text(&self, state: &RiseMediaTransferState) -> Option<String> {
        match state.phase {
            RiseMediaTransferPhase::Failed { .. } => Some(tr("chat_media_download_failed")),
            RiseMediaTransferPhase::Queued | RiseMediaTransferPhase::Downloading => self.label_text.clone(),
            RiseMediaTransferPhase::Idle => self.label_text.clone(),
            RiseMediaTransferPhase::Cached => None,
        }
    }

    fn render_progress(&mut self, fraction: f64, animated: bool, cx: &App) {
        let target = fraction.clamp(0., 1.) as f32;
        let now = RiseClock::monotonic();
        let current = self.progress.value(now);
        let animates = animated
            && self.rendered_fraction >= 0.
            && self.is_in_window
            && !reduce_motion(cx)
            && f64::from((target - current).abs()) >= Self::FRACTION_EPSILON;
        if animates {
            self.progress.set(target, linear(Self::PROGRESS_DURATION), now, false);
        } else {
            self.progress.jump(target);
        }
    }

    fn ensure_rotation(&mut self, cx: &App) {
        if self.is_hidden
            || self.is_paused
            || !self.is_in_window
            || self.is_ring_progress_hidden
            || reduce_motion(cx)
            || self.rotation_started_at.is_some()
        {
            return;
        }
        self.rotation_started_at = Some(RiseClock::monotonic());
    }

    /// what the chip draws and answers to, none while hidden
    #[allow(clippy::type_complexity)]
    fn drawn_state(&self) -> Option<(bool, bool, &'static str, Option<String>, u32, bool, bool)> {
        (!self.is_hidden).then(|| {
            (
                self.is_ring_progress_hidden,
                self.is_glyph_hidden,
                self.glyph,
                self.label_text.clone(),
                self.progress.target().to_bits(),
                self.rotation_started_at.is_some(),
                self.is_actionable,
            )
        })
    }

    fn stop_rotation(&mut self) {
        self.rotation_started_at = None;
    }

    fn retire(&mut self) {
        self.is_actionable = false;
        self.is_hidden = true;
        self.label_text = None;
        self.accessibility_label = None;
        self.stop_rotation();
        self.progress.jump(0.);
    }
}

/// points on a circle around center from start through sweep radians, clockwise with y down
fn arc_points(center: Point<Pixels>, radius: f32, start: f32, sweep: f32) -> Vec<Point<Pixels>> {
    let steps = ((sweep.abs() / (2. * PI)) * 96.).ceil().max(2.) as usize;
    (0..=steps)
        .map(|step| {
            let angle = start + sweep * step as f32 / steps as f32;
            point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
        })
        .collect()
}

fn stroke_arc(points: &[Point<Pixels>], line_width: f32, closed: bool, color: Hsla, window: &mut Window) {
    let mut builder = gpui::PathBuilder::stroke(px(line_width));
    builder.add_polygon(points, closed);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// round line cap, gpui strokes are butt ended so each end gets a dot of line width
fn round_cap(at: Point<Pixels>, line_width: f32, color: Hsla, window: &mut Window) {
    let half = line_width / 2.;
    let bounds = Bounds::new(point(at.x - px(half), at.y - px(half)), size(px(line_width), px(line_width)));
    window.paint_quad(fill(bounds, color).corner_radii(px(half)));
}

impl Render for RiseMediaTransferChipView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = self.metrics;
        let now = RiseClock::monotonic();
        let progress = self.progress.value(now);
        let spin = match self.rotation_started_at {
            Some(started_at) if !self.is_ring_progress_hidden => {
                ring_spin_degrees(now.saturating_sub(started_at), Self::ROTATION_PERIOD)
            }
            _ => 0.,
        };
        if !self.is_hidden && (self.progress.is_running(now) || self.rotation_started_at.is_some()) {
            window.request_animation_frame();
        }

        let ring_side = metrics.ring_side.min(metrics.height).max(0.);
        let glyph_side = metrics.glyph_side.min(ring_side - 2.).max(0.);
        let shows_ring_progress = !self.is_ring_progress_hidden;
        let ring = div()
            .relative()
            .flex_none()
            .size(px(ring_side))
            .flex()
            .items_center()
            .justify_center()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, _| {
                        if !shows_ring_progress {
                            return;
                        }
                        let center = bounds.center();
                        let radius = (ring_side - metrics.line_width) / 2.;
                        if radius <= 0. {
                            return;
                        }
                        let track = arc_points(center, radius, 0., 2. * PI);
                        stroke_arc(&track, metrics.line_width, true, metrics.track_color, window);
                        let start = spin.to_radians();
                        let arc = arc_points(center, radius, start, 2. * PI * progress);
                        stroke_arc(&arc, metrics.line_width, false, metrics.ring_color, window);
                        if let (Some(first), Some(last)) = (arc.first(), arc.last()) {
                            round_cap(*first, metrics.line_width, metrics.ring_color, window);
                            round_cap(*last, metrics.line_width, metrics.ring_color, window);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .when(!self.is_glyph_hidden, |ring| {
                ring.children(IconUi::sized(self.glyph, px(glyph_side), metrics.glyph_color, false))
            });

        let label_font = metrics.label_font();
        let label_text = self.label_text.clone().filter(|_| self.shows_byte_counts);
        let reserved_text = self.reserved_text().filter(|_| self.shows_byte_counts);
        let has_label = label_text.is_some() || reserved_text.is_some();
        let label = zstack(Alignment::Leading)
            .min_w_0()
            .swiftui_font(label_font)
            .text_color(metrics.label_color)
            .when_some(reserved_text, |stack, reserved| {
                stack.child(div().invisible().whitespace_nowrap().child(reserved))
            })
            .when_some(label_text, |stack, text| {
                stack.child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(text))
            });

        let mut chip = div()
            .id(ElementId::from(SharedString::from("rise-media-transfer-chip")))
            .flex()
            .flex_row()
            .items_center()
            .h(px(metrics.height))
            .max_w_full()
            .overflow_hidden()
            .rounded(px(metrics.height / 2.))
            .when(metrics.shows_background, |chip| chip.bg(hsla(0., 0., 0., 0.48)))
            .when(metrics.shows_ring, |chip| chip.pl(px(metrics.leading_inset)).child(ring))
            .when(has_label, |chip| {
                chip.when(metrics.shows_ring, |chip| chip.child(div().flex_none().w(px(metrics.spacing))))
                    .child(label)
                    .pr(px(metrics.trailing_inset))
            })
            .when(!has_label && metrics.shows_ring, |chip| chip.pr(px(metrics.leading_inset)))
            .when(self.is_hidden, |chip| chip.invisible());
        if let Some(accessibility_label) = self.accessibility_label.clone() {
            chip = chip.aria_label(accessibility_label);
        }
        div().flex().flex_row().child(chip)
    }
}

/// chip for bubbles the mosaic doesnt draw, like documents and audio rows
/// the host owns the tap through the bubble tap router
#[derive(IntoElement)]
pub struct RiseMediaTransferChip {
    id: ElementId,
    key: Option<RiseMediaTransferKey>,
    declared_byte_count: i64,
    metrics: RiseMediaTransferChipViewMetrics,
    shows_byte_counts: bool,
    shows_idle_affordance: bool,
    shows_size_when_cached: bool,
    is_interactive: bool,
    hides_small_transfers: bool,
    is_layout_measurement: bool,
    fills_sized_box: bool,
    hugs_content_cached: bool,
}

impl RiseMediaTransferChip {
    pub fn new(id: impl Into<ElementId>, key: Option<RiseMediaTransferKey>) -> Self {
        Self {
            id: id.into(),
            key,
            declared_byte_count: 0,
            metrics: RiseMediaTransferChipViewMetrics::standard(),
            shows_byte_counts: true,
            shows_idle_affordance: true,
            shows_size_when_cached: false,
            is_interactive: true,
            hides_small_transfers: true,
            is_layout_measurement: false,
            fills_sized_box: false,
            hugs_content_cached: false,
        }
    }

    /// mounts the chip as a cached view filling the host box
    /// only for hosts with a definite width and height
    pub fn fills_sized_box(mut self, fills_sized_box: bool) -> Self {
        self.fills_sized_box = fills_sized_box;
        self
    }

    /// mounts the chip as a cached view in a box of its preferred width, for hosts that let it hug a label
    /// chip state changes notify the host so the box follows the label
    pub fn hugs_content_cached(mut self, hugs_content_cached: bool) -> Self {
        self.hugs_content_cached = hugs_content_cached;
        self
    }

    pub fn declared_byte_count(mut self, declared_byte_count: i64) -> Self {
        self.declared_byte_count = declared_byte_count;
        self
    }

    pub fn metrics(mut self, metrics: RiseMediaTransferChipViewMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn shows_byte_counts(mut self, shows_byte_counts: bool) -> Self {
        self.shows_byte_counts = shows_byte_counts;
        self
    }

    pub fn shows_idle_affordance(mut self, shows_idle_affordance: bool) -> Self {
        self.shows_idle_affordance = shows_idle_affordance;
        self
    }

    pub fn shows_size_when_cached(mut self, shows_size_when_cached: bool) -> Self {
        self.shows_size_when_cached = shows_size_when_cached;
        self
    }

    pub fn is_interactive(mut self, is_interactive: bool) -> Self {
        self.is_interactive = is_interactive;
        self
    }

    pub fn hides_small_transfers(mut self, hides_small_transfers: bool) -> Self {
        self.hides_small_transfers = hides_small_transfers;
        self
    }

    pub fn is_layout_measurement(mut self, is_layout_measurement: bool) -> Self {
        self.is_layout_measurement = is_layout_measurement;
        self
    }
}

#[derive(Clone, PartialEq)]
struct RiseMediaTransferChipConfiguration {
    key: Option<RiseMediaTransferKey>,
    declared_byte_count: i64,
    metrics: RiseMediaTransferChipViewMetrics,
    shows_byte_counts: bool,
    shows_idle_affordance: bool,
    shows_size_when_cached: bool,
    is_interactive: bool,
    hides_small_transfers: bool,
}

impl RenderOnce for RiseMediaTransferChip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view: Entity<RiseMediaTransferChipView> =
            window.use_keyed_state(self.id.clone(), cx, |_, cx| cx.new(RiseMediaTransferChipView::new)).read(cx).clone();
        let configuration = RiseMediaTransferChipConfiguration {
            key: if self.is_layout_measurement { None } else { self.key },
            declared_byte_count: self.declared_byte_count,
            metrics: self.metrics,
            shows_byte_counts: self.shows_byte_counts,
            shows_idle_affordance: self.shows_idle_affordance,
            shows_size_when_cached: self.shows_size_when_cached,
            is_interactive: self.is_interactive,
            hides_small_transfers: self.hides_small_transfers,
        };
        let configure_view = view.clone();
        on_change((self.id, "configuration"), configuration, true, window, cx, move |_, configuration, _, cx| {
            let configuration = configuration.clone();
            configure_view.update(cx, |view, cx| {
                view.configure(
                    configuration.key,
                    configuration.declared_byte_count,
                    configuration.metrics,
                    configuration.shows_byte_counts,
                    configuration.shows_idle_affordance,
                    configuration.shows_size_when_cached,
                    configuration.is_interactive,
                    configuration.hides_small_transfers,
                    cx,
                );
            });
        });
        if self.fills_sized_box {
            return gpui::AnyView::from(view)
                .cached(gpui::StyleRefinement::default().size_full())
                .replay_when_moved()
                .into_any_element();
        }
        if self.hugs_content_cached {
            let width = view.read(cx).preferred_width(f32::MAX, window);
            return div()
                .flex_none()
                .w(px(width))
                .max_w_full()
                .h(px(self.metrics.height))
                .child(
                    gpui::AnyView::from(view)
                        .cached(gpui::StyleRefinement::default().size_full())
                        .replay_when_moved(),
                )
                .into_any_element();
        }
        view.into_any_element()
    }
}

/// phase only stream for hosts that must restructure around a transfer, eg swap icon for ring
/// byte ticks never reach it, numbers stay in the chip view
pub enum RiseMediaTransferPhaseObserver {}

impl RiseMediaTransferPhaseObserver {
    pub async fn observe(
        key: RiseMediaTransferKey,
        cx: &mut AsyncApp,
        mut on_change: impl FnMut(RiseMediaTransferPhase, &mut App),
    ) {
        let mut last: Option<RiseMediaTransferPhase> = None;
        let mut updates = RiseMediaTransferHub::shared().updates(&key);
        while let Some(state) = updates.next().await {
            if last.as_ref() == Some(&state.phase) {
                continue;
            }
            last = Some(state.phase.clone());
            let phase = state.phase;
            cx.update(|cx| on_change(phase, cx));
        }
    }
}
