use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use super::NativeSystemActivity;
use crate::system_activity::*;

#[test]
fn the_sleep_observer_registers_and_unregisters_cleanly() {
    let watch = NativeSystemActivity
        .watch_sleep(Arc::new(|_event| {}))
        .expect("the workspace notification center accepts an observer");
    drop(watch);
}

#[test]
fn the_path_monitor_reports_the_current_path_on_start() {
    let (sender, receiver) = mpsc::channel();
    let watch = NativeSystemActivity
        .watch_network_path(Arc::new(move |event| {
            let _ = sender.send(event);
        }))
        .expect("the Network framework creates a path monitor on every supported macOS");
    let first = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("nw_path_monitor_start reports the current path");
    assert!(matches!(first, SystemActivityEvent::NetworkPath { .. }));
    drop(watch);
}

#[test]
fn a_second_subscriber_gets_the_known_path_and_the_last_one_stops_the_monitor() {
    let (sender, receiver) = mpsc::channel();
    let first = NativeSystemActivity
        .watch_network_path(Arc::new(move |event| {
            let _ = sender.send(event);
        }))
        .expect("first subscriber");
    receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the first snapshot arrives");
    let (late_sender, late_receiver) = mpsc::channel();
    let second = NativeSystemActivity
        .watch_network_path(Arc::new(move |event| {
            let _ = late_sender.send(event);
        }))
        .expect("second subscriber");
    let known = late_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("the late subscriber receives the snapshot the monitor already reported");
    assert!(matches!(known, SystemActivityEvent::NetworkPath { .. }));
    drop(first);
    drop(second);
}
