use std::time::Duration;

use super::memory_pressure::MemoryPressureLevel::{Critical, Normal, Warning};
use super::psi::*;

const CALM_FILE: &str = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n\
                         full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

#[test]
fn the_triggers_are_the_kernel_text_with_a_two_second_window() {
    assert_eq!(PsiTrigger::WARNING.command(), "some 300000 2000000");
    assert_eq!(PsiTrigger::CRITICAL.command(), "full 200000 2000000");
    for trigger in PsiTrigger::ALL {
        assert_eq!(trigger.window_us % 2_000_000, 0, "unprivileged windows are multiples of 2 s");
        assert!(trigger.stall_us < trigger.window_us);
    }
    assert_eq!(PsiTrigger::ALL.map(|trigger| trigger.level), [Warning, Critical]);
}

#[test]
fn the_file_parses_both_lines() {
    let text = "some avg10=12.50 avg60=3.10 avg300=0.80 total=123456\n\
                full avg10=6.25 avg60=1.00 avg300=0.10 total=4567\n";
    assert_eq!(
        parse_psi(text),
        Some(PsiReading {
            some_avg10: 12.5,
            full_avg10: 6.25
        })
    );
    assert_eq!(parse_psi(CALM_FILE), Some(PsiReading::default()));
}

#[test]
fn a_missing_full_line_reads_as_zero_and_no_some_line_is_no_reading() {
    assert_eq!(
        parse_psi("some avg10=1.00 avg60=0.00 avg300=0.00 total=1\n"),
        Some(PsiReading {
            some_avg10: 1.0,
            full_avg10: 0.0
        })
    );
    assert_eq!(parse_psi("full avg10=1.00 avg60=0.00 avg300=0.00 total=1\n"), None);
    assert_eq!(parse_psi("some avg10=oops total=1\n"), None);
    assert_eq!(parse_psi("some avg10=-1.00 total=1\n"), None);
    assert_eq!(parse_psi(""), None);
}

#[test]
fn rereads_map_below_the_trigger_thresholds() {
    let reading = |some_avg10, full_avg10| level_from_reading(PsiReading { some_avg10, full_avg10 });
    assert_eq!(reading(0.0, 0.0), Normal);
    assert_eq!(reading(9.99, 4.99), Normal);
    assert_eq!(reading(10.0, 0.0), Warning);
    assert_eq!(reading(40.0, 4.99), Warning);
    assert_eq!(reading(5.0, 5.0), Critical);
}

#[test]
fn events_only_raise_the_level() {
    let mut tracker = PsiLevelTracker::default();
    assert_eq!(tracker.on_event(Warning, secs(1)), Some(Warning));
    assert_eq!(tracker.on_event(Warning, secs(3)), None);
    assert_eq!(tracker.on_event(Critical, secs(5)), Some(Critical));
    assert_eq!(tracker.on_event(Warning, secs(7)), None);
    assert_eq!(tracker.level(), Critical);
}

#[test]
fn normal_comes_from_a_reread_after_the_calm_period() {
    let mut tracker = PsiLevelTracker::default();
    assert!(!tracker.recheck_due(secs(100)), "Normal never rereads");
    tracker.on_event(Warning, secs(1));
    tracker.on_event(Warning, secs(3));
    assert!(!tracker.recheck_due(secs(3) + PSI_CALM - Duration::from_millis(1)));
    assert!(tracker.recheck_due(secs(3) + PSI_CALM));
    assert_eq!(tracker.on_reading(Warning, secs(13)), None, "still above the bar: hold");
    assert!(!tracker.recheck_due(secs(14)), "a reread restarts the calm period");
    assert_eq!(tracker.on_reading(Normal, secs(23)), Some(Normal));
    assert!(!tracker.recheck_due(secs(60)));
}

#[test]
fn a_reread_can_step_down_one_level() {
    let mut tracker = PsiLevelTracker::default();
    tracker.on_event(Critical, secs(0));
    assert_eq!(tracker.on_reading(Warning, secs(10)), Some(Warning));
    assert_eq!(tracker.on_event(Critical, secs(11)), Some(Critical));
}

#[test]
fn the_first_read_reports_pressure_already_there() {
    let mut tracker = PsiLevelTracker::default();
    assert_eq!(tracker.on_reading(Normal, secs(0)), None);
    assert_eq!(tracker.on_reading(Warning, secs(0)), Some(Warning));
}

#[test]
fn refusals_name_the_file_and_the_cause() {
    let missing = psi_refusal(PsiStep::Open, Some(2), "No such file or directory");
    assert!(missing.starts_with("opening /proc/pressure/memory: "));
    assert!(missing.contains("CONFIG_PSI"));
    assert!(psi_refusal(PsiStep::Trigger, Some(13), "denied").contains("CAP_SYS_RESOURCE"));
    assert!(psi_refusal(PsiStep::Trigger, Some(1), "denied").contains("CAP_SYS_RESOURCE"));
    assert!(psi_refusal(PsiStep::Trigger, Some(22), "invalid").contains("rejected the trigger"));
    assert!(psi_refusal(PsiStep::Open, Some(95), "unsupported").contains("psi=0"));
    assert_eq!(
        psi_refusal(PsiStep::Read, Some(5), "Input/output error"),
        "reading /proc/pressure/memory: Input/output error"
    );
    assert_eq!(
        psi_refusal(PsiStep::Read, None, "not a reading"),
        "reading /proc/pressure/memory: not a reading"
    );
}
