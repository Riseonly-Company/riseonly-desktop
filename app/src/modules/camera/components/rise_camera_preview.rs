use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{
    App, Bounds, Context, ElementId, Entity, IntoElement, Pixels, Point, Render, RenderOnce, Size, Subscription, Task,
    WeakEntity, Window, black, div, point, prelude::*, px, size,
};
use rise_media::camera_capture::call_camera::host_nv12_of;
use rise_media::texture::native_surface::{
    PreparedHostFrame, mirror_host_nv12_frame, prepare_host_frame, present_host_frame,
};
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::measure::measure;

use crate::modules::camera::capture::rise_camera_capture::RiseCameraCapture;

static NEXT_FEED_CAMERA_STREAM: AtomicU64 = AtomicU64::new(0x00cd_0000_0000_0000);

/// converts a tap on the preview into the capture device focus space
#[derive(Clone, Default)]
pub struct RiseCameraPreviewBridge {
    view: Rc<RefCell<Option<WeakEntity<PreviewView>>>>,
}

impl RiseCameraPreviewBridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// view_point is in the preview own space, the answer is the unit point of the picture
    pub fn device_point(&self, view_point: Point<Pixels>, cx: &App) -> Option<Point<f32>> {
        let view = self.view.borrow().as_ref()?.upgrade()?;
        view.read(cx).capture_device_point_converted(view_point)
    }

    fn set_view(&self, view: &Entity<PreviewView>) {
        *self.view.borrow_mut() = Some(view.downgrade());
    }
}

/// live preview of the feed camera, fills its frame
#[derive(IntoElement)]
pub struct RiseCameraPreview {
    id: ElementId,
    capture: Entity<RiseCameraCapture>,
    bridge: RiseCameraPreviewBridge,
}

impl RiseCameraPreview {
    pub fn new(id: impl Into<ElementId>, capture: Entity<RiseCameraCapture>, bridge: RiseCameraPreviewBridge) -> Self {
        Self { id: id.into(), capture, bridge }
    }

    fn make_ui_view(&self, window: &mut Window, cx: &mut App) -> Entity<PreviewView> {
        let view = window.use_keyed_state(self.id.clone(), cx, |_, cx| PreviewView::new(cx));
        self.bridge.set_view(&view);
        view
    }

    fn update_ui_view(&self, ui_view: &Entity<PreviewView>, cx: &mut App) {
        self.bridge.set_view(ui_view);
        let capture = self.capture.clone();
        ui_view.update(cx, |view, cx| view.attach(capture, cx));
    }

    fn dismantle_ui_view(ui_view: &Entity<PreviewView>, cx: &mut App) {
        ui_view.update(cx, |view, cx| view.detach(cx));
    }
}

impl RenderOnce for RiseCameraPreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ui_view = self.make_ui_view(window, cx);
        self.update_ui_view(&ui_view, cx);
        let dismantled = ui_view.downgrade();
        div()
            .size_full()
            .bg(black())
            .child(ui_view)
            .on_disappear(ElementId::NamedChild(Arc::new(self.id.clone()), "dismantle".into()), move |_, cx| {
                if let Some(ui_view) = dismantled.upgrade() {
                    Self::dismantle_ui_view(&ui_view, cx);
                }
            })
    }
}

/// preview surface, polls the newest capture frame on every display refresh while the session runs
/// aspect fill over its box
pub struct PreviewView {
    attached_session: Option<Entity<RiseCameraCapture>>,
    session_observation: Option<Subscription>,
    stream_id: u64,
    shown_generation: u64,
    picture: Option<PreparedHostFrame>,
    picture_size: Size<f32>,
    picture_mirrored: bool,
    box_size: Size<f32>,
    prepare_task: Option<Task<()>>,
}

impl PreviewView {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            attached_session: None,
            session_observation: None,
            stream_id: NEXT_FEED_CAMERA_STREAM.fetch_add(1, Ordering::Relaxed),
            shown_generation: 0,
            picture: None,
            picture_size: Size::default(),
            picture_mirrored: false,
            box_size: Size::default(),
            prepare_task: None,
        }
    }

    pub fn attach(&mut self, session: Entity<RiseCameraCapture>, cx: &mut Context<Self>) {
        if self.attached_session.as_ref().map(Entity::entity_id) == Some(session.entity_id()) {
            return;
        }
        self.session_observation = Some(cx.observe(&session, |_, _, cx| cx.notify()));
        self.attached_session = Some(session);
        self.shown_generation = 0;
        cx.notify();
    }

    pub fn detach(&mut self, cx: &mut Context<Self>) {
        if self.attached_session.is_none() {
            return;
        }
        self.attached_session = None;
        self.session_observation = None;
        self.picture = None;
        self.prepare_task = None;
        cx.notify();
    }

    fn capture_device_point_converted(&self, layer_point: Point<Pixels>) -> Option<Point<f32>> {
        let drawn = self.picture_frame();
        if drawn.size.width <= 0.0 || drawn.size.height <= 0.0 {
            return None;
        }
        let x = ((f32::from(layer_point.x) - drawn.origin.x) / drawn.size.width).clamp(0.0, 1.0);
        let y = ((f32::from(layer_point.y) - drawn.origin.y) / drawn.size.height).clamp(0.0, 1.0);
        Some(point(if self.picture_mirrored { 1.0 - x } else { x }, y))
    }

    fn pull_latest_frame(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.attached_session.as_ref() else {
            return;
        };
        let (generation, frame) = session.read(cx).latest_frame();
        if generation == self.shown_generation || self.prepare_task.is_some() {
            return;
        }
        let Some(frame) = frame else {
            return;
        };
        self.shown_generation = generation;
        let stream_id = self.stream_id;
        let mirrored = frame.mirrored;
        let prepared = cx.background_spawn(async move {
            let host = host_nv12_of(&frame, stream_id, generation).ok()?;
            let (width, height) = (host.width, host.height);
            let host = if frame.mirrored { mirror_host_nv12_frame(&host)? } else { host };
            let prepared = prepare_host_frame(host).ok()?;
            Some((prepared, size(width as f32, height as f32)))
        });
        self.prepare_task = Some(cx.spawn(async move |this, cx| {
            let prepared = prepared.await;
            let _ = this.update(cx, |view, cx| {
                view.prepare_task = None;
                if let Some((picture, picture_size)) = prepared
                    && view.attached_session.is_some()
                {
                    view.picture = Some(picture);
                    view.picture_size = picture_size;
                    view.picture_mirrored = mirrored;
                    cx.notify();
                }
            });
        }));
    }

    fn picture_frame(&self) -> Bounds<f32> {
        if self.picture_size.width <= 0.0 || self.picture_size.height <= 0.0 {
            return Bounds { origin: point(0.0, 0.0), size: self.box_size };
        }
        let scale =
            (self.box_size.width / self.picture_size.width).max(self.box_size.height / self.picture_size.height);
        let drawn = size(self.picture_size.width * scale, self.picture_size.height * scale);
        Bounds {
            origin: point((self.box_size.width - drawn.width) / 2.0, (self.box_size.height - drawn.height) / 2.0),
            size: drawn,
        }
    }
}

impl Render for PreviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.pull_latest_frame(cx);
        if self.attached_session.as_ref().is_some_and(|session| session.read(cx).is_session_running()) {
            window.request_animation_frame();
        }
        let picture = self.picture.as_ref().and_then(|picture| present_host_frame(picture).ok());
        let drawn = self.picture_frame();
        let this = cx.weak_entity();
        div()
            .relative()
            .size_full()
            .bg(black())
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
            .child(measure("rise-camera-preview-size", move |bounds, _, cx| {
                let box_size = size(f32::from(bounds.size.width), f32::from(bounds.size.height));
                let _ = this.update(cx, |view, cx| {
                    if view.box_size != box_size {
                        view.box_size = box_size;
                        cx.notify();
                    }
                });
            }))
    }
}
