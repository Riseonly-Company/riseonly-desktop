use super::system_clock::*;
use crate::host_os::HostOs;

struct Unwritten;

impl SystemClockSource for Unwritten {}

#[test]
fn an_unwritten_layer_names_itself_instead_of_staying_silent() {
    let answer = Unwritten.watch(Box::new(|_| {}));
    assert_eq!(
        answer.err(),
        Some(SystemClockUnavailable::BranchNotWritten(HostOs::current()))
    );
}

#[test]
fn a_layer_with_a_zone_file_names_no_zone_itself() {
    assert_eq!(Unwritten.system_zone_name(), None);
}

#[test]
fn a_new_zone_value_touches_the_zone() {
    assert!(timedate_change_touches_zone(["Timezone"].into_iter(), &[]));
    assert!(timedate_change_touches_zone(["NTP", "Timezone"].into_iter(), &[]));
}

#[test]
fn an_invalidated_zone_touches_the_zone() {
    assert!(timedate_change_touches_zone(
        std::iter::empty(),
        &["Timezone".to_owned()]
    ));
}

#[test]
fn the_ntp_and_rtc_switches_do_not_touch_the_zone() {
    assert!(!timedate_change_touches_zone(
        ["NTP", "LocalRTC"].into_iter(),
        &["NTPSynchronized".to_owned()]
    ));
    assert!(!timedate_change_touches_zone(std::iter::empty(), &[]));
}

#[test]
fn only_the_intl_setting_is_a_locale_change() {
    assert!(windows_setting_is_locale(Some("intl")));
    assert!(!windows_setting_is_locale(Some("Environment")));
    assert!(!windows_setting_is_locale(Some("Policy")));
    assert!(!windows_setting_is_locale(None));
}

#[test]
fn only_a_new_zone_name_counts_as_moved() {
    let mut seen = Some("Europe/Moscow".to_owned());
    assert!(!zone_name_moved(&mut seen, Some("Europe/Moscow".to_owned())));
    assert!(!zone_name_moved(&mut seen, None));
    assert_eq!(seen.as_deref(), Some("Europe/Moscow"));
    assert!(zone_name_moved(&mut seen, Some("Asia/Almaty".to_owned())));
    assert_eq!(seen.as_deref(), Some("Asia/Almaty"));
}

#[test]
fn a_first_read_after_a_failed_one_counts_as_moved() {
    let mut seen = None;
    assert!(zone_name_moved(&mut seen, Some("Asia/Almaty".to_owned())));
}
