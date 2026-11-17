use std::time::Duration;

use super::location::{
    fix_time_ms, unreported_accuracy, DesiredAccuracy, LocationAuthorization, LocationFailure, LocationFix,
    LocationPermission, UpdateRequest,
};
use crate::capture_permission::CaptureAccess;

pub const LOCATION_CAPABILITY: &str = "location";

pub const ACCESS_UNSPECIFIED: i32 = 0;
pub const ACCESS_ALLOWED: i32 = 1;
pub const ACCESS_DENIED: i32 = 2;

/// maps the geolocator access answer to a level
/// allowed means Always since the app reads location while the process lives
/// unknown value counts as a refusal
pub const fn level_from_access_status(raw: i32) -> LocationAuthorization {
    match raw {
        ACCESS_ALLOWED => LocationAuthorization::Always,
        ACCESS_UNSPECIFIED => LocationAuthorization::NotDetermined,
        _ => LocationAuthorization::Denied,
    }
}

/// same as the access status mapping but for the capture style answer, granted is Always
pub const fn level_from_capability_access(access: CaptureAccess) -> LocationAuthorization {
    match access {
        CaptureAccess::Granted => LocationAuthorization::Always,
        CaptureAccess::Denied => LocationAuthorization::Denied,
        CaptureAccess::Restricted => LocationAuthorization::Restricted,
        CaptureAccess::NotDetermined => LocationAuthorization::NotDetermined,
    }
}

/// windows has no reduced accuracy grant, so the level alone decides
pub const fn permission_of(level: LocationAuthorization) -> LocationPermission {
    LocationPermission { level, is_precise: true }
}

/// desired accuracy in meters for the geolocator
/// 100 or less asks for gnss providers, more lets windows use wifi or ip
pub const fn accuracy_in_meters(accuracy: DesiredAccuracy) -> u32 {
    match accuracy {
        DesiredAccuracy::BestForNavigation | DesiredAccuracy::Best => 1,
        DesiredAccuracy::NearestTenMeters => 10,
        DesiredAccuracy::HundredMeters => 100,
        DesiredAccuracy::Kilometer => 1_000,
        DesiredAccuracy::ThreeKilometers => 3_000,
    }
}

/// movement threshold in meters, 0 reports every movement
pub fn movement_threshold(request: UpdateRequest) -> f64 {
    request.distance_filter().unwrap_or(0.0)
}

pub const STATUS_READY: i32 = 0;
pub const STATUS_INITIALIZING: i32 = 1;
pub const STATUS_NO_DATA: i32 = 2;
pub const STATUS_DISABLED: i32 = 3;
pub const STATUS_NOT_INITIALIZED: i32 = 4;
pub const STATUS_NOT_AVAILABLE: i32 = 5;

pub const PLATFORM_FAILURE_CODE: i64 = -1;

/// what a status change means for the map
/// disabled is the location switch off, nodata is no fix yet and the service keeps trying
/// notavailable is a windows build without the api
pub const fn failure_for_status(raw: i32) -> Option<LocationFailure> {
    match raw {
        STATUS_DISABLED => Some(LocationFailure::Denied),
        STATUS_NO_DATA => Some(LocationFailure::LocationUnknown),
        STATUS_NOT_AVAILABLE => Some(LocationFailure::Other(PLATFORM_FAILURE_CODE)),
        _ => None,
    }
}

/// true when a status change means the authorization should be reread
pub const fn status_changes_authorization(raw: i32) -> bool {
    matches!(raw, STATUS_DISABLED | STATUS_READY | STATUS_INITIALIZING)
}

pub const HRESULT_ACCESS_DENIED: i32 = 0x8007_0005_u32 as i32;
pub const HRESULT_TIMEOUT: i32 = 0x8007_05B4_u32 as i32;
pub const HRESULT_ABORT: i32 = 0x8000_4004_u32 as i32;

/// the failure a one shot fix reports for an hresult
pub const fn failure_for_hresult(code: i32) -> LocationFailure {
    match code {
        HRESULT_ACCESS_DENIED => LocationFailure::Denied,
        HRESULT_TIMEOUT | HRESULT_ABORT => LocationFailure::LocationUnknown,
        _ => LocationFailure::Other(PLATFORM_FAILURE_CODE),
    }
}

pub const FIX_TIMEOUT: Duration = Duration::from_secs(30);

pub const FIX_MAXIMUM_AGE: Duration = Duration::from_secs(10);

/// duration as windows ticks of 100 nanoseconds
pub fn timespan_ticks(duration: Duration) -> i64 {
    i64::try_from(duration.as_nanos() / 100).unwrap_or(i64::MAX)
}

const EPOCH_GAP_SECONDS: f64 = 11_644_473_600.0;

/// windows datetime ticks since 1601 as seconds since 1970
pub fn universal_time_seconds_since_1970(universal_time: i64) -> f64 {
    universal_time as f64 / 10_000_000.0 - EPOCH_GAP_SECONDS
}

/// values of one geocoordinate, a null reference is none
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct WindowsCoordinate {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
    pub altitude: Option<f64>,
    pub altitude_accuracy: Option<f64>,
    pub speed: Option<f64>,
    pub heading: Option<f64>,
    pub universal_time: i64,
}

impl WindowsCoordinate {
    /// turns it into a location fix, unknown speed heading and altitude accuracy read -1
    /// a known altitude without accuracy takes the horizontal one
    pub fn to_fix(&self) -> LocationFix {
        let horizontal = if self.accuracy.is_finite() { self.accuracy } else { -1.0 };
        let known = |value: Option<f64>| value.filter(|value| value.is_finite() && *value >= 0.0);
        let altitude = self.altitude.filter(|meters| meters.is_finite());
        let vertical = match altitude {
            Some(_) => known(self.altitude_accuracy).unwrap_or(horizontal.max(0.0)),
            None => -1.0,
        };
        let speed = known(self.speed).unwrap_or(-1.0);
        let course = known(self.heading).unwrap_or(-1.0);
        LocationFix {
            latitude: self.latitude,
            longitude: self.longitude,
            horizontal_accuracy_meters: horizontal,
            altitude_meters: altitude.unwrap_or(0.0),
            vertical_accuracy_meters: vertical,
            speed_meters_per_second: speed,
            speed_accuracy_meters_per_second: unreported_accuracy(speed),
            course_degrees: course,
            course_accuracy_degrees: unreported_accuracy(course),
            time_ms: fix_time_ms(universal_time_seconds_since_1970(self.universal_time)),
        }
    }
}
