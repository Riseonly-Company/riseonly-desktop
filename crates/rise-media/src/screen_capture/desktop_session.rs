use super::screen_capture::CaptureSourceKind;

/// what libwebrtc captures in this session
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DesktopSession {
    Portal,
    X11,
    NoDisplay,
}

/// session from XDG_SESSION_TYPE, whether WAYLAND_DISPLAY is set at all and DISPLAY
/// libwebrtc tests presence only
pub fn desktop_session(session_type: &str, wayland_display_set: bool, x_display: &str) -> DesktopSession {
    if session_type.starts_with("wayland") && wayland_display_set {
        DesktopSession::Portal
    } else if !x_display.is_empty() {
        DesktopSession::X11
    } else {
        DesktopSession::NoDisplay
    }
}

pub const X11_SCREEN_ID_PREFIX: &str = "x11-screen:";
pub const X11_WINDOW_ID_PREFIX: &str = "x11-window:";

pub const FULL_DESKTOP_SCREEN_ID: u64 = u64::MAX;
pub const WHOLE_ROOT_SCREEN_ID: u64 = 0;

pub fn x11_source_id(kind: CaptureSourceKind, native: u64) -> String {
    match kind {
        CaptureSourceKind::Display => format!("{X11_SCREEN_ID_PREFIX}{native}"),
        CaptureSourceKind::Window => format!("{X11_WINDOW_ID_PREFIX}{native}"),
    }
}

/// kind and libwebrtc id of an x11 source, none for any other id (a portal pick)
pub fn parse_x11_source_id(id: &str) -> Option<(CaptureSourceKind, u64)> {
    if let Some(native) = id.strip_prefix(X11_SCREEN_ID_PREFIX) {
        return native.parse().ok().map(|native| (CaptureSourceKind::Display, native));
    }
    id.strip_prefix(X11_WINDOW_ID_PREFIX)?.parse().ok().map(|native| (CaptureSourceKind::Window, native))
}

/// one active randr monitor, its name atom and size in pixels
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct X11Monitor {
    pub name: u64,
    pub width: u32,
    pub height: u32,
}

/// pixel size of a screen libwebrtc listed
/// the monitor whose name atom is the id, the root window for whole desktop ids
/// none when the monitor is gone or has no area
pub fn x11_screen_pixels(id: u64, monitors: &[X11Monitor], root: (u32, u32)) -> Option<(u32, u32)> {
    let size = match monitors.iter().find(|monitor| monitor.name == id) {
        Some(monitor) => (monitor.width, monitor.height),
        None if id == WHOLE_ROOT_SCREEN_ID || id == FULL_DESKTOP_SCREEN_ID => root,
        None => return None,
    };
    nonzero(size)
}

/// pixel size of a window from its x geometry, the client area that the capturer copies
pub fn x11_window_pixels(width: u16, height: u16) -> Option<(u32, u32)> {
    nonzero((u32::from(width), u32::from(height)))
}

fn nonzero(size: (u32, u32)) -> Option<(u32, u32)> {
    (size.0 > 0 && size.1 > 0).then_some(size)
}
