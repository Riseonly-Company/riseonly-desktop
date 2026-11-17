pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const INHIBIT_INTERFACE: &str = "org.freedesktop.portal.Inhibit";
pub const INHIBIT_METHOD: &str = "Inhibit";
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
pub const CLOSE_METHOD: &str = "Close";
pub const HANDLE_TOKEN_OPTION: &str = "handle_token";
pub const REASON_OPTION: &str = "reason";

pub const SCREEN_SAVER_BUS_NAME: &str = "org.freedesktop.ScreenSaver";
pub const SCREEN_SAVER_OBJECT_PATH: &str = "/org/freedesktop/ScreenSaver";
pub const SCREEN_SAVER_INTERFACE: &str = "org.freedesktop.ScreenSaver";
pub const SCREEN_SAVER_INHIBIT_METHOD: &str = "Inhibit";
pub const SCREEN_SAVER_UNINHIBIT_METHOD: &str = "UnInhibit";

pub const SCREEN_SAVER_APPLICATION_NAME: &str = "Riseonly";

pub const IDLE_FLAG: u32 = 8;

pub const NO_PARENT_WINDOW: &str = "";

/// services tried for one hold, in order
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InhibitRoute {
    Portal,
    ScreenSaver,
}

pub const INHIBIT_ROUTES: [InhibitRoute; 2] = [InhibitRoute::Portal, InhibitRoute::ScreenSaver];

/// handle token of one hold, the spec allows letters and digits and underscores only
pub fn inhibit_token(serial: u64) -> String {
    format!("riseonly_display_{serial}")
}

/// reason sent to the system, never empty since gnome lists holds by it
pub fn inhibit_reason(reason: &str) -> &str {
    if reason.chars().all(char::is_whitespace) {
        super::display_sleep::CALL_IN_PROGRESS_REASON
    } else {
        reason
    }
}

/// no service text after every route failed, one route and error part per route
pub fn no_service_detail(failures: &[(InhibitRoute, String)]) -> String {
    if failures.is_empty() {
        return "no inhibit route was tried".to_owned();
    }
    failures
        .iter()
        .map(|(route, error)| format!("{route:?}: {error}"))
        .collect::<Vec<_>>()
        .join("; ")
}
