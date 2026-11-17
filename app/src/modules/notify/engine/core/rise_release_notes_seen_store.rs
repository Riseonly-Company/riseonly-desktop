use std::collections::HashSet;
use std::sync::LazyLock;

use rise_canon::{RiseClientPlatform, RiseLimits};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::app_info::AppInfo;
use rise_platform::host_os::HostOs;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{RELEASE_NOTES_LAST_VERSION, RELEASE_NOTES_SEEN};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::user::shared::contact_device_book::RiseInstallationIdentity;

static MAXIMUM_REMEMBERED_RELEASES: LazyLock<usize> = LazyLock::new(|| {
    usize::try_from(RiseLimits::client_value(
        "rise_release_notes_seen_store_maximum_remembered_releases",
    ))
    .unwrap_or(0)
});

/// what this device was already shown and which build it last ran
/// not in the account store since reading whats new in 2.4 is a device fact
/// else sign out or account switch replays the modal
pub enum RiseReleaseNotesSeenStore {}

impl RiseReleaseNotesSeenStore {
    fn defaults() -> UserDefaults {
        UserDefaults::standard()
    }

    pub fn seen_release_ids() -> HashSet<i64> {
        Self::defaults()
            .array(RELEASE_NOTES_SEEN)
            .and_then(|stored| {
                stored
                    .iter()
                    .map(|value| value.as_i64())
                    .collect::<Option<HashSet<i64>>>()
            })
            .unwrap_or_default()
    }

    pub fn mark_seen(release_ids: impl IntoIterator<Item = i64>) {
        let mut merged = Self::seen_release_ids();
        merged.extend(release_ids.into_iter().filter(|id| *id > 0));
        let mut bounded: Vec<i64> = merged.into_iter().collect();
        bounded.sort_unstable_by(|left, right| right.cmp(left));
        bounded.truncate(*MAXIMUM_REMEMBERED_RELEASES);
        Self::defaults().set(RELEASE_NOTES_SEEN, bounded);
    }

    pub fn has_seen(release_id: i64) -> bool {
        Self::seen_release_ids().contains(&release_id)
    }

    pub fn last_run_version() -> Option<String> {
        let value = Self::defaults().string(RELEASE_NOTES_LAST_VERSION)?;
        let trimmed = trimming_whitespaces_and_newlines(&value);
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    }

    pub fn record_run_version(version: &str) {
        let trimmed = trimming_whitespaces_and_newlines(version);
        if trimmed.is_empty() {
            return;
        }
        Self::defaults().set(RELEASE_NOTES_LAST_VERSION, trimmed);
    }

    /// test seam, never called from the app
    pub fn reset() {
        Self::defaults().remove_object(RELEASE_NOTES_SEEN);
        Self::defaults().remove_object(RELEASE_NOTES_LAST_VERSION);
    }
}

/// what this build is for the release query
/// one place so the three values that decide the release can be asserted in a test
pub enum RiseReleaseNotesDeviceContext {}

impl RiseReleaseNotesDeviceContext {
    /// desktop names the operating system it runs on from the canon platform list
    /// server can then target a release per desktop os
    pub fn platform() -> &'static str {
        match HostOs::current() {
            HostOs::MacOs => RiseClientPlatform::Macos.wire(),
            HostOs::Linux => RiseClientPlatform::Linux.wire(),
            HostOs::Windows => RiseClientPlatform::Windows.wire(),
        }
    }

    /// app version string through the app_info seam
    pub fn app_version() -> String {
        let value = AppInfo::current(AppEnvironment::compiled().as_str())
            .map(|info| trimming_whitespaces_and_newlines(&info.short_version).to_owned())
            .unwrap_or_default();
        if value.is_empty() { "0.0.0".to_owned() } else { value }
    }

    /// the build number
    /// two builds of 1.1.2 are two binaries so the version alone doesnt identify it
    /// read here since it was read ad hoc in three debug tracers
    pub fn build_number() -> String {
        let value = AppInfo::current(AppEnvironment::compiled().as_str())
            .map(|info| trimming_whitespaces_and_newlines(&info.build).to_owned())
            .unwrap_or_default();
        if value.is_empty() { "0".to_owned() } else { value }
    }

    /// keychain backed install id that rtc uses for per device call routing
    /// stable across launches and never shared
    /// not derived from any account, server only hashes it into a rollout bucket
    pub fn device_id() -> Option<String> {
        let identity = RiseInstallationIdentity::current().ok()?;
        let trimmed = trimming_whitespaces_and_newlines(&identity);
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    }
}
