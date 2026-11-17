use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, ElementId, Entity, InteractiveElement, IntoElement, ParentElement, PinchEvent, Pixels,
    Point, RenderOnce, SharedString, Size, Styled, TouchPhase, Window, div, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::transition::UnitPoint;

/// where the pinched media sits, scaled about the start point then moved by the fingers center
/// anchor is kept on release so the spring back goes to the same point
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShortsPinchTransform {
    pub scale: f32,
    pub anchor: UnitPoint,
    pub translation: Size<Pixels>,
}

impl Default for ShortsPinchTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            anchor: UnitPoint::CENTER,
            translation: size(px(0.0), px(0.0)),
        }
    }
}

impl ShortsPinchTransform {
    /// box the media fills at this transform, relative to the layer origin
    pub fn frame(&self, base: Size<Pixels>) -> Bounds<Pixels> {
        let width = f32::from(base.width);
        let height = f32::from(base.height);
        Bounds {
            origin: point(
                px(self.anchor.x * width * (1.0 - self.scale)) + self.translation.width,
                px(self.anchor.y * height * (1.0 - self.scale)) + self.translation.height,
            ),
            size: size(px(width * self.scale), px(height * self.scale)),
        }
    }
}

type ActiveChanged = Rc<dyn Fn(bool, &mut Window, &mut App)>;
type PinchContent = Box<dyn FnOnce(Size<Pixels>, &mut Window, &mut App) -> AnyElement>;

/// per frame state of the pinch layer
struct ShortsPinchZoomState {
    scale: Animated<f32>,
    translation: Animated<Size<Pixels>>,
    anchor: UnitPoint,
    recognizer_scale: f32,
    start_centroid: Point<Pixels>,
    is_tracking: bool,
    is_pinch_enabled: bool,
    bounds: Bounds<Pixels>,
}

impl ShortsPinchZoomState {
    const MAXIMUM_SCALE: f32 = 4.0;
    const MINIMUM_MEASURED_SIDE: f32 = 1.0;

    fn new() -> Self {
        Self {
            scale: Animated::new(1.0),
            translation: Animated::new(size(px(0.0), px(0.0))),
            anchor: UnitPoint::CENTER,
            recognizer_scale: 1.0,
            start_centroid: Point::default(),
            is_tracking: false,
            is_pinch_enabled: true,
            bounds: Bounds::default(),
        }
    }

    fn spring_back() -> MotionSpec {
        spring(0.34, 0.86)
    }

    fn local_point(&self, position: Point<Pixels>) -> Point<Pixels> {
        point(position.x - self.bounds.origin.x, position.y - self.bounds.origin.y)
    }

    fn can_measure(&self) -> bool {
        f32::from(self.bounds.size.width) > Self::MINIMUM_MEASURED_SIDE
            && f32::from(self.bounds.size.height) > Self::MINIMUM_MEASURED_SIDE
    }

    fn published_transform(&self, position: Point<Pixels>) -> ShortsPinchTransform {
        let location = self.local_point(position);
        ShortsPinchTransform {
            scale: self.recognizer_scale.clamp(1.0, Self::MAXIMUM_SCALE),
            anchor: UnitPoint::new(
                f32::from(self.start_centroid.x) / f32::from(self.bounds.size.width),
                f32::from(self.start_centroid.y) / f32::from(self.bounds.size.height),
            ),
            translation: size(location.x - self.start_centroid.x, location.y - self.start_centroid.y),
        }
    }

    fn publish(&mut self, position: Point<Pixels>) {
        if !self.can_measure() {
            return;
        }
        let transform = self.published_transform(position);
        self.scale.jump(transform.scale);
        self.translation.jump(transform.translation);
        self.anchor = transform.anchor;
    }
}

/// pinch on a riff, media follows two fingers and springs back
/// no staying zoom, pagers own the drags
/// no layer transform in gpui, content lays out scaled
#[derive(IntoElement)]
pub struct ShortsPinchZoomLayer {
    id: ElementId,
    size: Size<Pixels>,
    is_enabled: bool,
    recognizes_alongside_all: bool,
    on_active_changed: ActiveChanged,
    content: PinchContent,
}

impl ShortsPinchZoomLayer {
    pub fn new(
        id: impl Into<ElementId>,
        size: Size<Pixels>,
        is_enabled: bool,
        on_active_changed: impl Fn(bool, &mut Window, &mut App) + 'static,
        content: impl FnOnce(Size<Pixels>, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            size,
            is_enabled,
            recognizes_alongside_all: false,
            on_active_changed: Rc::new(on_active_changed),
            content: Box::new(content),
        }
    }

    pub fn recognizes_alongside_all(mut self, recognizes_alongside_all: bool) -> Self {
        self.recognizes_alongside_all = recognizes_alongside_all;
        self
    }

    fn child_id(id: &ElementId, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(suffix))
    }

    fn handle_pinch(
        state: &Entity<ShortsPinchZoomState>,
        on_active_changed: &ActiveChanged,
        recognizes_alongside_all: bool,
        event: &PinchEvent,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (is_enabled, is_tracking, can_measure) = {
            let state = state.read(cx);
            (state.is_pinch_enabled, state.is_tracking, state.can_measure())
        };
        if !is_enabled {
            return;
        }
        if !recognizes_alongside_all {
            cx.stop_propagation();
        }
        match event.phase {
            TouchPhase::Started | TouchPhase::Moved if !is_tracking => {
                if !can_measure {
                    return;
                }
                state.update(cx, |state, cx| {
                    state.start_centroid = state.local_point(event.position);
                    state.recognizer_scale = 1.0 + event.delta;
                    state.is_tracking = true;
                    state.publish(event.position);
                    cx.notify();
                });
                on_active_changed(true, window, cx);
            }
            TouchPhase::Started | TouchPhase::Moved => {
                state.update(cx, |state, cx| {
                    state.recognizer_scale *= 1.0 + event.delta;
                    state.publish(event.position);
                    cx.notify();
                });
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if is_tracking {
                    state.update(cx, |state, _| state.is_tracking = false);
                    Self::finish(state, on_active_changed, window, cx);
                }
            }
        }
    }

    fn finish(
        state: &Entity<ShortsPinchZoomState>,
        on_active_changed: &ActiveChanged,
        window: &mut Window,
        cx: &mut App,
    ) {
        on_active_changed(false, window, cx);
        let now = RiseClock::monotonic();
        state.update(cx, |state, cx| {
            state.scale.set_in(1.0, ShortsPinchZoomState::spring_back(), now, cx);
            state.translation.set_in(size(px(0.0), px(0.0)), ShortsPinchZoomState::spring_back(), now, cx);
            cx.notify();
        });
    }

    fn finish_deferred(
        state: &Entity<ShortsPinchZoomState>,
        on_active_changed: &ActiveChanged,
        window: &mut Window,
        cx: &mut App,
    ) {
        state.update(cx, |state, _| state.is_tracking = false);
        let state = state.clone();
        let on_active_changed = on_active_changed.clone();
        window.defer(cx, move |window, cx| Self::finish(&state, &on_active_changed, window, cx));
    }
}

impl RenderOnce for ShortsPinchZoomLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(Self::child_id(&self.id, "state"), cx, |_, _| ShortsPinchZoomState::new());

        let is_enabled = self.is_enabled;
        let forced_release = !is_enabled && state.read(cx).is_tracking;
        state.update(cx, |state, _| state.is_pinch_enabled = is_enabled);
        if forced_release {
            Self::finish_deferred(&state, &self.on_active_changed, window, cx);
        }

        let now = RiseClock::monotonic();
        let transform = {
            let state = state.read(cx);
            ShortsPinchTransform {
                scale: state.scale.frame_value(now, window),
                anchor: state.anchor,
                translation: state.translation.frame_value(now, window),
            }
        };
        let frame = transform.frame(self.size);
        let content = (self.content)(frame.size, window, cx);

        let measured_state = state.clone();
        let pinch_state = state.clone();
        let pinch_active = self.on_active_changed.clone();
        let recognizes_alongside_all = self.recognizes_alongside_all;
        let disappear_state = state;
        let disappear_active = self.on_active_changed;
        div()
            .id(self.id.clone())
            .relative()
            .w(self.size.width)
            .h(self.size.height)
            .on_pinch(move |event: &PinchEvent, window, cx| {
                Self::handle_pinch(&pinch_state, &pinch_active, recognizes_alongside_all, event, window, cx);
            })
            .child(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(content),
            )
            .child(measure(Self::child_id(&self.id, "bounds"), move |bounds, _, cx| {
                measured_state.update(cx, |state, _| state.bounds = bounds);
            }))
            .on_disappear(Self::child_id(&self.id, "disappear"), move |window, cx| {
                if let Some(window) = window
                    && disappear_state.read(cx).is_tracking
                {
                    Self::finish_deferred(&disappear_state, &disappear_active, window, cx);
                }
            })
    }
}
