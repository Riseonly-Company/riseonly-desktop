use super::glass::MacGlassSurface;
use crate::materials::{GlassSurface, MacOsVersion, WindowsBuild};

pub(crate) fn current_glass_surface() -> Box<dyn GlassSurface> {
    Box::new(MacGlassSurface::new())
}

pub(crate) fn read_macos_version() -> Option<MacOsVersion> {
    let version = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();

    let major = u32::try_from(version.majorVersion).ok()?;
    if major == 0 {
        return None;
    }

    Some(MacOsVersion::new(
        major,
        u32::try_from(version.minorVersion).unwrap_or(0),
        u32::try_from(version.patchVersion).unwrap_or(0),
    ))
}

pub(crate) fn read_windows_build() -> Option<WindowsBuild> {
    None
}
