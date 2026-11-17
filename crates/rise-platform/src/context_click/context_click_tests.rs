use gpui::{Keystroke, Modifiers, MouseButton, NavigationDirection};

use crate::context_click::*;
use crate::host_os::HostOs;

#[test]
fn right_button_opens_the_menu_on_every_host() {
    for host in HostOs::ALL {
        assert!(is_context_click(MouseButton::Right, &Modifiers::none(), host), "{host:?}");
        assert!(is_context_click(MouseButton::Right, &Modifiers::control(), host), "{host:?}");
    }
}

#[test]
fn control_click_opens_the_menu_only_on_macos() {
    assert!(is_context_click(MouseButton::Left, &Modifiers::control(), HostOs::MacOs));
    assert!(is_context_click(MouseButton::Left, &Modifiers::control_shift(), HostOs::MacOs));
    assert!(!is_context_click(MouseButton::Left, &Modifiers::control(), HostOs::Windows));
    assert!(!is_context_click(MouseButton::Left, &Modifiers::control(), HostOs::Linux));
}

#[test]
fn plain_and_command_left_clicks_stay_primary() {
    for host in HostOs::ALL {
        assert!(!is_context_click(MouseButton::Left, &Modifiers::none(), host), "{host:?}");
        assert!(!is_context_click(MouseButton::Left, &Modifiers::command(), host), "{host:?}");
        assert!(!is_context_click(MouseButton::Left, &Modifiers::shift(), host), "{host:?}");
    }
}

#[test]
fn middle_and_navigation_buttons_never_open_the_menu() {
    for host in HostOs::ALL {
        for button in [
            MouseButton::Middle,
            MouseButton::Navigate(NavigationDirection::Back),
            MouseButton::Navigate(NavigationDirection::Forward),
        ] {
            assert!(!is_context_click(button, &Modifiers::control(), host), "{host:?} {button:?}");
        }
    }
}

#[test]
fn macos_binds_no_context_menu_key() {
    assert!(context_menu_keystrokes(HostOs::MacOs).is_empty());
}

#[test]
fn windows_and_linux_bind_menu_key_and_shift_f10() {
    for host in [HostOs::Windows, HostOs::Linux] {
        assert_eq!(context_menu_keystrokes(host), vec![MENU_KEY, SHIFT_F10], "{host:?}");
    }
}

#[test]
fn bound_keystrokes_parse_as_gpui_keystrokes() {
    let menu = Keystroke::parse(MENU_KEY).expect("menu parses");
    assert_eq!(menu.key, "menu");
    assert_eq!(menu.modifiers, Modifiers::none());

    let shift_f10 = Keystroke::parse(SHIFT_F10).expect("shift-f10 parses");
    assert_eq!(shift_f10.key, "f10");
    assert_eq!(shift_f10.modifiers, Modifiers::shift());
}

#[test]
fn control_click_policy_matches_the_click_answer() {
    for host in HostOs::ALL {
        assert_eq!(
            is_context_click(MouseButton::Left, &Modifiers::control(), host),
            opens_on_control_click(host),
            "{host:?}"
        );
    }
}
