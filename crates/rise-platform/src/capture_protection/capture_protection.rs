use crate::gpui_shim::PlatformSupport;

pub fn set_window_excluded_from_capture(
    window: &gpui::Window,
    excluded: bool,
) -> PlatformSupport {
    super::os::set_window_excluded_from_capture(window, excluded)
}
