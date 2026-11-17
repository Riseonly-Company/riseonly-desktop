use std::time::Duration;

use super::location::{
    fix_time_ms, unreported_accuracy, DesiredAccuracy, LocationAuthorization, LocationFailure, LocationFix,
    LocationPermission, UpdateRequest,
};

pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";
pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const LOCATION_INTERFACE: &str = "org.freedesktop.portal.Location";
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
pub const SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";
pub const CREATE_SESSION_METHOD: &str = "CreateSession";
pub const START_METHOD: &str = "Start";
pub const CLOSE_METHOD: &str = "Close";
pub const LOCATION_UPDATED_SIGNAL: &str = "LocationUpdated";
pub const RESPONSE_SIGNAL: &str = "Response";

pub const SESSION_HANDLE_TOKEN_OPTION: &str = "session_handle_token";
pub const HANDLE_TOKEN_OPTION: &str = "handle_token";
pub const DISTANCE_THRESHOLD_OPTION: &str = "distance-threshold";
pub const TIME_THRESHOLD_OPTION: &str = "time-threshold";
pub const ACCURACY_OPTION: &str = "accuracy";

pub const LATITUDE_KEY: &str = "Latitude";
pub const LONGITUDE_KEY: &str = "Longitude";
pub const ACCURACY_KEY: &str = "Accuracy";
pub const ALTITUDE_KEY: &str = "Altitude";
pub const SPEED_KEY: &str = "Speed";
pub const HEADING_KEY: &str = "Heading";
pub const TIMESTAMP_KEY: &str = "Timestamp";

const REQUEST_PATH_PREFIX: &str = "/org/freedesktop/portal/desktop/request";

pub const FIX_TIMEOUT: Duration = Duration::from_secs(30);

pub const BUS_FAILURE_CODE: i64 = -1;

/// accuracy option of CreateSession
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PortalAccuracy {
    None,
    Country,
    City,
    Neighborhood,
    Street,
    Exact,
}

impl PortalAccuracy {
    pub const fn code(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Country => 1,
            Self::City => 2,
            Self::Neighborhood => 3,
            Self::Street => 4,
            Self::Exact => 5,
        }
    }

    /// nearest portal level that still meets the wanted accuracy
    /// street is a few hundred meters, neighborhood about a km
    pub const fn from_desired(accuracy: DesiredAccuracy) -> Self {
        match accuracy {
            DesiredAccuracy::BestForNavigation | DesiredAccuracy::Best | DesiredAccuracy::NearestTenMeters => {
                Self::Exact
            }
            DesiredAccuracy::HundredMeters => Self::Street,
            DesiredAccuracy::Kilometer => Self::Neighborhood,
            DesiredAccuracy::ThreeKilometers => Self::City,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SessionOptions {
    pub distance_threshold: u32,
    pub time_threshold: u32,
    pub accuracy: PortalAccuracy,
}

impl SessionOptions {
    pub fn for_updates(request: UpdateRequest) -> Self {
        let distance_threshold = request
            .distance_filter()
            .map(|meters| meters.ceil().min(f64::from(u32::MAX)) as u32)
            .unwrap_or(0);
        Self { distance_threshold, time_threshold: 0, accuracy: PortalAccuracy::from_desired(request.accuracy) }
    }

    /// options for the permission prompt and one shot request, best fix and no thresholds
    pub const fn one_shot() -> Self {
        Self { distance_threshold: 0, time_threshold: 0, accuracy: PortalAccuracy::Exact }
    }
}

/// response code the portal sends for Start
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StartResponse {
    Granted,
    Cancelled,
    Other,
}

impl StartResponse {
    pub const fn from_code(code: u32) -> Self {
        match code {
            0 => Self::Granted,
            1 => Self::Cancelled,
            _ => Self::Other,
        }
    }

    /// a grant is Always since the app reads location while the process lives
    /// the map never offers an Always prompt
    pub const fn level(self) -> LocationAuthorization {
        match self {
            Self::Granted => LocationAuthorization::Always,
            Self::Cancelled | Self::Other => LocationAuthorization::Denied,
        }
    }
}

/// level before the first start answer
/// sandboxed apps are undecided until they ask, others start granted and get corrected
pub const fn initial_level(sandboxed: bool) -> LocationAuthorization {
    if sandboxed { LocationAuthorization::NotDetermined } else { LocationAuthorization::Always }
}

/// portal doesnt say which accuracy was allowed so the level alone decides
pub const fn permission_of(level: LocationAuthorization) -> LocationPermission {
    LocationPermission { level, is_precise: true }
}

pub fn is_unavailable_error(error_name: &str) -> bool {
    matches!(
        error_name,
        "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.UnknownMethod"
            | "org.freedesktop.DBus.Error.UnknownInterface"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.portal.Error.NotAllowed"
            | "org.freedesktop.portal.Error.NotFound"
    )
}

/// what a failed portal call reports, the level it settles if any and the failure event
pub fn failure_for_error(error_name: Option<&str>) -> (Option<LocationAuthorization>, LocationFailure) {
    match error_name {
        Some(name) if is_unavailable_error(name) => (Some(LocationAuthorization::Restricted), LocationFailure::Denied),
        _ => (None, LocationFailure::Other(BUS_FAILURE_CODE)),
    }
}

/// values of one LocationUpdated, absent keys are none
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct PortalLocation {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub accuracy: Option<f64>,
    pub altitude: Option<f64>,
    pub speed: Option<f64>,
    pub heading: Option<f64>,
    pub timestamp: Option<(u64, u64)>,
}

const UNKNOWN_ALTITUDE_BELOW: f64 = -1.0e300;

impl PortalLocation {
    /// turns it into a fix, unknown speed and heading stay negative
    /// no vertical accuracy so altitude takes the horizontal one
    /// missing timestamp is arrival time
    pub fn to_fix(&self, now_seconds_since_1970: f64) -> LocationFix {
        let horizontal = self.accuracy.filter(|meters| meters.is_finite()).unwrap_or(-1.0);
        let altitude = self.altitude.filter(|meters| meters.is_finite() && *meters > UNKNOWN_ALTITUDE_BELOW);
        let known = |value: Option<f64>| value.filter(|value| value.is_finite() && *value >= 0.0).unwrap_or(-1.0);
        let seconds = match self.timestamp {
            Some((seconds, microseconds)) if seconds > 0 => seconds as f64 + microseconds as f64 / 1_000_000.0,
            _ => now_seconds_since_1970,
        };
        let speed = known(self.speed);
        let course = known(self.heading);
        LocationFix {
            latitude: self.latitude.unwrap_or(f64::NAN),
            longitude: self.longitude.unwrap_or(f64::NAN),
            horizontal_accuracy_meters: horizontal,
            altitude_meters: altitude.unwrap_or(0.0),
            vertical_accuracy_meters: if altitude.is_some() { horizontal.max(0.0) } else { -1.0 },
            speed_meters_per_second: speed,
            speed_accuracy_meters_per_second: unreported_accuracy(speed),
            course_degrees: course,
            course_accuracy_degrees: unreported_accuracy(course),
            time_ms: fix_time_ms(seconds),
        }
    }
}

/// session handle token of one session, letters digits and underscores only
pub fn session_token(serial: u64) -> String {
    format!("riseonly_location_{serial}")
}

/// handle token of one start request
pub fn request_token(serial: u64) -> String {
    format!("riseonly_location_start_{serial}")
}

/// request object path known before the call so the subscription exists in time
/// unique name without the colon and dots as underscores, then the token
pub fn request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.strip_prefix(':').unwrap_or(unique_name).replace('.', "_");
    format!("{REQUEST_PATH_PREFIX}/{sender}/{token}")
}
