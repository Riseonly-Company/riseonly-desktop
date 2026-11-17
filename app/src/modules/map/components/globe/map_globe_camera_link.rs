use gpui::{App, Context, Entity, WeakEntity};
use rise_media::map_view::{CameraChangeReason, LatLng};

use crate::modules::map::components::globe::map_globe_view::MapGlobeView;
use crate::modules::map::components::map_compass_button::MapCompassNeedleView;
use crate::modules::map::components::rise_map_host::{RiseMapHost, WeakRiseMapHost};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;

/// one camera for two pictures
/// flat map and globe keep the same height and rate across a handover
/// per frame, not through page state
pub struct MapGlobeCameraLink {
    pub globe: Option<WeakEntity<MapGlobeView>>,
    pub map_view: Option<WeakRiseMapHost>,
    pub compass: Option<WeakEntity<MapCompassNeedleView>>,
}

impl MapGlobeCameraLink {
    const GESTURES: CameraChangeReason = CameraChangeReason(
        CameraChangeReason::GESTURE_PAN.0
            | CameraChangeReason::GESTURE_PINCH.0
            | CameraChangeReason::GESTURE_ROTATE.0
            | CameraChangeReason::GESTURE_ZOOM_IN.0
            | CameraChangeReason::GESTURE_ZOOM_OUT.0
            | CameraChangeReason::GESTURE_ONE_FINGER_ZOOM.0,
    );

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            globe: None,
            map_view: None,
            compass: None,
        }
    }

    pub fn globe(&self) -> Option<Entity<MapGlobeView>> {
        self.globe.as_ref().and_then(WeakEntity::upgrade)
    }

    pub fn map_view(&self) -> Option<RiseMapHost> {
        self.map_view.as_ref().and_then(WeakRiseMapHost::upgrade)
    }

    pub fn set_map_view(&mut self, map_view: Option<WeakRiseMapHost>) {
        self.map_view = map_view;
    }

    pub fn compass(&self) -> Option<Entity<MapCompassNeedleView>> {
        self.compass.as_ref().and_then(WeakEntity::upgrade)
    }

    pub fn update_compass(&self, cx: &mut App) {
        let Some(compass) = self.compass() else {
            return;
        };
        let is_globe_shown = MapInteractionsStore::shared(cx).read(cx).globe_entry.is_some();
        let bearing = if is_globe_shown {
            0.0
        } else {
            self.map_view().map_or(0.0, |map| map.direction())
        };
        compass.update(cx, |compass, cx| compass.set_bearing(bearing, cx));
    }

    pub fn reset_north(&self, cx: &mut App) {
        if let Some(map) = self.map_view() {
            map.set_direction(0.0, true, cx);
        }
    }

    pub fn map_camera_changed(&self, reason: CameraChangeReason, cx: &mut App) {
        let (Some(map), Some(globe)) = (self.map_view(), self.globe()) else {
            return;
        };
        if !reason.intersects(Self::GESTURES) {
            return;
        }
        let (centre, zoom_level) = (map.center_coordinate(), map.zoom_level());
        globe.update(cx, |globe, cx| globe.follow(centre, zoom_level, cx));
    }

    pub fn globe_camera_changed(&self, centre: LatLng, zoom_level: f64, cx: &mut App) {
        let Some(map) = self.map_view() else {
            return;
        };
        let zoom_level = map.maximum_zoom_level().min(zoom_level);
        map.set_center(centre, Some(zoom_level), None, false, cx);
    }

    pub fn coast_map(&self, zoom_velocity: f64, cx: &mut App) {
        let Some(map) = self.map_view() else {
            return;
        };
        if zoom_velocity.abs() <= 0.05 {
            return;
        }
        let target = map.maximum_zoom_level().min(map.zoom_level() + zoom_velocity * 0.28);
        map.set_zoom_level(target, true, cx);
    }
}
