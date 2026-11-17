use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gpui::{
    AnyElement, App, Bounds, ContentMask, Element, ElementId, Global, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, TransformationMatrix, Window, point,
    px, size,
};

pub type ZoomTransitionNamespace = u64;
/// id of the matched transition source and of the zoom source
pub type ZoomTransitionSourceId = u64;

const MAX_RECORDED_FRAMES: usize = 1024;
const KEPT_RECORDED_FRAMES: u64 = 512;
const ZOOM_FADE_END: f32 = 0.2;

#[derive(Default)]
struct ZoomSourceFrames {
    frames: RefCell<HashMap<(ZoomTransitionNamespace, ZoomTransitionSourceId), (Bounds<Pixels>, u64)>>,
    next_stamp: Cell<u64>,
}

impl Global for ZoomSourceFrames {}

impl ZoomSourceFrames {
    fn record(
        &self,
        namespace: ZoomTransitionNamespace,
        id: ZoomTransitionSourceId,
        bounds: Bounds<Pixels>,
    ) {
        let stamp = self.next_stamp.get();
        self.next_stamp.set(stamp + 1);
        let mut frames = self.frames.borrow_mut();
        frames.insert((namespace, id), (bounds, stamp));
        if frames.len() > MAX_RECORDED_FRAMES {
            let oldest_kept = stamp.saturating_sub(KEPT_RECORDED_FRAMES);
            frames.retain(|_, (_, recorded)| *recorded >= oldest_kept);
        }
    }
}

pub fn zoom_source_frame(
    namespace: ZoomTransitionNamespace,
    id: ZoomTransitionSourceId,
    cx: &App,
) -> Option<Bounds<Pixels>> {
    let frames = cx.try_global::<ZoomSourceFrames>()?.frames.borrow();
    frames.get(&(namespace, id)).map(|(bounds, _)| *bounds)
}

/// lays content out unchanged and records its frame
pub fn matched_transition_source(
    content: impl IntoElement,
    namespace: ZoomTransitionNamespace,
    id: ZoomTransitionSourceId,
) -> MatchedTransitionSource {
    MatchedTransitionSource {
        child: content.into_any_element(),
        namespace,
        id,
    }
}

pub struct MatchedTransitionSource {
    child: AnyElement,
    namespace: ZoomTransitionNamespace,
    id: ZoomTransitionSourceId,
}

impl IntoElement for MatchedTransitionSource {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MatchedTransitionSource {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if cx.try_global::<ZoomSourceFrames>().is_none() {
            cx.set_global(ZoomSourceFrames::default());
        }
        cx.global::<ZoomSourceFrames>()
            .record(self.namespace, self.id, bounds);
        self.child.paint(window, cx);
    }
}

/// where the destination is at one moment of the flight
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomFlightFrame {
    pub clip: Bounds<Pixels>,
    pub scale: f32,
}

fn lerp(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
}

/// flight frame at progress, 0 on the source and 1 on container
/// at 0 the destination covers the source edge to edge so nothing shows outside it
pub fn zoom_flight_frame(
    source: Bounds<Pixels>,
    container: Bounds<Pixels>,
    progress: f32,
) -> ZoomFlightFrame {
    let progress = progress.clamp(0.0, 1.0);
    let container_width = f32::from(container.size.width);
    let container_height = f32::from(container.size.height);
    let start_scale = if container_width > 0.0 && container_height > 0.0 {
        (f32::from(source.size.width) / container_width)
            .max(f32::from(source.size.height) / container_height)
    } else {
        1.0
    };
    let clip = Bounds {
        origin: point(
            px(lerp(
                f32::from(source.origin.x),
                f32::from(container.origin.x),
                progress,
            )),
            px(lerp(
                f32::from(source.origin.y),
                f32::from(container.origin.y),
                progress,
            )),
        ),
        size: size(
            px(lerp(f32::from(source.size.width), container_width, progress)),
            px(lerp(f32::from(source.size.height), container_height, progress)),
        ),
    };
    ZoomFlightFrame {
        clip,
        scale: lerp(start_scale, 1.0, progress),
    }
}

/// matrix in scaled window pixels that puts container_center
/// on the center of frame.clip with frame.scale
pub fn zoom_flight_matrix(
    frame: ZoomFlightFrame,
    container_center: Point<Pixels>,
    scale_factor: f32,
) -> TransformationMatrix {
    TransformationMatrix::unit()
        .translate(frame.clip.center().scale(scale_factor))
        .scale(size(frame.scale, frame.scale))
        .translate(container_center.scale(-scale_factor))
}

pub fn zoom_content_opacity(progress: f32) -> f32 {
    (progress / ZOOM_FADE_END).clamp(0.0, 1.0)
}

/// zoom navigation transition at progress, driven by the navigation stack
pub struct ZoomNavigationTransition {
    child: AnyElement,
    namespace: ZoomTransitionNamespace,
    source_id: ZoomTransitionSourceId,
    progress: f32,
}

impl ZoomNavigationTransition {
    pub fn new(
        content: impl IntoElement,
        namespace: ZoomTransitionNamespace,
        source_id: ZoomTransitionSourceId,
        progress: f32,
    ) -> Self {
        Self {
            child: content.into_any_element(),
            namespace,
            source_id,
            progress,
        }
    }
}

impl IntoElement for ZoomNavigationTransition {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ZoomNavigationTransition {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let source = zoom_source_frame(self.namespace, self.source_id, cx);
        let Some(source) = source.filter(|_| self.progress < 1.0) else {
            self.child.paint(window, cx);
            return;
        };
        let frame = zoom_flight_frame(source, bounds, self.progress);
        let matrix = zoom_flight_matrix(frame, bounds.center(), window.scale_factor());
        let child = &mut self.child;
        window.with_content_mask(
            Some(ContentMask { bounds: frame.clip }),
            |window| window.with_transformation(matrix, |window| child.paint(window, cx)),
        );
    }
}
