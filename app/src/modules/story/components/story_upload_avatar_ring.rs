use std::f32::consts::TAU;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, ElementId, Entity, Hsla, IntoElement, PathBuilder, Pixels, Point, RenderOnce, SharedString,
    Styled, Task, Window, canvas, fill, point, px, size, transparent_black,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_engine::rise_upload::rise_upload_progress::{RiseUploadPhase, RiseUploadProgress};
use rise_platform::power::system_low_power_mode;
use rise_ui::swiftui::angular_gradient::{AngularGradient, point_on_circle, sector_count};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::{ease_out, linear};

use crate::modules::story::shared::story_publish_staging::StoryUploadProgressCenter;

/// publishing state of the avatar ring,
/// one gradient arc whose length is the transfer fraction and rotation is liveness
#[derive(IntoElement)]
pub struct StoryUploadAvatarRing {
    id: ElementId,
    pub line_width: f32,
    pub colors: Vec<Hsla>,
    pub track_color: Hsla,
}

impl StoryUploadAvatarRing {
    pub fn new(id: impl Into<ElementId>, line_width: f32, colors: Vec<Hsla>, track_color: Hsla) -> Self {
        Self {
            id: id.into(),
            line_width,
            colors,
            track_color,
        }
    }

    pub fn make_ui_view() -> StoryUploadAvatarRingView {
        StoryUploadAvatarRingView::new()
    }

    pub fn update_ui_view(&self, ui_view: &mut StoryUploadAvatarRingView, cx: &mut Context<StoryUploadAvatarRingView>) {
        ui_view.did_move_to_window(true, cx);
        ui_view.configure(self.line_width, self.colors.clone(), self.track_color, cx);
    }

    pub fn dismantle_ui_view(ui_view: &mut StoryUploadAvatarRingView) {
        ui_view.prepare_for_reuse();
    }
}

impl RenderOnce for StoryUploadAvatarRing {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view: Entity<StoryUploadAvatarRingView> = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("story-upload-ring")),
            cx,
            |_, _| Self::make_ui_view(),
        );
        let reduce_motion = reduce_motion(cx);
        let reduces_effects = system_low_power_mode()
            || RiseEnergySavingStore::snapshot_reduces(RiseEnergySavingFeature::InterfaceEffects);
        view.update(cx, |view, cx| {
            view.reduce_motion = reduce_motion;
            view.reduces_effects = reduces_effects;
            self.update_ui_view(view, cx);
        });

        let snapshot = view.read(cx).frame(RiseClock::monotonic());
        if snapshot.needs_next_frame {
            window.request_animation_frame();
        }
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                if snapshot.is_hidden {
                    return;
                }
                paint_ring(bounds, &snapshot, window);
            },
        )
        .size_full()
    }
}

#[derive(Clone, Debug)]
struct RingFrame {
    is_hidden: bool,
    needs_next_frame: bool,
    line_width: f32,
    track_color: Hsla,
    colors: Vec<Hsla>,
    stroke_end: f32,
    rotation: f32,
}

/// the oval both shape layers stroke
#[derive(Clone, Copy, Debug)]
struct RingGeometry {
    center: (f32, f32),
    radius: f32,
}

const OPAQUE_SEAM_OVERLAP: f32 = 0.2;

fn stroke_track(geometry: RingGeometry, line_width: f32, color: Hsla, window: &mut Window) {
    let steps = sector_count(geometry.radius * 2.0);
    let points: Vec<Point<Pixels>> = (0..steps)
        .map(|step| {
            let (x, y) = point_on_circle(geometry.center, geometry.radius, 360.0 * step as f32 / steps as f32);
            point(px(x), px(y))
        })
        .collect();
    let mut builder = PathBuilder::stroke(px(line_width));
    builder.add_polygon(&points, true);
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// each end of the arc gets a dot of the line width in the gradient color under it
fn round_cap(at: (f32, f32), line_width: f32, color: Hsla, window: &mut Window) {
    let half = line_width / 2.0;
    let bounds = Bounds::new(point(px(at.0 - half), px(at.1 - half)), size(px(line_width), px(line_width)));
    window.paint_quad(fill(bounds, color).corner_radii(px(half)));
}

fn paint_ring(bounds: Bounds<Pixels>, frame: &RingFrame, window: &mut Window) {
    let Some(geometry) = StoryUploadAvatarRingView::layout_subviews(bounds, frame.line_width) else {
        return;
    };
    if frame.track_color.a > 0.0 {
        stroke_track(geometry, frame.line_width, frame.track_color, window);
    }
    let stroke_end = frame.stroke_end.clamp(0.0, 1.0);
    if stroke_end <= 0.0 || frame.colors.is_empty() {
        return;
    }
    let gradient = AngularGradient::new(frame.colors.iter().copied()).angle(0.0);
    let rotation_degrees = frame.rotation.to_degrees();
    let sweep = 360.0 * stroke_end;
    let sectors = ((sector_count(geometry.radius * 2.0) as f32 * stroke_end).ceil() as usize).max(1);
    let step = sweep / sectors as f32;
    let inner = (geometry.radius - frame.line_width / 2.0).max(0.0);
    let outer = geometry.radius + frame.line_width / 2.0;
    for sector in 0..sectors {
        let local_start = sector as f32 * step;
        let color = gradient.color_at_angle(local_start + step / 2.0);
        let overlap = if color.a >= 1.0 && sector + 1 < sectors {
            OPAQUE_SEAM_OVERLAP
        } else {
            0.0
        };
        let start = rotation_degrees + local_start;
        let end = start + step + overlap;
        let corners = [
            point_on_circle(geometry.center, inner, start),
            point_on_circle(geometry.center, outer, start),
            point_on_circle(geometry.center, outer, end),
            point_on_circle(geometry.center, inner, end),
        ];
        let mut path = gpui::Path::new(point(px(corners[0].0), px(corners[0].1)));
        for &(x, y) in &corners[1..] {
            path.line_to(point(px(x), px(y)));
        }
        window.paint_path(path, color);
    }
    round_cap(
        point_on_circle(geometry.center, geometry.radius, rotation_degrees),
        frame.line_width,
        gradient.color_at_angle(0.0),
        window,
    );
    round_cap(
        point_on_circle(geometry.center, geometry.radius, rotation_degrees + sweep),
        frame.line_width,
        gradient.color_at_angle(sweep),
        window,
    );
}

pub struct StoryUploadAvatarRingView {
    progress_task: Option<Task<()>>,
    subscription_generation: u64,
    rendered_fraction: f64,
    highest_fraction: f64,
    did_settle: bool,

    is_in_window: bool,
    is_hidden: bool,
    line_width: f32,
    track_color: Hsla,
    colors: Vec<Hsla>,
    stroke_end: Animated<f32>,
    rotation: Animated<f32>,
    is_rotating: bool,
    is_settling: bool,
    spin_transform: f32,
    reduce_motion: bool,
    reduces_effects: bool,
}

impl StoryUploadAvatarRingView {
    const MINIMUM_FRACTION: f64 = 0.06;
    const STROKE_DURATION: f64 = 0.18;
    const ROTATION_DURATION: f64 = 1.05;
    const SETTLE_DURATION: f64 = 0.2;

    fn new() -> Self {
        Self {
            progress_task: None,
            subscription_generation: 0,
            rendered_fraction: -1.0,
            highest_fraction: 0.0,
            did_settle: false,
            is_in_window: false,
            is_hidden: true,
            line_width: 0.0,
            track_color: transparent_black(),
            colors: Vec::new(),
            stroke_end: Animated::new(0.0),
            rotation: Animated::new(0.0),
            is_rotating: false,
            is_settling: false,
            spin_transform: 0.0,
            reduce_motion: false,
            reduces_effects: false,
        }
    }

    fn frame(&self, now: Duration) -> RingFrame {
        let is_turning = self.is_rotating || (self.is_settling && self.rotation.is_running(now));
        let is_moving = self.stroke_end.is_running(now) || is_turning;
        RingFrame {
            is_hidden: self.is_hidden,
            needs_next_frame: !self.is_hidden && is_moving,
            line_width: self.line_width,
            track_color: self.track_color,
            colors: self.colors.clone(),
            stroke_end: self.stroke_end.value(now),
            rotation: if is_turning {
                self.rotation.value(now)
            } else {
                self.spin_transform
            },
        }
    }

    pub fn did_move_to_window(&mut self, is_in_window: bool, cx: &mut Context<Self>) {
        let was_in_window = self.is_in_window;
        self.is_in_window = is_in_window;
        if !is_in_window {
            self.cancel_subscription();
            self.stop_rotation();
        } else if !was_in_window {
            self.begin_subscription_if_needed(cx);
            self.ensure_rotation();
        }
    }

    fn layout_subviews(bounds: Bounds<Pixels>, line_width: f32) -> Option<RingGeometry> {
        let inset = line_width / 2.0;
        let width = f32::from(bounds.size.width) - 2.0 * inset;
        let height = f32::from(bounds.size.height) - 2.0 * inset;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let center = bounds.center();
        Some(RingGeometry {
            center: (f32::from(center.x), f32::from(center.y)),
            radius: width.min(height) / 2.0,
        })
    }

    pub fn configure(&mut self, line_width: f32, colors: Vec<Hsla>, track_color: Hsla, cx: &mut Context<Self>) {
        self.line_width = line_width.max(1.0);
        self.track_color = track_color;
        self.colors = colors;
        self.begin_subscription_if_needed(cx);
    }

    pub fn prepare_for_reuse(&mut self) {
        self.cancel_subscription();
        self.rendered_fraction = -1.0;
        self.highest_fraction = 0.0;
        self.did_settle = false;
        self.hide();
    }

    fn begin_subscription_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.is_in_window || self.progress_task.is_some() {
            return;
        }
        let key = StoryUploadProgressCenter::key();
        let hub = StoryUploadProgressCenter::hub();
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        let generation = self.subscription_generation;
        self.progress_task = Some(cx.spawn(async move |this, cx| {
            let mut updates = hub.updates_for(key).await;
            while let Some(progress) = updates.next().await {
                let delivered = this.update(cx, |view, cx| {
                    if view.subscription_generation != generation {
                        return false;
                    }
                    match progress {
                        Some(progress) => view.apply(&progress),
                        None => view.reset(),
                    }
                    cx.notify();
                    true
                });
                if !matches!(delivered, Ok(true)) {
                    return;
                }
            }
        }));
    }

    fn cancel_subscription(&mut self) {
        self.subscription_generation = self.subscription_generation.wrapping_add(1);
        self.progress_task = None;
    }

    fn apply(&mut self, progress: &RiseUploadProgress) {
        match progress.phase() {
            RiseUploadPhase::Failed { .. } => {
                self.hide();
                return;
            }
            RiseUploadPhase::Completed => {
                self.is_hidden = false;
                self.render(1.0);
                self.settle();
                return;
            }
            RiseUploadPhase::Preparing | RiseUploadPhase::Uploading | RiseUploadPhase::Processing => {
                self.is_hidden = false;
                self.did_settle = false;
                self.ensure_rotation();
            }
        }

        let fraction = match progress.phase() {
            RiseUploadPhase::Preparing => Self::MINIMUM_FRACTION,
            RiseUploadPhase::Processing => 1.0,
            _ => Self::MINIMUM_FRACTION.max(progress.fraction().min(1.0)),
        };
        self.render(self.highest_fraction.max(fraction));
    }

    fn render(&mut self, fraction: f64) {
        self.highest_fraction = self.highest_fraction.max(fraction);
        let target = fraction.max(0.0).min(1.0);
        if (target - self.rendered_fraction).abs() < 0.002 {
            return;
        }
        let previous = self.rendered_fraction;
        self.rendered_fraction = target;

        let now = RiseClock::monotonic();
        let current = self.stroke_end.value(now);
        let target = target as f32;
        if previous < 0.0 || !self.is_in_window || self.reduce_motion || (target - current).abs() < 0.002 {
            self.stroke_end.jump(target);
            return;
        }
        self.stroke_end.set(target, linear(Self::STROKE_DURATION), now, false);
    }

    fn reset(&mut self) {
        self.rendered_fraction = -1.0;
        self.highest_fraction = 0.0;
        self.did_settle = false;
        self.stroke_end.jump(0.0);
        self.hide();
    }

    fn hide(&mut self) {
        self.is_hidden = true;
        self.stop_rotation();
    }

    fn settle(&mut self) {
        if self.did_settle {
            return;
        }
        self.did_settle = true;
        let now = RiseClock::monotonic();
        let presented = if self.is_rotating {
            self.rotation.value(now)
        } else {
            self.spin_transform
        };
        let angle = presented.rem_euclid(TAU);
        self.stop_rotation();
        self.spin_transform = 0.0;
        self.rotation.jump(0.0);
        if angle <= 0.01 || !self.is_in_window || self.reduce_motion {
            return;
        }
        self.rotation.jump(angle);
        self.rotation.set(TAU, ease_out(Self::SETTLE_DURATION), now, false);
        self.is_settling = true;
    }

    fn ensure_rotation(&mut self) {
        if self.did_settle
            || self.is_hidden
            || !self.is_in_window
            || self.reduce_motion
            || self.reduces_effects
            || self.is_rotating
        {
            return;
        }
        self.is_settling = false;
        let now = RiseClock::monotonic();
        self.rotation.jump(0.0);
        self.rotation
            .set(TAU, linear(Self::ROTATION_DURATION).repeat_forever(false), now, false);
        self.is_rotating = true;
    }

    fn stop_rotation(&mut self) {
        self.is_rotating = false;
        self.is_settling = false;
        self.rotation.jump(self.spin_transform);
    }
}

impl Drop for StoryUploadAvatarRingView {
    fn drop(&mut self) {
        StoryUploadAvatarRing::dismantle_ui_view(self);
    }
}
