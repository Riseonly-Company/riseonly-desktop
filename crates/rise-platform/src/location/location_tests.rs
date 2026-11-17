use std::future::Future;
use std::pin::pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::location::*;

/// test source with fixed permission, records commands and keeps the sink so a test can play the os
struct FakeSource {
    level: LocationAuthorization,
    machine_runs: Vec<LocationService>,
    refuse_install: bool,
    sink: Mutex<Option<LocationSink>>,
    installs: Mutex<usize>,
    sent: Mutex<Vec<LocationCommand>>,
}

impl FakeSource {
    fn new(level: LocationAuthorization, machine_runs: &[LocationService]) -> Self {
        Self {
            level,
            machine_runs: machine_runs.to_vec(),
            refuse_install: false,
            sink: Mutex::new(None),
            installs: Mutex::new(0),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn report(&self, event: LocationEvent) {
        let sink = self.sink.lock().clone().expect("installed");
        sink(event);
    }

    fn sent(&self) -> Vec<LocationCommand> {
        self.sent.lock().clone()
    }
}

impl LocationSource for FakeSource {
    fn permission(&self) -> Result<LocationPermission, LocationUnavailable> {
        Ok(LocationPermission { level: self.level, is_precise: true })
    }

    fn service_available(&self, service: LocationService) -> Result<(), LocationUnavailable> {
        if self.machine_runs.contains(&service) { Ok(()) } else { Err(LocationUnavailable::ServiceOff(service)) }
    }

    fn install(&self, sink: LocationSink) -> Result<(), LocationUnavailable> {
        *self.installs.lock() += 1;
        if self.refuse_install {
            return Err(LocationUnavailable::NotMainThread);
        }
        *self.sink.lock() = Some(sink);
        Ok(())
    }

    fn perform(&self, command: LocationCommand) -> Result<(), LocationUnavailable> {
        self.sent.lock().push(command);
        Ok(())
    }
}

struct NoBranch;

impl LocationSource for NoBranch {}

fn fix(latitude: f64, accuracy: f64) -> LocationFix {
    LocationFix {
        latitude,
        longitude: 37.6,
        horizontal_accuracy_meters: accuracy,
        altitude_meters: 150.0,
        vertical_accuracy_meters: -1.0,
        speed_meters_per_second: -1.0,
        speed_accuracy_meters_per_second: -1.0,
        course_degrees: -1.0,
        course_accuracy_degrees: -1.0,
        time_ms: 1_700_000_000_000,
    }
}

fn poll_now<F: Future>(future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn cl_status_maps_like_the_ios_level() {
    assert_eq!(LocationAuthorization::from_cl_status(0), LocationAuthorization::NotDetermined);
    assert_eq!(LocationAuthorization::from_cl_status(1), LocationAuthorization::Restricted);
    assert_eq!(LocationAuthorization::from_cl_status(2), LocationAuthorization::Denied);
    assert_eq!(LocationAuthorization::from_cl_status(3), LocationAuthorization::Always);
    assert_eq!(LocationAuthorization::from_cl_status(4), LocationAuthorization::WhenInUse);
    assert_eq!(LocationAuthorization::from_cl_status(9), LocationAuthorization::Denied);
    assert!(LocationAuthorization::Always.allows_foreground());
    assert!(LocationAuthorization::WhenInUse.allows_foreground());
    assert!(!LocationAuthorization::Restricted.allows_foreground());
    assert!(LocationPermission::from_cl(3, 0).is_precise);
    assert!(!LocationPermission::from_cl(3, 1).is_precise);
}

#[test]
fn access_action_key_follows_the_onboarding() {
    use LocationAuthorization as L;
    assert_eq!(access_action_key(L::NotDetermined, false), "map_onboarding_allow_location");
    assert_eq!(access_action_key(L::WhenInUse, false), "map_onboarding_allow_always");
    assert_eq!(access_action_key(L::WhenInUse, true), "map_onboarding_open_settings");
    for level in [L::Always, L::Denied, L::Restricted] {
        assert_eq!(access_action_key(level, false), "map_onboarding_open_settings");
    }
}

#[test]
fn prompts_only_from_the_level_ios_asks_from() {
    use LocationAuthorization as L;
    assert!(should_prompt(AuthorizationScope::WhenInUse, L::NotDetermined));
    assert!(!should_prompt(AuthorizationScope::WhenInUse, L::Denied));
    assert!(should_prompt(AuthorizationScope::Always, L::WhenInUse));
    assert!(!should_prompt(AuthorizationScope::Always, L::NotDetermined));
    assert!(!should_prompt(AuthorizationScope::Always, L::Always));
}

#[test]
fn hosts_offer_what_their_system_runs() {
    for host in HostOs::ALL {
        assert!(host_offers(host, LocationService::BackgroundRelaunch).is_err(), "{host:?}");
        assert!(host_offers(host, LocationService::Heading).is_err(), "{host:?}");
    }
    for service in [LocationService::SignificantChanges, LocationService::Visits, LocationService::RegionMonitoring] {
        assert_eq!(host_offers(HostOs::MacOs, service), Ok(()));
        assert_eq!(host_offers(HostOs::Linux, service), Err(LocationUnavailable::NoSuchService(HostOs::Linux, service)));
        assert_eq!(
            host_offers(HostOs::Windows, service),
            Err(LocationUnavailable::NoSuchService(HostOs::Windows, service))
        );
    }
}

#[test]
fn a_missing_service_never_reaches_the_os() {
    let source = FakeSource::new(LocationAuthorization::Always, &[]);
    assert_eq!(
        perform_on(&source, HostOs::Linux, LocationCommand::StartVisits),
        Err(LocationUnavailable::NoSuchService(HostOs::Linux, LocationService::Visits))
    );
    assert_eq!(
        perform_on(&source, HostOs::MacOs, LocationCommand::StartSignificantChanges),
        Err(LocationUnavailable::ServiceOff(LocationService::SignificantChanges))
    );
    assert_eq!(perform_on(&source, HostOs::Linux, LocationCommand::StopUpdates), Ok(()));
    assert_eq!(source.sent(), vec![LocationCommand::StopUpdates]);
}

#[test]
fn the_recorder_falls_back_to_coarse_updates_without_wakeups() {
    let all = [LocationService::SignificantChanges, LocationService::Visits, LocationService::RegionMonitoring];
    let mac = FakeSource::new(LocationAuthorization::Always, &all);
    assert_eq!(
        stationary_watch(&mac, HostOs::MacOs, 150.0),
        StationaryWatch::SystemWakeups { significant_changes: true, visits: true, exit_region: true }
    );
    let coarse = StationaryWatch::CoarseUpdates(UpdateRequest {
        accuracy: DesiredAccuracy::HundredMeters,
        distance_filter_meters: Some(150.0),
    });
    for host in [HostOs::Linux, HostOs::Windows] {
        assert_eq!(stationary_watch(&mac, host, 150.0), coarse, "{host:?}");
    }
    let mac_without_services = FakeSource::new(LocationAuthorization::Always, &[]);
    assert_eq!(stationary_watch(&mac_without_services, HostOs::MacOs, 150.0), coarse);
}

#[test]
fn distance_filter_none_for_non_positive_values() {
    let request = |filter| UpdateRequest { accuracy: DesiredAccuracy::Best, distance_filter_meters: filter };
    assert_eq!(request(Some(3.0)).distance_filter(), Some(3.0));
    assert_eq!(request(Some(0.0)).distance_filter(), None);
    assert_eq!(request(Some(-1.0)).distance_filter(), None);
    assert_eq!(request(Some(f64::NAN)).distance_filter(), None);
    assert_eq!(request(None).distance_filter(), None);
}

#[test]
fn fixes_heading_and_dates_read_like_corelocation() {
    assert!(fix(55.7, 12.0).is_usable());
    assert!(!fix(55.7, -1.0).is_usable());
    assert!(!fix(91.0, 5.0).is_usable());
    assert!(!fix(f64::NAN, 5.0).is_usable());
    assert!(!fix(55.7, 5.0).has_course());
    assert!(!fix(55.7, 5.0).has_altitude());

    assert_eq!(LocationHeading::from_cl(90.0, 80.0, 5.0), Some(LocationHeading { degrees: 90.0, accuracy_degrees: 5.0 }));
    assert_eq!(LocationHeading::from_cl(-1.0, 80.0, 5.0).map(|h| h.degrees), Some(80.0));
    assert_eq!(LocationHeading::from_cl(90.0, 80.0, -1.0), None);

    assert_eq!(visit_date_ms(1_700_000_000.5), Some(1_700_000_000_500));
    assert_eq!(visit_date_ms(-62_135_596_800.0), None);
    assert_eq!(visit_date_ms(64_092_211_200.0), None);
    assert_eq!(fix_time_ms(-62_135_596_800.0), 0);
    assert_eq!(fix_time_ms(1.0), 1_000);
}

#[test]
fn regions_and_failures_map() {
    let region = CircularRegion { identifier: "exit".into(), latitude: 1.0, longitude: 2.0, radius_meters: 200.0 };
    assert_eq!(region.clamped_radius(150.0), Some(150.0));
    assert_eq!(region.clamped_radius(1_000.0), Some(200.0));
    assert_eq!(region.clamped_radius(-1.0), None);
    assert_eq!(RegionState::from_cl(1), RegionState::Inside);
    assert_eq!(RegionState::from_cl(2), RegionState::Outside);
    assert_eq!(RegionState::from_cl(0), RegionState::Unknown);

    assert_eq!(LocationFailure::from_cl_code(1), LocationFailure::Denied);
    assert!(LocationFailure::from_cl_code(0).is_transient());
    assert!(LocationFailure::from_cl_code(2).is_transient());
    assert!(!LocationFailure::Denied.is_transient());
    assert_eq!(LocationFailure::from_cl_code(5), LocationFailure::RegionMonitoring);
    assert_eq!(LocationFailure::from_cl_code(17), LocationFailure::Other(17));
}

#[test]
fn subscribers_get_every_event_until_dropped_and_install_runs_once() {
    let source = FakeSource::new(LocationAuthorization::Always, &[]);
    let hub = Arc::new(LocationHub::default());
    let installed = OnceLock::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let first_seen = Arc::clone(&seen);
    let first = subscribe_to(&source, &hub, &installed, Arc::new(move |event| first_seen.lock().push(event)))
        .expect("subscribed");
    let second = subscribe_to(&source, &hub, &installed, Arc::new(|_| {})).expect("subscribed");
    assert_eq!(*source.installs.lock(), 1);
    assert_eq!(hub.subscriber_count(), 2);

    source.report(LocationEvent::ExitedRegion("exit".into()));
    drop(first);
    source.report(LocationEvent::Failed(LocationFailure::Network));
    assert_eq!(*seen.lock(), vec![LocationEvent::ExitedRegion("exit".into())]);
    drop(second);
    assert_eq!(hub.subscriber_count(), 0);
}

#[test]
fn a_refused_install_refuses_every_subscription() {
    let mut source = FakeSource::new(LocationAuthorization::Always, &[]);
    source.refuse_install = true;
    let hub = Arc::new(LocationHub::default());
    let installed = OnceLock::new();
    for _ in 0..2 {
        assert!(matches!(
            subscribe_to(&source, &hub, &installed, Arc::new(|_| {})),
            Err(LocationUnavailable::NotMainThread)
        ));
    }
    assert_eq!(*source.installs.lock(), 1);
}

#[test]
fn current_fix_answers_with_the_latest_usable_fix() {
    let source = FakeSource::new(LocationAuthorization::Always, &[]);
    let hub = Arc::new(LocationHub::default());
    let installed = OnceLock::new();
    let mut request = pin!(current_fix_from(&source, &hub, &installed));
    assert!(poll_now(request.as_mut()).is_pending());
    assert_eq!(source.sent(), vec![LocationCommand::RequestFix]);

    source.report(LocationEvent::Authorization(LocationPermission {
        level: LocationAuthorization::Always,
        is_precise: true,
    }));
    assert!(poll_now(request.as_mut()).is_pending());
    source.report(LocationEvent::Fixes(vec![fix(55.0, 10.0), fix(56.0, 8.0), fix(57.0, -1.0)]));
    source.report(LocationEvent::Fixes(vec![fix(58.0, 8.0)]));
    assert_eq!(poll_now(request.as_mut()), Poll::Ready(Some(fix(56.0, 8.0))));
    assert_eq!(hub.subscriber_count(), 0);
}

#[test]
fn current_fix_ends_on_any_failure_and_without_permission() {
    let source = FakeSource::new(LocationAuthorization::WhenInUse, &[]);
    let hub = Arc::new(LocationHub::default());
    let installed = OnceLock::new();
    let mut request = pin!(current_fix_from(&source, &hub, &installed));
    source.report(LocationEvent::Failed(LocationFailure::LocationUnknown));
    assert_eq!(poll_now(request.as_mut()), Poll::Ready(None));

    let denied = FakeSource::new(LocationAuthorization::Denied, &[]);
    let mut refused = pin!(current_fix_from(&denied, &hub, &installed));
    assert_eq!(poll_now(refused.as_mut()), Poll::Ready(None));
    assert!(denied.sent().is_empty());

    let mut unwritten = pin!(current_fix_from(&NoBranch, &Arc::new(LocationHub::default()), &OnceLock::new()));
    assert_eq!(poll_now(unwritten.as_mut()), Poll::Ready(None));
}

#[test]
fn an_unwritten_branch_says_so() {
    assert_eq!(NoBranch.permission(), Err(LocationUnavailable::BranchNotWritten(HostOs::current())));
    assert_eq!(
        perform_on(&NoBranch, HostOs::MacOs, LocationCommand::RequestFix),
        Err(LocationUnavailable::BranchNotWritten(HostOs::current()))
    );
}

#[test]
fn settings_links_per_host() {
    assert_eq!(
        settings_url(HostOs::MacOs),
        Some("x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices")
    );
    assert_eq!(settings_url(HostOs::Windows), Some("ms-settings:privacy-location"));
    assert_eq!(settings_url(HostOs::Linux), None);
}

#[test]
fn no_settings_link_asks_again_even_after_a_denial() {
    let source = FakeSource::new(LocationAuthorization::Denied, &[]);
    assert_eq!(ask_again_without_settings(&source), PlatformSupport::Unsupported);
    assert_eq!(source.sent(), vec![LocationCommand::RequestAuthorization(AuthorizationScope::WhenInUse)]);
}

#[test]
fn directions_open_apple_maps_on_macos() {
    assert_eq!(
        directions_url(HostOs::MacOs, 55.7558, 37.6173, "Red Square"),
        "maps://?daddr=55.7558,37.6173&q=Red%20Square"
    );
    assert_eq!(
        directions_url(HostOs::MacOs, -33.5, 151.25, "a&b=c"),
        "maps://?daddr=-33.5,151.25&q=a%26b%3Dc"
    );
    assert_eq!(
        directions_url(HostOs::MacOs, 43.0, 76.0, "\u{0410}"),
        "maps://?daddr=43,76&q=%D0%90"
    );
    assert_eq!(directions_url(HostOs::MacOs, 1.5, 2.5, ""), "maps://?daddr=1.5,2.5");
}

#[test]
fn directions_open_openstreetmap_on_linux_and_windows() {
    for host in [HostOs::Linux, HostOs::Windows] {
        assert_eq!(
            directions_url(host, 55.7558, 37.6173, "Red Square"),
            "https://www.openstreetmap.org/directions?route=%3B55.7558%2C37.6173"
        );
        assert_eq!(
            directions_url(host, -0.25, -170.0, ""),
            "https://www.openstreetmap.org/directions?route=%3B-0.25%2C-170"
        );
    }
}

#[test]
fn unreported_accuracy_is_exact_for_a_known_value_only() {
    assert_eq!(unreported_accuracy(1.5), 0.0);
    assert_eq!(unreported_accuracy(0.0), 0.0);
    assert_eq!(unreported_accuracy(-1.0), -1.0);
    assert_eq!(unreported_accuracy(f64::NAN), -1.0);
}
