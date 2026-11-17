use crate::display_sleep::freedesktop_inhibit::*;
use crate::display_sleep::CALL_IN_PROGRESS_REASON;

#[test]
fn the_portal_is_tried_before_the_screen_saver() {
    assert_eq!(INHIBIT_ROUTES, [InhibitRoute::Portal, InhibitRoute::ScreenSaver]);
}

#[test]
fn only_the_idle_flag_is_held() {
    assert_eq!(IDLE_FLAG, 8);
}

#[test]
fn the_token_uses_only_letters_digits_and_underscores() {
    let token = inhibit_token(42);
    assert_eq!(token, "riseonly_display_42");
    assert!(token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
}

#[test]
fn a_blank_reason_falls_back_to_the_call_reason() {
    assert_eq!(inhibit_reason(""), CALL_IN_PROGRESS_REASON);
    assert_eq!(inhibit_reason("  \n"), CALL_IN_PROGRESS_REASON);
    assert_eq!(inhibit_reason("video call"), "video call");
}

#[test]
fn the_no_service_detail_names_every_failed_route() {
    let detail = no_service_detail(&[
        (InhibitRoute::Portal, "no portal".to_owned()),
        (InhibitRoute::ScreenSaver, "unknown name".to_owned()),
    ]);
    assert_eq!(detail, "Portal: no portal; ScreenSaver: unknown name");
    assert_eq!(no_service_detail(&[]), "no inhibit route was tried");
}
