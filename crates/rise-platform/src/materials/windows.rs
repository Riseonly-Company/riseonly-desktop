use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

use super::materials::*;

pub(super) fn current_glass_surface() -> Box<dyn GlassSurface> {
    painted_glass_surface()
}

pub(super) fn read_macos_version() -> Option<MacOsVersion> {
    None
}

pub(super) fn read_windows_build() -> Option<WindowsBuild> {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: u32::try_from(std::mem::size_of::<OSVERSIONINFOW>()).ok()?,
        ..Default::default()
    };
    let status = unsafe { RtlGetVersion(&mut info) };
    if status.0 < 0 || info.dwMajorVersion == 0 {
        return None;
    }
    Some(WindowsBuild(info.dwBuildNumber))
}
