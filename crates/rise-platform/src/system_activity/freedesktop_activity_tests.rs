use std::path::{Path, PathBuf};

use super::freedesktop_activity::*;
use super::system_activity::SystemActivityEvent;

#[test]
fn prepare_for_sleep_reads_as_sleep_and_wake() {
    assert_eq!(prepare_for_sleep_event(true), SystemActivityEvent::WillSleep);
    assert_eq!(prepare_for_sleep_event(false), SystemActivityEvent::DidWake);
}

#[test]
fn only_no_network_is_unsatisfied() {
    for state in [0, 20, 70] {
        assert!(!network_manager_satisfied(NM_CONNECTIVITY_NONE, state));
        assert!(network_manager_satisfied(NM_CONNECTIVITY_PORTAL, state));
        assert!(network_manager_satisfied(NM_CONNECTIVITY_LIMITED, state));
        assert!(network_manager_satisfied(NM_CONNECTIVITY_FULL, state));
    }
}

#[test]
fn unknown_connectivity_falls_back_to_the_state() {
    assert!(!network_manager_satisfied(NM_CONNECTIVITY_UNKNOWN, 20));
    assert!(!network_manager_satisfied(NM_CONNECTIVITY_UNKNOWN, 40));
    assert!(network_manager_satisfied(NM_CONNECTIVITY_UNKNOWN, 50));
    assert!(network_manager_satisfied(NM_CONNECTIVITY_UNKNOWN, 70));
    assert!(network_manager_satisfied(99, 70));
}

#[test]
fn a_sandbox_reads_the_path_through_the_portal_only() {
    assert_eq!(network_path_routes(true), &[NetworkPathRoute::Portal]);
    assert_eq!(
        network_path_routes(false),
        &[NetworkPathRoute::NetworkManager, NetworkPathRoute::Portal]
    );
}

#[test]
fn the_system_bus_defaults_to_the_specification_socket() {
    assert_eq!(
        system_bus_socket(None),
        BusSocket::Path(PathBuf::from("/var/run/dbus/system_bus_socket"))
    );
    assert_eq!(
        system_bus_socket(Some("")),
        BusSocket::Path(PathBuf::from("/var/run/dbus/system_bus_socket"))
    );
    assert_eq!(
        system_bus_socket(Some("unix:path=/run/dbus/system_bus_socket")),
        BusSocket::Path(PathBuf::from("/run/dbus/system_bus_socket"))
    );
}

#[test]
fn bus_addresses_parse_as_the_client_library_tries_them() {
    assert_eq!(bus_socket(None), BusSocket::Missing);
    assert_eq!(bus_socket(Some("")), BusSocket::Missing);
    assert_eq!(bus_socket(Some("garbage")), BusSocket::Missing);
    assert_eq!(
        bus_socket(Some("unix:abstract=/tmp/dbus-x,guid=1")),
        BusSocket::Unverifiable
    );
    assert_eq!(
        bus_socket(Some("tcp:host=localhost,port=1;unix:path=/run/user/1000/bus")),
        BusSocket::Path(PathBuf::from("/run/user/1000/bus"))
    );
    assert_eq!(
        bus_socket(Some("unix:path=/tmp/a%20b,guid=2")),
        BusSocket::Path(PathBuf::from("/tmp/a b"))
    );
    assert_eq!(bus_socket(Some("unix:path=/tmp/%zz")), BusSocket::Missing);
}

#[test]
fn the_session_bus_falls_back_to_the_runtime_dir() {
    assert_eq!(
        session_bus_socket(None, Some(Path::new("/run/user/1000"))),
        BusSocket::Path(PathBuf::from("/run/user/1000/bus"))
    );
    assert_eq!(session_bus_socket(None, Some(Path::new("relative"))), BusSocket::Missing);
    assert_eq!(session_bus_socket(None, None), BusSocket::Missing);
    assert_eq!(
        session_bus_socket(Some("unix:path=/x/bus"), None),
        BusSocket::Path(PathBuf::from("/x/bus"))
    );
}

#[test]
fn a_missing_socket_names_its_bus() {
    assert_eq!(
        missing_bus_detail("system", &BusSocket::Missing).as_deref(),
        Some("no system bus address")
    );
    let absent = BusSocket::Path(PathBuf::from("/nonexistent/riseonly/bus"));
    assert_eq!(
        missing_bus_detail("session", &absent).as_deref(),
        Some("no session bus socket at /nonexistent/riseonly/bus")
    );
    assert_eq!(missing_bus_detail("system", &BusSocket::Unverifiable), None);
}

#[test]
fn the_first_snapshot_reports_and_repeats_do_not() {
    let mut reporter = NetworkPathReporter::default();
    assert_eq!(
        reporter.report(false),
        Some(SystemActivityEvent::NetworkPath { satisfied: false })
    );
    assert_eq!(reporter.report(false), None);
    assert_eq!(
        reporter.report(true),
        Some(SystemActivityEvent::NetworkPath { satisfied: true })
    );
    assert_eq!(reporter.report(true), None);
}
