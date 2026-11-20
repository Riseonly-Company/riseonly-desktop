use super::window_chrome::*;
use crate::gpui_shim::PlatformSupport;

pub(super) fn round_window_corner(_window: &gpui::Window) -> PlatformSupport {
    PlatformSupport::Unsupported
}

pub(super) fn place_traffic_lights(_window: &gpui::Window, _chrome: &WindowChrome) -> PlatformSupport {
    PlatformSupport::Unsupported
}

pub(super) fn start_titlebar_move(window: &gpui::Window) -> PlatformSupport {
    window.start_window_move();
    PlatformSupport::Performed
}

pub(super) fn toggle_maximize(window: &gpui::Window) -> PlatformSupport {
    window.zoom_window();
    PlatformSupport::Performed
}

pub(super) fn set_system_controls_appearance(
    _window: &gpui::Window,
    _appearance: Option<gpui::WindowAppearance>,
) -> PlatformSupport {
    PlatformSupport::Unsupported
}

#[cfg(feature = "dev-drive")]
pub(super) fn conceal_window_for_dev_drive(_window: &gpui::Window) -> PlatformSupport {
    PlatformSupport::Unsupported
}

#[cfg(feature = "dev-drive")]
pub(super) fn keep_app_in_background_for_dev_drive() -> PlatformSupport {
    PlatformSupport::Unsupported
}

#[cfg(feature = "dev-drive")]
pub(super) fn watch_windows_for_dev_drive(
    _wake: Box<dyn Fn() + Send + Sync>,
) -> Option<Box<dyn std::any::Any>> {
    None
}
