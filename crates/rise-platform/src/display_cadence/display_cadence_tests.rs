use crate::display_cadence::*;

#[test]
fn no_answer_is_a_sixty_hertz_panel() {
    assert_eq!(frames_per_second_or_fallback(None), 60);
    assert_eq!(
        frames_per_second_or_fallback(Some(0)),
        60,
        "Windows and Wayland report 0 for a mode without a rate"
    );
    assert_eq!(
        frames_per_second_or_fallback(Some(1)),
        60,
        "Windows reports 1 for the hardware default rate"
    );
}

#[test]
fn a_reported_rate_is_kept_as_is() {
    assert_eq!(frames_per_second_or_fallback(Some(59)), 59);
    assert_eq!(frames_per_second_or_fallback(Some(120)), 120);
    assert_eq!(frames_per_second_or_fallback(Some(240)), 240);
}

#[test]
fn millihertz_round_to_whole_frames() {
    assert_eq!(frames_per_second_from_millihertz(59_940), 60);
    assert_eq!(frames_per_second_from_millihertz(60_000), 60);
    assert_eq!(frames_per_second_from_millihertz(143_856), 144);
    assert_eq!(frames_per_second_from_millihertz(0), 0);
}
