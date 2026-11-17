use crate::tray::*;

fn anchor_word(x: i16, y: i16) -> usize {
    (x as u16 as usize) | ((y as u16 as usize) << 16)
}

#[test]
fn notification_codes_match_shellapi_and_winuser() {
    assert_eq!(TRAY_NIN_SELECT, 0x400);
    assert_eq!(TRAY_NIN_KEYSELECT, 0x401);
    assert_eq!(TRAY_WM_CONTEXTMENU, 0x7B);
    assert_eq!(TRAY_WM_LBUTTONUP, 0x202);
    assert_eq!(TRAY_WM_RBUTTONUP, 0x205);
}

#[test]
fn a_version_4_select_activates() {
    assert_eq!(
        tray_icon_action(true, anchor_word(10, 20), TRAY_NIN_SELECT as isize),
        Some(TrayIconAction::Activate)
    );
}

#[test]
fn a_version_4_keyselect_activates() {
    assert_eq!(
        tray_icon_action(true, 0, TRAY_NIN_KEYSELECT as isize),
        Some(TrayIconAction::Activate)
    );
}

#[test]
fn a_version_4_context_menu_anchors_at_the_point_in_wparam() {
    assert_eq!(
        tray_icon_action(true, anchor_word(1800, 1050), TRAY_WM_CONTEXTMENU as isize),
        Some(TrayIconAction::Menu {
            anchor: Some((1800, 1050))
        })
    );
}

#[test]
fn a_monitor_left_of_and_above_the_primary_gives_negative_points() {
    assert_eq!(
        tray_icon_action(true, anchor_word(-1200, -40), TRAY_WM_CONTEXTMENU as isize),
        Some(TrayIconAction::Menu {
            anchor: Some((-1200, -40))
        })
    );
}

#[test]
fn the_icon_id_in_the_high_word_of_lparam_is_ignored() {
    let lparam = ((7_isize) << 16) | TRAY_NIN_SELECT as isize;
    assert_eq!(
        tray_icon_action(true, 0, lparam),
        Some(TrayIconAction::Activate)
    );
}

#[test]
fn an_unknown_version_4_event_does_nothing() {
    assert_eq!(tray_icon_action(true, 0, 0x200), None);
    assert_eq!(tray_icon_action(true, 0, TRAY_WM_LBUTTONUP as isize), None);
}

#[test]
fn the_legacy_left_button_up_activates() {
    assert_eq!(
        tray_icon_action(false, 0, TRAY_WM_LBUTTONUP as isize),
        Some(TrayIconAction::Activate)
    );
}

#[test]
fn the_legacy_right_button_up_opens_the_menu_at_the_cursor() {
    assert_eq!(
        tray_icon_action(false, anchor_word(5, 5), TRAY_WM_RBUTTONUP as isize),
        Some(TrayIconAction::Menu { anchor: None })
    );
}

#[test]
fn an_unknown_legacy_event_does_nothing() {
    assert_eq!(tray_icon_action(false, 0, TRAY_NIN_SELECT as isize), None);
    assert_eq!(tray_icon_action(false, 0, 0x200), None);
}
