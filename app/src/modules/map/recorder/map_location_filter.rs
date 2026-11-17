use rise_canon::RiseTimingCanon;
use rise_platform::location::LocationFix;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::modules::map::recorder::map_recorder_policy::MapRecorderPolicy;
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;
use crate::modules::map::recorder::riseonly_location_shared::rise_location_motion::RiseLocationMotion;

/// keeps a stationary reference instead of letting overlapping gps errors random walk
#[derive(Clone, Default, Debug)]
pub struct MapLocationFilter {
    reference: Option<LocationFix>,
    last_timestamp_ms: Option<i64>,
    is_moving: bool,
}

impl MapLocationFilter {
    fn motion_freshness_ms() -> i64 {
        RiseTimingCanon::freshness_ms("map_friend_motion")
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn accept(&mut self, location: &LocationFix) -> Option<LocationFix> {
        let coordinate_is_valid = (-90.0..=90.0).contains(&location.latitude)
            && (-180.0..=180.0).contains(&location.longitude);
        let is_newer = self.last_timestamp_ms.is_none_or(|last| location.time_ms > last);
        if !(coordinate_is_valid
            && location.horizontal_accuracy_meters.is_finite()
            && location.horizontal_accuracy_meters >= 0.0
            && is_newer)
        {
            return None;
        }

        let speed = RiseLocationMotion::reliable_speed(location);
        let accurate = location.horizontal_accuracy_meters <= MapRecorderPolicy::minimum_accuracy_meters();
        let mut latitude = location.latitude;
        let mut longitude = location.longitude;
        let mut accuracy = location.horizontal_accuracy_meters;
        let mut moving = false;

        if let Some(reference) = self.reference {
            let distance = MapTrackGeometry::haversine_meters(
                location.latitude,
                location.longitude,
                reference.latitude,
                reference.longitude,
            );
            let uncertainty = MapRecorderPolicy::minimum_step_meters()
                .max(reference.horizontal_accuracy_meters + location.horizontal_accuracy_meters);
            let continuous = self
                .last_timestamp_ms
                .is_some_and(|last| (location.time_ms - last) as f64 <= Self::motion_freshness_ms() as f64);
            moving = accurate
                && speed >= RiseLocationMotion::MOVING_METERS_PER_SECOND
                && ((self.is_moving && continuous) || distance >= MapRecorderPolicy::minimum_step_meters());

            let improves_reference = accurate
                && (distance + location.horizontal_accuracy_meters < reference.horizontal_accuracy_meters
                    || reference.horizontal_accuracy_meters > MapRecorderPolicy::minimum_accuracy_meters());
            if !moving && !improves_reference && (!accurate || distance <= uncertainty) {
                latitude = reference.latitude;
                longitude = reference.longitude;
                accuracy = reference.horizontal_accuracy_meters.max(distance + location.horizontal_accuracy_meters);
            } else {
                self.reference = Some(*location);
            }
        } else {
            self.reference = Some(*location);
        }

        self.last_timestamp_ms = Some(location.time_ms);
        self.is_moving = moving;
        let course = if moving
            && location.course_degrees.is_finite()
            && location.course_degrees >= 0.0
            && location.course_accuracy_degrees.is_finite()
            && location.course_accuracy_degrees >= 0.0
        {
            location.course_degrees
        } else {
            -1.0
        };
        Some(LocationFix {
            latitude,
            longitude,
            horizontal_accuracy_meters: accuracy,
            altitude_meters: location.altitude_meters,
            vertical_accuracy_meters: location.vertical_accuracy_meters,
            speed_meters_per_second: if moving { speed } else { 0.0 },
            speed_accuracy_meters_per_second: if moving { location.speed_accuracy_meters_per_second } else { 0.0 },
            course_degrees: course,
            course_accuracy_degrees: if course >= 0.0 { location.course_accuracy_degrees } else { -1.0 },
            time_ms: location.time_ms,
        })
    }

    pub fn stop(&mut self) {
        self.is_moving = false;
    }
}
