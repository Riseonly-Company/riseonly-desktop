use super::location::{DesiredAccuracy, LocationAuthorization, LocationFailure, UpdateRequest};
use super::xdg_location::*;

#[test]
fn accuracy_codes_follow_the_spec() {
    let codes: Vec<u32> = [
        PortalAccuracy::None,
        PortalAccuracy::Country,
        PortalAccuracy::City,
        PortalAccuracy::Neighborhood,
        PortalAccuracy::Street,
        PortalAccuracy::Exact,
    ]
    .iter()
    .map(|accuracy| accuracy.code())
    .collect();
    assert_eq!(codes, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn desired_accuracy_maps_to_the_nearest_level_that_meets_it() {
    assert_eq!(PortalAccuracy::from_desired(DesiredAccuracy::BestForNavigation), PortalAccuracy::Exact);
    assert_eq!(PortalAccuracy::from_desired(DesiredAccuracy::Best), PortalAccuracy::Exact);
    assert_eq!(PortalAccuracy::from_desired(DesiredAccuracy::NearestTenMeters), PortalAccuracy::Exact);
    assert_eq!(PortalAccuracy::from_desired(DesiredAccuracy::HundredMeters), PortalAccuracy::Street);
    assert_eq!(PortalAccuracy::from_desired(DesiredAccuracy::Kilometer), PortalAccuracy::Neighborhood);
    assert_eq!(PortalAccuracy::from_desired(DesiredAccuracy::ThreeKilometers), PortalAccuracy::City);
}

#[test]
fn update_options_carry_the_distance_filter_in_whole_meters() {
    let options = SessionOptions::for_updates(UpdateRequest {
        accuracy: DesiredAccuracy::HundredMeters,
        distance_filter_meters: Some(150.2),
    });
    assert_eq!(options.distance_threshold, 151);
    assert_eq!(options.time_threshold, 0);
    assert_eq!(options.accuracy, PortalAccuracy::Street);
}

#[test]
fn no_or_invalid_distance_filter_reports_every_movement() {
    for filter in [None, Some(-5.0), Some(0.0), Some(f64::NAN), Some(f64::INFINITY)] {
        let options =
            SessionOptions::for_updates(UpdateRequest { accuracy: DesiredAccuracy::Best, distance_filter_meters: filter });
        assert_eq!(options.distance_threshold, 0);
    }
}

#[test]
fn huge_distance_filter_saturates() {
    let options =
        SessionOptions::for_updates(UpdateRequest { accuracy: DesiredAccuracy::Best, distance_filter_meters: Some(1.0e12) });
    assert_eq!(options.distance_threshold, u32::MAX);
}

#[test]
fn one_shot_asks_for_the_best_fix() {
    assert_eq!(
        SessionOptions::one_shot(),
        SessionOptions { distance_threshold: 0, time_threshold: 0, accuracy: PortalAccuracy::Exact }
    );
}

#[test]
fn start_response_grants_always_and_refuses_otherwise() {
    assert_eq!(StartResponse::from_code(0), StartResponse::Granted);
    assert_eq!(StartResponse::from_code(1), StartResponse::Cancelled);
    assert_eq!(StartResponse::from_code(2), StartResponse::Other);
    assert_eq!(StartResponse::from_code(7), StartResponse::Other);
    assert_eq!(StartResponse::Granted.level(), LocationAuthorization::Always);
    assert_eq!(StartResponse::Cancelled.level(), LocationAuthorization::Denied);
    assert_eq!(StartResponse::Other.level(), LocationAuthorization::Denied);
}

#[test]
fn only_a_sandbox_starts_undecided() {
    assert_eq!(initial_level(true), LocationAuthorization::NotDetermined);
    assert_eq!(initial_level(false), LocationAuthorization::Always);
    assert!(permission_of(LocationAuthorization::Always).is_precise);
}

#[test]
fn missing_portal_or_lockdown_is_restricted() {
    for name in ["org.freedesktop.DBus.Error.ServiceUnknown", "org.freedesktop.portal.Error.NotAllowed"] {
        assert_eq!(
            failure_for_error(Some(name)),
            (Some(LocationAuthorization::Restricted), LocationFailure::Denied)
        );
    }
}

#[test]
fn other_bus_errors_fail_without_settling_the_level() {
    assert_eq!(failure_for_error(None), (None, LocationFailure::Other(BUS_FAILURE_CODE)));
    assert_eq!(
        failure_for_error(Some("org.freedesktop.DBus.Error.NoReply")),
        (None, LocationFailure::Other(BUS_FAILURE_CODE))
    );
    assert!(!LocationFailure::Other(BUS_FAILURE_CODE).is_transient());
}

fn full_location() -> PortalLocation {
    PortalLocation {
        latitude: Some(43.238_949),
        longitude: Some(76.889_709),
        accuracy: Some(25.0),
        altitude: Some(812.5),
        speed: Some(1.5),
        heading: Some(270.0),
        timestamp: Some((1_762_250_000, 250_000)),
    }
}

#[test]
fn a_full_update_becomes_a_usable_fix() {
    let fix = full_location().to_fix(1.0);
    assert!(fix.is_usable());
    assert_eq!(fix.latitude, 43.238_949);
    assert_eq!(fix.longitude, 76.889_709);
    assert_eq!(fix.horizontal_accuracy_meters, 25.0);
    assert_eq!(fix.altitude_meters, 812.5);
    assert_eq!(fix.vertical_accuracy_meters, 25.0);
    assert!(fix.has_altitude());
    assert_eq!(fix.speed_meters_per_second, 1.5);
    assert_eq!(fix.speed_accuracy_meters_per_second, 0.0);
    assert_eq!(fix.course_degrees, 270.0);
    assert_eq!(fix.course_accuracy_degrees, 0.0);
    assert!(fix.has_course());
    assert_eq!(fix.time_ms, 1_762_250_000_250);
}

#[test]
fn geoclue_unknown_marks_stay_unknown() {
    let location = PortalLocation { altitude: Some(-f64::MAX), speed: Some(-1.0), heading: Some(-1.0), ..full_location() };
    let fix = location.to_fix(1.0);
    assert!(!fix.has_altitude());
    assert_eq!(fix.altitude_meters, 0.0);
    assert!(fix.speed_meters_per_second < 0.0);
    assert_eq!(fix.speed_accuracy_meters_per_second, -1.0);
    assert!(!fix.has_course());
    assert_eq!(fix.course_accuracy_degrees, -1.0);
}

#[test]
fn missing_timestamp_takes_the_arrival_time() {
    let location = PortalLocation { timestamp: None, ..full_location() };
    assert_eq!(location.to_fix(1_762_250_001.5).time_ms, 1_762_250_001_500);
    let zero = PortalLocation { timestamp: Some((0, 0)), ..full_location() };
    assert_eq!(zero.to_fix(10.0).time_ms, 10_000);
}

#[test]
fn missing_coordinates_are_not_usable() {
    let location = PortalLocation { latitude: None, ..full_location() };
    assert!(!location.to_fix(1.0).is_usable());
    let no_accuracy = PortalLocation { accuracy: None, ..full_location() };
    assert!(!no_accuracy.to_fix(1.0).is_usable());
}

#[test]
fn request_path_is_known_before_the_call() {
    let token = request_token(3);
    assert_eq!(token, "riseonly_location_start_3");
    assert_eq!(
        request_path(":1.42", &token),
        "/org/freedesktop/portal/desktop/request/1_42/riseonly_location_start_3"
    );
    assert_eq!(session_token(9), "riseonly_location_9");
}
