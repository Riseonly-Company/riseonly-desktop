use std::rc::Rc;

use gpui::{App, ElementId, Entity, IntoElement, ParentElement, RenderOnce, Styled, StyleRefinement, Window, div};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::map_view::LatLng;

use crate::modules::map::components::globe::map_globe_view::{MapGlobePerson, MapGlobeView};
use crate::modules::map::components::pins::map_friend_pin_view::MapPinPalette;

/// where the globe opens, a new id is a new entry even on the same spot
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapGlobeEntry {
    pub id: Uuid,
    pub latitude: f64,
    pub longitude: f64,
    pub zoom_level: Option<f64>,
}

/// moves the flat map camera once per id, the way back from the globe
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapCameraRequest {
    pub id: Uuid,
    pub latitude: f64,
    pub longitude: f64,
    pub zoom_level: f64,
    pub is_animated: bool,
}

impl MapCameraRequest {
    /// plain init, animated is left at its default
    pub fn new(id: Uuid, latitude: f64, longitude: f64, zoom_level: f64) -> Self {
        Self {
            id,
            latitude,
            longitude,
            zoom_level,
            is_animated: false,
        }
    }
}

/// back to the flat map with a coordinate and the zoom that matches the globe scale
pub type MapGlobeCameraHandler = Rc<dyn Fn(LatLng, f64, &mut Window, &mut App)>;
pub type MapGlobePersonHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// always mounted so the first show costs no setup
/// hidden it takes no input so it cant eat the map's pointer events
#[derive(IntoElement)]
pub struct MapGlobeLayer {
    pub entry: Option<MapGlobeEntry>,
    pub people: Vec<MapGlobePerson>,
    pub palette: MapPinPalette,
    pub style_json: String,
    pub style_key: String,
    pub on_dive_in: MapGlobeCameraHandler,
    pub on_settle: MapGlobeCameraHandler,
    pub on_select_person: MapGlobePersonHandler,
    pub is_hidden: bool,
}

/// coordinator for the globe layer, remembers the entry it last opened on
pub struct MapGlobeLayerCoordinator {
    pub entered_id: Option<Uuid>,
}

impl MapGlobeLayer {
    fn globe(window: &mut Window, cx: &mut App) -> Entity<MapGlobeView> {
        window.use_keyed_state(ElementId::Name("map-globe-view".into()), cx, |_, cx| MapGlobeView::new(cx))
    }

    fn coordinator(window: &mut Window, cx: &mut App) -> Entity<MapGlobeLayerCoordinator> {
        window.use_keyed_state(ElementId::Name("map-globe-coordinator".into()), cx, |_, _| {
            MapGlobeLayerCoordinator { entered_id: None }
        })
    }
}

impl RenderOnce for MapGlobeLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let globe = Self::globe(window, cx);
        let coordinator = Self::coordinator(window, cx);
        let Self {
            entry,
            people,
            palette,
            style_json,
            style_key,
            on_dive_in,
            on_settle,
            on_select_person,
            is_hidden,
        } = self;
        globe.update(cx, |globe, cx| {
            globe.on_dive_in = on_dive_in;
            globe.on_settle = on_settle;
            globe.on_select_person = on_select_person;
            let Some(entry) = entry else {
                return;
            };
            globe.apply_texture(&style_json, &style_key, cx);
            globe.update(people, palette, cx);
            let entered = coordinator.read(cx).entered_id;
            if entered != Some(entry.id) {
                coordinator.update(cx, |coordinator, _| coordinator.entered_id = Some(entry.id));
                globe.enter(LatLng::new(entry.latitude, entry.longitude), entry.zoom_level, window, cx);
            }
        });
        if is_hidden {
            return div().size_full();
        }
        div().size_full().child(globe.cached(StyleRefinement::default().size_full()))
    }
}
