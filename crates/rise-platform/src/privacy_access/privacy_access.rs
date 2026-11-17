use thiserror::Error;

use crate::capture_permission::{self, CaptureAccess, CaptureKind};
use crate::contacts::{self, ContactAuthorization};
use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::location::{self, LocationAuthorization};
use crate::notifications::{GpuiNotificationPresenter, NotificationAvailability, NotificationPresenter};
use crate::open_url;
use crate::photo_library::{self, PhotoLibraryAccessLevel};

/// the private data the app asks the user about
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PrivacyAccessKind {
    Camera,
    Microphone,
    Screen,
    Notifications,
    Location,
    Contacts,
    Photos,
}

const CAMERA_USAGE: &[&str] = &[CaptureKind::Camera.usage_description_key()];
const MICROPHONE_USAGE: &[&str] = &[CaptureKind::Microphone.usage_description_key()];
const LOCATION_USAGE: &[&str] = &["NSLocationWhenInUseUsageDescription", "NSLocationUsageDescription"];
const CONTACTS_USAGE: &[&str] = &["NSContactsUsageDescription"];
const PHOTOS_USAGE: &[&str] = &["NSPhotoLibraryUsageDescription", "NSPhotoLibraryAddUsageDescription"];
const NO_USAGE: &[&str] = &[];

impl PrivacyAccessKind {
    pub const ALL: [Self; 7] = [
        Self::Camera,
        Self::Microphone,
        Self::Screen,
        Self::Notifications,
        Self::Location,
        Self::Contacts,
        Self::Photos,
    ];

    /// the seam that reads this kind and shows its prompt
    pub const fn seam(self) -> AccessSeam {
        match self {
            Self::Camera | Self::Microphone => AccessSeam::CapturePermission,
            Self::Screen => AccessSeam::ScreenCapture,
            Self::Notifications => AccessSeam::Notifications,
            Self::Location => AccessSeam::Location,
            Self::Contacts => AccessSeam::Contacts,
            Self::Photos => AccessSeam::PhotoLibrary,
        }
    }

    /// plist keys whose text the macos prompt shows
    /// without them tcc kills the process on the request so the plist template carries every one
    /// screen recording and notifications take no key
    pub const fn usage_description_keys(self) -> &'static [&'static str] {
        match self {
            Self::Camera => CAMERA_USAGE,
            Self::Microphone => MICROPHONE_USAGE,
            Self::Location => LOCATION_USAGE,
            Self::Contacts => CONTACTS_USAGE,
            Self::Photos => PHOTOS_USAGE,
            Self::Screen | Self::Notifications => NO_USAGE,
        }
    }

    /// macos silently refuses access even after the user agreed
    /// unless this hardened runtime entitlement is set
    pub const fn entitlement(self) -> Option<&'static str> {
        match self {
            Self::Camera => Some("com.apple.security.device.camera"),
            Self::Microphone => Some("com.apple.security.device.audio-input"),
            Self::Location => Some("com.apple.security.personal-information.location"),
            Self::Contacts => Some("com.apple.security.personal-information.addressbook"),
            Self::Photos => Some("com.apple.security.personal-information.photos-library"),
            Self::Screen | Self::Notifications => None,
        }
    }
}

/// where a kind is read
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AccessSeam {
    CapturePermission,
    ScreenCapture,
    Notifications,
    Location,
    Contacts,
    PhotoLibrary,
}

impl AccessSeam {
    pub const ALL: [Self; 6] = [
        Self::CapturePermission,
        Self::ScreenCapture,
        Self::Notifications,
        Self::Location,
        Self::Contacts,
        Self::PhotoLibrary,
    ];

    /// module a caller goes to for the prompt
    pub const fn module_path(self) -> &'static str {
        match self {
            Self::CapturePermission => "rise_platform::capture_permission",
            Self::ScreenCapture => "rise_media::screen_capture",
            Self::Notifications => "rise_platform::notifications",
            Self::Location => "rise_platform::location",
            Self::Contacts => "rise_platform::contacts",
            Self::PhotoLibrary => "rise_platform::photo_library",
        }
    }

    /// true when status can read it from this crate
    pub const fn answered_here(self) -> bool {
        !matches!(self, Self::ScreenCapture)
    }
}

/// the four answers permission code switches over
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AccessStatus {
    NotDetermined,
    Denied,
    Restricted,
    Authorized,
}

impl AccessStatus {
    pub const fn is_authorized(self) -> bool {
        matches!(self, Self::Authorized)
    }

    /// true when asking shows the system prompt, otherwise the caller offers the settings
    pub const fn can_request(self) -> bool {
        matches!(self, Self::NotDetermined)
    }

    pub const fn from_capture(access: CaptureAccess) -> Self {
        match access {
            CaptureAccess::NotDetermined => Self::NotDetermined,
            CaptureAccess::Denied => Self::Denied,
            CaptureAccess::Restricted => Self::Restricted,
            CaptureAccess::Granted => Self::Authorized,
        }
    }

    /// either granted level is access while the app is open, all the desktop asks for
    pub const fn from_location(level: LocationAuthorization) -> Self {
        match level {
            LocationAuthorization::NotDetermined => Self::NotDetermined,
            LocationAuthorization::Denied => Self::Denied,
            LocationAuthorization::Restricted => Self::Restricted,
            LocationAuthorization::WhenInUse | LocationAuthorization::Always => Self::Authorized,
        }
    }

    /// a limited book is readable
    pub const fn from_contacts(authorization: ContactAuthorization) -> Self {
        match authorization {
            ContactAuthorization::NotDetermined => Self::NotDetermined,
            ContactAuthorization::Denied => Self::Denied,
            ContactAuthorization::Restricted => Self::Restricted,
            ContactAuthorization::Authorized | ContactAuthorization::Limited => Self::Authorized,
        }
    }

    /// a limited library is browsable
    pub const fn from_photo_library(level: PhotoLibraryAccessLevel) -> Self {
        match level {
            PhotoLibraryAccessLevel::NotDetermined => Self::NotDetermined,
            PhotoLibraryAccessLevel::Denied => Self::Denied,
            PhotoLibraryAccessLevel::Restricted => Self::Restricted,
            PhotoLibraryAccessLevel::Authorized | PhotoLibraryAccessLevel::Limited => Self::Authorized,
        }
    }

    /// gpui reports no refusal of notifications, only whether a post reaches the os at all
    /// an unmet precondition isnt something the user can change so it reads as restricted
    pub const fn from_notification_availability(availability: NotificationAvailability) -> Self {
        match availability {
            NotificationAvailability::Available => Self::Authorized,
            NotificationAvailability::Blocked(_) => Self::Restricted,
        }
    }
}

/// why status has no answer
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum AccessUnreadable {
    #[error("{} could not read access: {reason}", .seam.module_path())]
    Seam { seam: AccessSeam, reason: String },
    #[error("access is read by {}", .0.module_path())]
    ReadElsewhere(AccessSeam),
}

impl AccessUnreadable {
    fn seam(seam: AccessSeam, reason: impl std::fmt::Display) -> Self {
        Self::Seam { seam, reason: reason.to_string() }
    }
}

/// current status of the kind as its seam reads it, without asking
pub fn status(kind: PrivacyAccessKind, app: &gpui::App) -> Result<AccessStatus, AccessUnreadable> {
    let seam = kind.seam();
    match kind {
        PrivacyAccessKind::Camera => capture_status(CaptureKind::Camera),
        PrivacyAccessKind::Microphone => capture_status(CaptureKind::Microphone),
        PrivacyAccessKind::Screen => Err(AccessUnreadable::ReadElsewhere(seam)),
        PrivacyAccessKind::Notifications => Ok(AccessStatus::from_notification_availability(
            GpuiNotificationPresenter::new(app).availability(),
        )),
        PrivacyAccessKind::Location => location::permission()
            .map(|permission| AccessStatus::from_location(permission.level))
            .map_err(|reason| AccessUnreadable::seam(seam, reason)),
        PrivacyAccessKind::Contacts => Ok(AccessStatus::from_contacts(contacts::authorization_status())),
        PrivacyAccessKind::Photos => photo_library::access_level()
            .map(AccessStatus::from_photo_library)
            .map_err(|reason| AccessUnreadable::seam(seam, reason)),
    }
}

fn capture_status(kind: CaptureKind) -> Result<AccessStatus, AccessUnreadable> {
    capture_permission::status(kind)
        .map(AccessStatus::from_capture)
        .map_err(|reason| AccessUnreadable::seam(AccessSeam::CapturePermission, reason))
}

/// what the open settings call opens here
/// ios shows the whole app settings page so desktop opens the whole privacy section
pub const fn settings_url(host: HostOs) -> Option<&'static str> {
    open_url::settings_url_on(host)
}

pub fn open_settings(app: &mut gpui::App) -> PlatformSupport {
    match settings_url(HostOs::current()) {
        Some(url) => {
            app.open_url(url);
            PlatformSupport::Performed
        }
        None => PlatformSupport::Unsupported,
    }
}
