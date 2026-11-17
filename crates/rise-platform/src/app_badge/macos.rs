use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::tray::dock_badge_label;

pub(super) fn apply_count(count: u32) -> PlatformSupport {
    let Some(mtm) = MainThreadMarker::new() else {
        return PlatformSupport::Unsupported;
    };

    let label = dock_badge_label(HostOs::MacOs, count).map(|text| NSString::from_str(&text));
    NSApplication::sharedApplication(mtm)
        .dockTile()
        .setBadgeLabel(label.as_deref());

    PlatformSupport::Performed
}
