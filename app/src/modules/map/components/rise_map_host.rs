use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::StyledImage;
use gpui::{
    AnyElement, AnyView, App, Bounds, ClickEvent, Context, Element, ElementId, Entity, EntityId, FocusHandle,
    GlobalElementId, Hsla, InspectorElementId, InteractiveElement, IntoElement, KeyDownEvent, LayoutId, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ObjectFit, ParentElement, PinchEvent, Pixels, RenderImage, Rgba, ScrollDelta, ScrollWheelEvent,
    SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Task, Window, canvas, div, img, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_media::map_view::{
    Annotation, Camera, CameraAnimation, CameraChangeReason, CameraCommand, CameraDriver, DragButton, InputModifiers,
    LatLng, MAX_ZOOM, MIN_ZOOM, MapEvent, MapHandle, MapInput, MapInputController, MapKey, MapOptions, MapRequest,
    MapUnavailable, Projection, RegionEvent, ScreenPoint, ScreenRect, Viewport, WheelDelta, open_map,
};
use rise_theme::RiseThemeAppearance;
use rise_theme::ui_color_system::UIColor;
use serde_json::Value;

use crate::modules::map::components::globe::map_globe_camera_link::MapGlobeCameraLink;
use crate::modules::map::components::location_map_hooks::{location_app_paths, location_map_image};
use crate::modules::map::components::map_zoom_buttons::MapZoomButtons;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;

const CLICK_SLOP: f64 = 4.0;

/// anything with a coordinate the map shows a view for
/// layers downcast through as_any
pub trait MapAnnotation: Any {
    fn coordinate(&self) -> LatLng;
    fn as_any(&self) -> &dyn Any;
}

/// identity of an annotation, compared like object identity
pub fn annotation_key(annotation: &Rc<dyn MapAnnotation>) -> usize {
    Rc::as_ptr(annotation) as *const () as usize
}

/// the gpui view drawn on the annotation's coordinate
/// frame size and the fraction of it that sits on the coordinate
#[derive(Clone)]
pub struct MapAnnotationView {
    pub view: AnyView,
    pub reuse_identifier: SharedString,
    pub width: f64,
    pub height: f64,
    pub anchor: (f64, f64),
    pub z_position: f64,
}

impl MapAnnotationView {
    pub fn new(view: AnyView, reuse_identifier: impl Into<SharedString>, width: f64, height: f64) -> Self {
        Self {
            view,
            reuse_identifier: reuse_identifier.into(),
            width,
            height,
            anchor: (0.5, 0.5),
            z_position: 0.0,
        }
    }
}

struct AnnotationSlot {
    annotation: Rc<dyn MapAnnotation>,
    view: Option<MapAnnotationView>,
}

/// a view added over the map, drawn over the whole map in order
struct MapSubview {
    view: AnyView,
    is_hidden: bool,
}

/// a style layer added at runtime, its id and the source it draws and its json
struct StyleLayerEntry {
    id: String,
    source: Option<String>,
}

/// map view delegate
/// every callback gets the map that raised it
pub trait RiseMapViewDelegate: 'static {
    fn region_did_change_animated(&mut self, _map: &RiseMapHost, _animated: bool, _window: &mut Window, _cx: &mut App) {}
    /// runtime sources and layers go in again after loading finishes
    fn did_finish_loading(&mut self, _map: &RiseMapHost, _window: &mut Window, _cx: &mut App) {}
    fn region_is_changing_with(
        &mut self,
        _map: &RiseMapHost,
        _reason: CameraChangeReason,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
    fn view_for(
        &mut self,
        _map: &RiseMapHost,
        _annotation: &Rc<dyn MapAnnotation>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<MapAnnotationView> {
        None
    }
    fn did_select(&mut self, _map: &RiseMapHost, _annotation: &Rc<dyn MapAnnotation>, _window: &mut Window, _cx: &mut App) {}
    fn did_click(&mut self, _map: &RiseMapHost, _point: ScreenPoint, _window: &mut Window, _cx: &mut App) {}
    fn did_secondary_click(&mut self, _map: &RiseMapHost, _point: ScreenPoint, _window: &mut Window, _cx: &mut App) {}
    fn dismantle(&mut self, _map: &RiseMapHost, _cx: &mut App) {}
}

struct RiseMapHostState {
    entity: Option<EntityId>,
    handle: Option<MapHandle>,
    driver: CameraDriver,
    input: MapInputController,
    viewport: Option<Viewport>,
    unplaced_center: Option<LatLng>,
    origin: gpui::Point<Pixels>,
    image: Option<Arc<RenderImage>>,
    image_camera: Option<Camera>,
    style_json: String,
    appearance: RiseThemeAppearance,
    is_style_loaded: bool,
    sources: HashMap<String, Value>,
    layers: Vec<StyleLayerEntry>,
    images: Vec<String>,
    annotations: Vec<AnnotationSlot>,
    reusable: HashMap<SharedString, Vec<MapAnnotationView>>,
    subviews: Vec<MapSubview>,
    bottom_inset: f64,
    maximum_zoom_level: f64,
    minimum_zoom_level: f64,
    is_scroll_enabled: bool,
    is_zoom_enabled: bool,
    is_rotate_enabled: bool,
    is_pitch_enabled: bool,
    shows_compass_view: bool,
    is_right_to_left: bool,
    pending_regions: Vec<RegionEvent>,
    pending_style_loaded: bool,
    wants_animation_frame: bool,
    first_symbol_layer: Option<String>,
    pressed_at: Option<(ScreenPoint, MouseButton)>,
    moved_since_press: bool,
    unavailable: Option<MapUnavailable>,
    pin_paint: PinPaint,
}

/// where the last render painted the pins
/// spots of those fully inside the clip and keys of those fully outside
/// and whether any straddled the edge
#[derive(Default)]
struct PinPaint {
    inside: HashMap<usize, (f64, f64)>,
    outside: HashSet<usize>,
    straddles: bool,
}

/// where a pin frame sits against the pin clip
enum PinPlace {
    Inside,
    Outside,
    Straddles,
}

fn pin_place(x: f64, y: f64, width: f64, height: f64, clip_width: f64, clip_height: f64) -> PinPlace {
    if x >= 0.0 && y >= 0.0 && x + width <= clip_width && y + height <= clip_height {
        PinPlace::Inside
    } else if x + width <= 0.0 || y + height <= 0.0 || x >= clip_width || y >= clip_height {
        PinPlace::Outside
    } else {
        PinPlace::Straddles
    }
}

/// the gpui shift key a pin of this surface is painted under, nonzero
fn pin_shift_key(surface: EntityId, key: usize) -> u64 {
    ((key as u64) ^ surface.as_u64().rotate_left(48)) | 1
}

#[derive(Clone)]
pub struct RiseMapHost(Rc<RefCell<RiseMapHostState>>);

#[derive(Clone)]
pub struct WeakRiseMapHost(Weak<RefCell<RiseMapHostState>>);

impl WeakRiseMapHost {
    pub fn upgrade(&self) -> Option<RiseMapHost> {
        self.0.upgrade().map(RiseMapHost)
    }
}

impl RiseMapHost {
    /// makes a map view with zero frame and the style json
    pub fn new(style_json: String, appearance: RiseThemeAppearance) -> Self {
        let first_symbol_layer = Self::first_symbol_layer_of(&style_json);
        Self(Rc::new(RefCell::new(RiseMapHostState {
            entity: None,
            handle: None,
            driver: CameraDriver::new(Camera::new(LatLng::default(), MIN_ZOOM)),
            input: MapInputController::default(),
            viewport: None,
            unplaced_center: None,
            origin: point(px(0.0), px(0.0)),
            image: None,
            image_camera: None,
            style_json,
            appearance,
            is_style_loaded: false,
            sources: HashMap::new(),
            layers: Vec::new(),
            images: Vec::new(),
            annotations: Vec::new(),
            reusable: HashMap::new(),
            subviews: Vec::new(),
            bottom_inset: 0.0,
            maximum_zoom_level: MAX_ZOOM,
            minimum_zoom_level: MIN_ZOOM,
            is_scroll_enabled: true,
            is_zoom_enabled: true,
            is_rotate_enabled: true,
            is_pitch_enabled: true,
            shows_compass_view: false,
            is_right_to_left: false,
            pending_regions: Vec::new(),
            pending_style_loaded: false,
            wants_animation_frame: false,
            first_symbol_layer,
            pressed_at: None,
            moved_since_press: false,
            unavailable: None,
            pin_paint: PinPaint::default(),
        })))
    }

    pub fn downgrade(&self) -> WeakRiseMapHost {
        WeakRiseMapHost(Rc::downgrade(&self.0))
    }

    pub fn same_map(&self, other: &RiseMapHost) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    fn send(&self, request: MapRequest) {
        let state = self.0.borrow();
        match &request {
            MapRequest::SetStyle(json) => x473_mark(|| format!("send set_style bytes={} open={}", json.len(), state.handle.is_some())),
            MapRequest::SetCamera(camera) => x473_mark(|| format!("send set_camera {camera:?}")),
            MapRequest::Resize(viewport) => x473_mark(|| format!("send resize {viewport:?}")),
            _ => {}
        }
        if let Some(handle) = &state.handle
            && let Err(error) = handle.send(request)
        {
            tracing::debug!(%error, "map request not delivered");
        }
    }

    /// redraws the element after a change made outside its own update
    pub fn set_needs_display(&self, cx: &mut App) {
        if let Some(entity) = self.0.borrow().entity {
            cx.notify(entity);
        }
    }

    /// true when something over the map frame is placed by the camera
    /// a pin with its view or waiting for one, or a shown subview
    /// otherwise a new camera only changes the renderer picture
    pub fn has_camera_bound_content(&self) -> bool {
        let state = self.0.borrow();
        !state.annotations.is_empty() || state.subviews.iter().any(|subview| !subview.is_hidden)
    }

    pub fn has_unresolved_visible_annotation(&self) -> bool {
        let visible = self.visible_annotations();
        if visible.is_empty() {
            return false;
        }
        let state = self.0.borrow();
        visible.iter().any(|annotation| {
            let key = annotation_key(annotation);
            state
                .annotations
                .iter()
                .any(|slot| annotation_key(&slot.annotation) == key && slot.view.is_none())
        })
    }

    /// asks for one more frame, a layer moving its pins over time calls it each step
    pub fn request_animation_frame(&self) {
        self.0.borrow_mut().wants_animation_frame = true;
    }

    pub fn has_window(&self) -> bool {
        self.0.borrow().viewport.is_some()
    }

    /// the element's size in logical points
    pub fn bounds(&self) -> Option<Viewport> {
        self.0.borrow().viewport
    }

    pub fn bounds_width(&self) -> f64 {
        self.0.borrow().viewport.map_or(0.0, |viewport| viewport.width)
    }

    pub fn bounds_height(&self) -> f64 {
        self.0.borrow().viewport.map_or(0.0, |viewport| viewport.height)
    }

    pub fn camera(&self) -> Camera {
        self.0.borrow().driver.camera()
    }

    pub fn zoom_level(&self) -> f64 {
        self.camera().zoom
    }

    pub fn direction(&self) -> f64 {
        self.camera().bearing
    }

    pub fn maximum_zoom_level(&self) -> f64 {
        self.0.borrow().maximum_zoom_level
    }

    pub fn set_maximum_zoom_level(&self, level: f64) {
        self.0.borrow_mut().maximum_zoom_level = level.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    pub fn minimum_zoom_level(&self) -> f64 {
        self.0.borrow().minimum_zoom_level
    }

    /// projection of the whole element, none before the first layout
    pub fn projection(&self) -> Option<Projection> {
        let state = self.0.borrow();
        state.viewport.map(|viewport| Projection::new(state.driver.camera(), viewport))
    }

    /// point the content inset leaves as the visual center
    fn inset_center(state: &RiseMapHostState) -> Option<ScreenPoint> {
        state
            .viewport
            .map(|viewport| ScreenPoint::new(viewport.width / 2.0, (viewport.height - state.bottom_inset) / 2.0))
    }

    /// coordinate at the center of the inset area
    pub fn center_coordinate(&self) -> LatLng {
        let state = self.0.borrow();
        let camera = state.driver.camera();
        match (state.viewport, Self::inset_center(&state)) {
            (Some(viewport), Some(center)) => Projection::new(camera, viewport).unproject(center).unwrap_or(camera.center),
            _ => camera.center,
        }
    }

    pub fn convert_point_to_coordinate(&self, point: ScreenPoint) -> Option<LatLng> {
        self.projection()?.unproject(point)
    }

    pub fn convert_coordinate_to_point(&self, coordinate: LatLng) -> Option<ScreenPoint> {
        self.projection()?.project(coordinate)
    }

    /// window coordinates to a point of the map element
    pub fn convert_window_point(&self, position: gpui::Point<Pixels>) -> ScreenPoint {
        let origin = self.0.borrow().origin;
        ScreenPoint::new(
            f64::from(f32::from(position.x - origin.x)),
            f64::from(f32::from(position.y - origin.y)),
        )
    }

    /// camera that shows center at the inset center with the given zoom and direction
    fn camera_centred(&self, center: LatLng, zoom: f64, direction: Option<f64>) -> Camera {
        let state = self.0.borrow();
        let current = state.driver.camera();
        let zoom = zoom.clamp(state.minimum_zoom_level, state.maximum_zoom_level);
        let target = Camera {
            center,
            zoom,
            bearing: direction.unwrap_or(current.bearing),
            pitch: current.pitch,
        };
        match (state.viewport, Self::inset_center(&state)) {
            (Some(viewport), Some(at)) => Projection::new(target, viewport).place_under(center, at).unwrap_or(target),
            _ => target,
        }
    }

    fn move_camera(&self, target: Camera, animated: bool, cx: &mut App) {
        let now = RiseClock::monotonic().as_secs_f64();
        let events = {
            let mut state = self.0.borrow_mut();
            if animated {
                let from = state.driver.camera();
                state.driver.apply(
                    CameraCommand::Animate {
                        animation: CameraAnimation::ease(from, target, rise_media::map_view::DEFAULT_ANIMATION_SECS),
                        reason: CameraChangeReason::PROGRAMMATIC,
                    },
                    now,
                )
            } else {
                state.driver.jump_to(target, CameraChangeReason::PROGRAMMATIC)
            }
        };
        self.0.borrow_mut().pending_regions.extend(events);
        self.send(MapRequest::SetCamera(self.camera()));
        self.set_needs_display(cx);
    }

    /// sets center with optional zoom and direction and animation
    pub fn set_center(&self, center: LatLng, zoom_level: Option<f64>, direction: Option<f64>, animated: bool, cx: &mut App) {
        let zoom = zoom_level.unwrap_or_else(|| self.zoom_level());
        let target = self.camera_centred(center, zoom, direction);
        self.move_camera(target, animated, cx);
        self.remember_unplaced_center(center);
    }

    /// sets the zoom, the inset center keeps its coordinate
    pub fn set_zoom_level(&self, zoom_level: f64, animated: bool, cx: &mut App) {
        let center = self.center_coordinate();
        self.set_center(center, Some(zoom_level), None, animated, cx);
    }

    pub fn set_direction(&self, direction: f64, animated: bool, cx: &mut App) {
        let center = self.center_coordinate();
        self.set_center(center, None, Some(direction), animated, cx);
    }

    pub fn set_camera(&self, camera: Camera, animated: bool, cx: &mut App) {
        self.0.borrow_mut().unplaced_center = None;
        self.move_camera(camera.clamped(), animated, cx);
    }

    /// fits the box inside the padded view
    pub fn set_visible_coordinate_bounds(
        &self,
        south_west: LatLng,
        north_east: LatLng,
        padding: (f64, f64, f64, f64),
        animated: bool,
        cx: &mut App,
    ) {
        let Some(viewport) = self.0.borrow().viewport else {
            return;
        };
        let (top, left, bottom, right) = padding;
        let width = (viewport.width - left - right).max(1.0);
        let height = (viewport.height - top - bottom).max(1.0);
        let (west, north) = rise_media::map_view::mercator(LatLng::new(north_east.latitude, south_west.longitude));
        let (east, south) = rise_media::map_view::mercator(LatLng::new(south_west.latitude, north_east.longitude));
        let span_x = (east - west).abs().max(f64::EPSILON);
        let span_y = (south - north).abs().max(f64::EPSILON);
        let tile = rise_media::map_view::TILE_SIZE;
        let zoom = (width / (tile * span_x)).log2().min((height / (tile * span_y)).log2());
        let middle = rise_media::map_view::inverse_mercator((west + east) / 2.0, (north + south) / 2.0);
        let current = self.camera();
        let target = Camera { center: middle, zoom: zoom.clamp(self.minimum_zoom_level(), self.maximum_zoom_level()), bearing: 0.0, pitch: current.pitch };
        let at = ScreenPoint::new(left + width / 2.0, top + height / 2.0);
        let placed = Projection::new(target, viewport).place_under(middle, at).unwrap_or(target);
        self.move_camera(placed, animated, cx);
    }

    pub fn set_content_inset_bottom(&self, bottom: f64, animated: bool, cx: &mut App) {
        let center = self.center_coordinate();
        self.0.borrow_mut().bottom_inset = bottom.max(0.0);
        let target = self.camera_centred(center, self.zoom_level(), None);
        self.move_camera(target, animated, cx);
        self.remember_unplaced_center(center);
    }

    fn remember_unplaced_center(&self, center: LatLng) {
        let mut state = self.0.borrow_mut();
        state.unplaced_center = state.viewport.is_none().then_some(center);
    }

    pub fn set_scroll_enabled(&self, enabled: bool) {
        self.0.borrow_mut().is_scroll_enabled = enabled;
    }

    pub fn set_zoom_enabled(&self, enabled: bool) {
        self.0.borrow_mut().is_zoom_enabled = enabled;
    }

    pub fn set_rotate_enabled(&self, enabled: bool) {
        self.0.borrow_mut().is_rotate_enabled = enabled;
    }

    pub fn set_pitch_enabled(&self, enabled: bool) {
        self.0.borrow_mut().is_pitch_enabled = enabled;
    }

    pub fn shows_compass_view(&self) -> bool {
        self.0.borrow().shows_compass_view
    }

    pub fn set_shows_compass_view(&self, shows: bool) {
        self.0.borrow_mut().shows_compass_view = shows;
    }

    /// true for a right to left layout direction
    pub fn is_right_to_left(&self) -> bool {
        self.0.borrow().is_right_to_left
    }

    pub fn set_right_to_left(&self, right_to_left: bool) {
        self.0.borrow_mut().is_right_to_left = right_to_left;
    }

    /// why this host draws no map, when the OS has no maplibre core for it
    /// none means the style finished loading
    pub fn unavailable(&self) -> Option<MapUnavailable> {
        self.0.borrow().unavailable.clone()
    }

    pub fn is_style_loaded(&self) -> bool {
        self.0.borrow().is_style_loaded
    }

    pub fn appearance(&self) -> RiseThemeAppearance {
        self.0.borrow().appearance
    }

    pub fn set_style_json(&self, style_json: String, appearance: RiseThemeAppearance) {
        {
            let mut state = self.0.borrow_mut();
            state.first_symbol_layer = Self::first_symbol_layer_of(&style_json);
            state.style_json = style_json.clone();
            state.appearance = appearance;
            state.is_style_loaded = false;
            state.sources.clear();
            state.layers.clear();
            state.images.clear();
        }
        self.send(MapRequest::SetStyle(style_json));
    }

    /// first label layer of the style
    pub fn first_symbol_layer(&self) -> Option<String> {
        self.0.borrow().first_symbol_layer.clone()
    }

    fn first_symbol_layer_of(style_json: &str) -> Option<String> {
        thread_local! {
            static FIRST_SYMBOL_LAYERS: RefCell<HashMap<u64, Option<String>>> = RefCell::new(HashMap::new());
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(style_json, &mut hasher);
        let key = std::hash::Hasher::finish(&hasher);
        if let Some(cached) = FIRST_SYMBOL_LAYERS.with(|layers| layers.borrow().get(&key).cloned()) {
            return cached;
        }
        let found = Self::parse_first_symbol_layer(style_json);
        FIRST_SYMBOL_LAYERS.with(|layers| layers.borrow_mut().insert(key, found.clone()));
        found
    }

    fn parse_first_symbol_layer(style_json: &str) -> Option<String> {
        let style: Value = serde_json::from_str(style_json).ok()?;
        style
            .get("layers")?
            .as_array()?
            .iter()
            .find(|layer| layer.get("type").and_then(Value::as_str) == Some("symbol"))
            .and_then(|layer| layer.get("id").and_then(Value::as_str))
            .map(str::to_string)
    }

    /// color as a style color resolved for the map's appearance
    pub fn style_color(&self, color: UIColor) -> String {
        Self::css_color(color.resolved_color(self.appearance()))
    }

    pub fn css_color(color: Hsla) -> String {
        let rgba = Rgba::from(color);
        format!(
            "rgba({}, {}, {}, {})",
            (rgba.r * 255.0).round() as u8,
            (rgba.g * 255.0).round() as u8,
            (rgba.b * 255.0).round() as u8,
            rgba.a
        )
    }

    pub fn has_source(&self, source_id: &str) -> bool {
        self.0.borrow().sources.contains_key(source_id)
    }

    /// adds the source or sets the shape of the existing one
    /// none clears the shape
    pub fn set_shape(&self, source_id: &str, geojson: Option<Value>) {
        let value = geojson.unwrap_or_else(|| serde_json::json!({ "type": "FeatureCollection", "features": [] }));
        let delivered = {
            let state = self.0.borrow();
            state.handle.is_some() && state.sources.get(source_id) == Some(&value)
        };
        if delivered {
            return;
        }
        let text = value.to_string();
        self.0.borrow_mut().sources.insert(source_id.to_string(), value);
        self.send(MapRequest::SetGeoJson { source_id: source_id.to_string(), geojson: text });
    }

    pub fn remove_source(&self, source_id: &str) {
        if self.0.borrow_mut().sources.remove(source_id).is_some() {
            self.send(MapRequest::RemoveSource(source_id.to_string()));
        }
    }

    pub fn has_layer(&self, layer_id: &str) -> bool {
        self.0.borrow().layers.iter().any(|layer| layer.id == layer_id)
    }

    /// adds a style spec layer, optionally below another
    /// a layer with an id already in the style is replaced so paint changes use the same call
    pub fn add_layer(&self, layer: Value, below: Option<&str>) {
        let Some(id) = layer.get("id").and_then(Value::as_str).map(str::to_string) else {
            tracing::debug!("map layer without an id skipped");
            return;
        };
        let source = layer.get("source").and_then(Value::as_str).map(str::to_string);
        if self.has_layer(&id) {
            self.remove_layer(&id);
        }
        {
            let mut state = self.0.borrow_mut();
            let entry = StyleLayerEntry { id, source };
            match below.and_then(|below| state.layers.iter().position(|layer| layer.id == below)) {
                Some(index) => state.layers.insert(index, entry),
                None => state.layers.push(entry),
            }
        }
        self.send(MapRequest::AddLayer {
            layer_json: layer.to_string(),
            before: below.map(str::to_string),
        });
    }

    pub fn remove_layer(&self, layer_id: &str) {
        let removed = {
            let mut state = self.0.borrow_mut();
            let before = state.layers.len();
            state.layers.retain(|layer| layer.id != layer_id);
            before != state.layers.len()
        };
        if removed {
            self.send(MapRequest::RemoveLayer(layer_id.to_string()));
        }
    }

    pub fn set_image(&self, image_id: &str, rgba: Vec<u8>, width: u32, height: u32, pixel_ratio: f32, signed_distance_field: bool) {
        {
            let mut state = self.0.borrow_mut();
            if !state.images.iter().any(|known| known == image_id) {
                state.images.push(image_id.to_string());
            }
        }
        self.send(MapRequest::AddImage {
            image_id: image_id.to_string(),
            rgba,
            width,
            height,
            pixel_ratio,
            signed_distance_field,
        });
    }

    pub fn has_image(&self, image_id: &str) -> bool {
        self.0.borrow().images.iter().any(|known| known == image_id)
    }

    pub fn remove_image(&self, image_id: &str) {
        let removed = {
            let mut state = self.0.borrow_mut();
            let before = state.images.len();
            state.images.retain(|known| known != image_id);
            before != state.images.len()
        };
        if removed {
            self.send(MapRequest::RemoveImage(image_id.to_string()));
        }
    }

    /// features of those layers' sources whose geometry crosses the rectangle on screen
    pub fn visible_features(&self, rect: ScreenRect, layer_ids: &[&str]) -> Vec<Value> {
        let Some(projection) = self.projection() else {
            return Vec::new();
        };
        let state = self.0.borrow();
        let mut seen: Vec<&str> = Vec::new();
        let mut found = Vec::new();
        for layer in state.layers.iter().filter(|layer| layer_ids.iter().any(|id| *id == layer.id)) {
            let Some(source) = layer.source.as_deref() else {
                continue;
            };
            if seen.contains(&source) {
                continue;
            }
            seen.push(source);
            let Some(shape) = state.sources.get(source) else {
                continue;
            };
            for feature in features_of(shape) {
                if feature.get("geometry").is_some_and(|geometry| geometry_crosses(geometry, &projection, &rect)) {
                    found.push(feature.clone());
                }
            }
        }
        found
    }

    pub fn add_annotation(&self, annotation: Rc<dyn MapAnnotation>) {
        let key = annotation_key(&annotation);
        let mut state = self.0.borrow_mut();
        if state.annotations.iter().any(|slot| annotation_key(&slot.annotation) == key) {
            return;
        }
        state.annotations.push(AnnotationSlot { annotation, view: None });
    }

    pub fn add_annotations(&self, annotations: Vec<Rc<dyn MapAnnotation>>) {
        for annotation in annotations {
            self.add_annotation(annotation);
        }
    }

    /// removes annotations, their views go back to the reuse queue
    pub fn remove_annotations(&self, annotations: &[Rc<dyn MapAnnotation>]) {
        let keys: Vec<usize> = annotations.iter().map(annotation_key).collect();
        let mut state = self.0.borrow_mut();
        let mut kept = Vec::with_capacity(state.annotations.len());
        for slot in std::mem::take(&mut state.annotations) {
            if keys.contains(&annotation_key(&slot.annotation)) {
                if let Some(view) = slot.view {
                    state.reusable.entry(view.reuse_identifier.clone()).or_default().push(view);
                }
            } else {
                kept.push(slot);
            }
        }
        state.annotations = kept;
    }

    pub fn remove_annotation(&self, annotation: &Rc<dyn MapAnnotation>) {
        self.remove_annotations(std::slice::from_ref(annotation));
    }

    pub fn annotations(&self) -> Vec<Rc<dyn MapAnnotation>> {
        self.0.borrow().annotations.iter().map(|slot| slot.annotation.clone()).collect()
    }

    /// annotations whose coordinate is on screen
    pub fn visible_annotations(&self) -> Vec<Rc<dyn MapAnnotation>> {
        let Some(projection) = self.projection() else {
            return Vec::new();
        };
        let state = self.0.borrow();
        let Some(viewport) = state.viewport else {
            return Vec::new();
        };
        state
            .annotations
            .iter()
            .filter(|slot| {
                projection.project(slot.annotation.coordinate()).is_some_and(|at| {
                    at.x >= 0.0 && at.y >= 0.0 && at.x <= viewport.width && at.y <= viewport.height
                })
            })
            .map(|slot| slot.annotation.clone())
            .collect()
    }

    /// the view the delegate gave, once the annotation has been on screen
    pub fn view_for(&self, annotation: &Rc<dyn MapAnnotation>) -> Option<AnyView> {
        let key = annotation_key(annotation);
        self.0
            .borrow()
            .annotations
            .iter()
            .find(|slot| annotation_key(&slot.annotation) == key)
            .and_then(|slot| slot.view.as_ref().map(|view| view.view.clone()))
    }

    pub fn set_annotation_view_frame(&self, annotation: &Rc<dyn MapAnnotation>, width: f64, height: f64, anchor: (f64, f64)) {
        let key = annotation_key(annotation);
        let mut state = self.0.borrow_mut();
        if let Some(view) = state
            .annotations
            .iter_mut()
            .find(|slot| annotation_key(&slot.annotation) == key)
            .and_then(|slot| slot.view.as_mut())
        {
            view.width = width;
            view.height = height;
            view.anchor = anchor;
        }
    }

    pub fn set_annotation_z_position(&self, annotation: &Rc<dyn MapAnnotation>, z_position: f64) {
        let key = annotation_key(annotation);
        let mut state = self.0.borrow_mut();
        if let Some(view) = state
            .annotations
            .iter_mut()
            .find(|slot| annotation_key(&slot.annotation) == key)
            .and_then(|slot| slot.view.as_mut())
        {
            view.z_position = z_position;
        }
    }

    /// takes a reusable annotation view by identifier
    pub fn dequeue_reusable_annotation_view(&self, reuse_identifier: &str) -> Option<MapAnnotationView> {
        self.0
            .borrow_mut()
            .reusable
            .get_mut(reuse_identifier)
            .and_then(Vec::pop)
    }

    /// desktop map keeps no selection so a select is just a click
    pub fn deselect_annotation(&self, _annotation: &Rc<dyn MapAnnotation>) {}

    pub fn add_subview(&self, view: AnyView) {
        let mut state = self.0.borrow_mut();
        if !state.subviews.iter().any(|subview| subview.view.entity_id() == view.entity_id()) {
            state.subviews.push(MapSubview { view, is_hidden: false });
        }
    }

    pub fn remove_subview(&self, entity: EntityId) {
        self.0.borrow_mut().subviews.retain(|subview| subview.view.entity_id() != entity);
    }

    /// hides or shows a subview
    pub fn set_subview_hidden(&self, entity: EntityId, hidden: bool) {
        if let Some(subview) = self
            .0
            .borrow_mut()
            .subviews
            .iter_mut()
            .find(|subview| subview.view.entity_id() == entity)
        {
            subview.is_hidden = hidden;
        }
    }

    pub fn bring_subview_to_front(&self, entity: EntityId) {
        let mut state = self.0.borrow_mut();
        if let Some(index) = state.subviews.iter().position(|subview| subview.view.entity_id() == entity) {
            let subview = state.subviews.remove(index);
            state.subviews.push(subview);
        }
    }

    /// applies map input like the map's own gestures
    /// gesture switches only count when respect_switches is set
    pub fn handle_input(&self, input: MapInput, respect_switches: bool, cx: &mut App) {
        let now = RiseClock::monotonic().as_secs_f64();
        let events = {
            let mut state = self.0.borrow_mut();
            let Some(viewport) = state.viewport else {
                return;
            };
            if respect_switches && !Self::input_allowed(&state, &input) {
                return;
            }
            let camera = state.driver.camera();
            let Some(command) = state.input.handle(input, &camera, &viewport) else {
                return;
            };
            let command = Self::limit_zoom(&state, command);
            let mut events = state.driver.apply(command, now);
            if matches!(input, MapInput::DragEnd) {
                events.extend(state.driver.settle());
            }
            events
        };
        self.0.borrow_mut().pending_regions.extend(events);
        self.send(MapRequest::SetCamera(self.camera()));
        self.set_needs_display(cx);
    }

    fn input_allowed(state: &RiseMapHostState, input: &MapInput) -> bool {
        match input {
            MapInput::DragStart { button: DragButton::Primary, modifiers, .. } => {
                if modifiers.control { state.is_rotate_enabled } else { state.is_scroll_enabled }
            }
            MapInput::DragStart { button: DragButton::Secondary, .. } => state.is_rotate_enabled || state.is_pitch_enabled,
            MapInput::DragMove { .. } | MapInput::DragEnd => true,
            MapInput::Wheel { delta: WheelDelta::Pixels { .. }, modifiers, .. } => {
                if modifiers.secondary || modifiers.control { state.is_zoom_enabled } else { state.is_scroll_enabled }
            }
            MapInput::Wheel { .. } | MapInput::Pinch { .. } | MapInput::DoubleClick { .. } => state.is_zoom_enabled,
            MapInput::Key { key: MapKey::ZoomIn | MapKey::ZoomOut, .. } => state.is_zoom_enabled,
            MapInput::Key { .. } => state.is_scroll_enabled,
        }
    }

    fn limit_zoom(state: &RiseMapHostState, command: CameraCommand) -> CameraCommand {
        let clamp = |camera: Camera| Camera {
            zoom: camera.zoom.clamp(state.minimum_zoom_level, state.maximum_zoom_level),
            ..camera
        };
        match command {
            CameraCommand::Jump { camera, reason } => CameraCommand::Jump { camera: clamp(camera), reason },
            CameraCommand::Animate { mut animation, reason } => {
                animation.to = clamp(animation.to);
                CameraCommand::Animate { animation, reason }
            }
        }
    }

    fn annotation_frames(&self) -> Vec<(usize, Annotation, f64)> {
        let state = self.0.borrow();
        state
            .annotations
            .iter()
            .filter_map(|slot| {
                let view = slot.view.as_ref()?;
                Some((
                    annotation_key(&slot.annotation),
                    Annotation {
                        id: annotation_key(&slot.annotation) as u64,
                        coordinate: slot.annotation.coordinate(),
                        width: view.width,
                        height: view.height,
                        anchor: view.anchor,
                    },
                    view.z_position,
                ))
            })
            .collect()
    }

    fn annotation_at(&self, point: ScreenPoint) -> Option<Rc<dyn MapAnnotation>> {
        let projection = self.projection()?;
        let mut frames = self.annotation_frames();
        frames.sort_by(|a, b| a.2.total_cmp(&b.2));
        let plain: Vec<Annotation> = frames.iter().map(|(_, annotation, _)| *annotation).collect();
        let hit = projection.hit_test(&plain, point)?;
        let state = self.0.borrow();
        state
            .annotations
            .iter()
            .find(|slot| annotation_key(&slot.annotation) as u64 == hit)
            .map(|slot| slot.annotation.clone())
    }
}

fn features_of(shape: &Value) -> Vec<&Value> {
    match shape.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => shape
            .get("features")
            .and_then(Value::as_array)
            .map(|features| features.iter().collect())
            .unwrap_or_default(),
        Some("Feature") => vec![shape],
        _ => Vec::new(),
    }
}

fn position_of(value: &Value) -> Option<LatLng> {
    let pair = value.as_array()?;
    Some(LatLng::new(pair.get(1)?.as_f64()?, pair.first()?.as_f64()?))
}

fn line_of(value: &Value) -> Vec<LatLng> {
    value
        .as_array()
        .map(|points| points.iter().filter_map(position_of).collect())
        .unwrap_or_default()
}

fn segment_crosses_rect(a: ScreenPoint, b: ScreenPoint, rect: &ScreenRect) -> bool {
    if rect.contains(a) || rect.contains(b) {
        return true;
    }
    let (left, top) = (rect.origin.x, rect.origin.y);
    let (right, bottom) = (left + rect.width, top + rect.height);
    let corners = [
        (ScreenPoint::new(left, top), ScreenPoint::new(right, top)),
        (ScreenPoint::new(right, top), ScreenPoint::new(right, bottom)),
        (ScreenPoint::new(right, bottom), ScreenPoint::new(left, bottom)),
        (ScreenPoint::new(left, bottom), ScreenPoint::new(left, top)),
    ];
    corners.iter().any(|(c, d)| segments_intersect(a, b, *c, *d))
}

fn segments_intersect(a: ScreenPoint, b: ScreenPoint, c: ScreenPoint, d: ScreenPoint) -> bool {
    let cross = |o: ScreenPoint, p: ScreenPoint, q: ScreenPoint| (p.x - o.x) * (q.y - o.y) - (p.y - o.y) * (q.x - o.x);
    let d1 = cross(c, d, a);
    let d2 = cross(c, d, b);
    let d3 = cross(a, b, c);
    let d4 = cross(a, b, d);
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
}

fn polyline_crosses(points: &[LatLng], projection: &Projection, rect: &ScreenRect) -> bool {
    let screen: Vec<ScreenPoint> = points.iter().filter_map(|point| projection.project(*point)).collect();
    match screen.as_slice() {
        [] => false,
        [only] => rect.contains(*only),
        _ => screen.windows(2).any(|pair| segment_crosses_rect(pair[0], pair[1], rect)),
    }
}

fn ring_contains(ring: &[ScreenPoint], point: ScreenPoint) -> bool {
    let mut inside = false;
    let mut j = ring.len().wrapping_sub(1);
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > point.y) != (b.y > point.y) && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn polygon_crosses(rings: &Value, projection: &Projection, rect: &ScreenRect) -> bool {
    let Some(rings) = rings.as_array() else {
        return false;
    };
    let Some(outer) = rings.first() else {
        return false;
    };
    let outer = line_of(outer);
    if polyline_crosses(&outer, projection, rect) {
        return true;
    }
    let screen: Vec<ScreenPoint> = outer.iter().filter_map(|point| projection.project(*point)).collect();
    ring_contains(&screen, rect.origin)
}

fn geometry_crosses(geometry: &Value, projection: &Projection, rect: &ScreenRect) -> bool {
    let coordinates = geometry.get("coordinates").unwrap_or(&Value::Null);
    match geometry.get("type").and_then(Value::as_str) {
        Some("Point") => position_of(coordinates)
            .and_then(|point| projection.project(point))
            .is_some_and(|at| rect.contains(at)),
        Some("MultiPoint" | "LineString") => polyline_crosses(&line_of(coordinates), projection, rect),
        Some("MultiLineString") => coordinates
            .as_array()
            .is_some_and(|lines| lines.iter().any(|line| polyline_crosses(&line_of(line), projection, rect))),
        Some("Polygon") => polygon_crosses(coordinates, projection, rect),
        Some("MultiPolygon") => coordinates
            .as_array()
            .is_some_and(|polygons| polygons.iter().any(|polygon| polygon_crosses(polygon, projection, rect))),
        Some("GeometryCollection") => geometry
            .get("geometries")
            .and_then(Value::as_array)
            .is_some_and(|geometries| geometries.iter().any(|inner| geometry_crosses(inner, projection, rect))),
        _ => false,
    }
}

/// element half of the map view
/// renderer and its frames and the delegate
pub struct RiseMapSurface<D: RiseMapViewDelegate> {
    pub host: RiseMapHost,
    pub delegate: D,
    focus_handle: FocusHandle,
    _pump: Option<Task<()>>,
    tick_scheduled: bool,
}

impl<D: RiseMapViewDelegate> RiseMapSurface<D> {
    pub fn new(host: RiseMapHost, delegate: D, cx: &mut Context<Self>) -> Self {
        host.0.borrow_mut().entity = Some(cx.entity_id());
        cx.on_release(|surface: &mut Self, cx| {
            let host = surface.host.clone();
            surface.delegate.dismantle(&host, cx);
            let mut state = host.0.borrow_mut();
            state.handle = None;
            state.entity = None;
            state.sources.clear();
        })
        .detach();
        Self { host, delegate, focus_handle: cx.focus_handle(), _pump: None, tick_scheduled: false }
    }

    fn layout(&mut self, bounds: Bounds<Pixels>, scale_factor: f32, window: &mut Window, cx: &mut Context<Self>) {
        let viewport = Viewport {
            width: f64::from(f32::from(bounds.size.width)),
            height: f64::from(f32::from(bounds.size.height)),
            scale_factor,
        };
        let (first, changed) = {
            let mut state = self.host.0.borrow_mut();
            state.origin = bounds.origin;
            if state.viewport == Some(viewport) || viewport.width < 1.0 || viewport.height < 1.0 {
                (false, false)
            } else {
                let first = state.viewport.is_none();
                state.viewport = Some(viewport);
                if let Some(center) = state.unplaced_center.take() {
                    let at = RiseMapHost::inset_center(&state);
                    let camera = state.driver.camera();
                    if let Some(placed) = at.and_then(|at| Projection::new(camera, viewport).place_under(center, at)) {
                        let events = state.driver.jump_to(placed, CameraChangeReason::PROGRAMMATIC);
                        state.pending_regions.extend(events);
                    }
                }
                (first, true)
            }
        };
        if !changed {
            return;
        }
        if first {
            self.open(viewport, window, cx);
        } else {
            self.host.send(MapRequest::Resize(viewport));
        }
        cx.notify();
    }

    fn open(&mut self, viewport: Viewport, window: &mut Window, cx: &mut Context<Self>) {
        let Some(paths) = location_app_paths() else {
            tracing::debug!("map has no app paths");
            return;
        };
        let (wake, mut woken) = mpsc::unbounded::<()>();
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake.unbounded_send(());
        });
        let (style_json, camera) = {
            let state = self.host.0.borrow();
            (state.style_json.clone(), state.driver.camera())
        };
        x473_mark(|| format!("open viewport={viewport:?} camera={camera:?} style_bytes={}", style_json.len()));
        match open_map(MapOptions::new(style_json, viewport, camera, paths, notify)) {
            Ok(handle) => self.host.0.borrow_mut().handle = Some(handle),
            Err(reason) => {
                self.host.0.borrow_mut().unavailable = Some(reason.clone());
                tracing::debug!(%reason, "map unavailable");
                x473_mark(|| format!("open unavailable {reason}"));
                cx.notify();
                return;
            }
        }
        self._pump = Some(cx.spawn_in(window, async move |this, cx| {
            while woken.next().await.is_some() {
                if this.update_in(cx, |this, window, cx| this.drain(window, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn drain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let events = match &self.host.0.borrow().handle {
            Some(handle) => handle.drain(),
            None => return,
        };
        for event in events {
            match event {
                MapEvent::Frame(frame) => {
                    x473_mark(|| format!("frame {}x{} camera={:?}", frame.width, frame.height, frame.camera));
                    let camera = frame.camera;
                    if let Some(image) = location_map_image(frame) {
                        let (old, same_camera) = {
                            let mut state = self.host.0.borrow_mut();
                            let same_camera = state.image_camera == Some(camera);
                            state.image_camera = Some(camera);
                            (state.image.replace(image.clone()), same_camera)
                        };
                        let animating = self.host.0.borrow().driver.is_animating();
                        let follows = same_camera
                            || !self.host.has_camera_bound_content()
                            || (animating && self.shift_pins(window, cx.entity_id()));
                        let swapped = follows
                            && old.as_ref().is_some_and(|old| {
                                window.swap_image(map_frame_swap_key(cx.entity_id()), old, &image, 0)
                            });
                        if let Some(old) = old {
                            let _ = window.drop_image(old);
                        }
                        if !swapped {
                            cx.notify();
                        }
                    }
                }
                MapEvent::StyleLoaded => {
                    x473_mark(|| "style loaded".to_owned());
                    let mut state = self.host.0.borrow_mut();
                    state.is_style_loaded = true;
                    state.pending_style_loaded = true;
                    cx.notify();
                }
                MapEvent::StyleFailed { failure, message } => {
                    tracing::debug!(?failure, %message, "map style failed");
                    x473_mark(|| format!("style failed {failure:?} {message}"));
                }
                MapEvent::RequestFailed { request, message } => {
                    tracing::debug!(request, %message, "map request refused");
                    x473_mark(|| format!("request refused {request} {message}"));
                }
                MapEvent::WillStartLoading => x473_mark(|| "will start loading".to_owned()),
                MapEvent::Idle => x473_mark(|| "idle".to_owned()),
            }
        }
    }

    /// delivers the queued delegate callbacks, style load first then camera changes
    pub fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.host.0.borrow().driver.is_animating() {
            self.schedule_tick(window, cx);
        }
        let style_loaded = std::mem::take(&mut self.host.0.borrow_mut().pending_style_loaded);
        if style_loaded {
            self.delegate.did_finish_loading(&self.host, window, cx);
        }
        self.deliver_regions(window, cx);
        self.resolve_views(window, cx);
        if std::mem::take(&mut self.host.0.borrow_mut().wants_animation_frame) {
            window.request_animation_frame();
        }
    }

    fn schedule_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tick_scheduled {
            return;
        }
        self.tick_scheduled = true;
        let surface = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let _ = surface.update(cx, |surface, cx| {
                surface.tick_scheduled = false;
                surface.tick(window, cx);
            });
        });
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic().as_secs_f64();
        let animating = {
            let mut state = self.host.0.borrow_mut();
            if !state.driver.is_animating() {
                return;
            }
            let events = state.driver.tick(now);
            state.pending_regions.extend(events);
            state.driver.is_animating()
        };
        self.host.send(MapRequest::SetCamera(self.host.camera()));
        if animating {
            self.schedule_tick(window, cx);
        }
        self.deliver_regions(window, cx);
        self.resolve_views(window, cx);
        if self.host.has_camera_bound_content() && (!animating || !self.shift_pins(window, cx.entity_id())) {
            cx.notify();
        }
    }

    /// moves the pins of the last render to their spots for the camera now
    /// no render of the hosting page needed
    /// false when that cant stand in for a render
    fn shift_pins(&mut self, window: &mut Window, surface: EntityId) -> bool {
        if self.host.has_unresolved_visible_annotation() {
            return false;
        }
        let Some(projection) = self.host.projection() else {
            return false;
        };
        let pixel_scale = f64::from(window.scale_factor());
        let snap = move |value: f64| (value * pixel_scale).round() / pixel_scale;
        let moves = {
            let state = self.host.0.borrow();
            let Some(viewport) = state.viewport else {
                return false;
            };
            if state.pin_paint.straddles || state.subviews.iter().any(|subview| !subview.is_hidden) {
                return false;
            }
            let clip_height = viewport.height - state.bottom_inset;
            let mut moves = Vec::new();
            for slot in &state.annotations {
                let Some(view) = &slot.view else {
                    continue;
                };
                let key = annotation_key(&slot.annotation);
                let frame = projection.annotation_frame(&Annotation {
                    id: 0,
                    coordinate: slot.annotation.coordinate(),
                    width: view.width,
                    height: view.height,
                    anchor: view.anchor,
                });
                let spot = frame.map(|frame| {
                    let (x, y) = (snap(frame.origin.x), snap(frame.origin.y));
                    (x, y, pin_place(x, y, frame.width, frame.height, viewport.width, clip_height))
                });
                if state.pin_paint.outside.contains(&key) {
                    if matches!(spot, None | Some((_, _, PinPlace::Outside))) {
                        continue;
                    }
                    return false;
                }
                let Some(&(x, y)) = state.pin_paint.inside.get(&key) else {
                    return false;
                };
                let Some((next_x, next_y, PinPlace::Inside)) = spot else {
                    return false;
                };
                if (next_x, next_y) != (x, y) {
                    moves.push((key, next_x, next_y, next_x - x, next_y - y));
                }
            }
            moves
        };
        for (key, x, y, dx, dy) in moves {
            if !window.shift_group(pin_shift_key(surface, key), point(px(dx as f32), px(dy as f32))) {
                return false;
            }
            self.host.0.borrow_mut().pin_paint.inside.insert(key, (x, y));
        }
        true
    }

    fn deliver_regions(&mut self, window: &mut Window, cx: &mut App) {
        let regions = std::mem::take(&mut self.host.0.borrow_mut().pending_regions);
        for region in regions {
            match region {
                RegionEvent::IsChanging { reason, .. } => {
                    self.delegate.region_is_changing_with(&self.host, reason, window, cx);
                }
                RegionEvent::DidChange { animated, .. } => {
                    self.delegate.region_did_change_animated(&self.host, animated, window, cx);
                }
            }
        }
    }

    /// asks the delegate for views of on screen annotations that dont have one yet
    fn resolve_views(&mut self, window: &mut Window, cx: &mut App) {
        let missing: Vec<Rc<dyn MapAnnotation>> = {
            let visible = self.host.visible_annotations();
            let state = self.host.0.borrow();
            visible
                .into_iter()
                .filter(|annotation| {
                    let key = annotation_key(annotation);
                    state
                        .annotations
                        .iter()
                        .any(|slot| annotation_key(&slot.annotation) == key && slot.view.is_none())
                })
                .collect()
        };
        for annotation in missing {
            let view = self.delegate.view_for(&self.host, &annotation, window, cx);
            let key = annotation_key(&annotation);
            let mut state = self.host.0.borrow_mut();
            if let Some(slot) = state.annotations.iter_mut().find(|slot| annotation_key(&slot.annotation) == key) {
                slot.view = view;
            }
        }
    }

    fn modifiers(modifiers: &Modifiers) -> InputModifiers {
        InputModifiers {
            secondary: modifiers.secondary(),
            control: modifiers.control,
            alt: modifiers.alt,
            shift: modifiers.shift,
        }
    }

    fn globe_zoom_levels(host: &RiseMapHost, key: &MapKey, cx: &mut App) -> Option<f64> {
        let levels = match key {
            MapKey::ZoomIn => 1.0,
            MapKey::ZoomOut => -1.0,
            _ => return None,
        };
        if MapInteractionsStore::shared(cx).read(cx).globe_entry.is_none() {
            return None;
        }
        let linked = MapGlobeCameraLink::shared(cx).read(cx).map_view();
        linked.is_some_and(|linked| linked.same_map(host)).then_some(levels)
    }

    fn map_key(key: &str) -> Option<MapKey> {
        match key {
            "+" | "=" | "add" => Some(MapKey::ZoomIn),
            "-" | "subtract" => Some(MapKey::ZoomOut),
            "up" => Some(MapKey::Up),
            "down" => Some(MapKey::Down),
            "left" => Some(MapKey::Left),
            "right" => Some(MapKey::Right),
            _ => None,
        }
    }

    fn press(&mut self, at: ScreenPoint, button: MouseButton, input: MapInput, cx: &mut Context<Self>) {
        {
            let mut state = self.host.0.borrow_mut();
            state.pressed_at = Some((at, button));
            state.moved_since_press = false;
        }
        self.host.handle_input(input, true, cx);
    }

    fn moved(&mut self, at: ScreenPoint, cx: &mut Context<Self>) {
        let dragging = {
            let mut state = self.host.0.borrow_mut();
            if let Some((start, _)) = state.pressed_at
                && (at.x - start.x).hypot(at.y - start.y) > CLICK_SLOP
            {
                state.moved_since_press = true;
            }
            state.input.is_dragging()
        };
        if dragging {
            self.host.handle_input(MapInput::DragMove { point: at }, false, cx);
        }
    }

    fn release(&mut self, button: MouseButton, window: &mut Window, cx: &mut Context<Self>) {
        let (pressed, moved) = {
            let mut state = self.host.0.borrow_mut();
            (state.pressed_at.take(), state.moved_since_press)
        };
        self.host.handle_input(MapInput::DragEnd, false, cx);
        let Some((at, pressed_button)) = pressed else {
            return;
        };
        if moved || pressed_button != button {
            return;
        }
        match button {
            MouseButton::Left => self.delegate.did_click(&self.host, at, window, cx),
            MouseButton::Right => self.delegate.did_secondary_click(&self.host, at, window, cx),
            _ => {}
        }
        cx.notify();
    }

    fn select(&mut self, key: usize, window: &mut Window, cx: &mut Context<Self>) {
        let annotation = self
            .host
            .0
            .borrow()
            .annotations
            .iter()
            .find(|slot| annotation_key(&slot.annotation) == key)
            .map(|slot| slot.annotation.clone());
        if let Some(annotation) = annotation {
            self.delegate.did_select(&self.host, &annotation, window, cx);
            cx.notify();
        }
    }

    fn unavailable_element(entity: &Entity<Self>, id: impl Into<ElementId>, host: &RiseMapHost) -> AnyElement {
        let layout_entity = entity.clone();
        let probe = canvas(
            move |bounds, window, cx| {
                let scale_factor = window.scale_factor();
                let entity = layout_entity.clone();
                window.defer(cx, move |window, cx| {
                    entity.update(cx, |surface, cx| surface.layout(bounds, scale_factor, window, cx));
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let color = UIColor::secondary_label().resolved_color(host.appearance());
        let missing_gpu = host.0.borrow().unavailable.as_ref().is_some_and(MapUnavailable::is_missing_gpu);
        let unavailable_key = if missing_gpu { "desktop_map_unavailable_gpu" } else { "map_view_unavailable" };
        div()
            .id(id)
            .relative()
            .size_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .px(px(24.0))
            .child(probe)
            .child(div().text_center().text_color(color).child(SharedString::from(tr(unavailable_key))))
            .into_any_element()
    }

    /// the map element
    /// frame and annotation views on their coordinates and subviews
    /// mouse and wheel and pinch and key input turned into camera moves
    pub fn element(entity: &Entity<Self>, id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> AnyElement {
        entity.update(cx, |surface, cx| surface.flush(window, cx));
        let surface = entity.read(cx);
        let host = surface.host.clone();
        let focus_handle = surface.focus_handle.clone();
        let image = host.0.borrow().image.clone();
        let pin_clip_bottom = host.0.borrow().bottom_inset as f32;
        let projection = host.projection();
        if host.0.borrow().unavailable.is_some() {
            return Self::unavailable_element(entity, id, &host);
        }

        let mut pins: Vec<(usize, MapAnnotationView, ScreenRect)> = Vec::new();
        if let Some(projection) = projection {
            let state = host.0.borrow();
            for slot in &state.annotations {
                let Some(view) = slot.view.clone() else {
                    continue;
                };
                let frame = projection.annotation_frame(&Annotation {
                    id: 0,
                    coordinate: slot.annotation.coordinate(),
                    width: view.width,
                    height: view.height,
                    anchor: view.anchor,
                });
                if let Some(frame) = frame {
                    pins.push((annotation_key(&slot.annotation), view, frame));
                }
            }
        }
        pins.sort_by(|a, b| a.1.z_position.total_cmp(&b.1.z_position));
        let subviews: Vec<AnyView> = host
            .0
            .borrow()
            .subviews
            .iter()
            .filter(|subview| !subview.is_hidden)
            .map(|subview| subview.view.clone())
            .collect();

        let layout_entity = entity.clone();
        let probe = canvas(
            move |bounds, window, cx| {
                let scale_factor = window.scale_factor();
                let entity = layout_entity.clone();
                window.defer(cx, move |window, cx| {
                    entity.update(cx, |surface, cx| surface.layout(bounds, scale_factor, window, cx));
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let pixel_scale = f64::from(window.scale_factor());
        let snap = move |value: f64| (value * pixel_scale).round() / pixel_scale;
        let mut pin_paint = PinPaint::default();
        let viewport = host.0.borrow().viewport;
        if let Some(viewport) = viewport {
            let clip_height = viewport.height - f64::from(pin_clip_bottom);
            for (key, _, frame) in &pins {
                let (x, y) = (snap(frame.origin.x), snap(frame.origin.y));
                match pin_place(x, y, frame.width, frame.height, viewport.width, clip_height) {
                    PinPlace::Inside => {
                        pin_paint.inside.insert(*key, (x, y));
                    }
                    PinPlace::Outside => {
                        pin_paint.outside.insert(*key);
                    }
                    PinPlace::Straddles => pin_paint.straddles = true,
                }
            }
            let state = host.0.borrow();
            for slot in &state.annotations {
                let key = annotation_key(&slot.annotation);
                if slot.view.is_some() && !pins.iter().any(|(painted, _, _)| *painted == key) {
                    pin_paint.outside.insert(key);
                }
            }
        }
        host.0.borrow_mut().pin_paint = pin_paint;
        let surface_id = entity.entity_id();
        let pin_elements = pins.into_iter().map(|(key, view, frame)| {
            let select = entity.clone();
            let pin = div()
                .id(ElementId::Integer(key as u64))
                .absolute()
                .left(px(snap(frame.origin.x) as f32))
                .top(px(snap(frame.origin.y) as f32))
                .w(px(frame.width as f32))
                .h(px(frame.height as f32))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_: &ClickEvent, window, cx| {
                    select.update(cx, |surface, cx| surface.select(key, window, cx));
                })
                .child(view.view.cached(StyleRefinement::default().size_full()).replay_when_moved());
            PinShiftKeyed { key: pin_shift_key(surface_id, key), child: pin.into_any_element() }
        });

        let down = entity.clone();
        let right_down = entity.clone();
        let moved = entity.clone();
        let up = entity.clone();
        let right_up = entity.clone();
        let wheel = entity.clone();
        let pinch = entity.clone();
        let key = entity.clone();
        let wheel_host = host.clone();
        let pinch_host = host.clone();
        let key_host = host.clone();
        div()
            .id(id)
            .track_focus(&focus_handle)
            .relative()
            .size_full()
            .overflow_hidden()
            .children(image.map(|image| MapFrameSwapKeyed {
                key: map_frame_swap_key(entity.entity_id()),
                child: img(image).absolute().size_full().object_fit(ObjectFit::Cover).into_any_element(),
            }))
            .child(probe)
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                down.update(cx, |surface, cx| {
                    window.focus(&surface.focus_handle, cx);
                    let at = surface.host.convert_window_point(event.position);
                    let modifiers = Self::modifiers(&event.modifiers);
                    let input = if event.click_count == 2 {
                        MapInput::DoubleClick { point: at, modifiers }
                    } else {
                        MapInput::DragStart { point: at, button: DragButton::Primary, modifiers }
                    };
                    surface.press(at, MouseButton::Left, input, cx);
                });
            })
            .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, _, cx| {
                right_down.update(cx, |surface, cx| {
                    let at = surface.host.convert_window_point(event.position);
                    let modifiers = Self::modifiers(&event.modifiers);
                    let input = MapInput::DragStart { point: at, button: DragButton::Secondary, modifiers };
                    surface.press(at, MouseButton::Right, input, cx);
                });
            })
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                moved.update(cx, |surface, cx| {
                    let at = surface.host.convert_window_point(event.position);
                    surface.moved(at, cx);
                });
            })
            .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, window, cx| {
                up.update(cx, |surface, cx| surface.release(MouseButton::Left, window, cx));
            })
            .on_mouse_up(MouseButton::Right, move |_: &MouseUpEvent, window, cx| {
                right_up.update(cx, |surface, cx| surface.release(MouseButton::Right, window, cx));
            })
            .on_scroll_wheel(move |event: &ScrollWheelEvent, _, cx| {
                let at = wheel_host.convert_window_point(event.position);
                let delta = match event.delta {
                    ScrollDelta::Pixels(delta) => WheelDelta::Pixels {
                        x: f64::from(f32::from(delta.x)),
                        y: f64::from(f32::from(delta.y)),
                    },
                    ScrollDelta::Lines(delta) => WheelDelta::Lines { x: f64::from(delta.x), y: f64::from(delta.y) },
                };
                let modifiers = Self::modifiers(&event.modifiers);
                wheel.update(cx, |surface, cx| {
                    surface.host.handle_input(MapInput::Wheel { point: at, delta, modifiers }, true, cx);
                });
            })
            .on_pinch(move |event: &PinchEvent, _, cx| {
                let at = pinch_host.convert_window_point(event.position);
                pinch.update(cx, |surface, cx| {
                    surface.host.handle_input(MapInput::Pinch { point: at, delta: f64::from(event.delta) }, true, cx);
                });
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                let Some(map_key) = Self::map_key(&keystroke.key) else {
                    return;
                };
                let modifiers = Self::modifiers(&keystroke.modifiers);
                if let Some(levels) = Self::globe_zoom_levels(&key_host, &map_key, cx) {
                    cx.stop_propagation();
                    MapZoomButtons::zoom(levels, window, cx);
                    return;
                }
                key.update(cx, |surface, cx| {
                    surface.host.handle_input(MapInput::Key { key: map_key, modifiers }, true, cx);
                });
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom(px(pin_clip_bottom))
                    .overflow_hidden()
                    .children(pin_elements),
            )
            .children(subviews.into_iter().map(|view| div().absolute().inset_0().child(view)))
            .into_any_element()
    }
}

fn x473_mark(message: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X473", "{}", message());
}

/// key the map frame is painted under so a new frame can replace it on screen
fn map_frame_swap_key(surface: EntityId) -> u32 {
    (surface.as_u64() as u32).wrapping_add(1)
}

struct MapFrameSwapKeyed {
    key: u32,
    child: AnyElement,
}

impl IntoElement for MapFrameSwapKeyed {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MapFrameSwapKeyed {
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
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_image_swap_key(self.key, |window| self.child.paint(window, cx));
    }
}

struct PinShiftKeyed {
    key: u64,
    child: AnyElement,
}

impl IntoElement for PinShiftKeyed {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PinShiftKeyed {
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
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_shift_group(self.key, |window| self.child.paint(window, cx));
    }
}
