use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, AppContext, ElementId, Entity, FontWeight, IntoElement, ParentElement, RenderOnce, StyleRefinement, Styled, Window, div,
    px,
};
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_media::map_view::{CameraChangeReason, LatLng, ScreenPoint, ScreenRect};
use rise_platform::haptics::{self, ImpactStyle};
use rise_platform::user_defaults::{DefaultsValue, UserDefaults};
use rise_platform::user_defaults_keys::MAP_CAMERA;
use rise_theme::RiseThemeAppearance;
use rise_theme::ui_color_system::UIColor;
use rise_i18n::tr;
use rise_ui::main_text::MainText;

use crate::modules::map::components::globe::map_globe_camera_link::MapGlobeCameraLink;
use crate::modules::map::components::globe::map_globe_layer::MapCameraRequest;
use crate::modules::map::components::map_editing_layer::MapEditingLayer;
use crate::modules::map::components::map_editing_session::MapEditingSession;
use crate::modules::map::components::pins::map_friend_pin_view::MapPinPalette;
use crate::modules::map::components::pins::map_photo_pin_layer::{MapPhotoAnnotation, MapPhotoPinLayer, MapPhotoPinView};
use crate::modules::map::components::pins::map_pin_decor::MapPinMotion;
use crate::modules::map::components::pins::map_pin_layer::MapPinLayer;
use crate::modules::map::components::pins::map_place_pin_layer::MapPlacePinLayer;
use crate::modules::map::components::pins::map_self_pin_layer::{MapSelfAnnotation, MapSelfPinLayer, MapSelfProfile};
use crate::modules::map::components::reactions::map_reaction_stage::MapReactionStage;
use crate::modules::map::components::rise_map_host::{
    MapAnnotation, MapAnnotationView, RiseMapHost, RiseMapSurface, RiseMapViewDelegate,
};
use crate::modules::map::components::tracks::map_track_layer::{MapTrackLayer, MapTrackLayerInput};
use crate::modules::map::engine::core::map_track_tile_store::MapTrackTileStore;
use crate::modules::map::engine::rise_map_tracks_repository::RiseMapTrackTilesTick;
use crate::modules::map::shared::rise_map_style::RiseMapStyle;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;
use crate::modules::map::stores::map_actions::map_tracks_types::{MapPlace, MapTrackBand};
use crate::modules::map::stores::map_actions::map_types::{MapFocusRequest, MapLiveFriend};

pub type MapSelectID = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;
pub type MapAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type MapCoordinateAction = Rc<dyn Fn(LatLng, &mut Window, &mut App)>;

/// maplibre map view for the app
/// friends and photos are annotations and routes are style layers each owned by its own layer object
/// map opens where it was left
#[derive(IntoElement)]
pub struct RiseMapView {
    id: ElementId,
    pub appearance: RiseThemeAppearance,
    pub language_code: String,
    pub self_profile: Option<MapSelfProfile>,
    pub friends: Vec<MapLiveFriend>,
    pub friends_revision: u64,
    pub selected_friend_id: Option<String>,
    pub focus_request: Option<MapFocusRequest>,
    pub pin_palette: Option<MapPinPalette>,
    pub bottom_inset: f64,
    pub on_select_friend: MapSelectID,
    pub on_background_tap: MapAction,
    pub on_select_self: MapAction,
    pub on_focus_handled: Rc<dyn Fn(Uuid, &mut Window, &mut App)>,
    pub track_input: Option<MapTrackLayerInput>,
    pub track_tick: RiseMapTrackTilesTick,
    pub track_bands: Vec<MapTrackBand>,
    pub track_store: Option<Arc<MapTrackTileStore>>,
    pub grids: Vec<MapPhotoGrid>,
    pub grids_revision: u64,
    pub shows_photos: bool,
    pub selected_grid_id: Option<String>,
    pub on_select_grid: MapSelectID,
    pub on_tap_own_track: Option<MapCoordinateAction>,
    pub is_globe_shown: bool,
    pub on_zoomed_out_to_globe: Option<Rc<dyn Fn(LatLng, f64, &mut Window, &mut App)>>,
    pub camera_request: Option<MapCameraRequest>,
    pub editing: Option<Entity<MapEditingSession>>,
    pub editing_revision: u64,
    pub places: Vec<MapPlace>,
    pub on_select_place: Rc<dyn Fn(String, &mut Window, &mut App)>,
    pub on_hold_map: Option<MapCoordinateAction>,
}

impl RiseMapView {
    pub fn new(id: impl Into<ElementId>, appearance: RiseThemeAppearance, language_code: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            appearance,
            language_code: language_code.into(),
            self_profile: None,
            friends: Vec::new(),
            friends_revision: 0,
            selected_friend_id: None,
            focus_request: None,
            pin_palette: None,
            bottom_inset: 0.0,
            on_select_friend: Rc::new(|_: Option<String>, _: &mut Window, _: &mut App| {}),
            on_background_tap: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_select_self: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_focus_handled: Rc::new(|_: Uuid, _: &mut Window, _: &mut App| {}),
            track_input: None,
            track_tick: RiseMapTrackTilesTick::idle(),
            track_bands: Vec::new(),
            track_store: None,
            grids: Vec::new(),
            grids_revision: 0,
            shows_photos: false,
            selected_grid_id: None,
            on_select_grid: Rc::new(|_: Option<String>, _: &mut Window, _: &mut App| {}),
            on_tap_own_track: None,
            is_globe_shown: false,
            on_zoomed_out_to_globe: None,
            camera_request: None,
            editing: None,
            editing_revision: 0,
            places: Vec::new(),
            on_select_place: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
            on_hold_map: None,
        }
    }

    fn make_ui_view(&self, cx: &mut App) -> Entity<RiseMapSurface<RiseMapViewCoordinator>> {
        let host = RiseMapHost::new(RiseMapStyle::json(self.appearance, &self.language_code), self.appearance);
        let mut coordinator = RiseMapViewCoordinator::new();
        coordinator.applied_style = Some(RiseMapViewCoordinatorStyleKey {
            appearance: self.appearance,
            language_code: self.language_code.clone(),
        });
        host.set_shows_compass_view(false);
        host.set_maximum_zoom_level(RiseMapCamera::MAXIMUM_ZOOM_LEVEL);
        RiseMapCamera::restore(&host, cx);
        let (wake, mut woken) = mpsc::unbounded::<()>();
        coordinator.observe_pin_motion(wake);
        MapGlobeCameraLink::shared(cx).update(cx, |link, _| link.set_map_view(Some(host.downgrade())));
        let (pins, self_pin) = (coordinator.pins.clone(), coordinator.self_pin.clone());
        MapReactionStage::shared(cx).update(cx, |stage, cx| stage.attach(&host, pins, self_pin, cx));
        let surface = cx.new(|cx| RiseMapSurface::new(host, coordinator, cx));
        let weak = surface.downgrade();
        cx.spawn(async move |cx| {
            while woken.next().await.is_some() {
                let refreshed = weak.update(cx, |surface, cx| {
                    let map = surface.host.clone();
                    surface.delegate.refresh_motion(&map, cx);
                });
                if refreshed.is_err() {
                    break;
                }
            }
        })
        .detach();
        surface
    }

    fn update_ui_view(&self, map: &RiseMapHost, coordinator: &mut RiseMapViewCoordinator, window: &mut Window, cx: &mut App) {
        map.set_shows_compass_view(false);
        MapGlobeCameraLink::shared(cx).update(cx, |link, cx| link.update_compass(cx));
        coordinator.apply_style(
            RiseMapViewCoordinatorStyleKey {
                appearance: self.appearance,
                language_code: self.language_code.clone(),
            },
            map,
        );

        let was_editing = coordinator.is_editing;
        coordinator.is_editing = self.editing.is_some();
        coordinator.on_hold_map = self.on_hold_map.clone();
        coordinator.places.on_select = self.on_select_place.clone();
        coordinator.places.update(&self.places, map, cx);
        coordinator.editing_session = self.editing.as_ref().map(Entity::downgrade);
        coordinator.editing_since_day = self.track_input.as_ref().map_or(0, |input| input.since_day);
        coordinator.editing_palette = self.pin_palette.clone();
        coordinator.on_select_friend = self.on_select_friend.clone();
        coordinator.on_background_tap = self.on_background_tap.clone();
        coordinator.on_select_self = self.on_select_self.clone();
        coordinator.pins.borrow_mut().on_select = self.on_select_friend.clone();
        let camera_requested = self
            .camera_request
            .as_ref()
            .is_some_and(|request| coordinator.applied_camera_request != Some(request.id))
            || self.focus_request.is_some();
        coordinator.apply_bottom_inset(self.bottom_inset, map, was_editing || coordinator.is_editing, camera_requested, cx);
        if let Some(pin_palette) = &self.pin_palette {
            coordinator
                .self_pin
                .borrow_mut()
                .update(self.self_profile.as_ref(), pin_palette, map, cx);
            coordinator.pins.borrow_mut().update(
                &self.friends,
                self.friends_revision,
                self.selected_friend_id.as_deref(),
                pin_palette,
                map,
                cx,
            );
        }
        coordinator.photos.borrow_mut().on_select = self.on_select_grid.clone();
        coordinator.on_tap_own_track = self.on_tap_own_track.clone();
        coordinator.is_globe_shown = self.is_globe_shown;
        coordinator.on_zoomed_out_to_globe = self.on_zoomed_out_to_globe.clone();
        coordinator.enter_globe_if_opened_far_out(map, window);
        if let Some(camera_request) = &self.camera_request
            && coordinator.applied_camera_request != Some(camera_request.id)
        {
            coordinator.editing_layer.cancel(cx);
            coordinator.applied_camera_request = Some(camera_request.id);
            let zoom = if camera_request.is_animated {
                map.zoom_level().max(camera_request.zoom_level)
            } else {
                camera_request.zoom_level
            };
            map.set_center(
                LatLng::new(camera_request.latitude, camera_request.longitude),
                Some(zoom),
                None,
                camera_request.is_animated,
                cx,
            );
        }
        if let Some(pin_palette) = &self.pin_palette {
            coordinator.photos.borrow_mut().update(
                &self.grids,
                self.grids_revision,
                self.shows_photos,
                self.selected_grid_id.as_deref(),
                pin_palette,
                map,
                cx,
            );
        }
        coordinator.tracks.provider.configure(self.track_store.clone());
        if let Some(track_input) = &self.track_input {
            coordinator
                .tracks
                .update(track_input, &self.track_tick, &self.track_bands, map, cx);
        }
        coordinator.editing_layer.update(
            self.editing.as_ref(),
            self.track_input.as_ref().map_or(0, |input| input.since_day),
            map,
            &mut coordinator.tracks,
            cx,
        );
        coordinator.decorate_editing_photos(map, cx);
        if let Some(focus_request) = &self.focus_request
            && coordinator.pins.borrow_mut().focus(&focus_request.user_id, map, cx)
        {
            let handled = self.on_focus_handled.clone();
            let id = focus_request.id;
            window.defer(cx, move |window, cx| handled(id, window, cx));
        }
    }
}

impl RenderOnce for RiseMapView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let surface = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| self.make_ui_view(cx))
            .read(cx)
            .clone();
        surface.update(cx, |surface, cx| {
            let map = surface.host.clone();
            self.update_ui_view(&map, &mut surface.delegate, window, cx);
        });
        let reactions = MapReactionStage::shared(cx).update(cx, |stage, cx| stage.overlay(cx));
        div()
            .relative()
            .size_full()
            .child(RiseMapSurface::element(&surface, self.id.clone(), window, cx))
            .child(reactions.cached(StyleRefinement::default().absolute().top_0().left_0().size_full()))
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapViewCoordinatorStyleKey {
    pub appearance: RiseThemeAppearance,
    pub language_code: String,
}

pub struct RiseMapViewCoordinator {
    pub applied_style: Option<RiseMapViewCoordinatorStyleKey>,
    pub pins: Rc<RefCell<MapPinLayer>>,
    pub self_pin: Rc<RefCell<MapSelfPinLayer>>,
    pub photos: Rc<RefCell<MapPhotoPinLayer>>,
    pub places: MapPlacePinLayer,
    pub on_hold_map: Option<MapCoordinateAction>,
    pub tracks: MapTrackLayer,
    pub editing_layer: MapEditingLayer,
    pub is_editing: bool,
    pub editing_since_day: i32,
    pub editing_session: Option<gpui::WeakEntity<MapEditingSession>>,
    pub editing_palette: Option<MapPinPalette>,
    pub on_select_friend: MapSelectID,
    pub on_background_tap: MapAction,
    pub on_select_self: MapAction,
    pub on_tap_own_track: Option<MapCoordinateAction>,
    pub is_globe_shown: bool,
    pub on_zoomed_out_to_globe: Option<Rc<dyn Fn(LatLng, f64, &mut Window, &mut App)>>,
    pub applied_camera_request: Option<Uuid>,
    last_zoom_level: f64,
    did_check_opening_zoom: bool,
    applied_bottom_inset: f64,
    requested_bottom_inset: Option<f64>,
    motion_observers: Vec<RiseSubscription>,
}

impl Default for RiseMapViewCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseMapViewCoordinator {
    const TRACK_TAP_REACH: f64 = 14.0;
    pub const GLOBE_ZOOM_LEVEL: f64 = 1.55;

    pub fn new() -> Self {
        Self {
            applied_style: None,
            pins: MapPinLayer::new(),
            self_pin: MapSelfPinLayer::new(),
            photos: MapPhotoPinLayer::new(),
            places: MapPlacePinLayer::new(),
            on_hold_map: None,
            tracks: MapTrackLayer::new(),
            editing_layer: MapEditingLayer::new(),
            is_editing: false,
            editing_since_day: 0,
            editing_session: None,
            editing_palette: None,
            on_select_friend: Rc::new(|_: Option<String>, _: &mut Window, _: &mut App| {}),
            on_background_tap: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_select_self: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_tap_own_track: None,
            is_globe_shown: false,
            on_zoomed_out_to_globe: None,
            applied_camera_request: None,
            last_zoom_level: f64::MAX,
            did_check_opening_zoom: false,
            applied_bottom_inset: 0.0,
            requested_bottom_inset: None,
            motion_observers: Vec::new(),
        }
    }

    pub fn decorate_editing_photos(&self, map: &RiseMapHost, cx: &mut App) {
        for annotation in map.visible_annotations() {
            if annotation.as_any().downcast_ref::<MapPhotoAnnotation>().is_none() {
                continue;
            }
            let Some(view) = map.view_for(&annotation).and_then(|view| view.downcast::<MapPhotoPinView>().ok()) else {
                continue;
            };
            self.decorate_editing_photo(&view, &annotation, map, cx);
        }
    }

    pub fn decorate_editing_photo(
        &self,
        view: &Entity<MapPhotoPinView>,
        annotation: &Rc<dyn MapAnnotation>,
        map: &RiseMapHost,
        cx: &mut App,
    ) {
        let Some(pin) = annotation.as_any().downcast_ref::<MapPhotoAnnotation>() else {
            return;
        };
        let selected = self
            .editing_session
            .as_ref()
            .and_then(|session| session.upgrade())
            .and_then(|session| {
                let session = session.read(cx);
                (!session.drawing()).then(|| pin.members.borrow().iter().any(|member| session.selected.contains(&member.id)))
            });
        let color = self
            .editing_palette
            .as_ref()
            .map_or_else(|| UIColor::system_blue().resolved_color(map.appearance()), |palette| palette.primary);
        view.update(cx, |view, cx| view.set_selection_indicator(selected, color, cx));
    }

    pub fn enter_globe_if_opened_far_out(&mut self, map: &RiseMapHost, window: &mut Window) {
        if self.did_check_opening_zoom || map.bounds_width() <= 0.0 {
            return;
        }
        self.did_check_opening_zoom = true;
        self.last_zoom_level = map.zoom_level();
        let Some(on_zoomed_out_to_globe) = self.on_zoomed_out_to_globe.clone() else {
            return;
        };
        if self.is_globe_shown || map.zoom_level() >= Self::GLOBE_ZOOM_LEVEL {
            return;
        }
        self.is_globe_shown = true;
        let centre = map.center_coordinate();
        let zoom = map.zoom_level();
        window.on_next_frame(move |window, cx| on_zoomed_out_to_globe(centre, zoom, window, cx));
    }

    pub fn hand_over_to_globe_if_zoomed_out(&mut self, map: &RiseMapHost, reason: CameraChangeReason, window: &mut Window, cx: &mut App) {
        let zoom = map.zoom_level();
        let last = std::mem::replace(&mut self.last_zoom_level, zoom);
        if self.is_globe_shown
            || zoom >= Self::GLOBE_ZOOM_LEVEL
            || zoom >= last - 0.0005
            || !reason.intersects(CameraChangeReason::ZOOM_GESTURES)
        {
            return;
        }
        let Some(on_zoomed_out_to_globe) = self.on_zoomed_out_to_globe.clone() else {
            return;
        };
        self.is_globe_shown = true;
        on_zoomed_out_to_globe(map.center_coordinate(), zoom, window, cx);
    }

    pub fn observe_pin_motion(&mut self, wake: mpsc::UnboundedSender<()>) {
        self.motion_observers = MapPinMotion::triggers()
            .into_iter()
            .map(|name| {
                let wake = wake.clone();
                RiseAppEvents::default_center().observe(name, move |_| {
                    let _ = wake.unbounded_send(());
                })
            })
            .collect();
    }

    pub fn stop_observing_pin_motion(&mut self) {
        for observer in self.motion_observers.drain(..) {
            observer.detach();
        }
    }

    pub fn refresh_motion(&self, map: &RiseMapHost, cx: &mut App) {
        self.pins.borrow_mut().refresh_motion(map, cx);
        self.self_pin.borrow_mut().refresh_motion(cx);
    }

    pub fn apply_bottom_inset(&mut self, inset: f64, map: &RiseMapHost, preserving_viewport: bool, camera_requested: bool, cx: &mut App) {
        let previous_request = self.requested_bottom_inset.replace(inset);
        if !(camera_requested || (!preserving_viewport && previous_request != Some(inset))) {
            return;
        }
        if (inset - self.applied_bottom_inset).abs() <= 0.5 {
            return;
        }
        self.applied_bottom_inset = inset;
        map.set_content_inset_bottom(inset, !self.is_editing, cx);
    }

    fn handle_background_tap(&self, map: &RiseMapHost, point: ScreenPoint, window: &mut Window, cx: &mut App) {
        if self.is_editing {
            return;
        }
        if let Some(on_tap_own_track) = &self.on_tap_own_track {
            let reach = Self::TRACK_TAP_REACH;
            let area = ScreenRect {
                origin: ScreenPoint::new(point.x - reach, point.y - reach),
                width: reach * 2.0,
                height: reach * 2.0,
            };
            if !map.visible_features(area, MapTrackLayer::TAPPABLE_LAYER_IDS).is_empty() {
                if let Some(coordinate) = map.convert_point_to_coordinate(point) {
                    on_tap_own_track(coordinate, window, cx);
                }
                return;
            }
        }
        (self.on_background_tap)(window, cx);
    }

    fn handle_place_hold(&self, map: &RiseMapHost, point: ScreenPoint, window: &mut Window, cx: &mut App) {
        if self.is_editing || self.is_globe_shown {
            return;
        }
        let Some(on_hold_map) = &self.on_hold_map else {
            return;
        };
        let _ = haptics::impact(ImpactStyle::Medium);
        if let Some(coordinate) = map.convert_point_to_coordinate(point) {
            on_hold_map(coordinate, window, cx);
        }
    }

    pub fn apply_style(&mut self, key: RiseMapViewCoordinatorStyleKey, map: &RiseMapHost) {
        if self.applied_style.as_ref() == Some(&key) {
            return;
        }
        map.set_style_json(RiseMapStyle::json(key.appearance, &key.language_code), key.appearance);
        self.applied_style = Some(key);
    }
}

impl RiseMapViewDelegate for RiseMapViewCoordinator {
    fn region_did_change_animated(&mut self, map: &RiseMapHost, _animated: bool, _window: &mut Window, cx: &mut App) {
        MapGlobeCameraLink::shared(cx).update(cx, |link, cx| link.update_compass(cx));
        RiseMapCamera::remember(map);
        self.tracks.region_did_change(map, cx);
        self.pins.borrow_mut().region_did_change(map, cx);
        self.self_pin.borrow_mut().camera_changed(map, true, cx);
        self.photos.borrow_mut().region_did_change(map, cx);
        self.decorate_editing_photos(map, cx);
    }

    fn did_finish_loading(&mut self, map: &RiseMapHost, _window: &mut Window, cx: &mut App) {
        self.tracks.style_did_load(map, cx);
        let session = self.editing_session.as_ref().and_then(|session| session.upgrade());
        self.editing_layer
            .update(session.as_ref(), self.editing_since_day, map, &mut self.tracks, cx);
    }

    fn region_is_changing_with(&mut self, map: &RiseMapHost, reason: CameraChangeReason, window: &mut Window, cx: &mut App) {
        MapGlobeCameraLink::shared(cx).update(cx, |link, cx| link.update_compass(cx));
        self.hand_over_to_globe_if_zoomed_out(map, reason, window, cx);
        if self.is_globe_shown {
            MapGlobeCameraLink::shared(cx).update(cx, |link, cx| link.map_camera_changed(reason, cx));
        }
        self.pins.borrow_mut().region_is_changing(map, cx);
        self.self_pin.borrow_mut().camera_changed(map, false, cx);
        self.photos.borrow_mut().region_is_changing(map, cx);
        self.decorate_editing_photos(map, cx);
    }

    fn view_for(&mut self, map: &RiseMapHost, annotation: &Rc<dyn MapAnnotation>, _window: &mut Window, cx: &mut App) -> Option<MapAnnotationView> {
        let view = self
            .self_pin
            .borrow_mut()
            .view(annotation, map, cx)
            .or_else(|| self.pins.borrow_mut().view(annotation, map, cx))
            .or_else(|| self.photos.borrow().view(annotation, map, cx))
            .or_else(|| self.places.view(annotation, map, cx));
        if let Some(pin_view) = view.as_ref().and_then(|view| view.view.clone().downcast::<MapPhotoPinView>().ok()) {
            self.decorate_editing_photo(&pin_view, annotation, map, cx);
        }
        view
    }

    fn did_select(&mut self, map: &RiseMapHost, annotation: &Rc<dyn MapAnnotation>, window: &mut Window, cx: &mut App) {
        if annotation.as_any().is::<MapSelfAnnotation>() {
            map.deselect_annotation(annotation);
            (self.on_select_self)(window, cx);
            return;
        }
        if self.places.did_select(annotation, map, window, cx) {
            return;
        }
        let photo_selected = self.photos.borrow_mut().did_select(annotation, map, window, cx);
        if photo_selected {
            return;
        }
        self.pins.borrow_mut().did_select(annotation, map, window, cx);
    }

    fn did_click(&mut self, map: &RiseMapHost, point: ScreenPoint, window: &mut Window, cx: &mut App) {
        self.handle_background_tap(map, point, window, cx);
    }

    fn did_secondary_click(&mut self, map: &RiseMapHost, point: ScreenPoint, window: &mut Window, cx: &mut App) {
        self.handle_place_hold(map, point, window, cx);
    }

    fn dismantle(&mut self, map: &RiseMapHost, cx: &mut App) {
        self.pins.borrow_mut().teardown(map);
        self.photos.borrow_mut().teardown(map);
        self.self_pin.borrow_mut().teardown(map);
        self.stop_observing_pin_motion();
        MapReactionStage::shared(cx).update(cx, |stage, cx| stage.detach(map, cx));
    }
}

/// persisted camera
/// first visit has nothing to restore so it shows the reader's side of the globe
/// time zone meridian guesses the longitude with no location access
pub enum RiseMapCamera {}

impl RiseMapCamera {
    pub const MAXIMUM_ZOOM_LEVEL: f64 = 20.0;

    const FIRST_VISIT_LATITUDE: f64 = 35.0;
    const FIRST_VISIT_ZOOM_LEVEL: f64 = 1.2;

    fn saved() -> Option<[f64; 4]> {
        let saved = UserDefaults::standard().array(MAP_CAMERA)?;
        let values: Vec<f64> = saved.iter().filter_map(DefaultsValue::as_f64).collect();
        <[f64; 4]>::try_from(values).ok().filter(|_| saved.len() == 4)
    }

    pub fn restore(map: &RiseMapHost, cx: &mut App) {
        if let Some(saved) = Self::saved() {
            map.set_center(LatLng::new(saved[0], saved[1]), Some(saved[2]), Some(saved[3]), false, cx);
            return;
        }
        let meridian = f64::from(TimeZone::current().seconds_from_gmt(&Date::now())) / 3600.0 * 15.0;
        map.set_center(
            LatLng::new(Self::FIRST_VISIT_LATITUDE, meridian),
            Some(Self::FIRST_VISIT_ZOOM_LEVEL),
            None,
            false,
            cx,
        );
    }

    pub fn saved_centre() -> Option<LatLng> {
        let saved = Self::saved()?;
        (saved[0].is_finite() && saved[1].is_finite() && saved[0].abs() <= 90.0 && saved[1].abs() <= 180.0)
            .then(|| LatLng::new(saved[0], saved[1]))
    }

    pub fn remember(map: &RiseMapHost) {
        let center = map.center_coordinate();
        UserDefaults::standard().set(
            MAP_CAMERA,
            vec![center.latitude, center.longitude, map.zoom_level(), map.direction()],
        );
    }
}

/// openmaptiles and openstreetmap contributors credit
/// tile licence asks for it wherever the map shows
/// small plate that reads on light and dark tiles and takes no pointer
#[derive(IntoElement, Default)]
pub struct RiseMapAttribution;

impl RenderOnce for RiseMapAttribution {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        div()
            .flex()
            .px(px(6.0))
            .py(px(2.0))
            .rounded_full()
            .bg(theme.bg._100.opacity(0.72))
            .child(
                MainText::new(tr("map_attribution"))
                    .px(9.0)
                    .font_weight(FontWeight::MEDIUM)
                    .color(theme.text.secondary)
                    .number_of_lines(1),
            )
    }
}
