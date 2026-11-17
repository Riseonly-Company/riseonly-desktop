use std::rc::{Rc, Weak};
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Pixels, Point, Render, RenderOnce, SharedString, Size, StatefulInteractiveElement, Styled, WeakEntity, Window, black,
    div, point, px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_media::rtc_media::livekit_room::RemoteVideoFrame;
use rise_media::texture::native_surface::present_host_frame_rotated;
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::liquid_chrome::{LiquidChromeModifiers, LiquidGlassContrast};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::modifiers::{ModifierExt, ViewModifier};
use rise_ui::swiftui::motion::MotionSpec;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};

use crate::modules::rtc::engine::rise_rtc_call_trace::RiseRTCCallTrace;
use crate::modules::rtc::media::rise_rtc_media_presentation::{VideoRenderer, VideoTrack};

const WINDOW_CORNER_RADIUS: f32 = 18.0;
const DRAG_MINIMUM_DISTANCE: f32 = 6.0;

pub type RiseRTCFloatingVideoWindowPositionHandler = Rc<dyn Fn(UnitPoint, &mut Window, &mut App)>;
pub type RiseRTCFloatingVideoWindowTapHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy)]
struct RiseRTCFloatingVideoWindowGeometry {
    position: UnitPoint,
    size: Size<f32>,
    insets: EdgeInsets,
    bounds: Size<f32>,
}

impl RiseRTCFloatingVideoWindowGeometry {
    fn movement_area(&self) -> Bounds<f32> {
        let min_x = (self.insets.leading + self.size.width / 2.0).min(self.bounds.width / 2.0);
        let min_y = (self.insets.top + self.size.height / 2.0).min(self.bounds.height / 2.0);
        let max_x = min_x.max(self.bounds.width - self.insets.trailing - self.size.width / 2.0);
        let max_y = min_y.max(self.bounds.height - self.insets.bottom - self.size.height / 2.0);
        Bounds { origin: point(min_x, min_y), size: size(max_x - min_x, max_y - min_y) }
    }

    fn resting_centre(&self) -> Point<f32> {
        let area = self.movement_area();
        point(
            area.origin.x + area.size.width * self.position.x,
            area.origin.y + area.size.height * self.position.y,
        )
    }

    fn clamped_centre(&self, centre: Point<f32>) -> Point<f32> {
        let area = self.movement_area();
        point(
            centre.x.max(area.origin.x).min(area.origin.x + area.size.width),
            centre.y.max(area.origin.y).min(area.origin.y + area.size.height),
        )
    }

    fn dragged_centre(&self, translation: Size<Pixels>) -> Point<f32> {
        let resting = self.resting_centre();
        self.clamped_centre(point(
            resting.x + f32::from(translation.width),
            resting.y + f32::from(translation.height),
        ))
    }

    fn released_position(&self, translation: Size<Pixels>) -> UnitPoint {
        let centre = self.dragged_centre(translation);
        let area = self.movement_area();
        UnitPoint {
            x: if area.size.width > 0.0 { (centre.x - area.origin.x) / area.size.width } else { self.position.x },
            y: if area.size.height > 0.0 { (centre.y - area.origin.y) / area.size.height } else { self.position.y },
        }
    }
}

#[derive(Default)]
struct RiseRTCFloatingVideoWindowDragState {
    drag_translation: Size<Pixels>,
    did_drag: bool,
}

/// freely positioned video window
/// only this subtree updates during a drag, the parent stores the normalized position when it ends
/// parent must be relative and of bounds size
#[derive(IntoElement)]
pub struct RiseRTCFloatingVideoWindow {
    id: ElementId,
    position: UnitPoint,
    size: Size<f32>,
    insets: EdgeInsets,
    bounds: Size<f32>,
    on_position_change: RiseRTCFloatingVideoWindowPositionHandler,
    on_tap: Option<RiseRTCFloatingVideoWindowTapHandler>,
    content: AnyElement,
}

impl RiseRTCFloatingVideoWindow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        position: UnitPoint,
        size: Size<f32>,
        insets: EdgeInsets,
        bounds: Size<f32>,
        on_position_change: impl Fn(UnitPoint, &mut Window, &mut App) + 'static,
        on_tap: Option<RiseRTCFloatingVideoWindowTapHandler>,
        content: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            position,
            size,
            insets,
            bounds,
            on_position_change: Rc::new(on_position_change),
            on_tap,
            content: content.into_any_element(),
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }
}

impl RenderOnce for RiseRTCFloatingVideoWindow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state_id = self.child_id("drag-state");
        let gesture_id = self.child_id("drag");
        let state = window.use_keyed_state(state_id, cx, |_, _| RiseRTCFloatingVideoWindowDragState::default());
        let geometry = RiseRTCFloatingVideoWindowGeometry {
            position: self.position,
            size: self.size,
            insets: self.insets,
            bounds: self.bounds,
        };
        let centre = geometry.dragged_centre(state.read(cx).drag_translation);

        let changed_state = state.clone();
        let ended_state = state.clone();
        let pressed_state = state.clone();
        let tapped_state = state;
        let on_position_change = self.on_position_change;
        let on_tap = self.on_tap;
        let gesture = DragGesture::new(DRAG_MINIMUM_DISTANCE)
            .on_changed(move |value, _, cx| {
                changed_state.update(cx, |state, cx| {
                    state.drag_translation = value.translation;
                    state.did_drag = true;
                    cx.notify();
                });
            })
            .on_ended(move |value, window, cx| {
                let released = geometry.released_position(value.translation);
                on_position_change(released, window, cx);
                ended_state.update(cx, |state, cx| {
                    state.drag_translation = Size::default();
                    cx.notify();
                });
            });

        let shadow = gpui::BoxShadow {
            color: black().opacity(0.32),
            offset: point(px(0.0), px(8.0)),
            blur_radius: px(16.0),
            spread_radius: px(0.0),
            inset: false,
        };

        div()
            .id(self.id)
            .absolute()
            .left(px(centre.x - self.size.width / 2.0))
            .top(px(centre.y - self.size.height / 2.0))
            .w(px(self.size.width))
            .h(px(self.size.height))
            .rounded(px(WINDOW_CORNER_RADIUS))
            .bg(black())
            .shadow(vec![shadow])
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                pressed_state.update(cx, |state, _| state.did_drag = false);
            })
            .on_click(move |_: &ClickEvent, window, cx| {
                if tapped_state.read(cx).did_drag {
                    return;
                }
                if let Some(on_tap) = &on_tap {
                    on_tap(window, cx);
                }
            })
            .child(
                div()
                    .w(px(self.size.width))
                    .h(px(self.size.height))
                    .clip_shape(Shape::continuous(WINDOW_CORNER_RADIUS))
                    .child(self.content)
                    .gesture(gesture_id, gesture),
            )
    }
}

struct RiseRTCCallChrome {
    on_live_surface: bool,
    stroke_opacity: f64,
}

impl ViewModifier for RiseRTCCallChrome {
    fn body(self, content: AnyElement, _window: &mut Window, _cx: &mut App) -> AnyElement {
        if self.on_live_surface {
            div()
                .flex()
                .relative()
                .rounded_full()
                .bg(black().opacity(0.42))
                .child(content)
                .child(centred_stroke(white().opacity(self.stroke_opacity as f32), 1.0, Shape::Capsule))
                .into_any_element()
        } else {
            content
                .liquid_chrome_capsule_with(self.stroke_opacity, true, None, false)
                .liquid_glass_contrast(LiquidGlassContrast::Media)
        }
    }
}

struct RiseRTCChromeVisibilityState {
    progress: Animated<f32>,
}

struct RiseRTCChromeVisibility {
    id: ElementId,
    hidden: bool,
    offset: f32,
    dissolves: bool,
    animation: Option<MotionSpec>,
}

impl ViewModifier for RiseRTCChromeVisibility {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        let hidden = self.hidden;
        let target = if hidden { 1.0 } else { 0.0 };
        let state = window.use_keyed_state(self.id, cx, move |_, _| RiseRTCChromeVisibilityState {
            progress: Animated::new(target),
        });
        let now = RiseClock::monotonic();
        let animation = self.animation;
        state.update(cx, |state, cx| state.progress.set_in(target, animation, now, cx));
        let progress = state.read(cx).progress.frame_value(now, window);
        let scale = if self.dissolves { 1.0 + (0.94 - 1.0) * progress } else { 1.0 };

        div()
            .relative()
            .top(px(self.offset * progress))
            .opacity(1.0 - progress)
            .when(hidden && progress >= 1.0, |chrome| chrome.invisible())
            .child(content)
            .when(hidden && progress < 1.0, |chrome| chrome.child(div().absolute().inset_0().occlude()))
            .scale_effect(scale)
            .into_any_element()
    }
}

pub trait RiseRTCFloatingVideoWindowModifiers: IntoElement + Sized {
    fn call_chrome(self, on_live_surface: bool) -> AnyElement {
        self.call_chrome_with(on_live_surface, 0.12)
    }

    /// call chrome styling, on live surface or not with a stroke opacity
    fn call_chrome_with(self, on_live_surface: bool, stroke_opacity: f64) -> AnyElement {
        self.modifier(RiseRTCCallChrome { on_live_surface, stroke_opacity }).into_any_element()
    }

    /// call chrome visibility with hidden, offset and dissolve
    /// animation is the callers spec and id keeps the progress between frames
    fn call_chrome_visibility(
        self,
        id: impl Into<ElementId>,
        hidden: bool,
        offset: f32,
        dissolves: bool,
        animation: impl Into<Option<MotionSpec>>,
    ) -> AnyElement {
        self.modifier(RiseRTCChromeVisibility {
            id: id.into(),
            hidden,
            offset,
            dissolves,
            animation: animation.into(),
        })
        .into_any_element()
    }
}

impl<E: IntoElement> RiseRTCFloatingVideoWindowModifiers for E {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoViewLayoutMode {
    Fit,
    Fill,
}

/// video view mirror mode
/// desktop surface cant flip, so local camera auto is handled where preview frames are made
/// auto only for own camera, off for remote and screen shares
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoViewMirrorMode {
    Auto,
    Off,
    Mirror,
}

struct RiseRTCVideoSurfaceRenderer {
    view: WeakEntity<RiseRTCVideoSurfaceView>,
}

impl VideoRenderer for RiseRTCVideoSurfaceRenderer {
    fn render(&self, frame: &RemoteVideoFrame, cx: &mut App) {
        let _ = self.view.update(cx, |view, cx| {
            view.frame = Some(frame.clone());
            cx.notify();
        });
    }
}

/// renderer of one video surface, flat or inside the zoom container
pub struct RiseRTCVideoSurfaceView {
    attached_track: Weak<VideoTrack>,
    renderer: Option<Rc<dyn VideoRenderer>>,
    frame: Option<RemoteVideoFrame>,
    layout_mode: VideoViewLayoutMode,
    mirror_mode: VideoViewMirrorMode,
    box_size: Size<f32>,
}

impl RiseRTCVideoSurfaceView {
    pub fn new() -> Self {
        Self {
            attached_track: Weak::new(),
            renderer: None,
            frame: None,
            layout_mode: VideoViewLayoutMode::Fill,
            mirror_mode: VideoViewMirrorMode::Off,
            box_size: Size::default(),
        }
    }

    pub fn mirror_mode(&self) -> VideoViewMirrorMode {
        self.mirror_mode
    }

    /// true when track is the one drawn now
    pub fn is_attached(&self, track: &Rc<VideoTrack>) -> bool {
        self.attached_track.upgrade().is_some_and(|attached| Rc::ptr_eq(&attached, track))
    }

    pub fn configure(
        &mut self,
        track: &Rc<VideoTrack>,
        layout_mode: VideoViewLayoutMode,
        mirror_mode: VideoViewMirrorMode,
        cx: &mut Context<Self>,
    ) {
        self.layout_mode = layout_mode;
        self.mirror_mode = mirror_mode;
        if self.is_attached(track) {
            return;
        }
        self.detach();
        let renderer: Rc<dyn VideoRenderer> = Rc::new(RiseRTCVideoSurfaceRenderer { view: cx.weak_entity() });
        track.add_video_renderer(renderer.clone());
        self.renderer = Some(renderer);
        self.attached_track = Rc::downgrade(track);
        self.frame = track.latest_frame();
    }

    fn detach(&mut self) {
        if let (Some(track), Some(renderer)) = (self.attached_track.upgrade(), self.renderer.as_ref()) {
            track.remove_video_renderer(renderer);
        }
        self.renderer = None;
        self.attached_track = Weak::new();
        self.frame = None;
    }

    fn set_box_size(&mut self, box_size: Size<f32>, cx: &mut Context<Self>) {
        if self.box_size == box_size {
            return;
        }
        self.box_size = box_size;
        cx.notify();
    }

    fn picture_frame(&self) -> Bounds<f32> {
        let whole = Bounds { origin: point(0.0, 0.0), size: self.box_size };
        let dimensions = self
            .attached_track
            .upgrade()
            .and_then(|track| track.dimensions())
            .map(|dimensions| (dimensions.width, dimensions.height))
            .filter(|(width, height)| *width > 0 && *height > 0)
            .or_else(|| {
                self.frame.as_ref().map(|frame| {
                    let (width, height) = frame.frame.size();
                    if frame.rotation_degrees % 180 == 90 { (height, width) } else { (width, height) }
                })
            });
        match (self.layout_mode, dimensions) {
            (VideoViewLayoutMode::Fill, Some((width, height))) if width > 0 && height > 0 => {
                let width = width as f32;
                let height = height as f32;
                let scale = (self.box_size.width / width).max(self.box_size.height / height);
                let drawn = size(width * scale, height * scale);
                Bounds {
                    origin: point((self.box_size.width - drawn.width) / 2.0, (self.box_size.height - drawn.height) / 2.0),
                    size: drawn,
                }
            }
            _ => whole,
        }
    }
}

impl Default for RiseRTCVideoSurfaceView {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RiseRTCVideoSurfaceView {
    fn drop(&mut self) {
        self.detach();
    }
}

impl Render for RiseRTCVideoSurfaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let picture = self
            .frame
            .as_ref()
            .and_then(|frame| present_host_frame_rotated(&frame.frame, frame.rotation_degrees).ok());
        let drawn = self.picture_frame();
        let this = cx.weak_entity();

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .left(px(drawn.origin.x))
                    .top(px(drawn.origin.y))
                    .w(px(drawn.size.width))
                    .h(px(drawn.size.height))
                    .grid()
                    .grid_cols(1)
                    .grid_rows(1)
                    .children(picture),
            )
            .child(measure("rtc-video-surface-size", move |bounds, _, cx| {
                let box_size = size(f32::from(bounds.size.width), f32::from(bounds.size.height));
                let _ = this.update(cx, |view, cx| view.set_box_size(box_size, cx));
            }))
    }
}

/// re evaluates only on real video changes, configure compares the track and both modes
/// pictures arrive through the renderer without running any parent
/// takes no clicks
#[derive(IntoElement)]
pub struct RiseRTCVideoSurface {
    id: ElementId,
    role: SharedString,
    track: Rc<VideoTrack>,
    layout_mode: VideoViewLayoutMode,
    mirror_mode: VideoViewMirrorMode,
}

impl RiseRTCVideoSurface {
    pub fn new(
        id: impl Into<ElementId>,
        role: impl Into<SharedString>,
        track: Rc<VideoTrack>,
        layout_mode: VideoViewLayoutMode,
        mirror_mode: VideoViewMirrorMode,
    ) -> Self {
        Self { id: id.into(), role: role.into(), track, layout_mode, mirror_mode }
    }
}

impl RenderOnce for RiseRTCVideoSurface {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        RiseRTCCallTrace::note_video_pass(&self.role);
        let view = window.use_keyed_state(self.id, cx, |_, _| RiseRTCVideoSurfaceView::new());
        view.update(cx, |view, cx| view.configure(&self.track, self.layout_mode, self.mirror_mode, cx));
        div().size_full().child(view)
    }
}
