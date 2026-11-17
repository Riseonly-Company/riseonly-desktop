use std::time::Duration;

use super::geolocator::*;
use super::location::{DesiredAccuracy, LocationAuthorization, LocationFailure, UpdateRequest};
use crate::capture_permission::CaptureAccess;

const NEW_YEAR_2024_TICKS: i64 = 133_485_408_000_000_000;
const NEW_YEAR_2024_MS: i64 = 1_704_067_200_000;

fn coordinate() -> WindowsCoordinate {
    WindowsCoordinate {
        latitude: 43.238,
        longitude: 76.945,
        accuracy: 12.0,
        altitude: Some(850.0),
        altitude_accuracy: Some(4.0),
        speed: Some(1.5),
        heading: Some(270.0),
        universal_time: NEW_YEAR_2024_TICKS,
    }
}

#[test]
fn access_status_maps_to_levels() {
    assert_eq!(level_from_access_status(ACCESS_ALLOWED), LocationAuthorization::Always);
    assert_eq!(level_from_access_status(ACCESS_DENIED), LocationAuthorization::Denied);
    assert_eq!(level_from_access_status(ACCESS_UNSPECIFIED), LocationAuthorization::NotDetermined);
    assert_eq!(level_from_access_status(7), LocationAuthorization::Denied);
    assert_eq!(level_from_access_status(-1), LocationAuthorization::Denied);
}

#[test]
fn capability_access_maps_to_levels() {
    assert_eq!(level_from_capability_access(CaptureAccess::Granted), LocationAuthorization::Always);
    assert_eq!(level_from_capability_access(CaptureAccess::Denied), LocationAuthorization::Denied);
    assert_eq!(level_from_capability_access(CaptureAccess::Restricted), LocationAuthorization::Restricted);
    assert_eq!(level_from_capability_access(CaptureAccess::NotDetermined), LocationAuthorization::NotDetermined);
}

#[test]
fn permission_is_always_precise() {
    let permission = permission_of(LocationAuthorization::Denied);
    assert_eq!(permission.level, LocationAuthorization::Denied);
    assert!(permission.is_precise);
}

#[test]
fn every_accuracy_has_a_meter_value_in_order() {
    let ordered = [
        DesiredAccuracy::BestForNavigation,
        DesiredAccuracy::Best,
        DesiredAccuracy::NearestTenMeters,
        DesiredAccuracy::HundredMeters,
        DesiredAccuracy::Kilometer,
        DesiredAccuracy::ThreeKilometers,
    ];
    let meters: Vec<u32> = ordered.iter().map(|accuracy| accuracy_in_meters(*accuracy)).collect();
    assert_eq!(meters, vec![1, 1, 10, 100, 1_000, 3_000]);
    assert!(accuracy_in_meters(DesiredAccuracy::HundredMeters) <= 100);
}

#[test]
fn movement_threshold_follows_the_distance_filter() {
    let request = |distance| UpdateRequest { accuracy: DesiredAccuracy::Best, distance_filter_meters: distance };
    assert_eq!(movement_threshold(request(None)), 0.0);
    assert_eq!(movement_threshold(request(Some(25.0))), 25.0);
    assert_eq!(movement_threshold(request(Some(-1.0))), 0.0);
    assert_eq!(movement_threshold(request(Some(f64::NAN))), 0.0);
}

#[test]
fn status_failures() {
    assert_eq!(failure_for_status(STATUS_DISABLED), Some(LocationFailure::Denied));
    assert_eq!(failure_for_status(STATUS_NO_DATA), Some(LocationFailure::LocationUnknown));
    assert_eq!(failure_for_status(STATUS_NOT_AVAILABLE), Some(LocationFailure::Other(PLATFORM_FAILURE_CODE)));
    assert_eq!(failure_for_status(STATUS_READY), None);
    assert_eq!(failure_for_status(STATUS_INITIALIZING), None);
    assert_eq!(failure_for_status(STATUS_NOT_INITIALIZED), None);
    assert!(failure_for_status(STATUS_NO_DATA).is_some_and(LocationFailure::is_transient));
    assert!(failure_for_status(STATUS_DISABLED).is_some_and(|failure| !failure.is_transient()));
}

#[test]
fn switch_moves_reread_the_authorization() {
    assert!(status_changes_authorization(STATUS_DISABLED));
    assert!(status_changes_authorization(STATUS_READY));
    assert!(status_changes_authorization(STATUS_INITIALIZING));
    assert!(!status_changes_authorization(STATUS_NO_DATA));
    assert!(!status_changes_authorization(STATUS_NOT_INITIALIZED));
    assert!(!status_changes_authorization(STATUS_NOT_AVAILABLE));
}

#[test]
fn hresults_map_to_failures() {
    assert_eq!(failure_for_hresult(HRESULT_ACCESS_DENIED), LocationFailure::Denied);
    assert_eq!(failure_for_hresult(HRESULT_TIMEOUT), LocationFailure::LocationUnknown);
    assert_eq!(failure_for_hresult(HRESULT_ABORT), LocationFailure::LocationUnknown);
    assert_eq!(failure_for_hresult(0x8000_4005_u32 as i32), LocationFailure::Other(PLATFORM_FAILURE_CODE));
}

#[test]
fn timespans_are_ticks() {
    assert_eq!(timespan_ticks(Duration::from_secs(30)), 300_000_000);
    assert_eq!(timespan_ticks(Duration::ZERO), 0);
    assert_eq!(timespan_ticks(Duration::from_nanos(250)), 2);
    assert!(timespan_ticks(FIX_MAXIMUM_AGE) < timespan_ticks(FIX_TIMEOUT));
}

#[test]
fn universal_time_converts_to_unix() {
    assert_eq!(universal_time_seconds_since_1970(NEW_YEAR_2024_TICKS), 1_704_067_200.0);
    assert_eq!(universal_time_seconds_since_1970(116_444_736_000_000_000), 0.0);
}

#[test]
fn full_coordinate_converts() {
    let fix = coordinate().to_fix();
    assert_eq!(fix.latitude, 43.238);
    assert_eq!(fix.longitude, 76.945);
    assert_eq!(fix.horizontal_accuracy_meters, 12.0);
    assert_eq!(fix.altitude_meters, 850.0);
    assert_eq!(fix.vertical_accuracy_meters, 4.0);
    assert_eq!(fix.speed_meters_per_second, 1.5);
    assert_eq!(fix.speed_accuracy_meters_per_second, 0.0);
    assert_eq!(fix.course_degrees, 270.0);
    assert_eq!(fix.course_accuracy_degrees, 0.0);
    assert_eq!(fix.time_ms, NEW_YEAR_2024_MS);
    assert!(fix.is_usable());
    assert!(fix.has_course());
    assert!(fix.has_altitude());
}

#[test]
fn null_references_read_as_unknown() {
    let fix = WindowsCoordinate {
        altitude: None,
        altitude_accuracy: None,
        speed: None,
        heading: None,
        ..coordinate()
    }
    .to_fix();
    assert_eq!(fix.altitude_meters, 0.0);
    assert_eq!(fix.vertical_accuracy_meters, -1.0);
    assert_eq!(fix.speed_meters_per_second, -1.0);
    assert_eq!(fix.speed_accuracy_meters_per_second, -1.0);
    assert_eq!(fix.course_degrees, -1.0);
    assert_eq!(fix.course_accuracy_degrees, -1.0);
    assert!(!fix.has_altitude());
    assert!(!fix.has_course());
    assert!(fix.is_usable());
}

#[test]
fn stationary_heading_is_no_course() {
    let fix = WindowsCoordinate { heading: Some(f64::NAN), speed: Some(0.0), ..coordinate() }.to_fix();
    assert_eq!(fix.course_degrees, -1.0);
    assert_eq!(fix.course_accuracy_degrees, -1.0);
    assert_eq!(fix.speed_meters_per_second, 0.0);
    assert_eq!(fix.speed_accuracy_meters_per_second, 0.0);
}

#[test]
fn altitude_without_its_accuracy_takes_the_horizontal_one() {
    let fix = WindowsCoordinate { altitude_accuracy: None, ..coordinate() }.to_fix();
    assert_eq!(fix.vertical_accuracy_meters, 12.0);
    assert!(fix.has_altitude());
}

#[test]
fn a_broken_accuracy_marks_the_fix_unusable() {
    let fix = WindowsCoordinate { accuracy: f64::NAN, ..coordinate() }.to_fix();
    assert_eq!(fix.horizontal_accuracy_meters, -1.0);
    assert!(!fix.is_usable());
}
