use std::time::Duration;

use super::foundation_date::{Date, RiseClock, TestClock};

#[test]
fn reference_date_matches_foundation() {
    let date = Date::from_time_interval_since_reference_date(0.0);
    assert_eq!(date.ms(), 978_307_200_000);
    assert_eq!(date.time_interval_since_1970(), 978_307_200.0);
    assert_eq!(date.time_interval_since_reference_date(), 0.0);
}

#[test]
fn intervals_round_to_the_millisecond() {
    let date = Date::from_time_interval_since_1970(1.2346);
    assert_eq!(date.ms(), 1_235);
    assert_eq!(date.adding_time_interval(-0.235).ms(), 1_000);
    assert_eq!(Date::from_ms(5_500).time_interval_since(&Date::from_ms(1_000)), 4.5);
    assert_eq!(Date::from_time_interval_since_1970(f64::NAN).ms(), 0);
}

#[test]
fn distant_dates_match_foundation() {
    assert_eq!(Date::distant_past().time_interval_since_reference_date(), -63_114_076_800.0);
    assert_eq!(Date::distant_future().time_interval_since_reference_date(), 63_113_904_000.0);
}

#[test]
fn installed_clock_drives_now_and_monotonic() {
    let clock = TestClock::new(Date::from_ms(1_000));
    let guard = RiseClock::install_test(clock.clone());
    assert_eq!(RiseClock::now(), Date::from_ms(1_000));
    assert_eq!(RiseClock::monotonic(), Duration::ZERO);
    clock.advance(Duration::from_millis(250));
    assert_eq!(Date::now(), Date::from_ms(1_250));
    assert_eq!(RiseClock::monotonic(), Duration::from_millis(250));
    clock.set_now(Date::from_ms(0));
    assert_eq!(RiseClock::now(), Date::from_ms(0));
    assert_eq!(RiseClock::monotonic(), Duration::from_millis(250));
    assert_eq!(Date::from_ms(10_000).time_interval_since_now(), 10.0);
    drop(guard);
    assert!(RiseClock::now().ms() > 1_600_000_000_000);
}

#[test]
fn nested_clocks_restore_the_outer_one() {
    let outer = RiseClock::install_test(TestClock::new(Date::from_ms(1)));
    {
        let _inner = RiseClock::install_test(TestClock::new(Date::from_ms(2)));
        assert_eq!(RiseClock::now(), Date::from_ms(2));
    }
    assert_eq!(RiseClock::now(), Date::from_ms(1));
    drop(outer);
}

#[test]
fn system_monotonic_never_goes_back() {
    let first = RiseClock::monotonic();
    let second = RiseClock::monotonic();
    assert!(second >= first);
}

