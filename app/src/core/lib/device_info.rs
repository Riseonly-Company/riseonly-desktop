use rise_catalog::SessionDeviceType;
use rise_platform::app_info::MARKETING_VERSION;
use rise_platform::device_info::{DeviceInfo as PlatformDeviceInfo, generic_machine};
use rise_platform::host_os::HostOs;
use rise_platform::window_chrome::current_window_corner;

/// the machine the app runs on, as session list and auth device payload and diagnostics read it
/// read once per process from the device seam
pub enum DeviceInfo {}

impl DeviceInfo {
    /// hardware id like MacBookPro18,3 or a dmi product name
    /// the generic machine name when the system gives none
    pub fn machine_identifier() -> String {
        PlatformDeviceInfo::current()
            .machine_identifier
            .clone()
            .unwrap_or_else(|| generic_machine(HostOs::current()).to_owned())
    }

    pub fn marketing_name() -> String {
        PlatformDeviceInfo::current().display_name.clone()
    }

    /// what the session list shows, eg Apple MacBook Pro or Lenovo ThinkPad X1
    /// the seam adds the maker since desktops arent always apple
    pub fn display_name() -> String {
        PlatformDeviceInfo::current().display_name.clone()
    }

    /// logical points of the window outer corner, the radius a fullscreen cover rounds to
    /// 0 where the compositor owns the corner or its square
    pub fn screen_corner_radius() -> f32 {
        current_window_corner().map_or(0.0, |corner| corner.radius)
    }

    pub fn system_version() -> String {
        PlatformDeviceInfo::current().system_version.clone()
    }

    /// session device type for auth bodies
    /// a desktop is neither tablet nor mobile and session matching compares the value verbatim
    pub fn device_type() -> &'static str {
        SessionDeviceType::Desktop.wire()
    }

    /// eg Riseonly-macOS/1.4.0 (Apple MacBook Pro, macOS 15.1)
    pub fn user_agent() -> String {
        let device = PlatformDeviceInfo::current();
        format!(
            "Riseonly-{}/{} ({}; {})",
            device.os_name,
            MARKETING_VERSION,
            device.display_name,
            device.system_label()
        )
    }
}
