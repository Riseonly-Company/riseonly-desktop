use crate::gpui_shim::PlatformSupport;

pub(super) fn set_window_excluded_from_capture(
    _window: &gpui::Window,
    _excluded: bool,
) -> PlatformSupport {
    PlatformSupport::Unsupported
}
