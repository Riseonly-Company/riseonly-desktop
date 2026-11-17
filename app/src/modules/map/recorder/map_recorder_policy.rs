use rise_canon::RiseLimits;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::map::recorder::map_track_codec::{MapTrackCodecPoint, MapTrackGeometry};
use crate::modules::map::recorder::rise_map_recorder::MapRecorderConfigurationHome;
use crate::modules::map::recorder::riseonly_location_shared::rise_location_motion::RiseLocationMotion;
use crate::modules::map::stores::map_actions::map_types::MapMotion;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapRecorderFix {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_meters: f64,
    pub speed_meters_per_second: f64,
    pub heading_degrees: f64,
    pub time_ms: i64,
}

impl MapRecorderFix {
    pub fn point(&self) -> MapTrackCodecPoint {
        MapTrackCodecPoint {
            lat_e6: (self.latitude * 1_000_000.0).round() as i32,
            lng_e6: (self.longitude * 1_000_000.0).round() as i32,
            time_ms: self.time_ms,
        }
    }
}

/// recorder decisions as pure functions of their inputs
/// every distance and delay is a canon client limit
/// speed bands only name a mode of movement and decide nothing the server depends on
pub enum MapRecorderPolicy {}

impl MapRecorderPolicy {
    pub fn minimum_accuracy_meters() -> f64 {
        RiseLimits::client_value("map_recorder_min_accuracy_m") as f64
    }

    pub fn minimum_step_meters() -> f64 {
        RiseLimits::client_value("map_recorder_min_step_m") as f64
    }

    pub fn stay_radius_meters() -> f64 {
        RiseLimits::client_value("map_recorder_stay_radius_m") as f64
    }

    pub fn stay_after_ms() -> i64 {
        RiseLimits::client_value("map_recorder_stay_after_ms")
    }

    pub fn exit_region_meters() -> f64 {
        RiseLimits::client_value("map_recorder_exit_region_m") as f64
    }

    pub fn segment_gap_ms() -> i64 {
        RiseLimits::client_value("map_recorder_segment_gap_ms")
    }

    pub fn economy_segment_gap_ms() -> i64 {
        RiseLimits::client_value("map_recorder_economy_segment_gap_ms")
    }

    pub fn segment_maximum_points() -> i64 {
        RiseLimits::client_value("map_recorder_segment_max_points")
    }

    pub fn segment_maximum_ms() -> i64 {
        RiseLimits::client_value("map_recorder_segment_max_ms")
    }

    pub fn maximum_speed_meters_per_second() -> f64 {
        RiseLimits::service_value("map_max_track_speed_mps") as f64
    }

    fn walk_filter_meters() -> f64 {
        RiseLimits::client_value("map_recorder_walk_filter_m") as f64
    }

    fn cycle_filter_meters() -> f64 {
        RiseLimits::client_value("map_recorder_cycle_filter_m") as f64
    }

    fn drive_filter_meters() -> f64 {
        RiseLimits::client_value("map_recorder_drive_filter_m") as f64
    }

    fn fly_filter_meters() -> f64 {
        RiseLimits::client_value("map_recorder_fly_filter_m") as f64
    }

    pub fn motion(speed_meters_per_second: f64) -> MapMotion {
        MapMotion::from_wire(RiseLocationMotion::classify(speed_meters_per_second))
    }

    pub fn distance_filter(motion: MapMotion) -> f64 {
        match motion {
            MapMotion::Unknown | MapMotion::Still | MapMotion::Walk => Self::walk_filter_meters(),
            MapMotion::Run | MapMotion::Cycle => Self::cycle_filter_meters(),
            MapMotion::Drive => Self::drive_filter_meters(),
            MapMotion::Fly => Self::fly_filter_meters(),
        }
    }

    /// accurate enough and far enough from the previous fix to be movement and not jitter
    /// and reachable from it at a speed the server wouldnt discard as a gps jump
    pub fn accepts(fix: &MapRecorderFix, after: Option<&MapRecorderFix>) -> bool {
        if !Self::is_usable(fix) {
            return false;
        }
        let Some(previous) = after else {
            return true;
        };
        if fix.time_ms <= previous.time_ms {
            return false;
        }

        let meters = Self::distance_meters(fix, previous);
        if meters < Self::minimum_step_meters() {
            return false;
        }
        let seconds = ((fix.time_ms - previous.time_ms) as f64 / 1_000.0).max(1.0);
        meters / seconds <= Self::maximum_speed_meters_per_second()
    }

    pub fn is_usable(fix: &MapRecorderFix) -> bool {
        fix.latitude.is_finite()
            && (-90.0..=90.0).contains(&fix.latitude)
            && fix.longitude.is_finite()
            && (-180.0..=180.0).contains(&fix.longitude)
            && fix.accuracy_meters.is_finite()
            && (0.0..=Self::minimum_accuracy_meters()).contains(&fix.accuracy_meters)
            && fix.time_ms >= 0
    }

    pub fn closes_segment(
        started_at_ms: i64,
        last_fix_ms: i64,
        point_count: i64,
        before: &MapRecorderFix,
        gap_ms: i64,
    ) -> bool {
        before.time_ms - last_fix_ms > gap_ms
            || before.time_ms - started_at_ms >= Self::segment_maximum_ms()
            || point_count >= Self::segment_maximum_points()
    }

    pub fn continues_segment(
        last_fix: &MapRecorderFix,
        last_observed_ms: i64,
        before: &MapRecorderFix,
        gap_ms: i64,
    ) -> bool {
        if before.time_ms - last_fix.time_ms > Self::segment_maximum_ms() {
            return false;
        }
        before.time_ms - last_observed_ms <= gap_ms
            || Self::distance_meters(before, last_fix) <= Self::minimum_step_meters()
    }

    pub fn is_inside(fix: &MapRecorderFix, centre_latitude: f64, centre_longitude: f64, radius_meters: f64) -> bool {
        MapTrackGeometry::haversine_meters(fix.latitude, fix.longitude, centre_latitude, centre_longitude)
            <= radius_meters
    }

    /// true if the fix may have been taken inside a no record zone, its accuracy circle reaches the zone
    /// indoor fixes scatter tens of meters around the building
    /// so a radius of a few meters alone would miss them
    pub fn may_be_inside(fix: &MapRecorderFix, centre_latitude: f64, centre_longitude: f64, radius_meters: f64) -> bool {
        Self::is_inside(fix, centre_latitude, centre_longitude, radius_meters + fix.accuracy_meters.max(0.0))
    }

    pub fn crosses_zone(a: &MapRecorderFix, b: &MapRecorderFix, zone: &MapRecorderConfigurationHome) -> bool {
        let metres_per_degree = MapTrackGeometry::haversine_meters(0.0, 0.0, 1.0, 0.0);
        let scale_x = metres_per_degree * (zone.latitude * std::f64::consts::PI / 180.0).cos().max(0.000_001);
        let offset = |longitude: f64| -> f64 {
            let mut delta = (longitude - zone.longitude) % 360.0;
            if delta > 180.0 {
                delta -= 360.0;
            }
            if delta < -180.0 {
                delta += 360.0;
            }
            delta * scale_x
        };
        let ax = offset(a.longitude);
        let ay = (a.latitude - zone.latitude) * metres_per_degree;
        let mut bx = offset(b.longitude);
        let by = (b.latitude - zone.latitude) * metres_per_degree;
        let half_world = 180.0 * scale_x;
        if bx - ax > half_world {
            bx -= 2.0 * half_world;
        }
        if bx - ax < -half_world {
            bx += 2.0 * half_world;
        }
        let dx = bx - ax;
        let dy = by - ay;
        let length = dx * dx + dy * dy;
        let t = if length > 0.0 { (-(ax * dx + ay * dy) / length).clamp(0.0, 1.0) } else { 0.0 };
        let x = ax + t * dx;
        let y = ay + t * dy;
        x * x + y * y <= zone.radius_meters * zone.radius_meters
    }

    pub fn distance_meters(a: &MapRecorderFix, b: &MapRecorderFix) -> f64 {
        MapTrackGeometry::haversine_meters(a.latitude, a.longitude, b.latitude, b.longitude)
    }
}
