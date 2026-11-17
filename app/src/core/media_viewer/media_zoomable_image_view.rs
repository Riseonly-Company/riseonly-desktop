use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, Bounds, ClickEvent, Context, ElementId, Entity, ImageSource, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, PinchEvent,
    Pixels, Point, Render, RenderOnce, ScrollWheelEvent, Size, StatefulInteractiveElement, Styled, StyledImage, Task,
    TouchPhase, Window, div, img, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change, use_task};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;

/// called with true when the picture is magnified past rest
pub type MediaZoomChanged = Rc<dyn Fn(bool, &mut App)>;
/// a single tap that was not the first half of a double tap
pub type MediaZoomSingleTap = Rc<dyn Fn(&mut Window, &mut App)>;
/// builds the view the container magnifies or keeps centered over it
pub type MediaZoomViewBuilder = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// one viewer page, picture from the shared cache and zoomable once its there
#[derive(IntoElement)]
pub struct MediaZoomablePage {
    pub id: ElementId,
    pub source: String,
    pub seed_image: Option<CleverImage>,
    pub is_current: bool,
    pub persist_to_disk: bool,
    pub cache_policy: CleverImageCachePolicy,
    pub on_zoom_changed: MediaZoomChanged,
    pub on_single_tap: MediaZoomSingleTap,
}

#[derive(Default)]
struct MediaZoomablePageState {
    ui_image: Option<CleverImage>,
}

impl MediaZoomablePage {
    fn adopt_seed_if_needed(
        state: &Entity<MediaZoomablePageState>,
        seed_image: Option<&CleverImage>,
        source: &str,
        cx: &mut App,
    ) {
        if state.read(cx).ui_image.is_some() {
            return;
        }
        let adopted = seed_image.cloned().or_else(|| {
            MediaViewerImageMemoryCache::memory_only(source, SaiAccountCacheScope::active_user_id().as_deref())
        });
        if let Some(adopted) = adopted {
            state.update(cx, |state, cx| {
                state.ui_image = Some(adopted);
                cx.notify();
            });
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }
}

impl RenderOnce for MediaZoomablePage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("media-zoomable-page"), cx, |_, _| {
            MediaZoomablePageState::default()
        });

        {
            let state = state.clone();
            let seed_image = self.seed_image.clone();
            let source = self.source.clone();
            let persist_to_disk = self.persist_to_disk;
            let cache_policy = self.cache_policy;
            use_task(
                self.child_id("media-zoomable-page-load"),
                self.source.clone(),
                window,
                cx,
                async move |cx| {
                    let _ = cx.update(|_, cx| Self::adopt_seed_if_needed(&state, seed_image.as_ref(), &source, cx));
                    let loaded =
                        MediaViewerBitmapLoader::load_into_memory(&source, true, persist_to_disk, cache_policy, cx)
                            .await;
                    state.update(cx, |state, cx| {
                        let full = loaded.or_else(|| {
                            MediaViewerImageMemoryCache::memory_only(
                                &source,
                                SaiAccountCacheScope::active_user_id().as_deref(),
                            )
                        });
                        if let Some(full) = full {
                            state.ui_image = Some(full);
                            cx.notify();
                        }
                    });
                },
            );
        }

        {
            let on_zoom_changed = Rc::clone(&self.on_zoom_changed);
            on_change(
                self.child_id("media-zoomable-page-current"),
                self.is_current,
                false,
                window,
                cx,
                move |_, current, _, cx| {
                    if !*current {
                        on_zoom_changed(false, cx);
                    }
                },
            );
        }

        let appear_id = self.child_id("media-zoomable-page-appear");
        let body = match state.read(cx).ui_image.clone() {
            Some(ui_image) => MediaZoomScrollView {
                id: self.child_id("media-zoom-scroll-view"),
                image: ui_image,
                is_current: self.is_current,
                max_zoom: MediaZoomScrollView::DEFAULT_MAX_ZOOM,
                on_zoom_changed: Rc::clone(&self.on_zoom_changed),
                on_single_tap: Rc::clone(&self.on_single_tap),
            }
            .into_any_element(),
            None => div().size_full().into_any_element(),
        };
        let seed_image = self.seed_image;
        let source = self.source;
        div().size_full().child(body).on_appear(appear_id, move |_, cx| {
            Self::adopt_seed_if_needed(&state, seed_image.as_ref(), &source, cx);
        })
    }
}

/// host of the zoom container, made once per id and updated each frame
#[derive(IntoElement)]
pub struct MediaZoomScrollView {
    pub id: ElementId,
    pub image: CleverImage,
    pub is_current: bool,
    pub max_zoom: f32,
    pub on_zoom_changed: MediaZoomChanged,
    pub on_single_tap: MediaZoomSingleTap,
}

impl MediaZoomScrollView {
    pub const DEFAULT_MAX_ZOOM: f32 = 4.0;
}

impl MediaZoomScrollView {
    fn make_ui_view(id: ElementId, window: &mut Window, cx: &mut App) -> Entity<MediaZoomImageContainerView> {
        window.use_keyed_state(id, cx, |_, cx| MediaZoomImageContainerView::new(cx))
    }

    fn update_ui_view(self, ui_view: &Entity<MediaZoomImageContainerView>, cx: &mut App) {
        ui_view.update(cx, |ui_view, cx| {
            let container = ui_view.container();
            container.update(cx, |container, _| {
                container.max_zoom = self.max_zoom;
                container.on_zoom_changed = Some(Rc::clone(&self.on_zoom_changed));
                container.on_single_tap = Some(Rc::clone(&self.on_single_tap));
            });
            ui_view.set_image(self.image, cx);
            if !self.is_current {
                container.update(cx, |container, cx| container.reset_zoom(false, cx));
            }
        });
    }
}

impl RenderOnce for MediaZoomScrollView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ui_view = Self::make_ui_view(self.id.clone(), window, cx);
        self.update_ui_view(&ui_view, cx);
        div().size_full().child(ui_view)
    }
}

/// zoom container drawing one picture, aspect fit at rest
pub struct MediaZoomImageContainerView {
    container: Entity<MediaZoomContainerView>,
    current_image: Option<CleverImage>,
}

impl MediaZoomImageContainerView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let image_view = cx.weak_entity();
        let zoomed_view: MediaZoomViewBuilder = Rc::new(move |_: &mut Window, cx: &mut App| {
            match image_view.upgrade().and_then(|view| view.read(cx).current_image.clone()) {
                Some(image) => img(ImageSource::Render(image))
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                None => div().into_any_element(),
            }
        });
        Self {
            container: cx.new(|_| MediaZoomContainerView::new(zoomed_view)),
            current_image: None,
        }
    }

    pub fn container(&self) -> Entity<MediaZoomContainerView> {
        self.container.clone()
    }

    pub fn set_image(&mut self, image: CleverImage, cx: &mut Context<Self>) {
        if self.current_image.as_ref().is_some_and(|current| Arc::ptr_eq(current, &image)) {
            return;
        }
        let size = image.size(0);
        let natural_size = Size {
            width: size.width.0 as f32,
            height: size.height.0 as f32,
        };
        self.current_image = Some(image);
        self.container
            .update(cx, |container, cx| container.set_content_geometry(natural_size, false, cx));
        cx.notify();
    }
}

impl Render for MediaZoomImageContainerView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.container.clone())
    }
}

/// pinch and double click and zoomed pan around one view
/// at rest it sits on the aspect fit rect or the whole viewport if media fills it
/// wheel and drags go to the pager at rest, ctrl or cmd wheel zooms at the pointer
pub struct MediaZoomContainerView {
    zoomed_view: MediaZoomViewBuilder,
    viewport_control: Option<MediaZoomViewBuilder>,
    natural_size: Size<f32>,
    fills_bounds: bool,
    last_zoomed_flag: bool,
    fitted_bounds_size: Size<f32>,
    is_resetting_programmatically: bool,
    pub max_zoom: f32,
    pub on_zoom_changed: Option<MediaZoomChanged>,
    pub on_single_tap: Option<MediaZoomSingleTap>,
    viewport: Bounds<Pixels>,
    rest_size: Size<f32>,
    zoom_scale: Animated<f32>,
    content_x: Animated<f32>,
    content_y: Animated<f32>,
    drag_last: Option<Point<Pixels>>,
    drag_moved: bool,
    pending_single_tap: Option<Task<()>>,
}

impl MediaZoomContainerView {
    const ZOOMED_THRESHOLD: f32 = 1.01;
    const REST_THRESHOLD: f32 = 1.0001;
    const DOUBLE_TAP_ZOOM: f32 = 2.6;
    const SINGLE_TAP_FAILURE_DELAY: Duration = Duration::from_millis(350);
    const TAP_SLOP: f32 = 4.0;
    const WHEEL_ZOOM_PIXELS_PER_E: f32 = 240.0;
    const WHEEL_LINE_HEIGHT: Pixels = px(20.0);

    pub fn new(zoomed_view: MediaZoomViewBuilder) -> Self {
        Self {
            zoomed_view,
            viewport_control: None,
            natural_size: Size::default(),
            fills_bounds: false,
            last_zoomed_flag: false,
            fitted_bounds_size: Size::default(),
            is_resetting_programmatically: false,
            max_zoom: MediaZoomScrollView::DEFAULT_MAX_ZOOM,
            on_zoom_changed: None,
            on_single_tap: None,
            viewport: Bounds::default(),
            rest_size: Size::default(),
            zoom_scale: Animated::new(1.0),
            content_x: Animated::new(0.0),
            content_y: Animated::new(0.0),
            drag_last: None,
            drag_moved: false,
            pending_single_tap: None,
        }
    }

    pub fn install_viewport_control(&mut self, control: MediaZoomViewBuilder, cx: &mut Context<Self>) {
        self.viewport_control = Some(control);
        cx.notify();
    }

    /// a new shape starts from rest, old zoom and offset
    /// would drop the user in a corner of an unseen picture
    pub fn set_content_geometry(&mut self, natural_size: Size<f32>, fills_bounds: bool, cx: &mut Context<Self>) {
        self.natural_size = natural_size;
        self.fills_bounds = fills_bounds;
        self.reset_zoom_programmatically(false, cx);
        self.layout_subviews(cx);
        cx.notify();
    }

    pub fn reset_zoom(&mut self, animated: bool, cx: &mut Context<Self>) {
        if *self.zoom_scale.target() <= Self::REST_THRESHOLD {
            return;
        }
        self.reset_zoom_programmatically(animated, cx);
    }

    fn reset_zoom_programmatically(&mut self, animated: bool, cx: &mut Context<Self>) {
        self.is_resetting_programmatically = true;
        let rest = self.clamped_origin(1.0, Point::default());
        self.set_zoom(1.0, rest, animated, cx);
        self.is_resetting_programmatically = false;
    }

    fn viewport_size(&self) -> Size<f32> {
        Size {
            width: f32::from(self.viewport.size.width),
            height: f32::from(self.viewport.size.height),
        }
    }

    fn set_viewport(&mut self, viewport: Bounds<Pixels>, cx: &mut Context<Self>) {
        if self.viewport.size == viewport.size {
            self.viewport.origin = viewport.origin;
            return;
        }
        self.viewport = viewport;
        self.layout_subviews(cx);
        cx.notify();
    }

    fn layout_subviews(&mut self, cx: &mut Context<Self>) {
        let bounds = self.viewport_size();
        if bounds.width <= 1.0
            || bounds.height <= 1.0
            || !(self.fills_bounds || (self.natural_size.width > 1.0 && self.natural_size.height > 1.0))
        {
            return;
        }
        if bounds != self.fitted_bounds_size && *self.zoom_scale.target() > Self::REST_THRESHOLD {
            self.reset_zoom_programmatically(false, cx);
        }
        if *self.zoom_scale.target() <= Self::REST_THRESHOLD {
            self.fitted_bounds_size = bounds;
            self.rest_size = if self.fills_bounds {
                bounds
            } else {
                Self::aspect_fit(self.natural_size, bounds)
            };
            let origin = self.clamped_origin(1.0, Point::default());
            self.content_x.jump(origin.x);
            self.content_y.jump(origin.y);
        }
    }

    fn aspect_fit(size: Size<f32>, in_box: Size<f32>) -> Size<f32> {
        if size.width <= 0.0 || size.height <= 0.0 || in_box.width <= 0.0 || in_box.height <= 0.0 {
            return in_box;
        }
        let scale = (in_box.width / size.width).min(in_box.height / size.height);
        Size {
            width: size.width * scale,
            height: size.height * scale,
        }
    }

    /// keeps content centered when smaller than the viewport and never past its edges when larger
    fn clamped_origin(&self, scale: f32, origin: Point<f32>) -> Point<f32> {
        let bounds = self.viewport_size();
        let axis = |bound: f32, content: f32, value: f32| {
            if content <= bound {
                (bound - content) / 2.0
            } else {
                value.clamp(bound - content, 0.0)
            }
        };
        Point {
            x: axis(bounds.width, self.rest_size.width * scale, origin.x),
            y: axis(bounds.height, self.rest_size.height * scale, origin.y),
        }
    }

    fn zoom_motion() -> MotionSpec {
        ease_in_out(0.3)
    }

    fn set_zoom(&mut self, scale: f32, origin: Point<f32>, animated: bool, cx: &mut Context<Self>) {
        let scale = scale.clamp(1.0, self.max_zoom.max(1.0));
        let origin = self.clamped_origin(scale, origin);
        if animated {
            let now = RiseClock::monotonic();
            let reduce = reduce_motion(cx);
            self.zoom_scale.set(scale, Self::zoom_motion(), now, reduce);
            self.content_x.set(origin.x, Self::zoom_motion(), now, reduce);
            self.content_y.set(origin.y, Self::zoom_motion(), now, reduce);
        } else {
            self.zoom_scale.jump(scale);
            self.content_x.jump(origin.x);
            self.content_y.jump(origin.y);
        }
        self.scroll_view_did_zoom(cx);
        cx.notify();
    }

    fn view_for_zooming(&self) -> MediaZoomViewBuilder {
        Rc::clone(&self.zoomed_view)
    }

    fn scroll_view_did_zoom(&mut self, cx: &mut Context<Self>) {
        self.report_zoom(cx);
    }

    fn scroll_view_did_end_zooming(&mut self, cx: &mut Context<Self>) {
        self.report_zoom(cx);
    }

    /// scales around an anchor in the viewport, the content point under it stays put
    fn zoom_around(&mut self, scale: f32, anchor: Point<f32>, cx: &mut Context<Self>) {
        let current = *self.zoom_scale.target();
        let scale = scale.clamp(1.0, self.max_zoom.max(1.0));
        if current <= 0.0 {
            return;
        }
        let ratio = scale / current;
        let origin = Point {
            x: anchor.x - (anchor.x - *self.content_x.target()) * ratio,
            y: anchor.y - (anchor.y - *self.content_y.target()) * ratio,
        };
        self.set_zoom(scale, origin, false, cx);
    }

    fn pan_by(&mut self, delta: Point<f32>, cx: &mut Context<Self>) {
        let origin = Point {
            x: *self.content_x.target() + delta.x,
            y: *self.content_y.target() + delta.y,
        };
        let scale = *self.zoom_scale.target();
        let origin = self.clamped_origin(scale, origin);
        self.content_x.jump(origin.x);
        self.content_y.jump(origin.y);
        cx.notify();
    }

    fn is_zoomed(&self) -> bool {
        *self.zoom_scale.target() > Self::ZOOMED_THRESHOLD
    }

    fn report_zoom(&mut self, cx: &mut Context<Self>) {
        let zoomed = self.is_zoomed();
        if zoomed == self.last_zoomed_flag {
            return;
        }
        self.last_zoomed_flag = zoomed;
        let Some(on_zoom_changed) = self.on_zoom_changed.clone() else {
            return;
        };
        if !self.is_resetting_programmatically {
            on_zoom_changed(zoomed, cx);
            return;
        }
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let still_current = this
                .upgrade()
                .is_some_and(|this| this.read(cx).last_zoomed_flag == zoomed);
            if still_current {
                on_zoom_changed(zoomed, cx);
            }
        });
    }

    fn local_point(&self, position: Point<Pixels>) -> Point<f32> {
        Point {
            x: f32::from(position.x - self.viewport.origin.x),
            y: f32::from(position.y - self.viewport.origin.y),
        }
    }

    fn handle_pinch(&mut self, event: &PinchEvent, cx: &mut Context<Self>) {
        let scale = *self.zoom_scale.target() * (1.0 + event.delta);
        let anchor = self.local_point(event.position);
        self.zoom_around(scale, anchor, cx);
        if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.scroll_view_did_end_zooming(cx);
        }
        cx.stop_propagation();
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(Self::WHEEL_LINE_HEIGHT);
        if event.modifiers.control || event.modifiers.platform {
            let factor = (f32::from(delta.y) / Self::WHEEL_ZOOM_PIXELS_PER_E).exp();
            let anchor = self.local_point(event.position);
            self.zoom_around(*self.zoom_scale.target() * factor, anchor, cx);
            cx.stop_propagation();
            return;
        }
        if !self.is_zoomed() {
            return;
        }
        self.pan_by(
            Point {
                x: f32::from(delta.x),
                y: f32::from(delta.y),
            },
            cx,
        );
        cx.stop_propagation();
    }

    fn handle_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.drag_moved = false;
        self.drag_last = self.is_zoomed().then_some(event.position);
        if self.drag_last.is_some() {
            cx.stop_propagation();
        }
    }

    fn handle_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(last) = self.drag_last else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag_last = None;
            return;
        }
        let delta = Point {
            x: f32::from(event.position.x - last.x),
            y: f32::from(event.position.y - last.y),
        };
        if delta.x.abs() > Self::TAP_SLOP || delta.y.abs() > Self::TAP_SLOP || self.drag_moved {
            self.drag_moved = true;
            self.drag_last = Some(event.position);
            self.pan_by(delta, cx);
        }
        cx.stop_propagation();
    }

    fn handle_mouse_up(&mut self, _event: &MouseUpEvent, _cx: &mut Context<Self>) {
        self.drag_last = None;
    }

    fn handle_click(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.drag_moved {
            self.drag_moved = false;
            return;
        }
        if let ClickEvent::Mouse(mouse) = event {
            let dx = f32::from(mouse.up.position.x - mouse.down.position.x);
            let dy = f32::from(mouse.up.position.y - mouse.down.position.y);
            if dx.abs() > Self::TAP_SLOP || dy.abs() > Self::TAP_SLOP {
                return;
            }
        }
        if event.click_count() >= 2 {
            self.pending_single_tap = None;
            self.handle_double_tap(event.position(), cx);
            return;
        }
        self.pending_single_tap = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Self::SINGLE_TAP_FAILURE_DELAY).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending_single_tap = None;
                if let Some(on_single_tap) = this.on_single_tap.clone() {
                    on_single_tap(window, cx);
                }
            });
        }));
    }

    fn handle_double_tap(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.is_zoomed() {
            let rest = self.clamped_origin(1.0, Point::default());
            self.set_zoom(1.0, rest, true, cx);
            return;
        }
        let target = self.max_zoom.min(Self::DOUBLE_TAP_ZOOM);
        let scale = *self.zoom_scale.target();
        let point = self.local_point(position);
        let location = Point {
            x: (point.x - *self.content_x.target()) / scale,
            y: (point.y - *self.content_y.target()) / scale,
        };
        let bounds = self.viewport_size();
        let origin = Point {
            x: bounds.width / 2.0 - location.x * target,
            y: bounds.height / 2.0 - location.y * target,
        };
        self.set_zoom(target, origin, true, cx);
    }
}

impl Render for MediaZoomContainerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let scale = self.zoom_scale.frame_value(now, window);
        let x = self.content_x.frame_value(now, window);
        let y = self.content_y.frame_value(now, window);
        let zoomed_view = self.view_for_zooming();
        let zoomed = zoomed_view(window, cx);
        let control = self.viewport_control.clone().map(|control| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(div().block_mouse_except_scroll().child(control(window, cx)))
        });
        let this = cx.weak_entity();

        div()
            .id("media-zoom-container")
            .relative()
            .size_full()
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(self.rest_size.width * scale))
                    .h(px(self.rest_size.height * scale))
                    .overflow_hidden()
                    .child(zoomed),
            )
            .children(control)
            .child(measure("media-zoom-viewport", move |bounds, _, cx| {
                let _ = this.update(cx, |this, cx| this.set_viewport(bounds, cx));
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| this.handle_pinch(event, cx)))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| this.handle_scroll_wheel(event, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| this.handle_mouse_down(event, cx)),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| this.handle_mouse_move(event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.handle_mouse_up(event, cx)),
            )
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| this.handle_click(event, window, cx)))
    }
}
