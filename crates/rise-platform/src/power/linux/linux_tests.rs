use super::NativePower;
use crate::power::*;

#[test]
fn rules_build() {
    let rules = super::properties_changed_rules().expect("the match rules are valid");
    assert_eq!(rules.len(), 2);
}

#[test]
#[ignore = "needs a system bus with UPower: stage 2"]
fn battery_reads_the_display_device() {
    let reading = NativePower.battery().expect("UPower answers or is absent");
    if let Some(percent) = reading.percent {
        assert!(percent <= 100);
    }
}

#[test]
#[ignore = "needs a system bus: stage 2"]
fn low_power_mode_answers_without_the_daemon() {
    NativePower
        .low_power_mode()
        .expect("power-profiles-daemon answers or is absent");
}

#[test]
#[ignore = "needs a system bus: stage 2"]
fn watch_starts_and_stops() {
    let watch = NativePower.watch(Box::new(|| {})).expect("the system bus takes the match rules");
    drop(watch);
}
