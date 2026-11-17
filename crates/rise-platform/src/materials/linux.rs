use super::materials::*;

pub(super) fn current_glass_surface() -> Box<dyn GlassSurface> {
    painted_glass_surface()
}

pub(super) fn read_macos_version() -> Option<MacOsVersion> {
    None
}

pub(super) fn read_windows_build() -> Option<WindowsBuild> {
    None
}
