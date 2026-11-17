use crate::modules::map::recorder::map_recorder_policy::{MapRecorderFix, MapRecorderPolicy};
use crate::modules::map::recorder::map_track_codec::MapTrackGeometry;

struct MapRecorderZoneTests {
    latitude: f64,
    longitude: f64,
}

impl MapRecorderZoneTests {
    fn new() -> Self {
        Self { latitude: 51.1282, longitude: 71.4304 }
    }

    fn fix(&self, east_meters: f64, accuracy: f64) -> MapRecorderFix {
        let meters_per_degree = MapTrackGeometry::haversine_meters(0.0, 0.0, 1.0, 0.0)
            * (self.latitude * std::f64::consts::PI / 180.0).cos();
        MapRecorderFix {
            latitude: self.latitude,
            longitude: self.longitude + east_meters / meters_per_degree,
            accuracy_meters: accuracy,
            speed_meters_per_second: 0.0,
            heading_degrees: -1.0,
            time_ms: 1,
        }
    }

    fn may_be_inside(&self, fix: &MapRecorderFix, radius: f64) -> bool {
        MapRecorderPolicy::may_be_inside(fix, self.latitude, self.longitude, radius)
    }
}

#[test]
fn scatter_around_home_stays_inside_a_small_zone() {
    let suite = MapRecorderZoneTests::new();
    let scattered = suite.fix(30.0, 35.0);
    assert!(!MapRecorderPolicy::is_inside(&scattered, suite.latitude, suite.longitude, 7.0));
    assert!(suite.may_be_inside(&scattered, 7.0));
}

#[test]
fn a_precise_fix_beyond_the_zone_is_recorded() {
    let suite = MapRecorderZoneTests::new();
    assert!(!suite.may_be_inside(&suite.fix(20.0, 5.0), 7.0));
    assert!(!suite.may_be_inside(&suite.fix(13.0, 5.0), 7.0));
}

#[test]
fn a_fix_inside_the_zone_is_inside_whatever_its_accuracy() {
    let suite = MapRecorderZoneTests::new();
    assert!(suite.may_be_inside(&suite.fix(3.0, 0.0), 7.0));
    assert!(suite.may_be_inside(&suite.fix(6.0, 65.0), 3.0));
}
