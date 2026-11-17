use std::ffi::OsStr;
use std::path::PathBuf;

use super::autostart::AutostartConfig;
use super::background_portal::*;

fn config(arguments: &[&str]) -> AutostartConfig {
    AutostartConfig {
        id: "com.riseonly.Riseonly".to_owned(),
        display_name: "Riseonly".to_owned(),
        executable: PathBuf::from(String::from("/app/bin/riseonly desktop")),
        arguments: arguments.iter().map(|argument| (*argument).to_owned()).collect(),
    }
}

#[test]
fn a_plain_session_writes_its_own_entry() {
    assert_eq!(Sandbox::detect(false, None), Sandbox::Host);
    assert_eq!(Sandbox::detect(false, Some(OsStr::new(""))), Sandbox::Host);
    assert!(!Sandbox::Host.needs_portal());
}

#[test]
fn flatpak_and_snap_go_through_the_portal() {
    assert_eq!(Sandbox::detect(true, None), Sandbox::Flatpak);
    assert_eq!(Sandbox::detect(false, Some(OsStr::new("riseonly"))), Sandbox::Snap);
    assert!(Sandbox::Flatpak.needs_portal());
    assert!(Sandbox::Snap.needs_portal());
}

#[test]
fn the_flatpak_marker_wins_over_a_stray_snap_variable() {
    assert_eq!(Sandbox::detect(true, Some(OsStr::new("riseonly"))), Sandbox::Flatpak);
}

#[test]
fn the_commandline_is_the_executable_then_its_arguments_unescaped() {
    assert_eq!(
        background_commandline(&config(&["--hidden", "a b"])),
        vec!["/app/bin/riseonly desktop".to_owned(), "--hidden".to_owned(), "a b".to_owned()],
    );
    assert_eq!(background_commandline(&config(&[])), vec!["/app/bin/riseonly desktop".to_owned()]);
}

#[test]
fn the_request_token_uses_only_allowed_characters() {
    let token = request_token(42);
    assert!(token.chars().all(|character| character.is_ascii_alphanumeric() || character == '_'));
    assert_ne!(request_token(1), request_token(2));
}

#[test]
fn the_request_path_follows_the_sender_rule() {
    assert_eq!(
        request_path(":1.42", "riseonly_autostart_7"),
        "/org/freedesktop/portal/desktop/request/1_42/riseonly_autostart_7",
    );
}

#[test]
fn success_with_the_asked_state_is_applied() {
    assert_eq!(background_outcome(0, Some(true), true), BackgroundOutcome::Applied);
    assert_eq!(background_outcome(0, Some(false), false), BackgroundOutcome::Applied);
    assert!(background_outcome(0, Some(true), true).refusal_reason().is_none());
}

#[test]
fn an_older_portal_without_the_result_counts_as_applied() {
    assert!(background_outcome(0, None, true).is_applied());
    assert!(background_outcome(0, None, false).is_applied());
}

#[test]
fn success_with_the_other_state_is_not_applied() {
    let outcome = background_outcome(0, Some(false), true);
    assert_eq!(outcome, BackgroundOutcome::NotApplied);
    assert!(!outcome.is_applied());
    assert!(outcome.refusal_reason().is_some());
}

#[test]
fn a_cancelled_or_failed_request_is_refused() {
    for code in [1, 2, 7] {
        let outcome = background_outcome(code, Some(true), true);
        assert_eq!(outcome, BackgroundOutcome::Refused);
        assert!(outcome.refusal_reason().is_some());
    }
}
