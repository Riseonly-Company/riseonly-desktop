use std::cell::RefCell;
use std::sync::OnceLock;

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, NSObject};
use objc2::{define_class, msg_send, AnyThread as _, ClassType, MainThreadMarker};
use objc2_foundation::NSString;

use super::location::*;
use crate::host_os::HostOs;

#[link(name = "CoreLocation", kind = "framework")]
unsafe extern "C" {
    static kCLLocationAccuracyBestForNavigation: f64;
    static kCLLocationAccuracyBest: f64;
    static kCLLocationAccuracyNearestTenMeters: f64;
    static kCLLocationAccuracyHundredMeters: f64;
    static kCLLocationAccuracyKilometer: f64;
    static kCLLocationAccuracyThreeKilometers: f64;
    static kCLDistanceFilterNone: f64;
}

/// plain lat lon coordinate
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Coordinate {
    latitude: f64,
    longitude: f64,
}

unsafe impl Encode for Coordinate {
    const ENCODING: Encoding = Encoding::Struct("CLLocationCoordinate2D", &[f64::ENCODING, f64::ENCODING]);
}

unsafe impl RefEncode for Coordinate {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

static SINK: OnceLock<LocationSink> = OnceLock::new();

fn emit(event: LocationEvent) {
    let Some(sink) = SINK.get() else {
        return;
    };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(event))).is_err() {
        tracing::error!(target: "riseonly", "location sink panicked");
    }
}

fn class(name: &'static std::ffi::CStr, label: &'static str) -> Result<&'static AnyClass, LocationUnavailable> {
    AnyClass::get(name).ok_or(LocationUnavailable::FrameworkMissing(label))
}

fn read_permission(manager: &AnyObject) -> LocationPermission {
    let (status, accuracy): (i32, isize) =
        unsafe { (msg_send![manager, authorizationStatus], msg_send![manager, accuracyAuthorization]) };
    LocationPermission::from_cl(i64::from(status), accuracy as i64)
}

fn seconds_since_1970(date: Option<Retained<AnyObject>>) -> Option<f64> {
    date.map(|date| unsafe { msg_send![&*date, timeIntervalSince1970] })
}

fn read_fix(location: &AnyObject) -> LocationFix {
    unsafe {
        let coordinate: Coordinate = msg_send![location, coordinate];
        let timestamp: Option<Retained<AnyObject>> = msg_send![location, timestamp];
        LocationFix {
            latitude: coordinate.latitude,
            longitude: coordinate.longitude,
            horizontal_accuracy_meters: msg_send![location, horizontalAccuracy],
            altitude_meters: msg_send![location, altitude],
            vertical_accuracy_meters: msg_send![location, verticalAccuracy],
            speed_meters_per_second: msg_send![location, speed],
            speed_accuracy_meters_per_second: msg_send![location, speedAccuracy],
            course_degrees: msg_send![location, course],
            course_accuracy_degrees: msg_send![location, courseAccuracy],
            time_ms: fix_time_ms(seconds_since_1970(timestamp).unwrap_or(0.0)),
        }
    }
}

fn read_visit(visit: &AnyObject) -> LocationVisit {
    unsafe {
        let coordinate: Coordinate = msg_send![visit, coordinate];
        let arrival: Option<Retained<AnyObject>> = msg_send![visit, arrivalDate];
        let departure: Option<Retained<AnyObject>> = msg_send![visit, departureDate];
        LocationVisit {
            latitude: coordinate.latitude,
            longitude: coordinate.longitude,
            horizontal_accuracy_meters: msg_send![visit, horizontalAccuracy],
            arrival_ms: seconds_since_1970(arrival).and_then(visit_date_ms),
            departure_ms: seconds_since_1970(departure).and_then(visit_date_ms),
        }
    }
}

fn region_identifier(region: &AnyObject) -> String {
    let identifier: Retained<NSString> = unsafe { msg_send![region, identifier] };
    identifier.to_string()
}

fn error_code(error: &AnyObject) -> i64 {
    let code: isize = unsafe { msg_send![error, code] };
    code as i64
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseLocationDelegate"]
    struct LocationDelegate;

    impl LocationDelegate {
        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn did_change_authorization(&self, manager: &AnyObject) {
            emit(LocationEvent::Authorization(read_permission(manager)));
        }

        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn did_update_locations(&self, _manager: &AnyObject, locations: &AnyObject) {
            let count: usize = unsafe { msg_send![locations, count] };
            let fixes = (0..count)
                .map(|index| {
                    let location: Retained<AnyObject> = unsafe { msg_send![locations, objectAtIndex: index] };
                    read_fix(&location)
                })
                .collect();
            emit(LocationEvent::Fixes(fixes));
        }

        #[unsafe(method(locationManager:didFailWithError:))]
        fn did_fail(&self, _manager: &AnyObject, error: &AnyObject) {
            emit(LocationEvent::Failed(LocationFailure::from_cl_code(error_code(error))));
        }

        #[unsafe(method(locationManager:didVisit:))]
        fn did_visit(&self, _manager: &AnyObject, visit: &AnyObject) {
            emit(LocationEvent::Visit(read_visit(visit)));
        }

        #[unsafe(method(locationManager:didExitRegion:))]
        fn did_exit_region(&self, _manager: &AnyObject, region: &AnyObject) {
            emit(LocationEvent::ExitedRegion(region_identifier(region)));
        }

        #[unsafe(method(locationManager:didDetermineState:forRegion:))]
        fn did_determine_state(&self, _manager: &AnyObject, state: isize, region: &AnyObject) {
            emit(LocationEvent::RegionState {
                identifier: region_identifier(region),
                state: RegionState::from_cl(state as i64),
            });
        }

        #[unsafe(method(locationManager:monitoringDidFailForRegion:withError:))]
        fn monitoring_did_fail(&self, _manager: &AnyObject, _region: Option<&AnyObject>, error: &AnyObject) {
            let code = error_code(error);
            tracing::debug!(target: "riseonly", code, "region monitoring failed");
            emit(LocationEvent::Failed(LocationFailure::RegionMonitoring));
        }
    }
);

static CONFORMANCE: OnceLock<bool> = OnceLock::new();

impl LocationDelegate {
    fn new() -> Retained<Self> {
        declare_conformance();
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn declare_conformance() -> bool {
    *CONFORMANCE.get_or_init(|| {
        let Some(protocol) = AnyProtocol::get(c"CLLocationManagerDelegate") else {
            tracing::warn!(target: "riseonly", "CLLocationManagerDelegate is not registered");
            return false;
        };
        let class: *const AnyClass = LocationDelegate::class();
        unsafe { objc2::ffi::class_addProtocol(class.cast_mut(), protocol).as_bool() }
    })
}

struct Manager {
    manager: Retained<AnyObject>,
    _delegate: Retained<LocationDelegate>,
}

thread_local! {
    static MANAGER: RefCell<Option<Manager>> = const { RefCell::new(None) };
}

/// main thread manager, made on first use
/// handed out as a retained clone so no borrow is held while callbacks come back
fn manager() -> Result<Retained<AnyObject>, LocationUnavailable> {
    MainThreadMarker::new().ok_or(LocationUnavailable::NotMainThread)?;
    MANAGER.with(|slot| {
        if let Some(held) = slot.borrow().as_ref() {
            return Ok(held.manager.clone());
        }
        let manager_class = class(c"CLLocationManager", "CLLocationManager")?;
        let delegate = LocationDelegate::new();
        let manager: Retained<AnyObject> = unsafe {
            let allocated: Allocated<AnyObject> = msg_send![manager_class, alloc];
            let manager: Option<Retained<AnyObject>> = msg_send![allocated, init];
            manager.ok_or(LocationUnavailable::FrameworkMissing("CLLocationManager"))?
        };
        *slot.borrow_mut() = Some(Manager { manager: manager.clone(), _delegate: delegate.clone() });
        unsafe {
            let _: () = msg_send![&*manager, setDelegate: &*delegate];
        }
        Ok(manager)
    })
}

/// runs f with the main thread manager
fn with_manager<T>(f: impl FnOnce(&AnyObject) -> Result<T, LocationUnavailable>) -> Result<T, LocationUnavailable> {
    let manager = manager()?;
    f(&manager)
}

fn accuracy_value(accuracy: DesiredAccuracy) -> f64 {
    unsafe {
        match accuracy {
            DesiredAccuracy::BestForNavigation => kCLLocationAccuracyBestForNavigation,
            DesiredAccuracy::Best => kCLLocationAccuracyBest,
            DesiredAccuracy::NearestTenMeters => kCLLocationAccuracyNearestTenMeters,
            DesiredAccuracy::HundredMeters => kCLLocationAccuracyHundredMeters,
            DesiredAccuracy::Kilometer => kCLLocationAccuracyKilometer,
            DesiredAccuracy::ThreeKilometers => kCLLocationAccuracyThreeKilometers,
        }
    }
}

fn send(manager: &AnyObject, command: &LocationCommand) -> Result<(), LocationUnavailable> {
    unsafe {
        match command {
            LocationCommand::RequestAuthorization(AuthorizationScope::WhenInUse) => {
                let _: () = msg_send![manager, requestWhenInUseAuthorization];
            }
            LocationCommand::RequestAuthorization(AuthorizationScope::Always) => {
                let _: () = msg_send![manager, requestAlwaysAuthorization];
            }
            LocationCommand::StartUpdates(request) => {
                let filter = request.distance_filter().unwrap_or(kCLDistanceFilterNone);
                let _: () = msg_send![manager, setDesiredAccuracy: accuracy_value(request.accuracy)];
                let _: () = msg_send![manager, setDistanceFilter: filter];
                let _: () = msg_send![manager, startUpdatingLocation];
            }
            LocationCommand::StopUpdates => {
                let _: () = msg_send![manager, stopUpdatingLocation];
            }
            LocationCommand::RequestFix => {
                let _: () = msg_send![manager, requestLocation];
            }
            LocationCommand::StartSignificantChanges => {
                let _: () = msg_send![manager, startMonitoringSignificantLocationChanges];
            }
            LocationCommand::StopSignificantChanges => {
                let _: () = msg_send![manager, stopMonitoringSignificantLocationChanges];
            }
            LocationCommand::StartVisits => {
                let _: () = msg_send![manager, startMonitoringVisits];
            }
            LocationCommand::StopVisits => {
                let _: () = msg_send![manager, stopMonitoringVisits];
            }
            LocationCommand::MonitorRegion(region) => monitor_region(manager, region)?,
            LocationCommand::StopRegion(identifier) => match monitored_region(manager, identifier) {
                Some(found) => {
                    let _: () = msg_send![manager, stopMonitoringForRegion: &*found];
                }
                None => tracing::debug!(target: "riseonly", %identifier, "region already not monitored"),
            },
            LocationCommand::RequestRegionState(identifier) => {
                let found = monitored_region(manager, identifier).ok_or(LocationUnavailable::UnknownRegion)?;
                let _: () = msg_send![manager, requestStateForRegion: &*found];
            }
        }
    }
    Ok(())
}

fn monitor_region(manager: &AnyObject, region: &CircularRegion) -> Result<(), LocationUnavailable> {
    let maximum: f64 = unsafe { msg_send![manager, maximumRegionMonitoringDistance] };
    let radius = region
        .clamped_radius(maximum)
        .ok_or(LocationUnavailable::ServiceOff(LocationService::RegionMonitoring))?;
    let region_class = class(c"CLCircularRegion", "CLCircularRegion")?;
    let identifier = NSString::from_str(&region.identifier);
    let center = Coordinate { latitude: region.latitude, longitude: region.longitude };
    unsafe {
        let allocated: Allocated<AnyObject> = msg_send![region_class, alloc];
        let circular: Option<Retained<AnyObject>> =
            msg_send![allocated, initWithCenter: center, radius: radius, identifier: &*identifier];
        let circular = circular.ok_or(LocationUnavailable::FrameworkMissing("CLCircularRegion"))?;
        let _: () = msg_send![&*circular, setNotifyOnEntry: Bool::NO];
        let _: () = msg_send![&*circular, setNotifyOnExit: Bool::YES];
        let _: () = msg_send![manager, startMonitoringForRegion: &*circular];
    }
    Ok(())
}

fn monitored_region(manager: &AnyObject, identifier: &str) -> Option<Retained<AnyObject>> {
    unsafe {
        let regions: Retained<AnyObject> = msg_send![manager, monitoredRegions];
        let all: Retained<AnyObject> = msg_send![&*regions, allObjects];
        let count: usize = msg_send![&*all, count];
        (0..count)
            .map(|index| -> Retained<AnyObject> { msg_send![&*all, objectAtIndex: index] })
            .find(|region| region_identifier(region) == identifier)
    }
}

fn class_answers(class_name: &'static std::ffi::CStr, label: &'static str, query: impl FnOnce(&AnyClass) -> bool) -> Result<bool, LocationUnavailable> {
    Ok(query(class(class_name, label)?))
}

pub(super) struct NativeLocation;

impl LocationSource for NativeLocation {
    fn permission(&self) -> Result<LocationPermission, LocationUnavailable> {
        with_manager(|manager| Ok(read_permission(manager)))
    }

    fn last_fix(&self) -> Result<Option<LocationFix>, LocationUnavailable> {
        with_manager(|manager| {
            let location: Option<Retained<AnyObject>> = unsafe { msg_send![manager, location] };
            Ok(location.map(|location| read_fix(&location)).filter(LocationFix::is_usable))
        })
    }

    fn service_available(&self, service: LocationService) -> Result<(), LocationUnavailable> {
        let available = match service {
            LocationService::SignificantChanges => class_answers(c"CLLocationManager", "CLLocationManager", |manager| {
                let answer: Bool = unsafe { msg_send![manager, significantLocationChangeMonitoringAvailable] };
                answer.as_bool()
            })?,
            LocationService::RegionMonitoring => {
                let region_class = class(c"CLCircularRegion", "CLCircularRegion")?;
                class_answers(c"CLLocationManager", "CLLocationManager", |manager| {
                    let answer: Bool = unsafe { msg_send![manager, isMonitoringAvailableForClass: region_class] };
                    answer.as_bool()
                })?
            }
            LocationService::Visits => true,
            LocationService::Heading | LocationService::BackgroundRelaunch => {
                return Err(LocationUnavailable::NoSuchService(HostOs::MacOs, service));
            }
        };
        if available { Ok(()) } else { Err(LocationUnavailable::ServiceOff(service)) }
    }

    fn install(&self, sink: LocationSink) -> Result<(), LocationUnavailable> {
        if SINK.set(sink).is_err() {
            tracing::debug!(target: "riseonly", "location sink was already installed");
        }
        Ok(())
    }

    fn perform(&self, command: LocationCommand) -> Result<(), LocationUnavailable> {
        with_manager(|manager| send(manager, &command))
    }
}
