use std::sync::Arc;

use parking_lot::Mutex;

use crate::host_os::HostOs;
use crate::system_activity::*;

use SystemActivityEvent::{DidWake, NetworkPath, WillSleep};

#[test]
fn the_app_is_active_while_awake_with_a_visible_window() {
    let mut gate = ApplicationActivityGate::default();
    assert!(!gate.is_active());
    assert!(gate.window_visibility(true));
    assert_eq!(gate.system_event(WillSleep), Some(false));
    assert_eq!(gate.system_event(DidWake), Some(true));
    assert!(!gate.window_visibility(false));
}

#[test]
fn every_input_answers_the_current_value_and_the_path_none() {
    let mut gate = ApplicationActivityGate::default();
    assert!(!gate.window_visibility(false));
    assert!(!gate.window_visibility(false));
    assert!(gate.window_visibility(true));
    assert_eq!(gate.system_event(DidWake), Some(true));
    assert_eq!(gate.system_event(DidWake), Some(true));
    assert_eq!(gate.system_event(NetworkPath { satisfied: false }), None);
}

#[test]
fn waking_with_every_window_hidden_stays_inactive() {
    let mut gate = ApplicationActivityGate::default();
    assert_eq!(gate.system_event(WillSleep), Some(false));
    assert_eq!(gate.system_event(DidWake), Some(false));
    assert!(gate.window_visibility(true));
}

struct Unwritten;

impl SystemActivitySource for Unwritten {}

#[test]
fn an_unwritten_branch_names_both_gaps_and_the_host() {
    let watches = watch_on(&Unwritten, |_event| {});
    let host = HostOs::current();
    assert_eq!(
        watches.gaps(),
        [
            SystemActivityUnavailable::BranchNotWritten {
                what: SystemActivityKind::SleepAndWake,
                host,
            },
            SystemActivityUnavailable::BranchNotWritten {
                what: SystemActivityKind::NetworkPath,
                host,
            },
        ]
    );
}

struct OnlyPath;

impl SystemActivitySource for OnlyPath {
    fn watch_network_path(
        &self,
        sink: EventSink,
    ) -> Result<ActivityWatch, SystemActivityUnavailable> {
        sink(NetworkPath { satisfied: true });
        Ok(ActivityWatch::new(()))
    }
}

#[test]
fn a_half_that_starts_delivers_while_the_other_is_a_gap() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let watches = watch_on(&OnlyPath, move |event| record.lock().push(event));
    assert_eq!(*seen.lock(), [NetworkPath { satisfied: true }]);
    assert_eq!(watches.gaps().len(), 1);
    assert!(matches!(
        watches.gaps()[0],
        SystemActivityUnavailable::BranchNotWritten {
            what: SystemActivityKind::SleepAndWake,
            ..
        }
    ));
}
