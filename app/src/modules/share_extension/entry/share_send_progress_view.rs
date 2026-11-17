use std::f32::consts::PI;

use gpui::{
    App, Bounds, ElementId, Entity, Hsla, IntoElement, ParentElement, PathBuilder, Pixels, Point, RenderOnce, Styled, Window,
    canvas, div, fill, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::{ease_out, linear};

use crate::modules::share_extension::entry::share_extension_strings::ShareStrings;
use crate::modules::share_extension::entry::share_extension_view::ShareSurface;

const FRAME_SIDE: f32 = 54.0;
const RING_LINE_WIDTH: f32 = 3.5;
const CHECK_LINE_WIDTH: f32 = 4.5;
const CHECK_PADDING: f32 = 14.0;
const MINIMUM_VISIBLE_TRIM: f64 = 0.075;
const RESTING_ROTATION_DEGREES: f32 = -90.0;
const SPUN_ROTATION_DEGREES: f32 = 270.0;
const SPIN_DURATION: f64 = 0.92;
const PROGRESS_DURATION: f64 = 0.22;
const CHECK_DURATION: f64 = 0.25;

/// progress ring shown while sending
#[derive(IntoElement)]
pub struct ShareSendProgressView {
    id: ElementId,
    pub progress: f64,
    pub succeeded: bool,
}

impl ShareSendProgressView {
    pub fn new(id: impl Into<ElementId>, progress: f64, succeeded: bool) -> Self {
        Self {
            id: id.into(),
            progress,
            succeeded,
        }
    }

    /// label for screen readers, gpui has no accessibility tree yet so the host reads it here
    pub fn accessibility_label(&self) -> String {
        if self.succeeded {
            ShareStrings::sent()
        } else {
            ShareStrings::sending()
        }
    }

    /// value for screen readers
    pub fn accessibility_value(&self) -> String {
        if self.succeeded {
            ShareStrings::sent()
        } else {
            ShareStrings::progress((self.progress * 100.0) as i64)
        }
    }
}

/// state of the progress ring plus its animations
struct ShareSendProgressViewState {
    rotating: bool,
    rotation: Animated<f32>,
    trim_end: Animated<f32>,
    check_progress: Animated<f32>,
    last_succeeded: Option<bool>,
}

impl ShareSendProgressViewState {
    fn new() -> Self {
        Self {
            rotating: false,
            rotation: Animated::new(RESTING_ROTATION_DEGREES),
            trim_end: Animated::new(MINIMUM_VISIBLE_TRIM as f32),
            check_progress: Animated::new(0.0),
            last_succeeded: None,
        }
    }

    fn trim_target(progress: f64) -> f32 {
        progress.min(1.0).max(MINIMUM_VISIBLE_TRIM) as f32
    }

    /// syncs the ring to new progress and success, skips animation with reduce motion
    fn sync(&mut self, progress: f64, succeeded: bool, reduce_motion: bool) {
        let now = RiseClock::monotonic();
        let Some(previous_succeeded) = self.last_succeeded else {
            self.last_succeeded = Some(succeeded);
            self.trim_end.jump(Self::trim_target(progress));
            self.rotating = true;
            self.sync_rotation(reduce_motion, now);
            return;
        };
        self.trim_end
            .set(Self::trim_target(progress), ease_out(PROGRESS_DURATION), now, false);
        if succeeded != previous_succeeded {
            self.last_succeeded = Some(succeeded);
            if succeeded {
                self.check_progress.set(1.0, ease_out(CHECK_DURATION), now, reduce_motion);
            }
        }
        self.sync_rotation(reduce_motion, now);
    }

    fn sync_rotation(&mut self, reduce_motion: bool, now: std::time::Duration) {
        if reduce_motion || !self.rotating {
            if self.rotation.is_running(now) || *self.rotation.target() != RESTING_ROTATION_DEGREES {
                self.rotation.jump(RESTING_ROTATION_DEGREES);
            }
            return;
        }
        if self.rotation.is_running(now) {
            return;
        }
        self.rotation.jump(RESTING_ROTATION_DEGREES);
        self.rotation.set(
            SPUN_ROTATION_DEGREES,
            linear(SPIN_DURATION).repeat_forever(false),
            now,
            false,
        );
    }
}

impl RenderOnce for ShareSendProgressView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state: Entity<ShareSendProgressViewState> =
            window.use_keyed_state(self.id.clone(), cx, |_, _| ShareSendProgressViewState::new());
        let reduce_motion = reduce_motion(cx);
        let progress = self.progress;
        let succeeded = self.succeeded;
        state.update(cx, |state, _| state.sync(progress, succeeded, reduce_motion));

        let now = RiseClock::monotonic();
        let snapshot = {
            let state = state.read(cx);
            let ring_moves = !succeeded && (state.rotation.is_running(now) || state.trim_end.is_running(now));
            let check_moves = succeeded && state.check_progress.is_running(now);
            if ring_moves || check_moves {
                window.request_animation_frame();
            }
            ShareSendProgressFrame {
                succeeded,
                rotation_degrees: state.rotation.value(now),
                trim_end: state.trim_end.value(now),
                check_progress: state.check_progress.value(now),
            }
        };

        div().size(px(FRAME_SIDE)).child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| paint_progress(bounds, &snapshot, window),
            )
            .size_full(),
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct ShareSendProgressFrame {
    succeeded: bool,
    rotation_degrees: f32,
    trim_end: f32,
    check_progress: f32,
}

fn paint_progress(bounds: Bounds<Pixels>, frame: &ShareSendProgressFrame, window: &mut Window) {
    let color = ShareSurface::label();
    if frame.succeeded {
        let circle = circle_points(bounds, RESTING_ROTATION_DEGREES.to_radians(), 2.0 * PI);
        stroke_points(&circle, RING_LINE_WIDTH, true, color, window);
        let check = ShareCheckmarkShape::path(inset_bounds(bounds, CHECK_PADDING));
        let trimmed = trimmed_polyline(&check, frame.check_progress.clamp(0.0, 1.0));
        if trimmed.len() >= 2 {
            stroke_points(&trimmed, CHECK_LINE_WIDTH, false, color, window);
            for vertex in &trimmed {
                round_dot(*vertex, CHECK_LINE_WIDTH, color, window);
            }
        }
        return;
    }
    let sweep = 2.0 * PI * frame.trim_end.clamp(0.0, 1.0);
    let arc = circle_points(bounds, frame.rotation_degrees.to_radians(), sweep);
    stroke_points(&arc, RING_LINE_WIDTH, false, color, window);
    if let (Some(first), Some(last)) = (arc.first(), arc.last()) {
        round_dot(*first, RING_LINE_WIDTH, color, window);
        round_dot(*last, RING_LINE_WIDTH, color, window);
    }
}

/// points along a circle in bounds from start for sweep radians, clockwise and starting at three oclock
fn circle_points(bounds: Bounds<Pixels>, start: f32, sweep: f32) -> Vec<Point<Pixels>> {
    let center = bounds.center();
    let radius = f32::from(bounds.size.width).min(f32::from(bounds.size.height)) / 2.0;
    let steps = ((sweep.abs() / (2.0 * PI)) * 96.0).ceil().max(2.0) as usize;
    (0..=steps)
        .map(|step| {
            let angle = start + sweep * step as f32 / steps as f32;
            point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
        })
        .collect()
}

fn inset_bounds(bounds: Bounds<Pixels>, inset: f32) -> Bounds<Pixels> {
    let width = (f32::from(bounds.size.width) - 2.0 * inset).max(0.0);
    let height = (f32::from(bounds.size.height) - 2.0 * inset).max(0.0);
    Bounds::new(
        point(bounds.origin.x + px(inset), bounds.origin.y + px(inset)),
        size(px(width), px(height)),
    )
}

struct ShareCheckmarkShape;

impl ShareCheckmarkShape {
    /// polyline the stroke follows
    fn path(rect: Bounds<Pixels>) -> Vec<Point<Pixels>> {
        let min_x = f32::from(rect.origin.x);
        let min_y = f32::from(rect.origin.y);
        let width = f32::from(rect.size.width);
        let height = f32::from(rect.size.height);
        let max_x = min_x + width;
        let max_y = min_y + height;
        let mid_y = min_y + height / 2.0;
        vec![
            point(px(min_x + width * 0.08), px(mid_y)),
            point(px(min_x + width * 0.40), px(max_y - height * 0.16)),
            point(px(max_x - width * 0.05), px(min_y + height * 0.10)),
        ]
    }
}

fn trimmed_polyline(points: &[Point<Pixels>], fraction: f32) -> Vec<Point<Pixels>> {
    let lengths: Vec<f32> = points
        .windows(2)
        .map(|pair| {
            let dx = f32::from(pair[1].x - pair[0].x);
            let dy = f32::from(pair[1].y - pair[0].y);
            (dx * dx + dy * dy).sqrt()
        })
        .collect();
    let total: f32 = lengths.iter().sum();
    let Some(first) = points.first() else {
        return Vec::new();
    };
    if total <= 0.0 || fraction <= 0.0 {
        return Vec::new();
    }
    let mut remaining = total * fraction.min(1.0);
    let mut trimmed = vec![*first];
    for (index, length) in lengths.iter().enumerate() {
        let from = points[index];
        let to = points[index + 1];
        if remaining >= *length {
            trimmed.push(to);
            remaining -= length;
            continue;
        }
        if *length > 0.0 {
            let t = remaining / length;
            trimmed.push(point(
                from.x + px(f32::from(to.x - from.x) * t),
                from.y + px(f32::from(to.y - from.y) * t),
            ));
        }
        break;
    }
    trimmed
}

fn stroke_points(points: &[Point<Pixels>], line_width: f32, closed: bool, color: Hsla, window: &mut Window) {
    let mut builder = PathBuilder::stroke(px(line_width));
    builder.add_polygon(points, closed);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn round_dot(at: Point<Pixels>, line_width: f32, color: Hsla, window: &mut Window) {
    let half = line_width / 2.0;
    let bounds = Bounds::new(point(at.x - px(half), at.y - px(half)), size(px(line_width), px(line_width)));
    window.paint_quad(fill(bounds, color).corner_radii(px(half)));
}
