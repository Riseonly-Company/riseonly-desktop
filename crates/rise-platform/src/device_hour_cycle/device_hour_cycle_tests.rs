use crate::device_hour_cycle::*;

#[test]
fn an_icu_pattern_with_a_day_period_is_twelve_hour() {
    assert!(HourPattern::Icu("h a".into()).is_twelve_hour());
    assert!(HourPattern::Icu("a h".into()).is_twelve_hour());
    assert!(!HourPattern::Icu("HH".into()).is_twelve_hour());
    assert!(
        !HourPattern::Icu(String::new()).is_twelve_hour(),
        "iOS reads an empty pattern as 24 hour time"
    );
}

#[test]
fn a_windows_format_is_twelve_hour_by_its_lowercase_hour_or_marker() {
    assert!(HourPattern::WindowsTimeFormat("h:mm:ss tt".into()).is_twelve_hour());
    assert!(HourPattern::WindowsTimeFormat("tt hh:mm:ss".into()).is_twelve_hour());
    assert!(!HourPattern::WindowsTimeFormat("HH:mm:ss".into()).is_twelve_hour());
    assert!(
        !HourPattern::WindowsTimeFormat("H' h 'mm".into()).is_twelve_hour(),
        "the quoted h is literal text, not the hour field"
    );
}

#[test]
fn a_posix_format_is_twelve_hour_by_its_conversion() {
    assert!(HourPattern::PosixTimeFormat("%I:%M:%S %p".into()).is_twelve_hour());
    assert!(HourPattern::PosixTimeFormat("%r".into()).is_twelve_hour());
    assert!(HourPattern::PosixTimeFormat("%-l:%M".into()).is_twelve_hour());
    assert!(!HourPattern::PosixTimeFormat("%H:%M:%S".into()).is_twelve_hour());
    assert!(!HourPattern::PosixTimeFormat("%T".into()).is_twelve_hour());
    assert!(
        !HourPattern::PosixTimeFormat("%%I".into()).is_twelve_hour(),
        "%% is a literal percent sign"
    );
}

#[test]
fn reading_the_real_hour_cycle_never_panics() {
    let _ = prefers_twelve_hour();
}
