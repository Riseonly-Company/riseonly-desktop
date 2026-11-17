use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, AppContext, Context, IntoElement, ParentElement, Render, Styled, Window, div, px};
use rise_canon::RiseGuideAnchor;
use rise_media::map_view::LatLng;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::map::components::map_place_sheet::MapPlaceGlyph;
use crate::modules::map::components::rise_map_host::{MapAnnotation, MapAnnotationView, RiseMapHost};
use crate::modules::map::stores::map_actions::map_tracks_types::MapPlace;

pub struct MapPlaceAnnotation {
    pub place_id: String,
    place: RefCell<MapPlace>,
}

impl MapPlaceAnnotation {
    pub fn new(place: MapPlace) -> Self {
        Self { place_id: place.id.clone(), place: RefCell::new(place) }
    }

    pub fn place(&self) -> MapPlace {
        self.place.borrow().clone()
    }

    pub fn set_place(&self, place: MapPlace) {
        *self.place.borrow_mut() = place;
    }
}

impl MapAnnotation for MapPlaceAnnotation {
    fn coordinate(&self) -> LatLng {
        let place = self.place.borrow();
        LatLng::new(place.latitude, place.longitude)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub type MapPlaceSelect = Rc<dyn Fn(String, &mut Window, &mut App)>;

pub struct MapPlacePinLayer {
    pub on_select: MapPlaceSelect,
    annotations: HashMap<String, Rc<MapPlaceAnnotation>>,
    current: Vec<MapPlace>,
}

impl Default for MapPlacePinLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl MapPlacePinLayer {
    pub fn new() -> Self {
        Self {
            on_select: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
            annotations: HashMap::new(),
            current: Vec::new(),
        }
    }

    pub fn update(&mut self, places: &[MapPlace], map: &RiseMapHost, cx: &mut App) {
        if places == self.current.as_slice() {
            return;
        }
        self.current = places.to_vec();
        let ids: HashSet<&str> = places.iter().map(|place| place.id.as_str()).collect();
        let removed: Vec<Rc<MapPlaceAnnotation>> = self
            .annotations
            .values()
            .filter(|annotation| !ids.contains(annotation.place_id.as_str()))
            .cloned()
            .collect();
        if !removed.is_empty() {
            let erased: Vec<Rc<dyn MapAnnotation>> =
                removed.iter().map(|annotation| annotation.clone() as Rc<dyn MapAnnotation>).collect();
            map.remove_annotations(&erased);
        }
        for annotation in &removed {
            self.annotations.remove(&annotation.place_id);
        }
        for place in places {
            if let Some(annotation) = self.annotations.get(&place.id) {
                if annotation.place() == *place {
                    continue;
                }
                annotation.set_place(place.clone());
                let erased: Rc<dyn MapAnnotation> = annotation.clone();
                if let Some(view) = map.view_for(&erased).and_then(|view| view.downcast::<MapPlacePinView>().ok()) {
                    view.update(cx, |view, cx| view.configure(place.clone(), cx));
                }
            } else {
                let annotation = Rc::new(MapPlaceAnnotation::new(place.clone()));
                self.annotations.insert(place.id.clone(), annotation.clone());
                map.add_annotation(annotation);
            }
        }
    }

    pub fn view(&self, annotation: &Rc<dyn MapAnnotation>, map: &RiseMapHost, cx: &mut App) -> Option<MapAnnotationView> {
        let annotation = annotation.as_any().downcast_ref::<MapPlaceAnnotation>()?;
        let mut view = map
            .dequeue_reusable_annotation_view(MapPlacePinView::REUSE_ID)
            .unwrap_or_else(|| {
                let pin = cx.new(|_| MapPlacePinView::new());
                MapAnnotationView::new(
                    pin.into(),
                    MapPlacePinView::REUSE_ID,
                    f64::from(MapPlacePinView::SIDE),
                    f64::from(MapPlacePinView::SIDE),
                )
            });
        if let Ok(pin) = view.view.clone().downcast::<MapPlacePinView>() {
            let place = annotation.place();
            pin.update(cx, |pin, cx| pin.configure(place, cx));
        }
        view.z_position = f64::from(MapPlacePinView::Z_POSITION);
        Some(view)
    }

    pub fn did_select(&self, annotation: &Rc<dyn MapAnnotation>, map: &RiseMapHost, window: &mut Window, cx: &mut App) -> bool {
        let Some(place) = annotation.as_any().downcast_ref::<MapPlaceAnnotation>() else {
            return false;
        };
        map.deselect_annotation(annotation);
        (self.on_select)(place.place_id.clone(), window, cx);
        true
    }
}

struct MapPlacePinView {
    place: Option<MapPlace>,
}

impl MapPlacePinView {
    const REUSE_ID: &'static str = "rise.map.place";
    const SIDE: f32 = 44.0;
    const Z_POSITION: i32 = -1;

    fn new() -> Self {
        Self { place: None }
    }

    fn configure(&mut self, place: MapPlace, cx: &mut Context<Self>) {
        if self.place.as_ref() == Some(&place) {
            return;
        }
        self.place = Some(place);
        cx.notify();
    }
}

impl Render for MapPlacePinView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(place) = self.place.clone() else {
            return div().size(px(Self::SIDE)).into_any_element();
        };
        let anchor = (place.id == GuideSandbox::map_place_id()).then_some(RiseGuideAnchor::MapPlacePin);
        div()
            .size(px(Self::SIDE))
            .flex()
            .items_center()
            .justify_center()
            .child(MapPlaceGlyph::new(place, 30.0).is_playing(false))
            .guide_anchor(anchor)
            .into_any_element()
    }
}
