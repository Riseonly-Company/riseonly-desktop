use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{App, AppContext, AsyncApp, Entity, WeakEntity, Window};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_theme::ui_color_system::UIColor;
use serde_json::json;

use crate::modules::map::components::map_editing_session::{MapEditingMode, MapEditingSession};
use crate::modules::map::components::map_journey_canvas::JourneyStrokeView;
use crate::modules::map::components::rise_map_host::RiseMapHost;
use crate::modules::map::components::tracks::map_track_layer::MapTrackLayer;
use crate::modules::map::engine::map_track_erasure_geometry::MapEraseStroke;
use crate::modules::map::engine::rise_map_journeys_repository::MapJourneyPoint;
use rise_media::map_view::{LatLng, Projection, ScreenPoint};

const OUTLINE_SOURCE: &str = "map.edit.outline";
const FILL_LAYER: &str = "map.edit.fill";
const BORDER_LAYER: &str = "map.edit.border";

static STROKE_ERROR_GENERATION: AtomicU64 = AtomicU64::new(0);

pub struct MapEditingLayer {
    stroke: Option<Entity<JourneyStrokeView>>,
    pending_stroke: Rc<RefCell<Option<MapEraseStroke>>>,
    cancellation: u64,
    since_day: i32,
    session_id: Option<Uuid>,
    polygon: Vec<MapJourneyPoint>,
}

impl Default for MapEditingLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl MapEditingLayer {
    const STROKE_ERROR_MS: u64 = 3_200;

    const REVEAL_MARGIN: f64 = 24.0;

    fn reveal_outline(map: &RiseMapHost, outline: &[MapJourneyPoint], cx: &mut App) {
        let Some(viewport) = map.bounds() else {
            return;
        };
        let points: Vec<ScreenPoint> = outline
            .iter()
            .filter_map(|point| map.convert_coordinate_to_point(LatLng::new(point.lat, point.lng)))
            .collect();
        if points.is_empty() || points.len() != outline.len() {
            return;
        }
        let margin = Self::REVEAL_MARGIN;
        let inset = f64::from(MapEditingSession::OUTLINED_JOURNEY_INSET);
        let (top, bottom) = (margin, viewport.height - inset - margin);
        let (left, right) = (margin, viewport.width - margin);
        if bottom <= top || right <= left {
            return;
        }
        let min_x = points.iter().map(|point| point.x).fold(f64::INFINITY, f64::min);
        let max_x = points.iter().map(|point| point.x).fold(f64::NEG_INFINITY, f64::max);
        let min_y = points.iter().map(|point| point.y).fold(f64::INFINITY, f64::min);
        let max_y = points.iter().map(|point| point.y).fold(f64::NEG_INFINITY, f64::max);
        if min_x >= left && max_x <= right && min_y >= top && max_y <= bottom {
            return;
        }
        let (width, height) = (max_x - min_x, max_y - min_y);
        if width <= right - left && height <= bottom - top {
            let middle = ScreenPoint::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
            let Some(anchor) = map.convert_point_to_coordinate(middle) else {
                return;
            };
            let at = ScreenPoint::new(middle.x.clamp(left + width / 2.0, right - width / 2.0), (top + bottom) / 2.0);
            if let Some(camera) = Projection::new(map.camera(), viewport).place_under(anchor, at) {
                map.set_camera(camera, true, cx);
            }
            return;
        }
        let south = outline.iter().map(|point| point.lat).fold(f64::INFINITY, f64::min);
        let north = outline.iter().map(|point| point.lat).fold(f64::NEG_INFINITY, f64::max);
        let west = outline.iter().map(|point| point.lng).fold(f64::INFINITY, f64::min);
        let east = outline.iter().map(|point| point.lng).fold(f64::NEG_INFINITY, f64::max);
        map.set_visible_coordinate_bounds(
            LatLng::new(south, west),
            LatLng::new(north, east),
            (margin, margin, inset + margin, margin),
            true,
            cx,
        );
    }

    fn show_stroke_error(session: &Entity<MapEditingSession>, text: String, cx: &mut App) {
        session.update(cx, |session, cx| {
            session.error = Some(text.clone());
            cx.notify();
        });
        let generation = STROKE_ERROR_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        let weak: WeakEntity<MapEditingSession> = session.downgrade();
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(Self::STROKE_ERROR_MS)).await;
            if STROKE_ERROR_GENERATION.load(Ordering::Relaxed) != generation {
                return;
            }
            let _ = weak.update(cx, |session, cx| {
                if session.error.as_deref() == Some(text.as_str()) {
                    session.error = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn new() -> Self {
        Self {
            stroke: None,
            pending_stroke: Rc::new(RefCell::new(None)),
            cancellation: 0,
            since_day: 0,
            session_id: None,
            polygon: Vec::new(),
        }
    }

    pub fn update(
        &mut self,
        session: Option<&Entity<MapEditingSession>>,
        since_day: i32,
        map: &RiseMapHost,
        tracks: &mut MapTrackLayer,
        cx: &mut App,
    ) {
        let state = session.map(|session| {
            let session = session.read(cx);
            (
                session.id,
                session.stroke_cancellation,
                !session.saving
                    && session.pending_save.is_none()
                    && (session.drawing() || session.mode == MapEditingMode::Eraser),
                session.mode == MapEditingMode::Eraser,
                session.brush_diameter(),
                !session.saving && (session.strokes.len() as i64) < MapEraseStroke::maximum_strokes(),
                session.strokes.clone(),
                session.polygon.clone(),
            )
        });
        let session_id = state.as_ref().map(|state| state.0);
        let cancellation = state.as_ref().map_or(0, |state| state.1);
        if self.session_id != session_id || self.cancellation != cancellation || self.since_day != since_day {
            if let Some(stroke) = &self.stroke {
                stroke.update(cx, |stroke, cx| stroke.cancel(cx));
            }
            *self.pending_stroke.borrow_mut() = None;
            self.session_id = session_id;
            self.cancellation = cancellation;
            self.since_day = since_day;
        }
        let active = state.as_ref().is_some_and(|state| state.2);
        map.set_scroll_enabled(!active);
        map.set_zoom_enabled(!active);
        map.set_rotate_enabled(!active);
        map.set_pitch_enabled(!active);
        if session.is_some() && self.stroke.is_none() {
            let weak_map = map.downgrade();
            let view = cx.new(|cx| JourneyStrokeView::new(Some(weak_map), cx));
            map.add_subview(view.clone().into());
            self.stroke = Some(view);
        }
        if let Some(stroke) = self.stroke.clone() {
            map.set_subview_hidden(stroke.entity_id(), !active);
            let erasing = state.as_ref().is_some_and(|state| state.3);
            let brush_diameter = state.as_ref().map_or(40.0, |state| state.4);
            let accepts_stroke = state.as_ref().is_some_and(|state| state.5);
            let weak_session = session.map(Entity::downgrade);
            let pending = self.pending_stroke.clone();
            let preview_map = map.downgrade();
            stroke.update(cx, |stroke, cx| {
                if !active {
                    stroke.cancel(cx);
                }
                stroke.erasing = erasing;
                stroke.brush_diameter = brush_diameter;
                stroke.since_day = since_day;
                stroke.accepts_stroke = accepts_stroke;
                let outline_session = weak_session.clone();
                let outline_map = map.downgrade();
                stroke.on_outline = Rc::new(move |outline: Vec<MapJourneyPoint>, _: &mut Window, cx: &mut App| {
                    let Some(session) = outline_session.as_ref().and_then(|session| session.upgrade()) else {
                        return;
                    };
                    let accepted = session.update(cx, |session, cx| {
                        if !session.is_current() {
                            return false;
                        }
                        session.set_polygon(outline.clone());
                        session.set_selected(HashSet::new());
                        session.error = None;
                        cx.notify();
                        true
                    });
                    if accepted && let Some(map) = outline_map.upgrade() {
                        Self::reveal_outline(&map, &outline, cx);
                    }
                });
                let invalid_session = weak_session.clone();
                stroke.on_invalid = Rc::new(move |_: &mut Window, cx: &mut App| {
                    if let Some(session) = invalid_session.as_ref().and_then(|session| session.upgrade()) {
                        Self::show_stroke_error(&session, tr("map_journey_outline_error"), cx);
                    }
                });
                let limit_session = weak_session.clone();
                stroke.on_limit = Rc::new(move |_: &mut Window, cx: &mut App| {
                    if let Some(session) = limit_session.as_ref().and_then(|session| session.upgrade()) {
                        Self::show_stroke_error(&session, tr("map_eraser_limit"), cx);
                    }
                });
                let erase_session = weak_session;
                stroke.on_erase = Rc::new(move |erased: MapEraseStroke, _: &mut Window, cx: &mut App| {
                    if let Some(session) = erase_session.as_ref().and_then(|session| session.upgrade()) {
                        session.update(cx, |session, cx| {
                            session.append(erased);
                            cx.notify();
                        });
                    }
                });
                stroke.on_erase_preview = Rc::new(move |preview: Option<MapEraseStroke>, cx: &mut App| {
                    *pending.borrow_mut() = preview;
                    if let Some(map) = preview_map.upgrade() {
                        map.set_needs_display(cx);
                    }
                });
            });
            if active {
                map.bring_subview_to_front(stroke.entity_id());
            }
        }
        let mut erasures = state.as_ref().map(|state| state.6.clone()).unwrap_or_default();
        erasures.extend(self.pending_stroke.borrow().iter().cloned());
        tracks.set_erasures(erasures, map, cx);
        if session.is_none() && self.polygon.is_empty() {
            return;
        }
        let outline = state.map(|state| state.7).unwrap_or_default();
        if !map.is_style_loaded() || (self.polygon == outline && map.has_source(OUTLINE_SOURCE)) {
            return;
        }
        self.polygon = outline.clone();
        let shape = (outline.len() >= 3).then(|| {
            let mut ring: Vec<[f64; 2]> = outline.iter().map(|point| [point.lng, point.lat]).collect();
            ring.push(ring[0]);
            json!({ "type": "Feature", "properties": {}, "geometry": { "type": "Polygon", "coordinates": [ring] } })
        });
        let fresh = !map.has_source(OUTLINE_SOURCE);
        map.set_shape(OUTLINE_SOURCE, shape);
        if fresh {
            let blue = map.style_color(UIColor::system_blue());
            map.add_layer(
                json!({
                    "id": FILL_LAYER,
                    "type": "fill",
                    "source": OUTLINE_SOURCE,
                    "paint": { "fill-color": blue, "fill-opacity": 0.12 },
                }),
                None,
            );
            map.add_layer(
                json!({
                    "id": BORDER_LAYER,
                    "type": "line",
                    "source": OUTLINE_SOURCE,
                    "paint": { "line-color": blue, "line-width": 2 },
                }),
                None,
            );
        }
    }

    pub fn cancel(&self, cx: &mut App) {
        if let Some(stroke) = &self.stroke {
            stroke.update(cx, |stroke, cx| stroke.cancel(cx));
        }
    }
}
