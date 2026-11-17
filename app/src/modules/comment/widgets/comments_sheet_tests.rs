use gpui::{TestAppContext, VisualContext};

use super::*;

fn keystroke(source: &str) -> Keystroke {
    Keystroke::parse(source).unwrap()
}

#[gpui::test]
fn escape_answers_only_in_the_window_of_the_sheet(cx: &mut TestAppContext) {
    let sheet_window = cx.add_empty_window().window_handle();
    let call_window = cx.add_empty_window().window_handle();
    let escape = keystroke("escape");

    assert!(is_thread_escape(false, &escape, sheet_window, sheet_window));
    assert!(!is_thread_escape(false, &escape, sheet_window, call_window));
}

#[gpui::test]
fn a_preview_a_modifier_and_another_key_never_answer(cx: &mut TestAppContext) {
    let window = cx.add_empty_window().window_handle();

    assert!(!is_thread_escape(true, &keystroke("escape"), window, window));
    assert!(!is_thread_escape(false, &keystroke("cmd-escape"), window, window));
    assert!(!is_thread_escape(false, &keystroke("enter"), window, window));
}
