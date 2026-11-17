use std::sync::Arc;

use parking_lot::Mutex;

use crate::host_os::HostOs;
use crate::memory_pressure::*;

use MemoryPressureLevel::{Critical, Normal, Warning};

fn posts(readings: &[MemoryPressureLevel]) -> Vec<bool> {
    let mut gate = MemoryWarningGate::default();
    readings.iter().map(|level| gate.should_post(*level)).collect()
}

#[test]
fn the_ios_name_is_the_raw_value_of_the_notification() {
    assert_eq!(DID_RECEIVE_MEMORY_WARNING, "UIApplicationDidReceiveMemoryWarningNotification");
}

#[test]
fn a_rise_into_warning_posts_once() {
    assert_eq!(posts(&[Warning, Warning, Warning]), [true, false, false]);
}

#[test]
fn a_further_rise_into_critical_posts_again() {
    assert_eq!(posts(&[Warning, Critical, Critical]), [true, true, false]);
}

#[test]
fn a_fall_posts_nothing_and_normal_rearms_the_gate() {
    assert_eq!(posts(&[Critical, Warning, Normal, Warning]), [true, false, false, true]);
}

#[test]
fn normal_alone_never_posts() {
    let mut gate = MemoryWarningGate::default();
    assert!(!gate.should_post(Normal));
    assert_eq!(gate.last(), Normal);
}

#[test]
fn dispatch_data_takes_the_worst_bit() {
    assert_eq!(level_from_dispatch_data(DISPATCH_MEMORYPRESSURE_NORMAL), Some(Normal));
    assert_eq!(level_from_dispatch_data(DISPATCH_MEMORYPRESSURE_WARN), Some(Warning));
    assert_eq!(level_from_dispatch_data(DISPATCH_MEMORYPRESSURE_CRITICAL), Some(Critical));
    assert_eq!(
        level_from_dispatch_data(DISPATCH_MEMORYPRESSURE_WARN | DISPATCH_MEMORYPRESSURE_CRITICAL),
        Some(Critical)
    );
    assert_eq!(level_from_dispatch_data(0), None);
    assert_eq!(level_from_dispatch_data(0x10), None);
}

#[test]
fn the_sink_calls_the_app_only_on_warnings() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&seen);
    let sink = warning_sink(move |level| recorded.lock().push(level));
    for level in [Normal, Warning, Warning, Critical, Normal, Normal, Warning] {
        sink(level);
    }
    assert_eq!(*seen.lock(), [Warning, Critical, Warning]);
}

struct Unwritten;

impl MemoryPressureSource for Unwritten {}

#[test]
fn an_unwritten_layer_names_the_gap_for_the_host() {
    let answer = Unwritten.start(Box::new(|_level| {})).expect_err("default answer is a refusal");
    assert_eq!(answer, MemoryPressureUnavailable::BranchNotWritten(HostOs::current()));
    for host in HostOs::ALL {
        assert_eq!(unavailable_by_default(host), MemoryPressureUnavailable::BranchNotWritten(host));
    }
}

#[test]
fn every_refusal_reads_as_a_sentence() {
    let reasons = [
        MemoryPressureUnavailable::BranchNotWritten(HostOs::Linux),
        MemoryPressureUnavailable::SourceRefused("NULL".to_owned()),
    ];
    for reason in reasons {
        assert!(!reason.to_string().is_empty());
    }
}
