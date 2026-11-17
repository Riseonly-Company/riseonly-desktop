use gpui::PromptButton;
use rise_platform::HostOs;

use crate::in_window_prompt::{answer_for_key, answers_fit_in_row, is_default_answer, uses_in_window_prompt};

fn delete_answers() -> Vec<PromptButton> {
    vec![
        PromptButton::cancel("Cancel"),
        PromptButton::ok("Delete"),
        PromptButton::new("Archive"),
    ]
}

#[test]
fn every_host_draws_the_dialog_inside_the_window() {
    assert!(uses_in_window_prompt(HostOs::Linux));
    assert!(uses_in_window_prompt(HostOs::MacOs));
    assert!(uses_in_window_prompt(HostOs::Windows));
}

#[test]
fn escape_picks_the_cancel_button_and_enter_the_ok_button() {
    let answers = delete_answers();
    assert_eq!(answer_for_key("escape", &answers), Some(0));
    assert_eq!(answer_for_key("enter", &answers), Some(1));
    assert_eq!(answer_for_key("space", &answers), None);
}

#[test]
fn without_a_cancel_button_escape_does_nothing() {
    let answers = vec![PromptButton::ok("Ok"), PromptButton::new("Later")];
    assert_eq!(answer_for_key("escape", &answers), None);
}

#[test]
fn without_an_ok_button_enter_takes_the_first() {
    let answers = vec![PromptButton::new("Open"), PromptButton::cancel("Cancel")];
    assert_eq!(answer_for_key("enter", &answers), Some(0));
    assert!(is_default_answer(0, &answers));
    assert!(!is_default_answer(1, &answers));
}

#[test]
fn no_buttons_no_answer() {
    assert_eq!(answer_for_key("enter", &[]), None);
    assert_eq!(answer_for_key("escape", &[]), None);
}

#[test]
fn two_short_answers_sit_side_by_side() {
    assert!(answers_fit_in_row(&[60.0, 50.0]));
    assert!(answers_fit_in_row(&[40.0]));
}

#[test]
fn a_third_answer_or_a_long_label_stacks_the_answers() {
    assert!(!answers_fit_in_row(&[60.0, 50.0, 40.0]));
    assert!(!answers_fit_in_row(&[60.0, 220.0]));
}
