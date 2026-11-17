use super::app_badge::BADGE_CAP;
use crate::autostart::desktop_file_name;

pub const LAUNCHER_ENTRY_INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
pub const LAUNCHER_ENTRY_SIGNAL: &str = "Update";
pub const LAUNCHER_ENTRY_PATH: &str = "/net/riseonly/desktop/LauncherEntry";
pub const LAUNCHER_COUNT_PROPERTY: &str = "count";
pub const LAUNCHER_COUNT_VISIBLE_PROPERTY: &str = "count-visible";

/// application:// plus desktop id, the entry bundle-linux.sh installs under the bundle id
pub fn launcher_app_uri(bundle_identifier: &str) -> String {
    format!("application://{}", desktop_file_name(bundle_identifier))
}

/// number the launcher draws, capped like the badge text
/// and whether it draws one, no text in this api so 999+ reads 999 and zero is hidden
pub fn launcher_count(count: u32) -> (i64, bool) {
    (i64::from(count.min(BADGE_CAP)), count > 0)
}
