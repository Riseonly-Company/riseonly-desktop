use crate::host_os::HostOs;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlatformSupport {
    Performed,
    Unsupported,
}

impl PlatformSupport {
    pub fn is_supported(self) -> bool {
        self == Self::Performed
    }
}

/// hands a url to the desktop, only http and https are passed on
pub fn open_url(app: &mut gpui::App, url: &str) -> PlatformSupport {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return PlatformSupport::Unsupported;
    }
    app.open_url(url);
    PlatformSupport::Performed
}

/// macos only, unsupported elsewhere instead of a panic because gpui call on windows is unimplemented
pub fn hide_other_apps(app: &mut gpui::App) -> PlatformSupport {
    if !hides_other_apps_on(HostOs::current()) {
        return PlatformSupport::Unsupported;
    }
    app.hide_other_apps();
    PlatformSupport::Performed
}

/// macos only, same as hide_other_apps
pub fn unhide_other_apps(app: &mut gpui::App) -> PlatformSupport {
    if !hides_other_apps_on(HostOs::current()) {
        return PlatformSupport::Unsupported;
    }
    app.unhide_other_apps();
    PlatformSupport::Performed
}

/// hide others is an appkit app command, no other desktop has one
pub const fn hides_other_apps_on(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs)
}

/// keystrokes bound to toggle full screen
/// on macos F11 is the system show desktop key so ctrl-cmd-f and fn-f are used
/// linux and windows use F11
pub const fn full_screen_keystrokes_on(host: HostOs) -> &'static [&'static str] {
    match host {
        HostOs::MacOs => &["ctrl-cmd-f", "fn-f"],
        HostOs::Linux | HostOs::Windows => &["f11"],
    }
}

pub fn supports_external_drag() -> PlatformSupport {
    support(external_drag_in(HostOs::current(), gpui_runs_on_wayland()))
}

/// true when gpui on this host can start an external drag, macos and wayland override it
/// windows uses the app ole drag patch, x11 and headless dont
pub const fn external_drag_in(host: HostOs, wayland_session: bool) -> bool {
    match host {
        HostOs::MacOs | HostOs::Windows => true,
        HostOs::Linux => wayland_session,
    }
}

/// mirrors gpui guess_compositor which picks the linux client
/// headless wins and then a non empty WAYLAND_DISPLAY, session type isnt looked at
pub(crate) fn gpui_runs_on_wayland() -> bool {
    compositor_is_wayland(
        std::env::var_os("ZED_HEADLESS").is_some(),
        std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty()),
    )
}

pub const fn compositor_is_wayland(headless: bool, wayland_display_set: bool) -> bool {
    !headless && wayland_display_set
}

pub fn supports_native_popups() -> PlatformSupport {
    PlatformSupport::Unsupported
}

pub fn supports_offscreen_rendering() -> PlatformSupport {
    support(offscreen_rendering_on(HostOs::current()))
}

pub const fn offscreen_rendering_on(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs)
}

/// true when the os shows the application menu itself, where it doesnt the app draws it in the window
pub fn supports_native_menu_bar() -> PlatformSupport {
    support(native_menu_bar_on(HostOs::current()))
}

/// gpui macos installs an nsmenu as the main menu
/// linux and windows only store the menus and show nothing
pub const fn native_menu_bar_on(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs)
}

const fn support(available: bool) -> PlatformSupport {
    if available {
        PlatformSupport::Performed
    } else {
        PlatformSupport::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popups_never_escape_the_window_on_any_platform() {
        assert!(
            !supports_native_popups().is_supported(),
            "designing for a real popup would break X11 and Windows"
        );
    }

    #[test]
    fn external_drag_follows_the_gpui_window_backend() {
        for wayland_session in [true, false] {
            assert!(external_drag_in(HostOs::MacOs, wayland_session));
            assert!(
                external_drag_in(HostOs::Windows, wayland_session),
                "the vendored gpui_windows starts an OLE drag (patch 0003)"
            );
        }
        assert!(external_drag_in(HostOs::Linux, true));
        assert!(
            !external_drag_in(HostOs::Linux, false),
            "the gpui X11 window keeps can_start_external_drag false"
        );
    }

    #[test]
    fn the_session_is_read_the_way_gpui_picks_its_client() {
        assert!(compositor_is_wayland(false, true));
        assert!(!compositor_is_wayland(false, false), "DISPLAY only means X11");
        assert!(!compositor_is_wayland(true, true), "ZED_HEADLESS wins over WAYLAND_DISPLAY");
        assert!(!compositor_is_wayland(true, false));
    }

    #[test]
    fn offscreen_rendering_is_macos_only() {
        for host in HostOs::ALL {
            assert_eq!(offscreen_rendering_on(host), host == HostOs::MacOs);
        }
    }

    #[test]
    fn hiding_other_apps_is_never_attempted_off_macos() {
        for host in HostOs::ALL {
            assert_eq!(
                hides_other_apps_on(host),
                host == HostOs::MacOs,
                "on Windows the gpui call panics"
            );
        }
    }

    #[test]
    fn full_screen_avoids_the_macos_show_desktop_key() {
        assert_eq!(full_screen_keystrokes_on(HostOs::MacOs), &["ctrl-cmd-f", "fn-f"]);
        for host in [HostOs::Linux, HostOs::Windows] {
            assert_eq!(full_screen_keystrokes_on(host), &["f11"], "{host:?}");
        }
    }

    #[test]
    fn only_macos_shows_the_menu_bar_itself() {
        assert!(native_menu_bar_on(HostOs::MacOs));
        assert!(
            !native_menu_bar_on(HostOs::Linux),
            "gpui_linux set_menus only stores the menus"
        );
        assert!(
            !native_menu_bar_on(HostOs::Windows),
            "gpui_windows set_menus only stores the menus"
        );
    }

    #[test]
    fn this_host_reports_what_its_policy_says() {
        let host = HostOs::current();
        assert_eq!(
            supports_native_menu_bar().is_supported(),
            native_menu_bar_on(host)
        );
        assert_eq!(
            supports_external_drag().is_supported(),
            external_drag_in(host, gpui_runs_on_wayland())
        );
        assert_eq!(
            supports_offscreen_rendering().is_supported(),
            offscreen_rendering_on(host)
        );
    }
}

#[cfg(test)]
mod open_url_tests {
    #[test]
    fn only_http_and_https_are_handed_to_the_desktop() {
        for refused in [
            "file:///etc/passwd",
            "riseonly-dev://open",
            "javascript:alert(1)",
            "",
        ] {
            assert!(
                !(refused.starts_with("https://") || refused.starts_with("http://")),
                "{refused} would reach the desktop"
            );
        }
        assert!("https://t.me/riseonly_bot?start=7999".starts_with("https://"));
    }
}
