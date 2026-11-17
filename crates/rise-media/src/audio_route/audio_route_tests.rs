use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use parking_lot::Mutex;
use rise_platform::HostOs;

use crate::audio_route::*;

fn snapshot(default_output: Option<&str>, outputs: &[&str]) -> AudioOutputSnapshot {
    AudioOutputSnapshot {
        default_output: default_output.map(str::to_owned),
        outputs: outputs.iter().map(|name| (*name).to_owned()).collect::<BTreeSet<String>>(),
    }
}

#[test]
fn reason_raw_values_match_ios() {
    let pairs = [
        (AudioRouteChangeReason::Unknown, 0),
        (AudioRouteChangeReason::NewDeviceAvailable, 1),
        (AudioRouteChangeReason::OldDeviceUnavailable, 2),
        (AudioRouteChangeReason::CategoryChange, 3),
        (AudioRouteChangeReason::Override, 4),
        (AudioRouteChangeReason::WakeFromSleep, 6),
        (AudioRouteChangeReason::NoSuitableRouteForCategory, 7),
        (AudioRouteChangeReason::RouteConfigurationChange, 8),
    ];
    for (reason, raw) in pairs {
        assert_eq!(reason.raw_value(), raw);
        assert_eq!(AudioRouteChangeReason::from_raw_value(raw), Some(reason));
    }
    assert_eq!(AudioRouteChangeReason::from_raw_value(5), None);
    assert_eq!(AudioRouteChangeReason::from_raw_value(9), None);
}

#[test]
fn unplugging_the_default_output_is_old_device_unavailable() {
    let before = snapshot(Some("headphones"), &["speakers", "headphones"]);
    let after = snapshot(Some("speakers"), &["speakers"]);
    assert_eq!(
        route_change_reason(&before, &after),
        Some(AudioRouteChangeReason::OldDeviceUnavailable)
    );
}

#[test]
fn losing_every_output_is_old_device_unavailable() {
    let before = snapshot(Some("speakers"), &["speakers"]);
    let after = snapshot(None, &[]);
    assert_eq!(
        route_change_reason(&before, &after),
        Some(AudioRouteChangeReason::OldDeviceUnavailable)
    );
}

#[test]
fn a_new_device_taking_the_route_is_new_device_available() {
    let before = snapshot(Some("speakers"), &["speakers"]);
    let after = snapshot(Some("headphones"), &["speakers", "headphones"]);
    assert_eq!(
        route_change_reason(&before, &after),
        Some(AudioRouteChangeReason::NewDeviceAvailable)
    );
}

#[test]
fn a_first_output_is_new_device_available() {
    let before = snapshot(None, &[]);
    let after = snapshot(Some("speakers"), &["speakers"]);
    assert_eq!(
        route_change_reason(&before, &after),
        Some(AudioRouteChangeReason::NewDeviceAvailable)
    );
}

#[test]
fn picking_another_present_output_is_override() {
    let before = snapshot(Some("speakers"), &["speakers", "monitor"]);
    let after = snapshot(Some("monitor"), &["speakers", "monitor"]);
    assert_eq!(
        route_change_reason(&before, &after),
        Some(AudioRouteChangeReason::Override)
    );
}

#[test]
fn outputs_changing_behind_the_default_is_no_route_change() {
    let before = snapshot(Some("speakers"), &["speakers"]);
    let plugged = snapshot(Some("speakers"), &["speakers", "headphones"]);
    assert_eq!(route_change_reason(&before, &plugged), None);
    assert_eq!(route_change_reason(&plugged, &before), None);
    assert_eq!(route_change_reason(&before, &before), None);
}

struct FakeBackend {
    current: Arc<Mutex<AudioOutputSnapshot>>,
}

impl AudioRouteBackend for FakeBackend {
    fn snapshot(&mut self) -> Result<AudioOutputSnapshot, AudioRouteError> {
        Ok(self.current.lock().clone())
    }
}

struct FakeSource {
    current: Arc<Mutex<AudioOutputSnapshot>>,
    signal_out: mpsc::Sender<RouteSignal>,
}

impl AudioRouteSource for FakeSource {
    fn open(&self, signal: RouteSignal) -> Result<Box<dyn AudioRouteBackend>, AudioRouteError> {
        let _ = self.signal_out.send(signal);
        Ok(Box::new(FakeBackend {
            current: Arc::clone(&self.current),
        }))
    }
}

struct RefusingSource;

impl AudioRouteSource for RefusingSource {
    fn open(&self, _signal: RouteSignal) -> Result<Box<dyn AudioRouteBackend>, AudioRouteError> {
        Err(AudioRouteError::Unavailable {
            host: HostOs::Linux,
            reason: "no sound server".to_owned(),
        })
    }
}

#[test]
fn monitor_reports_one_settled_change_per_burst() {
    let current = Arc::new(Mutex::new(snapshot(Some("headphones"), &["speakers", "headphones"])));
    let (signal_out, signals) = mpsc::channel::<RouteSignal>();
    let (changes_out, changes) = mpsc::channel::<AudioRouteChange>();
    let monitor = AudioRouteMonitor::start_with(
        Box::new(FakeSource {
            current: Arc::clone(&current),
            signal_out,
        }),
        move |change| {
            let _ = changes_out.send(change);
        },
    )
    .expect("fake source opens");
    let signal = signals.recv_timeout(Duration::from_secs(2)).expect("signal handed to the source");

    *current.lock() = snapshot(Some("speakers"), &["speakers"]);
    signal();
    signal();
    signal();

    let change = changes.recv_timeout(Duration::from_secs(2)).expect("a route change");
    assert_eq!(change.reason, AudioRouteChangeReason::OldDeviceUnavailable);
    assert!(changes.recv_timeout(Duration::from_millis(400)).is_err());

    signal();
    assert!(changes.recv_timeout(Duration::from_millis(400)).is_err());

    drop(monitor);
}

#[test]
fn monitor_start_returns_the_source_error() {
    let started = AudioRouteMonitor::start_with(Box::new(RefusingSource), |_| {});
    assert!(matches!(started, Err(AudioRouteError::Unavailable { .. })));
}
