use super::NativePower;
use crate::power::*;

#[test]
fn low_power_mode_is_answered() {
    NativePower
        .low_power_mode()
        .expect("NSProcessInfo answers isLowPowerModeEnabled on every supported macOS");
}

#[test]
fn battery_reading_is_consistent() {
    let battery = NativePower
        .battery()
        .expect("IOKit lists the power sources on every Mac");
    if let Some(percent) = battery.percent {
        assert!(percent <= 100, "a percent above 100: {battery:?}");
        assert_ne!(
            battery.state,
            BatteryState::Unknown,
            "a Mac that reports a battery level also reports its power source state"
        );
    }
    if battery.state == BatteryState::Unknown {
        assert_eq!(battery, BatteryReading::NO_BATTERY);
    }
}

#[test]
fn snapshot_has_no_fallback_on_macos() {
    let snapshot = power_snapshot();
    assert!(
        snapshot.unavailable.is_empty(),
        "macOS answers every power reading: {:?}",
        snapshot.unavailable
    );
}

#[test]
fn watch_starts_and_stops_cleanly() {
    let first = NativePower
        .watch(Box::new(|| {}))
        .expect("the power state observer and the IOKit source both register");
    drop(first);
    let second = watch(|_, _| {}).expect("a new watch registers after the old one dropped");
    drop(second);
}
