use futures::channel::oneshot;
use gpui::{AnyWindowHandle, App};
use rise_platform::gpui_shim::PlatformSupport;
use rise_platform::photo_library::{self, PhotoLibraryUnavailable};
use rise_platform::privacy_access;

pub use rise_platform::photo_library::PhotoLibraryAccessLevel;

pub enum PhotoLibraryPermissions {}

impl PhotoLibraryPermissions {
    pub fn current() -> PhotoLibraryAccessLevel {
        Self::level_of(photo_library::access_level())
    }

    pub async fn request() -> PhotoLibraryAccessLevel {
        let (sender, receiver) = oneshot::channel();
        let started = photo_library::request_access(move |level| {
            let _ = sender.send(level);
        });
        if let Err(reason) = started {
            return Self::level_of(Err(reason));
        }
        receiver.await.unwrap_or(PhotoLibraryAccessLevel::Denied)
    }

    pub fn open_system_settings(cx: &mut App) {
        let _ = privacy_access::open_settings(cx);
    }

    pub fn present_limited_selector(_from: AnyWindowHandle) -> PlatformSupport {
        PlatformSupport::Unsupported
    }

    fn level_of(level: Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable>) -> PhotoLibraryAccessLevel {
        match level {
            Ok(level) | Err(PhotoLibraryUnavailable::NotAuthorized(level)) => level,
            Err(_) => PhotoLibraryAccessLevel::Denied,
        }
    }
}
