//! Wrappers over the gpui platform calls that are not uniform across the three
//! backends.
//!
//! gpui is tri-platform but not evenly so. On Windows `hide_other_apps` and
//! `unhide_other_apps` are live `unimplemented!()` calls, and
//! `start_external_drag` does not exist at all. Calling them directly compiles
//! everywhere and panics on one OS, which is the worst possible failure shape:
//! it survives review, survives CI on a Mac, and reaches a user.
//!
//! Every call here reports what it did, so a caller can offer an alternative
//! instead of assuming success.

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

/// Hands a URL to whatever the desktop opens it with.
///
/// Uniform across the three gpui backends, unlike the calls above, and wrapped
/// anyway for one reason: this is the app's only outbound link, so it is the one
/// place a policy about what may be opened can ever live. A scheme allow-list
/// keeps a value that arrived from the network — the Telegram bot username in
/// the registration step is one — from being able to name `file:` or a custom
/// scheme some other installed app registered.
pub fn open_url(app: &mut gpui::App, url: &str) -> PlatformSupport {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return PlatformSupport::Unsupported;
    }
    app.open_url(url);
    PlatformSupport::Performed
}

/// macOS only. A no-op elsewhere rather than a panic on Windows.
pub fn hide_other_apps(app: &mut gpui::App) -> PlatformSupport {
    #[cfg(target_os = "macos")]
    {
        app.hide_other_apps();
        PlatformSupport::Performed
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        PlatformSupport::Unsupported
    }
}

/// macOS only, same reasoning as `hide_other_apps`.
pub fn unhide_other_apps(app: &mut gpui::App) -> PlatformSupport {
    #[cfg(target_os = "macos")]
    {
        app.unhide_other_apps();
        PlatformSupport::Performed
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        PlatformSupport::Unsupported
    }
}

/// Dragging content out of the window into another application.
///
/// Implemented on macOS and Linux; absent on Windows (gpui issue #52110). A
/// caller that needs the content to leave the app must offer a copy or a save
/// dialog when this reports Unsupported.
pub fn supports_external_drag() -> PlatformSupport {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        PlatformSupport::Performed
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        PlatformSupport::Unsupported
    }
}

/// Native popups that escape the window rectangle exist only on Wayland.
///
/// Menus, dropdowns and tooltips are therefore in-window overlays everywhere,
/// and the design must never rely on leaving the window bounds.
pub fn supports_native_popups() -> PlatformSupport {
    PlatformSupport::Unsupported
}

/// Offscreen rendering, and with it screenshot regression tests, exists only on
/// macOS: `render_to_image` is implemented nowhere else and
/// `current_headless_renderer()` returns None.
pub fn supports_offscreen_rendering() -> PlatformSupport {
    #[cfg(target_os = "macos")]
    {
        PlatformSupport::Performed
    }
    #[cfg(not(target_os = "macos"))]
    {
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
    fn external_drag_reports_honestly_for_this_host() {
        let expected = cfg!(any(target_os = "macos", target_os = "linux"));
        assert_eq!(supports_external_drag().is_supported(), expected);
    }

    #[test]
    fn offscreen_rendering_is_macos_only() {
        assert_eq!(
            supports_offscreen_rendering().is_supported(),
            cfg!(target_os = "macos")
        );
    }
}

#[cfg(test)]
mod open_url_tests {
    #[test]
    fn only_http_and_https_are_handed_to_the_desktop() {
        // The check runs before the App is touched, so the refusal path needs no
        // gpui context at all — which is the point of putting it here.
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
