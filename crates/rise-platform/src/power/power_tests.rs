use std::sync::Arc;

use parking_lot::Mutex;

use crate::host_os::HostOs;
use crate::power::*;

#[test]
fn battery_state_raw_values_match_uidevice() {
    for (state, raw) in BatteryState::ALL.into_iter().zip(0..) {
        assert_eq!(state.raw_value(), raw, "{state:?} keeps the iOS raw value");
        assert_eq!(BatteryState::from_raw_value(raw), Some(state));
    }
    assert_eq!(BatteryState::from_raw_value(4), None);
    assert_eq!(BatteryState::from_raw_value(-1), None);
}

#[test]
fn battery_state_from_power_source() {
    assert_eq!(BatteryState::from_power_source(false, false), BatteryState::Unplugged);
    assert_eq!(BatteryState::from_power_source(false, true), BatteryState::Unplugged);
    assert_eq!(BatteryState::from_power_source(true, false), BatteryState::Charging);
    assert_eq!(BatteryState::from_power_source(true, true), BatteryState::Full);
}

#[test]
fn percent_from_level_rounds_like_ios() {
    assert_eq!(battery_percent_from_level(-1.0), None, "level -1 is no battery");
    assert_eq!(battery_percent_from_level(f32::NAN), None);
    assert_eq!(battery_percent_from_level(0.0), Some(0));
    assert_eq!(battery_percent_from_level(0.2), Some(20));
    assert_eq!(battery_percent_from_level(0.125), Some(13), "half rounds away from zero");
    assert_eq!(battery_percent_from_level(1.0), Some(100));
    assert_eq!(battery_percent_from_level(1.5), Some(100));
}

#[test]
fn percent_from_capacity_rounds_half_up() {
    assert_eq!(battery_percent_from_capacity(50, 100), Some(50));
    assert_eq!(battery_percent_from_capacity(1, 3), Some(33));
    assert_eq!(battery_percent_from_capacity(2, 3), Some(67));
    assert_eq!(battery_percent_from_capacity(1, 8), Some(13));
    assert_eq!(battery_percent_from_capacity(0, 100), Some(0));
    assert_eq!(battery_percent_from_capacity(120, 100), Some(100));
    assert_eq!(battery_percent_from_capacity(4_500, 5_000), Some(90));
    assert_eq!(battery_percent_from_capacity(10, 0), None);
    assert_eq!(battery_percent_from_capacity(-1, 100), None);
}

fn reading(percent: Option<u8>, state: BatteryState, low_power_mode: bool) -> PowerReading {
    PowerReading {
        battery: BatteryReading { percent, state, present: Some(true) },
        low_power_mode,
    }
}

#[test]
fn changes_name_the_ios_notifications_in_order() {
    let before = reading(Some(40), BatteryState::Unplugged, false);
    assert!(before.changes_since(&before).is_empty());
    assert_eq!(
        reading(Some(39), BatteryState::Unplugged, false).changes_since(&before),
        [BATTERY_LEVEL_DID_CHANGE]
    );
    assert_eq!(
        reading(Some(40), BatteryState::Charging, true).changes_since(&before),
        [BATTERY_STATE_DID_CHANGE, POWER_STATE_DID_CHANGE]
    );
    assert_eq!(
        reading(None, BatteryState::Unknown, false).changes_since(&before),
        [BATTERY_LEVEL_DID_CHANGE, BATTERY_STATE_DID_CHANGE]
    );
}

#[test]
fn notification_names_are_the_ios_raw_values() {
    assert_eq!(BATTERY_LEVEL_DID_CHANGE, "UIDeviceBatteryLevelDidChangeNotification");
    assert_eq!(BATTERY_STATE_DID_CHANGE, "UIDeviceBatteryStateDidChangeNotification");
    assert_eq!(POWER_STATE_DID_CHANGE, "NSProcessInfoPowerStateDidChangeNotification");
}

#[test]
fn change_sink_reports_only_real_changes() {
    let first = reading(Some(50), BatteryState::Unplugged, false);
    let script = Arc::new(Mutex::new(vec![
        first,
        reading(Some(49), BatteryState::Unplugged, false),
        reading(Some(49), BatteryState::Unplugged, false),
        reading(Some(49), BatteryState::Unplugged, true),
    ]));
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let source = Arc::clone(&script);
    let sink_log = Arc::clone(&delivered);
    let sink = change_sink(
        first,
        move || source.lock().remove(0),
        move |reading, names| sink_log.lock().push((reading, names.to_vec())),
    );
    for _ in 0..4 {
        sink();
    }
    let delivered = delivered.lock();
    assert_eq!(delivered.len(), 2, "a repeated reading is not a change: {delivered:?}");
    assert_eq!(delivered[0].0.battery.percent, Some(49));
    assert_eq!(delivered[0].1, [BATTERY_LEVEL_DID_CHANGE]);
    assert!(delivered[1].0.low_power_mode);
    assert_eq!(delivered[1].1, [POWER_STATE_DID_CHANGE]);
}

#[test]
fn snapshot_falls_back_with_reasons() {
    let missing_mode = unavailable_by_default(PowerReadingKind::LowPowerMode, HostOs::Linux);
    let missing_battery = unavailable_by_default(PowerReadingKind::Battery, HostOs::Linux);
    let snapshot =
        PowerSnapshot::from_results(Err(missing_mode.clone()), Err(missing_battery.clone()));
    assert_eq!(snapshot.reading, PowerReading::default());
    assert_eq!(snapshot.reading.battery, BatteryReading::UNKNOWN);
    assert!(!snapshot.reading.low_power_mode);
    assert_eq!(snapshot.unavailable, [missing_mode, missing_battery]);

    let battery = BatteryReading {
        percent: Some(80),
        state: BatteryState::Charging,
        present: Some(true),
    };
    let answered = PowerSnapshot::from_results(Ok(true), Ok(battery));
    assert!(answered.unavailable.is_empty());
    assert_eq!(answered.reading.battery, battery);
    assert!(answered.reading.low_power_mode);
}

struct Unwritten;

impl PowerSource for Unwritten {}

#[test]
fn defaults_name_the_gap_for_the_host() {
    let host = HostOs::current();
    assert_eq!(
        Unwritten.low_power_mode(),
        Err(PowerUnavailable::BranchNotWritten {
            what: PowerReadingKind::LowPowerMode,
            host
        })
    );
    assert_eq!(
        Unwritten.battery(),
        Err(PowerUnavailable::BranchNotWritten {
            what: PowerReadingKind::Battery,
            host
        })
    );
    let watch = Unwritten.watch(Box::new(|| {}));
    assert!(matches!(
        watch,
        Err(PowerUnavailable::BranchNotWritten {
            what: PowerReadingKind::Changes,
            ..
        })
    ));
}

#[test]
fn snapshot_always_answers() {
    let snapshot = power_snapshot();
    if let Some(percent) = snapshot.reading.battery.percent {
        assert!(percent <= 100);
    }
    assert_eq!(system_low_power_mode(), snapshot.reading.low_power_mode);
}

#[test]
fn percent_from_upower_percentage() {
    assert_eq!(battery_percent_from_percentage(f64::NAN), None);
    assert_eq!(battery_percent_from_percentage(-1.0), None);
    assert_eq!(battery_percent_from_percentage(0.0), Some(0));
    assert_eq!(battery_percent_from_percentage(49.4), Some(49));
    assert_eq!(battery_percent_from_percentage(49.5), Some(50));
    assert_eq!(battery_percent_from_percentage(100.0), Some(100));
    assert_eq!(battery_percent_from_percentage(104.0), Some(100), "a calibration overshoot is full");
}

#[test]
fn state_from_upower_device_state() {
    assert_eq!(battery_state_from_upower(0), BatteryState::Unknown);
    assert_eq!(battery_state_from_upower(1), BatteryState::Charging);
    assert_eq!(battery_state_from_upower(2), BatteryState::Unplugged);
    assert_eq!(battery_state_from_upower(3), BatteryState::Unplugged, "empty is still on battery");
    assert_eq!(battery_state_from_upower(4), BatteryState::Full);
    assert_eq!(battery_state_from_upower(5), BatteryState::Charging, "charge hold on the cable");
    assert_eq!(battery_state_from_upower(6), BatteryState::Unplugged);
    assert_eq!(battery_state_from_upower(7), BatteryState::Unknown);
}

#[test]
fn reading_from_upower_display_device() {
    assert_eq!(battery_reading_from_upower(false, 80.0, 2), BatteryReading::NO_BATTERY);
    assert_eq!(
        battery_reading_from_upower(true, 80.4, 2),
        BatteryReading {
            percent: Some(80),
            state: BatteryState::Unplugged,
            present: Some(true),
        }
    );
}

#[test]
fn low_power_mode_is_the_power_saver_profile() {
    assert!(low_power_mode_from_profile("power-saver"));
    assert!(!low_power_mode_from_profile("balanced"));
    assert!(!low_power_mode_from_profile("performance"));
    assert!(!low_power_mode_from_profile(""));
}

#[test]
fn battery_from_system_power_status() {
    let reading = |percent, state| BatteryReading { percent, state, present: Some(true) };
    assert_eq!(battery_reading_from_system_power_status(0, 128, 255), BatteryReading::NO_BATTERY);
    assert_eq!(
        battery_reading_from_system_power_status(1, 128 | 8, 50),
        BatteryReading::NO_BATTERY,
        "no system battery wins over every other bit"
    );
    assert_eq!(
        battery_reading_from_system_power_status(1, 255, 50),
        BatteryReading { percent: Some(50), state: BatteryState::Unknown, present: None },
        "flag 255 is unknown, not no battery"
    );
    assert_eq!(
        battery_reading_from_system_power_status(255, 0, 70),
        reading(Some(70), BatteryState::Unknown)
    );
    assert_eq!(
        battery_reading_from_system_power_status(1, 8, 40),
        reading(Some(40), BatteryState::Charging)
    );
    assert_eq!(
        battery_reading_from_system_power_status(1, 0, 100),
        reading(Some(100), BatteryState::Full)
    );
    assert_eq!(
        battery_reading_from_system_power_status(1, 8, 100),
        reading(Some(100), BatteryState::Charging),
        "still charging at 100 percent is not full"
    );
    assert_eq!(
        battery_reading_from_system_power_status(1, 0, 95),
        reading(Some(95), BatteryState::Charging),
        "on the cable below full without charging is a charge hold"
    );
    assert_eq!(
        battery_reading_from_system_power_status(0, 0, 70),
        reading(Some(70), BatteryState::Unplugged)
    );
    assert_eq!(battery_reading_from_system_power_status(0, 0, 255).percent, None);
    assert_eq!(battery_reading_from_system_power_status(0, 0, 101).percent, None);
}

#[test]
fn low_power_mode_from_battery_saver_flag() {
    assert!(low_power_mode_from_system_status(1));
    assert!(!low_power_mode_from_system_status(0));
}

#[test]
fn missing_daemons_are_absent_not_failures() {
    assert!(is_absent_error_name("org.freedesktop.DBus.Error.ServiceUnknown"));
    assert!(is_absent_error_name("org.freedesktop.DBus.Error.NameHasNoOwner"));
    assert!(is_absent_error_name("org.freedesktop.DBus.Error.UnknownObject"));
    assert!(is_absent_error_name("org.freedesktop.DBus.Error.UnknownInterface"));
    assert!(is_absent_error_name("org.freedesktop.DBus.Error.UnknownProperty"));
    assert!(!is_absent_error_name("org.freedesktop.DBus.Error.AccessDenied"));
    assert!(!is_absent_error_name("org.freedesktop.DBus.Error.NoReply"));
}

#[test]
fn battery_presence_is_said_by_the_system_not_guessed() {
    assert_eq!(battery_reading_from_upower(false, 0.0, 0).present, Some(false));
    assert_eq!(battery_reading_from_upower(true, 55.0, 2).present, Some(true));
    assert_eq!(battery_reading_from_system_power_status(1, 128, 255).present, Some(false));
    assert_eq!(battery_reading_from_system_power_status(1, 255, 255).present, None, "unreadable flag is no answer");
    assert_eq!(battery_reading_from_system_power_status(0, 0, 255).present, Some(true));
    let missing = unavailable_by_default(PowerReadingKind::Battery, HostOs::Windows);
    let unanswered = PowerSnapshot::from_results(Ok(false), Err(missing));
    assert_eq!(unanswered.reading.battery.present, None);
}
