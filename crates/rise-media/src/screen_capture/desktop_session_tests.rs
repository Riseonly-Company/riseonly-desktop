use crate::screen_capture::desktop_session::*;
use crate::screen_capture::screen_capture::CaptureSourceKind;

#[test]
fn wayland_needs_the_session_type_and_the_display_variable() {
    assert_eq!(desktop_session("wayland", true, ":0"), DesktopSession::Portal);
    assert_eq!(desktop_session("wayland", true, ""), DesktopSession::Portal);
    assert_eq!(desktop_session("wayland", false, ":0"), DesktopSession::X11);
    assert_eq!(desktop_session("wayland", false, ""), DesktopSession::NoDisplay);
}

#[test]
fn x11_sessions_and_unset_session_types_capture_through_the_x_display() {
    assert_eq!(desktop_session("x11", false, ":0"), DesktopSession::X11);
    assert_eq!(desktop_session("x11", true, ":1"), DesktopSession::X11);
    assert_eq!(desktop_session("", false, ":0"), DesktopSession::X11);
    assert_eq!(desktop_session("tty", false, ""), DesktopSession::NoDisplay);
}

#[test]
fn x11_ids_round_trip() {
    let sources: [(CaptureSourceKind, u64); 3] =
        [(CaptureSourceKind::Display, 0), (CaptureSourceKind::Display, 571), (CaptureSourceKind::Window, 0x0420_0007)];
    for (kind, native) in sources {
        assert_eq!(parse_x11_source_id(&x11_source_id(kind, native)), Some((kind, native)));
    }
    assert_eq!(x11_source_id(CaptureSourceKind::Window, 12), "x11-window:12");
}

#[test]
fn portal_and_broken_ids_are_not_x11() {
    assert_eq!(parse_x11_source_id("portal-3"), None);
    assert_eq!(parse_x11_source_id("x11-screen:"), None);
    assert_eq!(parse_x11_source_id("x11-window:-1"), None);
    assert_eq!(parse_x11_source_id("screen:1"), None);
}

#[test]
fn a_screen_takes_the_size_of_its_monitor() {
    let monitors = [
        X11Monitor { name: 300, width: 2560, height: 1440 },
        X11Monitor { name: 301, width: 1920, height: 1080 },
    ];
    assert_eq!(x11_screen_pixels(301, &monitors, (4480, 1440)), Some((1920, 1080)));
    assert_eq!(x11_screen_pixels(999, &monitors, (4480, 1440)), None);
}

#[test]
fn whole_desktop_ids_take_the_root_size() {
    assert_eq!(x11_screen_pixels(WHOLE_ROOT_SCREEN_ID, &[], (1366, 768)), Some((1366, 768)));
    assert_eq!(x11_screen_pixels(FULL_DESKTOP_SCREEN_ID, &[], (1366, 768)), Some((1366, 768)));
    assert_eq!(x11_screen_pixels(WHOLE_ROOT_SCREEN_ID, &[], (0, 768)), None);
}

#[test]
fn empty_windows_have_no_size() {
    assert_eq!(x11_window_pixels(800, 600), Some((800, 600)));
    assert_eq!(x11_window_pixels(0, 600), None);
    assert_eq!(x11_window_pixels(800, 0), None);
}
