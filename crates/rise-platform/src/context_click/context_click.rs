use gpui::{Modifiers, MouseButton};

use crate::host_os::HostOs;

pub const MENU_KEY: &str = "menu";

pub const SHIFT_F10: &str = "shift-f10";

/// true when this press asks for the context menu on host
pub fn is_context_click(button: MouseButton, modifiers: &Modifiers, host: HostOs) -> bool {
    match button {
        MouseButton::Right => true,
        MouseButton::Left => opens_on_control_click(host) && modifiers.control,
        MouseButton::Middle | MouseButton::Navigate(_) => false,
    }
}

/// is_context_click for the running host
pub fn is_context_click_here(button: MouseButton, modifiers: &Modifiers) -> bool {
    is_context_click(button, modifiers, HostOs::current())
}

/// true when a control click is the secondary click of host
pub const fn opens_on_control_click(host: HostOs) -> bool {
    match host {
        HostOs::MacOs => true,
        HostOs::Windows | HostOs::Linux => false,
    }
}

/// gpui keystrokes that open the context menu of the focused source
/// empty when the host has no such keys
pub fn context_menu_keystrokes(host: HostOs) -> Vec<&'static str> {
    match host {
        HostOs::MacOs => Vec::new(),
        HostOs::Windows | HostOs::Linux => vec![MENU_KEY, SHIFT_F10],
    }
}

pub fn context_menu_keystrokes_here() -> Vec<&'static str> {
    context_menu_keystrokes(HostOs::current())
}
