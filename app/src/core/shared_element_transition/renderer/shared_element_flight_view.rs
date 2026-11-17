use std::sync::Arc;
use std::time::Duration;

use rise_core::foundation::foundation_date::RiseClock;

use gpui::{
    AnyElement, AnyView, Bounds, Context, Div, IntoElement, ObjectFit, ParentElement as _, Pixels, Render, RenderImage,
    Styled as _, StyledImage as _, Window, div, img, point, px, size,
};
use rise_ui::swiftui::motion::{CubicBezier, SPRING_SETTLE_EPSILON, Spring};
use rise_ui::swiftui::shapes::SwiftUiShapes as _;

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementCubicTiming, SharedElementGeometry,
};
use crate::core::shared_element_transition::renderer::shared_element_flight_runner::SharedElementFlightAlongside;
use crate::core::shared_element_transition::renderer::shared_element_live_video_view::SharedElementLiveVideoView;
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCoverContent, SharedElementFlightContent,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SharedElementFlightViewLayout {
    pub clip: Bounds<Pixels>,
    pub corner_radius: f32,
    pub content_frame: Bounds<Pixels>,
    pub backdrop_frame: Bounds<Pixels>,
    pub backdrop_alpha: f32,
    pub cover_frame: Bounds<Pixels>,
}

impl SharedElementFlightViewLayout {
    /// layout under a uniform scale around the clip center
    fn transformed(self, scale: f32) -> Self {
        if scale == 1.0 {
            return self;
        }
        let scaled = |frame: Bounds<Pixels>| Bounds {
            origin: point(frame.origin.x * scale, frame.origin.y * scale),
            size: size(frame.size.width * scale, frame.size.height * scale),
        };
        let clip_size = size(self.clip.size.width * scale, self.clip.size.height * scale);
        let clip_origin = point(
            self.clip.origin.x + (self.clip.size.width - clip_size.width) * 0.5,
            self.clip.origin.y + (self.clip.size.height - clip_size.height) * 0.5,
        );
        Self {
            clip: Bounds { origin: clip_origin, size: clip_size },
            corner_radius: self.corner_radius * scale,
            content_frame: scaled(self.content_frame),
            backdrop_frame: scaled(self.backdrop_frame),
            backdrop_alpha: self.backdrop_alpha,
            cover_frame: scaled(self.cover_frame),
        }
    }

    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        Self {
            clip: SharedElementGeometry::lerp_rect(a.clip, b.clip, t),
            corner_radius: SharedElementGeometry::lerp(a.corner_radius, b.corner_radius, t),
            content_frame: SharedElementGeometry::lerp_rect(a.content_frame, b.content_frame, t),
            backdrop_frame: SharedElementGeometry::lerp_rect(a.backdrop_frame, b.backdrop_frame, t),
            backdrop_alpha: SharedElementGeometry::lerp(a.backdrop_alpha, b.backdrop_alpha, t),
            cover_frame: SharedElementGeometry::lerp_rect(a.cover_frame, b.cover_frame, t),
        }
    }
}

/// timing parameters of one animator
#[derive(Clone, Copy, Debug)]
pub(crate) enum SharedElementFlightTiming {
    Cubic(SharedElementCubicTiming),
    Spring { spring: Spring, initial_velocity: f32 },
    EaseOut,
}

impl SharedElementFlightTiming {
    /// spring the animator runs, response is scaled so it settles at the animator duration
    pub(crate) fn spring(duration: f64, damping_ratio: f32, initial_velocity: f32) -> Self {
        let probe = Spring::new(duration.max(0.0), f64::from(damping_ratio));
        let settling = probe.settling_time(1.0, -f64::from(initial_velocity), SPRING_SETTLE_EPSILON);
        let response = if settling > 0.0 && settling.is_finite() {
            duration * duration / settling
        } else {
            duration
        };
        Self::Spring { spring: Spring::new(response.max(0.0), f64::from(damping_ratio)), initial_velocity }
    }

    /// progress at elapsed seconds of an animation that lasts duration seconds
    fn progress(&self, elapsed: f64, duration: f64) -> f32 {
        if duration <= 0.0 || elapsed >= duration {
            return 1.0;
        }
        let elapsed = elapsed.max(0.0);
        match self {
            Self::Cubic(timing) => timing.progress((elapsed / duration) as f32),
            Self::Spring { spring, initial_velocity } => {
                let (displacement, _) = spring.state(1.0, -f64::from(*initial_velocity), elapsed);
                (1.0 - displacement) as f32
            }
            Self::EaseOut => CubicBezier::EASE_OUT.progress(elapsed / duration) as f32,
        }
    }
}

/// animations one animator holds on the flight view, with their start values
pub(crate) struct SharedElementFlightAnimation {
    pub(crate) start: Duration,
    pub(crate) duration: f64,
    pub(crate) timing: SharedElementFlightTiming,
    pub(crate) layout: Option<(SharedElementFlightViewLayout, SharedElementFlightViewLayout)>,
    pub(crate) layout_base_progress: f32,
    pub(crate) alpha: Option<(f32, f32)>,
    pub(crate) cover_alpha: Option<(f32, f32, f32)>,
    pub(crate) alongside: Option<SharedElementFlightAlongside>,
}

impl SharedElementFlightAnimation {
    fn progress(&self, now: Duration) -> f32 {
        self.timing.progress(now.saturating_sub(self.start).as_secs_f64(), self.duration)
    }

    fn delayed_progress(&self, progress: f32, delay_factor: f32) -> f32 {
        if delay_factor <= 0.0 {
            return progress;
        }
        let delay_progress = self.timing.progress(self.duration * f64::from(delay_factor), self.duration);
        if delay_progress >= 1.0 {
            return if progress >= 1.0 { 1.0 } else { 0.0 };
        }
        ((progress - delay_progress) / (1.0 - delay_progress)).clamp(0.0, 1.0)
    }
}

/// linear opacity keyframes over duration, filled backwards
pub(crate) struct SharedElementOpacityKeyframes {
    pub(crate) start: Duration,
    pub(crate) duration: f64,
    pub(crate) values: Vec<f32>,
    pub(crate) key_times: Vec<f32>,
}

impl SharedElementOpacityKeyframes {
    /// none once the animation is over and removed on completion
    fn value(&self, now: Duration) -> Option<f32> {
        let elapsed = now.saturating_sub(self.start).as_secs_f64();
        if self.duration <= 0.0 || elapsed >= self.duration {
            return None;
        }
        let time = (elapsed / self.duration) as f32;
        let (first, last) = (self.values.first()?, self.values.last()?);
        if time <= self.key_times.first().copied().unwrap_or(0.0) {
            return Some(*first);
        }
        for index in 1..self.key_times.len().min(self.values.len()) {
            let (t0, t1) = (self.key_times[index - 1], self.key_times[index]);
            if time <= t1 {
                let span = t1 - t0;
                let local = if span > 0.0 { (time - t0) / span } else { 1.0 };
                return Some(SharedElementGeometry::lerp(self.values[index - 1], self.values[index], local));
            }
        }
        Some(*last)
    }
}

enum SharedElementFlightViewContent {
    Bitmap(Arc<RenderImage>),
    Replicant(AnyView),
}

pub struct SharedElementFlightView {
    content: Option<SharedElementFlightViewContent>,
    backdrop_image: Option<Arc<RenderImage>>,
    cover_image: Option<SharedElementCoverContent>,
    placeholder_image: Option<Arc<RenderImage>>,
    model_layout: Option<SharedElementFlightViewLayout>,
    model_alpha: f32,
    model_cover_alpha: f32,
    model_transform_scale: f32,
    animation: Option<SharedElementFlightAnimation>,
    cover_keyframes: Option<SharedElementOpacityKeyframes>,
    flight_keyframes: Option<SharedElementOpacityKeyframes>,
}

/// what the view draws at one instant
struct SharedElementFlightPresentation {
    layout: Option<SharedElementFlightViewLayout>,
    alpha: f32,
    cover_alpha: f32,
}

impl SharedElementFlightView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            content: None,
            backdrop_image: None,
            cover_image: None,
            placeholder_image: None,
            model_layout: None,
            model_alpha: 1.0,
            model_cover_alpha: 0.0,
            model_transform_scale: 1.0,
            animation: None,
            cover_keyframes: None,
            flight_keyframes: None,
        }
    }

    pub fn set_content(&mut self, content: SharedElementFlightContent, cx: &mut Context<Self>) {
        self.remove_replicant(cx);
        self.content = Some(match content {
            SharedElementFlightContent::Bitmap(image, _) => SharedElementFlightViewContent::Bitmap(image),
            SharedElementFlightContent::Replicant(view, _) => SharedElementFlightViewContent::Replicant(view),
        });
        cx.notify();
    }

    /// blur and dim are already baked into the image
    pub fn set_backdrop_image(&mut self, image: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        self.backdrop_image = image;
        cx.notify();
    }

    pub fn set_cover_image(&mut self, image: Option<SharedElementCoverContent>, cx: &mut Context<Self>) {
        self.cover_image = image;
        cx.notify();
    }

    /// shown under replicant content like a live video layer with no frame yet
    /// hidden while the content is a bitmap
    pub fn set_placeholder(&mut self, image: impl Into<Option<Arc<RenderImage>>>, cx: &mut Context<Self>) {
        self.placeholder_image = image.into();
        cx.notify();
    }

    pub fn has_cover_image(&self) -> bool {
        self.cover_image.is_some()
    }

    pub fn apply(&mut self, layout: SharedElementFlightViewLayout, cx: &mut Context<Self>) {
        self.model_layout = Some(layout);
        cx.notify();
    }

    pub fn clear_contents(&mut self, cx: &mut Context<Self>) {
        self.remove_replicant(cx);
        self.content = None;
        self.placeholder_image = None;
        self.backdrop_image = None;
        self.cover_image = None;
        cx.notify();
    }

    pub(crate) fn set_alpha(&mut self, alpha: f32, cx: &mut Context<Self>) {
        self.model_alpha = alpha;
        cx.notify();
    }

    pub(crate) fn set_transform_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        self.model_transform_scale = scale;
        cx.notify();
    }

    pub(crate) fn set_cover_alpha(&mut self, alpha: f32, cx: &mut Context<Self>) {
        self.model_cover_alpha = alpha;
        cx.notify();
    }

    pub(crate) fn model_layout(&self) -> Option<SharedElementFlightViewLayout> {
        self.model_layout
    }

    pub(crate) fn model_alpha(&self) -> f32 {
        self.model_alpha
    }

    pub(crate) fn model_cover_alpha(&self) -> f32 {
        self.model_cover_alpha
    }

    pub(crate) fn animation_start(&self) -> Option<Duration> {
        self.animation.as_ref().map(|animation| animation.start)
    }

    pub(crate) fn start_animation(&mut self, animation: SharedElementFlightAnimation, cx: &mut Context<Self>) {
        if let Some((_, to)) = animation.layout {
            self.model_layout = Some(to);
        }
        if let Some((_, to)) = animation.alpha {
            self.model_alpha = to;
        }
        if let Some((_, to, _)) = animation.cover_alpha {
            self.model_cover_alpha = to;
        }
        self.animation = Some(animation);
        cx.notify();
    }

    pub(crate) fn retarget_animation(
        &mut self,
        to: SharedElementFlightViewLayout,
        now: Duration,
        cx: &mut Context<Self>,
    ) -> bool {
        let presented = self.presentation(now).layout;
        let Some(animation) = self.animation.as_mut() else { return false };
        let progress = animation.progress(now);
        if let Some(from) = presented {
            animation.layout = Some((from, to));
            animation.layout_base_progress = progress;
        }
        self.model_layout = Some(to);
        cx.notify();
        true
    }

    /// presentation values become the model values
    pub(crate) fn stop_animation(&mut self, now: Duration, cx: &mut Context<Self>) {
        if self.animation.is_none() {
            return;
        }
        let presented = self.animated_presentation(now);
        self.model_layout = presented.layout;
        self.model_alpha = presented.alpha;
        self.model_cover_alpha = presented.cover_alpha;
        self.animation = None;
        cx.notify();
    }

    pub(crate) fn finish_animation(&mut self, cx: &mut Context<Self>) -> Option<SharedElementFlightAlongside> {
        let alongside = self.animation.take().and_then(|animation| animation.alongside);
        cx.notify();
        alongside
    }

    pub(crate) fn add_cover_keyframes(&mut self, keyframes: SharedElementOpacityKeyframes, cx: &mut Context<Self>) {
        self.cover_keyframes = Some(keyframes);
        cx.notify();
    }

    pub(crate) fn add_flight_keyframes(&mut self, keyframes: SharedElementOpacityKeyframes, cx: &mut Context<Self>) {
        self.flight_keyframes = Some(keyframes);
        cx.notify();
    }

    pub(crate) fn remove_cover_keyframes(&mut self, cx: &mut Context<Self>) {
        self.cover_keyframes = None;
        cx.notify();
    }

    pub(crate) fn remove_flight_keyframes(&mut self, cx: &mut Context<Self>) {
        self.flight_keyframes = None;
        cx.notify();
    }

    /// drops all animations on the view and its cover
    pub(crate) fn remove_all_animations(&mut self, cx: &mut Context<Self>) {
        self.animation = None;
        self.cover_keyframes = None;
        self.flight_keyframes = None;
        cx.notify();
    }

    fn remove_replicant(&mut self, cx: &mut Context<Self>) {
        if let Some(SharedElementFlightViewContent::Replicant(view)) = self.content.take() {
            Self::detach_live_video(view, cx);
        }
    }

    fn detach_live_video(view: AnyView, cx: &mut Context<Self>) {
        if let Ok(live) = view.downcast::<SharedElementLiveVideoView>() {
            live.update(cx, |live, cx| live.detach_player(cx));
        }
    }

    fn animated_presentation(&self, now: Duration) -> SharedElementFlightPresentation {
        let mut presentation = SharedElementFlightPresentation {
            layout: self.model_layout,
            alpha: self.model_alpha,
            cover_alpha: self.model_cover_alpha,
        };
        let Some(animation) = &self.animation else { return presentation };
        let progress = animation.progress(now);
        if let Some((from, to)) = &animation.layout {
            let base = animation.layout_base_progress;
            let local = if base >= 1.0 { 1.0 } else { (progress - base) / (1.0 - base) };
            presentation.layout = Some(SharedElementFlightViewLayout::lerp(from, to, local));
        }
        if let Some((from, to)) = animation.alpha {
            presentation.alpha = SharedElementGeometry::lerp(from, to, progress);
        }
        if let Some((from, to, delay_factor)) = animation.cover_alpha {
            presentation.cover_alpha =
                SharedElementGeometry::lerp(from, to, animation.delayed_progress(progress, delay_factor));
        }
        presentation
    }

    fn presentation(&self, now: Duration) -> SharedElementFlightPresentation {
        let mut presentation = self.animated_presentation(now);
        if let Some(value) = self.flight_keyframes.as_ref().and_then(|keyframes| keyframes.value(now)) {
            presentation.alpha = value;
        }
        if let Some(value) = self.cover_keyframes.as_ref().and_then(|keyframes| keyframes.value(now)) {
            presentation.cover_alpha = value;
        }
        presentation
    }

    fn is_animating(&self, now: Duration) -> bool {
        let running = |start: Duration, duration: f64| now.saturating_sub(start).as_secs_f64() < duration;
        self.animation.as_ref().is_some_and(|animation| running(animation.start, animation.duration))
            || self.cover_keyframes.as_ref().is_some_and(|keyframes| running(keyframes.start, keyframes.duration))
            || self.flight_keyframes.as_ref().is_some_and(|keyframes| running(keyframes.start, keyframes.duration))
    }

    fn frame_element(frame: Bounds<Pixels>) -> Div {
        div()
            .absolute()
            .left(frame.origin.x)
            .top(frame.origin.y)
            .w(frame.size.width)
            .h(frame.size.height)
    }

    fn image_element(image: &Arc<RenderImage>) -> AnyElement {
        img(image.clone()).size_full().object_fit(ObjectFit::Fill).into_any_element()
    }
}

impl Render for SharedElementFlightView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        if self.is_animating(now) {
            window.request_animation_frame();
        }
        if let Some(animation) = &self.animation
            && let Some(alongside) = animation.alongside.clone()
        {
            let progress = animation.progress(now);
            alongside(progress, &mut **cx);
        }
        let presentation = self.presentation(now);
        let Some(layout) = presentation.layout else {
            return div().absolute().size(px(0.0)).into_any_element();
        };
        let layout = layout.transformed(self.model_transform_scale);
        let local = Bounds { origin: point(px(0.0), px(0.0)), size: layout.clip.size };

        let mut content = Self::frame_element(layout.content_frame);
        let replicant_shown = matches!(self.content, Some(SharedElementFlightViewContent::Replicant(_)));
        if replicant_shown && let Some(placeholder) = &self.placeholder_image {
            content = content.child(Self::frame_element(Bounds { origin: local.origin, size: layout.content_frame.size })
                .child(Self::image_element(placeholder)));
        }
        match &self.content {
            Some(SharedElementFlightViewContent::Bitmap(image)) => {
                content = content.child(Self::image_element(image));
            }
            Some(SharedElementFlightViewContent::Replicant(view)) => {
                content = content.child(div().absolute().top_0().left_0().size_full().child(view.clone()));
            }
            None => {}
        }

        let mut clip = div()
            .absolute()
            .left(layout.clip.origin.x)
            .top(layout.clip.origin.y)
            .w(layout.clip.size.width)
            .h(layout.clip.size.height)
            .continuous_corners(layout.corner_radius)
            .overflow_hidden()
            .opacity(presentation.alpha.clamp(0.0, 1.0));
        if let Some(backdrop) = &self.backdrop_image {
            clip = clip.child(
                Self::frame_element(layout.backdrop_frame)
                    .opacity(layout.backdrop_alpha.clamp(0.0, 1.0))
                    .child(Self::image_element(backdrop)),
            );
        }
        clip = clip.child(content);
        if let Some(cover) = self.cover_image.clone() {
            let cover = match cover {
                SharedElementCoverContent::Image(image) => Self::image_element(&image),
                SharedElementCoverContent::Element(element) => element(window, cx),
            };
            clip = clip.child(
                Self::frame_element(layout.cover_frame)
                    .opacity(presentation.cover_alpha.clamp(0.0, 1.0))
                    .child(cover),
            );
        }
        clip.into_any_element()
    }
}
