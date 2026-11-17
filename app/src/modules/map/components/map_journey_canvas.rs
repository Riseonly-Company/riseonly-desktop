use std::cell::RefCell;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext, BorderStyle, Bounds, Context, ElementId, Entity, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, PinchEvent, Render,
    RenderOnce, ScrollDelta, ScrollWheelEvent, Styled, Subscription, Task, Window, black, canvas, div, point, px,
    quad, size, white,
};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::map_view::{InputModifiers, LatLng, MapInput, ScreenPoint, ScreenRect, WheelDelta};
use rise_theme::RiseThemeAppearance;
use rise_theme::ui_color_system::UIColor;
use serde_json::{Value, json};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::modules::map::components::pins::map_friend_pin_view::MapPinPalette;
use crate::modules::map::components::pins::map_photo_pin_layer::{MapPhotoAnnotation, MapPhotoPinLayer, MapPhotoPinView};
use crate::modules::map::components::rise_map_host::{
    MapAnnotation, MapAnnotationView, RiseMapHost, RiseMapSurface, RiseMapViewDelegate, WeakRiseMapHost,
};
use crate::modules::map::components::rise_map_view::RiseMapCamera;
use crate::modules::map::components::tracks::map_track_layer::{MapTrackHomeExclusion, MapTrackLayer, MapTrackLayerInput};
use crate::modules::map::engine::core::map_track_heat::MapHeatStretch;
use crate::modules::map::engine::core::map_track_tile_store::MapTrackTileStore;
use crate::modules::map::engine::map_journey_geometry::MapJourneyGeometry;
use crate::modules::map::engine::map_track_erasure_geometry::{MapErasePoint, MapEraseStroke};
use crate::modules::map::engine::rise_map_journeys_repository::{MapJourneyPoint, MapJourneySnapshot};
use crate::modules::map::engine::rise_map_tracks_repository::{RiseMapTrackTilesTick, RiseMapTracksSnapshot};
use crate::modules::map::shared::rise_map_style::RiseMapStyle;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;
use crate::modules::map::stores::map_actions::map_types::MapSettings;

const AREA_SOURCE: &str = "journey-area";
const ROUTES_SOURCE: &str = "journey-routes";
const FILL_LAYER: &str = "journey-fill";
const BORDER_LAYER: &str = "journey-border";
const LINES_LAYER: &str = "journey-lines";

#[derive(Clone, IntoElement)]
pub struct MapJourneyCanvas {
    id: ElementId,
    pub appearance: RiseThemeAppearance,
    pub language_code: String,
    pub pin_palette: MapPinPalette,
    pub drawing: bool,
    pub polygon: Vec<MapJourneyPoint>,
    pub snapshot: Option<MapJourneySnapshot>,
    pub heated_lines: Vec<MapHeatStretch>,
    pub grids: Vec<MapPhotoGrid>,
    pub selected: HashSet<String>,
    pub tracks: Option<RiseMapTracksSnapshot>,
    pub track_store: Option<Arc<MapTrackTileStore>>,
    pub tick: RiseMapTrackTilesTick,
    pub home_track_radius_meters: i64,
    pub on_outline: Rc<dyn Fn(Vec<MapJourneyPoint>, &mut Window, &mut App)>,
    pub on_invalid: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_grid: Rc<dyn Fn(String, &mut Window, &mut App)>,
}

impl MapJourneyCanvas {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        appearance: RiseThemeAppearance,
        language_code: impl Into<String>,
        pin_palette: MapPinPalette,
        drawing: bool,
        polygon: Vec<MapJourneyPoint>,
        snapshot: Option<MapJourneySnapshot>,
        grids: Vec<MapPhotoGrid>,
        selected: HashSet<String>,
        tracks: Option<RiseMapTracksSnapshot>,
        tick: RiseMapTrackTilesTick,
        on_outline: Rc<dyn Fn(Vec<MapJourneyPoint>, &mut Window, &mut App)>,
        on_invalid: Rc<dyn Fn(&mut Window, &mut App)>,
        on_grid: Rc<dyn Fn(String, &mut Window, &mut App)>,
    ) -> Self {
        Self {
            id: id.into(),
            appearance,
            language_code: language_code.into(),
            pin_palette,
            drawing,
            polygon,
            snapshot,
            heated_lines: Vec::new(),
            grids,
            selected,
            tracks,
            track_store: None,
            tick,
            home_track_radius_meters: MapSettings::default_home_track_radius(),
            on_outline,
            on_invalid,
            on_grid,
        }
    }

    fn make_ui_view(&self, cx: &mut App) -> Entity<RiseMapSurface<MapJourneyCanvasCoordinator>> {
        let map = RiseMapHost::new(RiseMapStyle::json(self.appearance, &self.language_code), self.appearance);
        map.set_shows_compass_view(false);
        RiseMapCamera::restore(&map, cx);
        let weak_map = map.downgrade();
        let overlay = cx.new(|cx| JourneyStrokeView::new(Some(weak_map), cx));
        map.add_subview(overlay.clone().into());
        let mut coordinator = MapJourneyCanvasCoordinator::new();
        coordinator.stroke = Some(overlay);
        cx.new(|cx| RiseMapSurface::new(map, coordinator, cx))
    }

    fn update_ui_view(&self, map: &RiseMapHost, c: &mut MapJourneyCanvasCoordinator, cx: &mut App) {
        c.input = Some(self.clone());
        map.set_shows_compass_view(!self.drawing);
        if let Some(stroke) = c.stroke.clone() {
            map.set_subview_hidden(stroke.entity_id(), !self.drawing);
            let drawing = self.drawing;
            let weak_map = map.downgrade();
            let on_outline = self.on_outline.clone();
            let on_invalid = self.on_invalid.clone();
            stroke.update(cx, |stroke, cx| {
                if !drawing {
                    stroke.cancel(cx);
                }
                stroke.map = Some(weak_map);
                stroke.on_outline = on_outline;
                stroke.on_invalid = on_invalid;
            });
        }
        map.set_scroll_enabled(!self.drawing);
        map.set_zoom_enabled(!self.drawing);
        map.set_rotate_enabled(!self.drawing);
        map.set_pitch_enabled(!self.drawing);
        c.render(map, cx);
        if self.drawing
            && let Some(stroke) = &c.stroke
        {
            map.bring_subview_to_front(stroke.entity_id());
        }
    }
}

impl RenderOnce for MapJourneyCanvas {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let surface = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| self.make_ui_view(cx))
            .read(cx)
            .clone();
        surface.update(cx, |surface, cx| {
            let map = surface.host.clone();
            self.update_ui_view(&map, &mut surface.delegate, cx);
        });
        RiseMapSurface::element(&surface, self.id.clone(), window, cx)
    }
}

pub struct MapJourneyCanvasCoordinator {
    pub input: Option<MapJourneyCanvas>,
    pub stroke: Option<Entity<JourneyStrokeView>>,
    pub fitted_id: Option<String>,
    pub geometry_key: Option<u64>,
    pub track_layer: MapTrackLayer,
    pub photos: Rc<RefCell<MapPhotoPinLayer>>,
}

impl Default for MapJourneyCanvasCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

fn hash_color(color: Hsla, hasher: &mut DefaultHasher) {
    for component in [color.h, color.s, color.l, color.a] {
        component.to_bits().hash(hasher);
    }
}

fn hash_points(points: &[MapJourneyPoint], hasher: &mut DefaultHasher) {
    points.len().hash(hasher);
    for point in points {
        point.lat.to_bits().hash(hasher);
        point.lng.to_bits().hash(hasher);
    }
}

fn line_feature(coordinates: Vec<[f64; 2]>, properties: Value) -> Value {
    json!({ "type": "Feature", "properties": properties, "geometry": { "type": "LineString", "coordinates": coordinates } })
}

impl MapJourneyCanvasCoordinator {
    pub fn new() -> Self {
        Self {
            input: None,
            stroke: None,
            fitted_id: None,
            geometry_key: None,
            track_layer: MapTrackLayer::new(),
            photos: MapPhotoPinLayer::new(),
        }
    }

    pub fn render(&mut self, map: &RiseMapHost, cx: &mut App) {
        let Some(input) = self.input.clone() else {
            return;
        };
        if !map.is_style_loaded() {
            return;
        }
        if let Some(tracks) = &input.tracks {
            self.track_layer.provider.configure(input.track_store.clone());
            let since_day = tracks
                .summary
                .as_ref()
                .and_then(|summary| summary.stats(tracks.period))
                .map_or(0, |stats| stats.since_day);
            let home_exclusion = if tracks.owner_id.is_none() {
                MapTrackHomeExclusion::new(tracks.home.as_ref(), input.home_track_radius_meters)
            } else {
                None
            };
            let bands = tracks.summary.as_ref().map(|summary| summary.bands.clone()).unwrap_or_default();
            self.track_layer.update(
                &MapTrackLayerInput { is_visible: true, since_day, tail: Vec::new(), home_exclusion, place_exclusions: Vec::new() },
                &input.tick,
                &bands,
                map,
                cx,
            );
        }
        let mut hasher = DefaultHasher::new();
        hash_points(&input.polygon, &mut hasher);
        input.snapshot.as_ref().map(|snapshot| snapshot.id.clone()).hash(&mut hasher);
        input.snapshot.as_ref().and_then(|snapshot| snapshot.show_heat).hash(&mut hasher);
        input.heated_lines.len().hash(&mut hasher);
        input.grids.hash(&mut hasher);
        let mut selected: Vec<&String> = input.selected.iter().collect();
        selected.sort();
        selected.hash(&mut hasher);
        input.drawing.hash(&mut hasher);
        hash_color(input.pin_palette.primary, &mut hasher);
        hash_color(input.pin_palette.stale, &mut hasher);
        let key = hasher.finish();
        if self.geometry_key == Some(key) {
            return;
        }
        self.geometry_key = Some(key);
        let outline = input
            .snapshot
            .as_ref()
            .map_or_else(|| input.polygon.clone(), |snapshot| snapshot.polygon.clone());
        let polygon = (outline.len() >= 3).then(|| {
            let mut ring: Vec<[f64; 2]> = outline.iter().map(|point| [point.lng, point.lat]).collect();
            ring.push(ring[0]);
            ring
        });
        map.set_shape(
            AREA_SOURCE,
            polygon.as_ref().map(|ring| {
                json!({ "type": "Feature", "properties": {}, "geometry": { "type": "Polygon", "coordinates": [ring] } })
            }),
        );
        let blue = map.style_color(UIColor::system_blue());
        let fill_opacity = if input.snapshot.is_none() { 0.12 } else { 0.0 };
        let fill = json!({
            "id": FILL_LAYER,
            "type": "fill",
            "source": AREA_SOURCE,
            "paint": { "fill-color": blue, "fill-opacity": fill_opacity },
        });
        if map.has_layer(BORDER_LAYER) {
            map.add_layer(fill, Some(BORDER_LAYER));
        } else {
            map.add_layer(fill, None);
            map.add_layer(
                json!({
                    "id": BORDER_LAYER,
                    "type": "line",
                    "source": AREA_SOURCE,
                    "paint": { "line-color": blue, "line-width": 2 },
                }),
                None,
            );
        }
        let lines: Vec<Value> = input
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.lines.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|line| line.len() >= 2)
            .map(|line| line_feature(line.iter().map(|point| [point.lng, point.lat]).collect(), json!({})))
            .collect();
        let heated: Vec<Value> = input
            .heated_lines
            .iter()
            .filter_map(|stretch| {
                let coordinates: Vec<[f64; 2]> =
                    stretch.coordinates.chunks_exact(2).map(|pair| [pair[1], pair[0]]).collect();
                (coordinates.len() >= 2).then(|| {
                    let mut properties = serde_json::Map::new();
                    properties.insert(MapTrackLayer::TEMPERATURE_KEY.to_string(), json!(stretch.temperature));
                    line_feature(coordinates, Value::Object(properties))
                })
            })
            .collect();
        let is_heated = !heated.is_empty();
        map.set_shape(
            ROUTES_SOURCE,
            Some(json!({ "type": "FeatureCollection", "features": if is_heated { heated } else { lines } })),
        );
        let line_color = if is_heated { MapTrackLayer::temperature_color() } else { json!(blue) };
        let sort_key = if is_heated { json!(["get", MapTrackLayer::TEMPERATURE_KEY]) } else { json!(0) };
        map.add_layer(
            json!({
                "id": LINES_LAYER,
                "type": "line",
                "source": ROUTES_SOURCE,
                "layout": { "line-cap": "round", "line-join": "round", "line-sort-key": sort_key },
                "paint": { "line-color": line_color, "line-width": 3 },
            }),
            None,
        );
        let drawing = input.drawing;
        let on_grid = input.on_grid.clone();
        self.photos.borrow_mut().on_select = Rc::new(move |id: Option<String>, window: &mut Window, cx: &mut App| {
            if drawing {
                return;
            }
            if let Some(id) = id {
                on_grid(id, window, cx);
            }
        });
        self.photos
            .borrow_mut()
            .update(&input.grids, key, true, None, &input.pin_palette, map, cx);
        self.refresh_selection(map, cx);
        if let Some(snapshot) = &input.snapshot
            && self.fitted_id.as_deref() != Some(snapshot.id.as_str())
            && polygon.is_some()
        {
            self.fitted_id = Some(snapshot.id.clone());
            let south_west = LatLng::new(
                outline.iter().map(|point| point.lat).fold(f64::INFINITY, f64::min),
                outline.iter().map(|point| point.lng).fold(f64::INFINITY, f64::min),
            );
            let north_east = LatLng::new(
                outline.iter().map(|point| point.lat).fold(f64::NEG_INFINITY, f64::max),
                outline.iter().map(|point| point.lng).fold(f64::NEG_INFINITY, f64::max),
            );
            map.set_visible_coordinate_bounds(south_west, north_east, (100.0, 32.0, 260.0, 32.0), false, cx);
        }
    }

    fn refresh_selection(&self, map: &RiseMapHost, cx: &mut App) {
        for annotation in map.visible_annotations() {
            if let Some(view) = map.view_for(&annotation).and_then(|view| view.downcast::<MapPhotoPinView>().ok()) {
                self.decorate(&view, &annotation, cx);
            }
        }
    }

    fn decorate(&self, view: &Entity<MapPhotoPinView>, annotation: &Rc<dyn MapAnnotation>, cx: &mut App) {
        let (Some(input), Some(pin)) = (&self.input, annotation.as_any().downcast_ref::<MapPhotoAnnotation>()) else {
            return;
        };
        let chosen = pin.members.borrow().iter().any(|member| input.selected.contains(&member.id));
        let indicator = (input.snapshot.is_none() && !input.drawing).then_some(chosen);
        let color = input.pin_palette.primary;
        view.update(cx, |view, cx| view.set_selection_indicator(indicator, color, cx));
    }
}

impl RiseMapViewDelegate for MapJourneyCanvasCoordinator {
    fn did_finish_loading(&mut self, map: &RiseMapHost, _window: &mut Window, cx: &mut App) {
        self.geometry_key = None;
        self.render(map, cx);
    }

    fn view_for(&mut self, map: &RiseMapHost, annotation: &Rc<dyn MapAnnotation>, _window: &mut Window, cx: &mut App) -> Option<MapAnnotationView> {
        let view = self.photos.borrow().view(annotation, map, cx);
        if let Some(pin_view) = view.as_ref().and_then(|view| view.view.clone().downcast::<MapPhotoPinView>().ok()) {
            self.decorate(&pin_view, annotation, cx);
        }
        view
    }

    fn did_select(&mut self, map: &RiseMapHost, annotation: &Rc<dyn MapAnnotation>, window: &mut Window, cx: &mut App) {
        if self.input.as_ref().is_none_or(|input| input.drawing) {
            map.deselect_annotation(annotation);
            return;
        }
        let _ = self.photos.borrow_mut().did_select(annotation, map, window, cx);
    }

    fn region_did_change_animated(&mut self, map: &RiseMapHost, _animated: bool, _window: &mut Window, cx: &mut App) {
        self.track_layer.region_did_change(map, cx);
        self.photos.borrow_mut().region_did_change(map, cx);
        self.refresh_selection(map, cx);
    }

    fn region_is_changing_with(
        &mut self,
        map: &RiseMapHost,
        _reason: rise_media::map_view::CameraChangeReason,
        _window: &mut Window,
        cx: &mut App,
    ) {
        self.photos.borrow_mut().region_is_changing(map, cx);
        self.refresh_selection(map, cx);
    }

    fn dismantle(&mut self, map: &RiseMapHost, _cx: &mut App) {
        self.photos.borrow_mut().teardown(map);
        self.input = None;
    }
}

pub type JourneyOutline = Rc<dyn Fn(Vec<MapJourneyPoint>, &mut Window, &mut App)>;
pub type JourneyAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type JourneyEraseStroke = Rc<dyn Fn(MapEraseStroke, &mut Window, &mut App)>;
pub type JourneyErasePreview = Rc<dyn Fn(Option<MapEraseStroke>, &mut App)>;

/// draws a contour or erases routes with the pointer over the map
/// primary drag draws, wheel and pinch still move the camera and drop the uncommitted stroke
pub struct JourneyStrokeView {
    pub map: Option<WeakRiseMapHost>,
    pub on_outline: JourneyOutline,
    pub on_invalid: JourneyAction,
    pub erasing: bool,
    pub brush_diameter: f64,
    pub since_day: i32,
    pub accepts_stroke: bool,
    pub on_erase_preview: JourneyErasePreview,
    pub on_erase: JourneyEraseStroke,
    pub on_limit: JourneyAction,
    erase_stroke: Option<MapEraseStroke>,
    erase_touched_route: bool,
    cursor: Option<ScreenRect>,
    is_drawing: bool,
    points: Vec<ScreenPoint>,
    pending: Option<Task<()>>,
    overflow: bool,
    scene_activation: Option<Subscription>,
    scene_background: Option<Subscription>,
}

impl JourneyStrokeView {
    fn max_vertices() -> usize {
        usize::try_from(RiseLimits::service_value("map_journey_max_polygon_vertices")).unwrap_or(0)
    }

    fn delay() -> Duration {
        Duration::from_millis(u64::try_from(RiseLimits::service_value("map_journey_draw_debounce_ms")).unwrap_or(0))
    }

    pub fn new(map: Option<WeakRiseMapHost>, _cx: &mut Context<Self>) -> Self {
        Self {
            map,
            on_outline: Rc::new(|_: Vec<MapJourneyPoint>, _: &mut Window, _: &mut App| {}),
            on_invalid: Rc::new(|_: &mut Window, _: &mut App| {}),
            erasing: false,
            brush_diameter: 40.0,
            since_day: 0,
            accepts_stroke: true,
            on_erase_preview: Rc::new(|_: Option<MapEraseStroke>, _: &mut App| {}),
            on_erase: Rc::new(|_: MapEraseStroke, _: &mut Window, _: &mut App| {}),
            on_limit: Rc::new(|_: &mut Window, _: &mut App| {}),
            erase_stroke: None,
            erase_touched_route: false,
            cursor: None,
            is_drawing: false,
            points: Vec::new(),
            pending: None,
            overflow: false,
            scene_activation: None,
            scene_background: None,
        }
    }

    fn live_map(&self) -> Option<RiseMapHost> {
        self.map.as_ref().and_then(WeakRiseMapHost::upgrade)
    }

    /// drops the uncommitted stroke and cancels an erase preview in flight
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        let changed = self.pending.is_some()
            || self.is_drawing
            || !self.points.is_empty()
            || self.cursor.is_some()
            || self.erase_stroke.is_some();
        self.pending = None;
        self.is_drawing = false;
        self.points.clear();
        self.cursor = None;
        if self.erase_stroke.take().is_some() {
            (self.on_erase_preview.clone())(None, cx);
        }
        if changed {
            cx.notify();
        }
    }

    /// moves the camera like the stroke view's own two finger gestures, the stroke is dropped
    fn move_camera(&mut self, input: MapInput, cx: &mut Context<Self>) {
        self.cancel(cx);
        if let Some(map) = self.live_map() {
            map.handle_input(input, false, cx);
        }
    }

    fn touches_began(&mut self, at: ScreenPoint, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel(cx);
        if self.scene_activation.is_none() {
            self.scene_activation = Some(cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    tracing::warn!(target: "riseonly::X2084", drawing = this.is_drawing, "stroke cancel on window deactivation");
                    this.cancel(cx);
                }
            }));
        }
        if self.scene_background.is_none() {
            let gate = RiseMediaForegroundGate::shared(cx);
            self.scene_background = Some(cx.observe(&gate, |this, gate, cx| {
                if !gate.read(cx).is_foreground() {
                    tracing::warn!(target: "riseonly::X2084", drawing = this.is_drawing, "stroke cancel on background");
                    this.cancel(cx);
                }
            }));
        }
        self.overflow = false;
        if self.erasing && !self.accepts_stroke {
            (self.on_limit.clone())(window, cx);
            return;
        }
        if !self.accepts_stroke {
            return;
        }
        self.is_drawing = true;
        self.points = vec![at];
        if self.erasing
            && let Some(map) = self.live_map()
            && let (Some(centre), Some(edge)) = (
                map.convert_point_to_coordinate(at),
                map.convert_point_to_coordinate(ScreenPoint::new(at.x + self.brush_diameter / 2.0, at.y)),
            )
        {
            let centre = MapErasePoint::new(centre);
            let edge = MapErasePoint::new(edge);
            let dx = edge.x - centre.x - (edge.x - centre.x).round();
            let radius = MapEraseStroke::maximum_radius().min(dx.hypot(edge.y - centre.y));
            self.erase_stroke = Some(MapEraseStroke {
                id: Uuid::new_v4().uuid_string().to_lowercase(),
                since_day: self.since_day,
                until_ms: (Date::now().time_interval_since_1970() * 1000.0) as i64,
                radius,
                points: vec![centre],
            });
            self.erase_touched_route = false;
            self.preview_erase(at, None, cx);
        }
        cx.notify();
    }

    fn touches_moved(&mut self, at: ScreenPoint, cx: &mut Context<Self>) {
        let Some(last) = self.points.last().copied() else {
            return;
        };
        if !self.is_drawing {
            return;
        }
        let step = if self.erasing { (self.brush_diameter / 8.0).max(3.0) } else { 3.0 };
        if (at.x - last.x).hypot(at.y - last.y) < step {
            return;
        }
        if self.erasing {
            let Some(map) = self.live_map() else {
                return;
            };
            let Some(mut stroke) = self.erase_stroke.clone() else {
                return;
            };
            if stroke.points.len() as i64 >= MapEraseStroke::maximum_points() {
                self.overflow = true;
                return;
            }
            let Some(coordinate) = map.convert_point_to_coordinate(at) else {
                return;
            };
            stroke.points.push(MapErasePoint::new(coordinate));
            self.erase_stroke = Some(stroke);
            self.points = vec![at];
            self.preview_erase(at, Some(last), cx);
            return;
        }
        if self.points.len() >= Self::max_vertices() * 8 {
            self.overflow = true;
            return;
        }
        self.points.push(at);
        cx.notify();
    }

    fn preview_erase(&mut self, at: ScreenPoint, previous: Option<ScreenPoint>, cx: &mut Context<Self>) {
        let (Some(map), Some(stroke)) = (self.live_map(), self.erase_stroke.clone()) else {
            return;
        };
        let reach = self.brush_diameter / 2.0;
        let previous = previous.unwrap_or(at);
        let swept = ScreenRect {
            origin: ScreenPoint::new(at.x.min(previous.x) - reach, at.y.min(previous.y) - reach),
            width: (at.x - previous.x).abs() + self.brush_diameter,
            height: (at.y - previous.y).abs() + self.brush_diameter,
        };
        self.erase_touched_route =
            self.erase_touched_route || !map.visible_features(swept, MapTrackLayer::TAPPABLE_LAYER_IDS).is_empty();
        self.cursor = Some(ScreenRect {
            origin: ScreenPoint::new(at.x - reach, at.y - reach),
            width: self.brush_diameter,
            height: self.brush_diameter,
        });
        (self.on_erase_preview.clone())(Some(stroke), cx);
        cx.notify();
    }

    fn touches_ended(&mut self, at: ScreenPoint, window: &mut Window, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X2084", drawing = self.is_drawing, "stroke touches ended");
        if !self.is_drawing {
            return;
        }
        self.is_drawing = false;
        if self.erasing {
            if self.overflow {
                self.cancel(cx);
                (self.on_limit.clone())(window, cx);
                cx.notify();
                return;
            }
            if let (Some(mut stroke), Some(map), Some(previous)) =
                (self.erase_stroke.clone(), self.live_map(), self.points.last().copied())
                && (at.x - previous.x).hypot(at.y - previous.y) > 0.0
            {
                if stroke.points.len() as i64 >= MapEraseStroke::maximum_points() {
                    self.cancel(cx);
                    (self.on_limit.clone())(window, cx);
                    cx.notify();
                    return;
                }
                if let Some(coordinate) = map.convert_point_to_coordinate(at) {
                    stroke.points.push(MapErasePoint::new(coordinate));
                    self.erase_stroke = Some(stroke);
                    self.preview_erase(at, Some(previous), cx);
                }
            }
            let stroke = self.erase_stroke.take();
            let touched = self.erase_touched_route;
            self.cursor = None;
            self.points.clear();
            if let Some(stroke) = stroke
                && touched
            {
                (self.on_erase.clone())(stroke, window, cx);
            }
            (self.on_erase_preview.clone())(None, cx);
            cx.notify();
            return;
        }
        if self.live_map().is_none() || self.overflow {
            self.cancel(cx);
            (self.on_invalid.clone())(window, cx);
            cx.notify();
            return;
        }
        let mut captured = self.points.clone();
        captured.push(at);
        let vertex_budget = Self::max_vertices() / 2;
        let delay = Self::delay();
        self.pending = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let ring = cx
                .background_executor()
                .spawn(async move { MapJourneyGeometry::closed_stroke(&captured, 32.0, vertex_budget) })
                .await;
            let sampled = this.update_in(cx, |this, window, cx| {
                let map = this.live_map()?;
                let Some(ring) = ring else {
                    this.cancel(cx);
                    (this.on_invalid.clone())(window, cx);
                    cx.notify();
                    return None;
                };
                Some(
                    ring.iter()
                        .filter_map(|point| map.convert_point_to_coordinate(*point))
                        .map(|coordinate| MapJourneyPoint { lat: coordinate.latitude, lng: coordinate.longitude })
                        .collect::<Vec<_>>(),
                )
            });
            let Ok(Some(sampled)) = sampled else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { MapJourneyGeometry::smooth(&sampled) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Some(result) => {
                        this.points.clear();
                        (this.on_outline.clone())(result, window, cx);
                    }
                    None => {
                        this.cancel(cx);
                        (this.on_invalid.clone())(window, cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn local(&self, position: gpui::Point<gpui::Pixels>) -> Option<ScreenPoint> {
        self.live_map().map(|map| map.convert_window_point(position))
    }
}

impl Render for JourneyStrokeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let appearance = self.live_map().map_or(RiseThemeAppearance::Light, |map| map.appearance());
        let ink_color = UIColor::system_blue().resolved_color(appearance);
        let ink: Vec<ScreenPoint> = if self.erasing { Vec::new() } else { self.points.clone() };
        let cursor = self.cursor;
        let paint = canvas(
            |_, _, _| {},
            move |bounds: Bounds<gpui::Pixels>, _, window, _| {
                let at = |p: &ScreenPoint| point(bounds.origin.x + px(p.x as f32), bounds.origin.y + px(p.y as f32));
                if ink.len() >= 2 {
                    let mut path = PathBuilder::stroke(px(3.0));
                    path.move_to(at(&ink[0]));
                    for p in &ink[1..] {
                        path.line_to(at(p));
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, ink_color);
                    }
                }
                if let Some(cursor) = cursor {
                    let side = px(cursor.width as f32);
                    let origin = at(&cursor.origin);
                    window.paint_quad(quad(
                        Bounds { origin, size: size(side, side) },
                        side / 2.0,
                        white().opacity(0.8),
                        px(1.0),
                        black().opacity(0.25),
                        BorderStyle::Solid,
                    ));
                }
            },
        )
        .size_full();
        div()
            .id("journey-stroke")
            .size_full()
            .child(paint)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if let Some(at) = this.local(event.position) {
                    this.touches_began(at, window, cx);
                }
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if let Some(at) = this.local(event.position) {
                    this.touches_moved(at, cx);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, event: &MouseUpEvent, window, cx| {
                if let Some(at) = this.local(event.position) {
                    this.touches_ended(at, window, cx);
                }
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                let Some(at) = this.local(event.position) else {
                    return;
                };
                let delta = match event.delta {
                    ScrollDelta::Pixels(delta) => {
                        WheelDelta::Pixels { x: f64::from(f32::from(delta.x)), y: f64::from(f32::from(delta.y)) }
                    }
                    ScrollDelta::Lines(delta) => WheelDelta::Lines { x: f64::from(delta.x), y: f64::from(delta.y) },
                };
                this.move_camera(MapInput::Wheel { point: at, delta, modifiers: InputModifiers::default() }, cx);
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| {
                cx.stop_propagation();
                if let Some(at) = this.local(event.position) {
                    this.move_camera(MapInput::Pinch { point: at, delta: f64::from(event.delta) }, cx);
                }
            }))
    }
}
