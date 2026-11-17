use crate::accessibility::*;
use crate::host_os::HostOs;

fn delivered(values: &[bool]) -> Vec<bool> {
    let mut gate = SettingChangeGate::default();
    values.iter().map(|value| gate.should_deliver(*value)).collect()
}

#[test]
fn the_first_value_is_always_a_change() {
    assert_eq!(delivered(&[false]), [true]);
    assert_eq!(delivered(&[true]), [true]);
}

#[test]
fn a_repeated_value_is_delivered_once() {
    assert_eq!(delivered(&[true, true, false, false, true]), [true, false, true, false, true]);
}

#[test]
fn a_seed_fills_an_empty_gate() {
    let mut gate = SettingChangeGate::default();
    assert!(gate.seed(true));
    assert_eq!(gate.last(), Some(true));
    assert!(!gate.should_deliver(true));
    assert!(gate.should_deliver(false));
}

#[test]
fn a_change_seen_before_the_seed_wins() {
    let mut gate = SettingChangeGate::default();
    assert!(gate.should_deliver(true));
    assert!(gate.seed(false));
    assert_eq!(gate.last(), Some(true));
}

#[test]
fn transparency_effects_off_is_reduce_transparency() {
    assert!(reduce_transparency_from_advanced_effects(false));
    assert!(!reduce_transparency_from_advanced_effects(true));
}

#[test]
fn no_answer_keeps_the_glass() {
    assert!(!reduce_transparency_or_default(Err(AccessibilityUnavailable::BranchNotWritten(
        HostOs::Linux
    ))));
    assert!(!reduce_transparency_or_default(Err(AccessibilityUnavailable::Unsupported("none"))));
    assert!(reduce_transparency_or_default(Ok(true)));
    assert!(!reduce_transparency_or_default(Ok(false)));
}

#[test]
fn an_unwritten_layer_names_the_host() {
    struct Unwritten;
    impl AccessibilitySettings for Unwritten {}

    let host = HostOs::current();
    assert_eq!(
        Unwritten.reduce_transparency(),
        Err(AccessibilityUnavailable::BranchNotWritten(host))
    );
    let watched = Unwritten.watch_reduce_transparency(Box::new(|_| {}));
    assert_eq!(watched.err(), Some(AccessibilityUnavailable::BranchNotWritten(host)));
    assert_eq!(Unwritten.reduce_motion(), Err(AccessibilityUnavailable::BranchNotWritten(host)));
    let watched = Unwritten.watch_reduce_motion(Box::new(|_| {}));
    assert_eq!(watched.err(), Some(AccessibilityUnavailable::BranchNotWritten(host)));
}

#[test]
fn no_answer_keeps_the_motion() {
    assert!(!reduce_motion_or_default(Err(AccessibilityUnavailable::BranchNotWritten(HostOs::Windows))));
    assert!(!reduce_motion_or_default(Err(AccessibilityUnavailable::SourceRefused("gone".into()))));
    assert!(reduce_motion_or_default(Ok(true)));
    assert!(!reduce_motion_or_default(Ok(false)));
}
