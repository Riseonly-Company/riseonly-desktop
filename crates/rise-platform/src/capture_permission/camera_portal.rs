use super::capture_permission::{CaptureAccess, CaptureKind};

pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";
pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const CAMERA_INTERFACE: &str = "org.freedesktop.portal.Camera";
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
pub const ACCESS_CAMERA_METHOD: &str = "AccessCamera";
pub const IS_CAMERA_PRESENT_PROPERTY: &str = "IsCameraPresent";
pub const RESPONSE_SIGNAL: &str = "Response";
pub const HANDLE_TOKEN_OPTION: &str = "handle_token";
const REQUEST_PATH_PREFIX: &str = "/org/freedesktop/portal/desktop/request";

/// the response code of the portal request response
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortalResponse {
    Granted,
    Cancelled,
    Other,
}

impl PortalResponse {
    pub const fn from_code(code: u32) -> Self {
        match code {
            0 => Self::Granted,
            1 => Self::Cancelled,
            _ => Self::Other,
        }
    }

    pub const fn is_granted(self) -> bool {
        matches!(self, Self::Granted)
    }
}

/// access the linux layer reports
/// camera_answer is the portal answer remembered in this process
/// portal cant read the stored permission so before the first ask the camera is undecided
pub const fn linux_access(
    sandboxed: bool,
    kind: CaptureKind,
    camera_answer: Option<bool>,
) -> CaptureAccess {
    if !sandboxed {
        return CaptureAccess::Granted;
    }
    match (kind, camera_answer) {
        (CaptureKind::Microphone, _) | (CaptureKind::Camera, Some(true)) => CaptureAccess::Granted,
        (CaptureKind::Camera, Some(false)) => CaptureAccess::Denied,
        (CaptureKind::Camera, None) => CaptureAccess::NotDetermined,
    }
}

/// handle token of one request, the spec allows letters and digits and underscores only
pub fn request_token(serial: u64) -> String {
    format!("riseonly_camera_{serial}")
}

/// object path the portal gives the request, known before the call
/// so the response subscription exists before the dialog answers
/// unique name without the colon, dots as underscores and then token
pub fn request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.strip_prefix(':').unwrap_or(unique_name).replace('.', "_");
    format!("{REQUEST_PATH_PREFIX}/{sender}/{token}")
}
