use gpui::Keystroke;

use super::context_menu::{ContextMenuKeyAction, context_menu_key_action, is_context_menu_keystroke};

fn keystroke(source: &str) -> Keystroke {
    Keystroke::parse(source).expect("valid keystroke")
}

#[test]
fn menu_key_and_shift_f10_open_the_menu() {
    assert!(is_context_menu_keystroke(&keystroke("menu")));
    assert!(is_context_menu_keystroke(&keystroke("shift-f10")));
}

#[test]
fn other_keys_do_not_open_the_menu() {
    assert!(!is_context_menu_keystroke(&keystroke("f10")));
    assert!(!is_context_menu_keystroke(&keystroke("ctrl-shift-f10")));
    assert!(!is_context_menu_keystroke(&keystroke("ctrl-menu")));
    assert!(!is_context_menu_keystroke(&keystroke("enter")));
}

#[test]
fn escape_dismisses_and_a_modified_or_foreign_key_passes_through() {
    let enabled = [true, true];
    assert_eq!(context_menu_key_action(&keystroke("escape"), None, &enabled), Some(ContextMenuKeyAction::Dismiss));
    assert_eq!(context_menu_key_action(&keystroke("shift-escape"), None, &enabled), None);
    assert_eq!(context_menu_key_action(&keystroke("a"), None, &enabled), None);
}

#[test]
fn arrows_walk_enabled_rows_and_stop_at_the_ends() {
    let enabled = [false, true, false, true];
    let highlight = |key: &str, from: Option<usize>| context_menu_key_action(&keystroke(key), from, &enabled);
    assert_eq!(highlight("down", None), Some(ContextMenuKeyAction::Highlight(1)));
    assert_eq!(highlight("down", Some(1)), Some(ContextMenuKeyAction::Highlight(3)));
    assert_eq!(highlight("down", Some(3)), Some(ContextMenuKeyAction::Swallow));
    assert_eq!(highlight("up", None), Some(ContextMenuKeyAction::Highlight(3)));
    assert_eq!(highlight("up", Some(3)), Some(ContextMenuKeyAction::Highlight(1)));
    assert_eq!(highlight("up", Some(1)), Some(ContextMenuKeyAction::Swallow));
    assert_eq!(highlight("home", Some(3)), Some(ContextMenuKeyAction::Highlight(1)));
    assert_eq!(highlight("end", Some(1)), Some(ContextMenuKeyAction::Highlight(3)));
}

#[test]
fn a_stale_highlight_past_a_shrunk_list_stays_in_bounds() {
    let enabled = [true];
    assert_eq!(context_menu_key_action(&keystroke("up"), Some(4), &enabled), Some(ContextMenuKeyAction::Highlight(0)));
    assert_eq!(context_menu_key_action(&keystroke("down"), Some(4), &enabled), Some(ContextMenuKeyAction::Swallow));
    assert_eq!(context_menu_key_action(&keystroke("enter"), Some(4), &enabled), Some(ContextMenuKeyAction::Swallow));
}

#[test]
fn enter_picks_only_a_highlighted_enabled_row() {
    let enabled = [true, false];
    assert_eq!(context_menu_key_action(&keystroke("enter"), Some(0), &enabled), Some(ContextMenuKeyAction::Activate(0)));
    assert_eq!(context_menu_key_action(&keystroke("enter"), Some(1), &enabled), Some(ContextMenuKeyAction::Swallow));
    assert_eq!(context_menu_key_action(&keystroke("enter"), None, &enabled), Some(ContextMenuKeyAction::Swallow));
}
