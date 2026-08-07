pub mod autostart;
pub mod deep_link;
pub mod device_locale;
pub mod file_dialog;
pub mod global_hotkey;
pub mod gpui_shim;
pub mod host_os;
pub mod keyring_store;
// Not `pub`. A public `macos` module is a compile-time OS oracle: a consumer
// could write `rise_platform::macos::glass::MacGlassSurface::new()`, and that
// path resolves on exactly one host — which is the property this crate exists to
// deny. `materials::current_glass_surface()` is the only way in, and it answers
// on every platform. The boundary checker greps for `#[cfg(target_os` and cannot
// see this, so the visibility is the enforcement.
#[cfg(target_os = "macos")]
pub(crate) mod macos;
pub mod materials;
pub mod media_keys;
pub mod notifications;
pub mod paths;
pub mod reveal_in_file_manager;
pub mod secure_store;
pub mod single_instance;
pub mod tray;
pub mod updater;
pub mod window_chrome;

pub use autostart::{Autostart, AutostartError, AutostartMechanism, AutostartState};
pub use deep_link::{DeepLink, DeepLinkError};
pub use device_locale::{DeviceLocale, LocaleSourceKind};
pub use file_dialog::{FileFilter, OpenRequest, SaveRequest, Selection};
pub use global_hotkey::{GlobalHotkeys, Hotkey, HotkeyError, Modifiers};
pub use gpui_shim::PlatformSupport;
pub use host_os::HostOs;
pub use keyring_store::KeyringSecureStore;
pub use materials::{MacOsVersion, Material, MaterialBacking};
pub use media_keys::{MediaCommand, MediaKeys, NowPlaying, PlaybackState};
pub use notifications::{
    LocalNotification, MuteScopes, NotificationDecision, NotificationPresenter, NotificationScope,
    NotificationTag, SuppressionReason,
};
pub use paths::{AppPaths, PathsError};
pub use reveal_in_file_manager::{OpenRisk, RevealError};
pub use secure_store::{
    InMemorySecureStore, SecretKey, SecretPurpose, SecureStore, SecureStoreError,
};
pub use single_instance::{Instance, PrimaryInstance, SingleInstanceError};
pub use tray::{Tray, TrayAvailability, TrayError};
pub use updater::{
    InstallPlan, ReleaseChannel, UpdateDecision, UpdateManifest, Version, VersionError,
};
pub use window_chrome::{ChromeStyle, ControlSide, DecorationMode, WindowChrome};
