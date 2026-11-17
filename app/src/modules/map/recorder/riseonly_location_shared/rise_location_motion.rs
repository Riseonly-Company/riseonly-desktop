use rise_platform::location::LocationFix;

use crate::modules::map::stores::map_actions::map_types::MapMotion;

pub enum RiseLocationMotion {}

impl RiseLocationMotion {
    pub const MOVING_METERS_PER_SECOND: f64 = 0.4;

    pub fn reliable_speed(location: &LocationFix) -> f64 {
        let speed = location.speed_meters_per_second;
        let speed_accuracy = location.speed_accuracy_meters_per_second;
        if !(speed.is_finite() && speed >= 0.0 && speed_accuracy.is_finite() && speed_accuracy >= 0.0) {
            return -1.0;
        }
        if speed - speed_accuracy >= Self::MOVING_METERS_PER_SECOND { speed } else { 0.0 }
    }

    pub fn classify(speed: f64) -> &'static str {
        if !(speed.is_finite() && speed >= 0.0) {
            return MapMotion::Unknown.raw_value();
        }
        if speed < Self::MOVING_METERS_PER_SECOND {
            MapMotion::Still.raw_value()
        } else if speed < 2.3 {
            MapMotion::Walk.raw_value()
        } else if speed < 4.5 {
            MapMotion::Run.raw_value()
        } else if speed < 9.0 {
            MapMotion::Cycle.raw_value()
        } else if speed < 70.0 {
            MapMotion::Drive.raw_value()
        } else {
            MapMotion::Fly.raw_value()
        }
    }
}
