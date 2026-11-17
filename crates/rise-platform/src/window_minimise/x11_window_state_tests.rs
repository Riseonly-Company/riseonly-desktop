use super::x11_window_state::*;

const HIDDEN: u32 = 400;
const MAXIMIZED: u32 = 401;
const NET_WM_STATE: u32 = 300;
const WM_STATE: u32 = 301;

fn state(supports: bool, net: &[u32], wm: Option<u32>) -> X11WindowState {
    X11WindowState {
        hidden_atom: HIDDEN,
        manager_supports_hidden: supports,
        net_wm_state: net.to_vec(),
        wm_state: wm,
    }
}

#[test]
fn an_ewmh_hidden_window_is_minimised() {
    assert!(state(true, &[MAXIMIZED, HIDDEN], Some(ICONIC_STATE)).minimised());
}

#[test]
fn an_ewmh_manager_iconic_on_another_desktop_is_not_minimised() {
    assert!(!state(true, &[MAXIMIZED], Some(ICONIC_STATE)).minimised());
}

#[test]
fn a_plain_icccm_manager_iconic_window_is_minimised() {
    assert!(state(false, &[], Some(ICONIC_STATE)).minimised());
}

#[test]
fn a_normal_window_is_visible() {
    assert!(!state(false, &[], Some(1)).minimised());
    assert!(!state(true, &[], None).minimised());
    assert!(!state(false, &[], None).minimised());
}

#[test]
fn a_hidden_atom_counts_even_when_not_announced() {
    assert!(state(false, &[HIDDEN], None).minimised());
}

#[test]
fn only_the_two_state_properties_trigger_a_reread() {
    assert!(is_state_property(NET_WM_STATE, NET_WM_STATE, WM_STATE));
    assert!(is_state_property(WM_STATE, NET_WM_STATE, WM_STATE));
    assert!(!is_state_property(HIDDEN, NET_WM_STATE, WM_STATE));
}
