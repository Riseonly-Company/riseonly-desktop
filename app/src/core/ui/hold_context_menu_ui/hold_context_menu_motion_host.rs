use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, IntoElement, ParentElement, Pixels, Point, RenderOnce, SharedString, Size,
    Styled, Task, Window, div, point, px,
};
use rise_canon::rise_key::otp::DURATION_MS;
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::display_cadence::maximum_frames_per_second;
use rise_platform::power::system_low_power_mode;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::rise_frame_rate::RiseMotionCadencePolicy;
use rise_ui::swiftui::{
    Alignment, Animated, SwiftUiAlignedFrame, TransitionEffect, TransitionModifiers, duration_from_seconds,
    ease_out, reduce_motion, spring,
};

use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;

const MOTION_BEGIN_EVENT: &str = "hold_menu.motion_begin";
const MOTION_END_EVENT: &str = "hold_menu.motion_end";
const MOTION_RENDERER: &str = "core_animation";

pub type HoldContextMenuMotionCompletion = Rc<dyn Fn(&mut App)>;

/// motion host with defaults, zero offset and scale and opacity 1
/// open response and damping, no completion
/// id names the motion state of the surface
#[derive(IntoElement)]
pub struct HoldContextMenuMotionHost {
    id: ElementId,
    surface: SharedString,
    size: Size<Pixels>,
    content_alignment: Option<Alignment>,
    offset: Point<Pixels>,
    scale: f32,
    opacity: f32,
    response: f64,
    damping: f64,
    phase: i64,
    on_motion_completed: Option<HoldContextMenuMotionCompletion>,
    content: AnyElement,
}

impl HoldContextMenuMotionHost {
    pub fn new(
        id: impl Into<ElementId>,
        surface: impl Into<SharedString>,
        size: Size<Pixels>,
        content: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            surface: surface.into(),
            size,
            content_alignment: None,
            offset: point(px(0.0), px(0.0)),
            scale: 1.0,
            opacity: 1.0,
            response: HoldContextMenuMetrics::OPEN_RESPONSE,
            damping: HoldContextMenuMetrics::OPEN_DAMPING,
            phase: 0,
            on_motion_completed: None,
            content: content.into_any_element(),
        }
    }

    pub fn content_alignment(mut self, content_alignment: Option<Alignment>) -> Self {
        self.content_alignment = content_alignment;
        self
    }

    pub fn offset(mut self, offset: Point<Pixels>) -> Self {
        self.offset = offset;
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn response(mut self, response: f64) -> Self {
        self.response = response;
        self
    }

    pub fn damping(mut self, damping: f64) -> Self {
        self.damping = damping;
        self
    }

    pub fn phase(mut self, phase: i64) -> Self {
        self.phase = phase;
        self
    }

    pub fn on_motion_completed(mut self, on_motion_completed: Option<HoldContextMenuMotionCompletion>) -> Self {
        self.on_motion_completed = on_motion_completed;
        self
    }
}

impl RenderOnce for HoldContextMenuMotionHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let surface = self.surface.clone();
        let controller = window.use_keyed_state(self.id.clone(), cx, move |_, _| Controller::new(surface));
        controller.update(cx, |controller, cx| {
            controller.apply(
                Motion {
                    offset: self.offset,
                    scale: self.scale,
                    opacity: self.opacity,
                    response: self.response,
                    damping: self.damping,
                    phase: self.phase,
                },
                self.on_motion_completed.clone(),
                window,
                cx,
            );
        });
        let now = RiseClock::monotonic();
        let (offset, scale, opacity) = {
            let controller = controller.read(cx);
            (
                controller.offset.frame_value(now, window),
                controller.scale.frame_value(now, window),
                controller.opacity.frame_value(now, window),
            )
        };

        let width = f32::from(self.size.width);
        let height = f32::from(self.size.height);
        let effect = TransitionEffect {
            opacity,
            offset: point(
                if width > 0.0 { f32::from(offset.x) / width } else { 0.0 },
                if height > 0.0 { f32::from(offset.y) / height } else { 0.0 },
            ),
            scale,
            ..TransitionEffect::IDENTITY
        };
        let content = match self.content_alignment {
            Some(alignment) => self.content.frame_max(alignment).into_any_element(),
            None => self.content,
        };
        div()
            .relative()
            .flex_none()
            .w(self.size.width)
            .h(self.size.height)
            .child(content)
            .transition_effect(effect)
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Motion {
    offset: Point<Pixels>,
    scale: f32,
    opacity: f32,
    response: f64,
    damping: f64,
    phase: i64,
}

struct Controller {
    surface: SharedString,
    offset: Animated<Point<Pixels>>,
    scale: Animated<f32>,
    opacity: Animated<f32>,
    target: Option<(Point<Pixels>, f32, f32)>,
    generation: u64,
    phase: i64,
    completion_task: Option<Task<()>>,
}

impl Controller {
    fn new(surface: SharedString) -> Self {
        Self {
            surface,
            offset: Animated::new(point(px(0.0), px(0.0))),
            scale: Animated::new(1.0),
            opacity: Animated::new(1.0),
            target: None,
            generation: 0,
            phase: 0,
            completion_task: None,
        }
    }

    fn apply(
        &mut self,
        motion: Motion,
        completion: Option<HoldContextMenuMotionCompletion>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let target = (motion.offset, motion.scale, motion.opacity);
        if self.target == Some(target) && self.phase == motion.phase {
            return;
        }

        let now = RiseClock::monotonic();
        let from_offset = self.offset.value(now);
        let from_scale = self.scale.value(now);
        let from_opacity = self.opacity.value(now);
        let animated = self.target.is_some() && motion.response > 0.0 && !reduce_motion(cx);
        self.target = Some(target);
        self.phase = motion.phase;
        self.generation += 1;
        let expected_generation = self.generation;

        let transform_changes = from_offset != motion.offset || from_scale != motion.scale;
        let opacity_changes = from_opacity != motion.opacity;
        if !animated || !(transform_changes || opacity_changes) {
            self.offset.jump(motion.offset);
            self.scale.jump(motion.scale);
            self.opacity.jump(motion.opacity);
            self.completion_task = Some(cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| {
                    if this.generation != expected_generation {
                        return;
                    }
                    if let Some(completion) = completion {
                        completion(cx);
                    }
                });
            }));
            return;
        }

        let duration = motion.response * HoldContextMenuMetrics::SOURCE_RESTORE_DELAY_FACTOR;
        let fps = RiseMotionCadencePolicy::preferred_fps(
            i64::from(maximum_frames_per_second(window)),
            system_low_power_mode(),
        );
        if transform_changes {
            let spec = spring(motion.response, motion.damping);
            self.offset.set(motion.offset, spec, now, false);
            self.scale.set(motion.scale, spec, now, false);
        } else {
            self.offset.jump(motion.offset);
            self.scale.jump(motion.scale);
        }
        if opacity_changes {
            self.opacity.set(motion.opacity, ease_out(duration), now, false);
        } else {
            self.opacity.jump(motion.opacity);
        }
        let fields = HashMap::from([
            ("surface".to_owned(), self.surface.to_string()),
            ("renderer".to_owned(), MOTION_RENDERER.to_owned()),
            (DURATION_MS.to_owned(), format!("{:.2}", duration * 1_000.0)),
            ("preferred_hz".to_owned(), format!("{}", fps as i64)),
        ]);
        ChatPerformanceTrace::shared().record(MOTION_BEGIN_EVENT, fields);

        self.completion_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration_from_seconds(duration)).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != expected_generation {
                    return;
                }
                let fields = HashMap::from([("surface".to_owned(), this.surface.to_string())]);
                ChatPerformanceTrace::shared().record(MOTION_END_EVENT, fields);
                if let Some(completion) = completion {
                    completion(cx);
                }
            });
        }));
    }
}
