use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, TimeZone};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_limits::limit::{DIAGNOSTICS_QUEUE_BYTES, DIAGNOSTICS_QUEUE_PAYLOADS};
use rise_platform::AppPaths;
use rise_platform::app_info::AppInfo;
use rise_platform::crash_diagnostics::{
    DiagnosticPayload, DiagnosticsSubscription, PayloadSink, install, prepare_directory,
};
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileAttributeType, FileManager};
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::diagnostics::rise_diagnostics_rpc::RiseDiagnosticsLocale;
use crate::core::lib::device_info::DeviceInfo;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;

const MAX_STORED_PAYLOADS: usize = DIAGNOSTICS_QUEUE_PAYLOADS as usize;
const MAX_STORED_BYTES: u64 = DIAGNOSTICS_QUEUE_BYTES;

static APPLICATION_SUPPORT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::current().canon())
        .ok()
        .map(|paths| paths.data_dir().to_path_buf())
});

static SHARED: LazyLock<RiseDiagnostics> = LazyLock::new(RiseDiagnostics::new);

static QUEUE_LANE: Mutex<()> = Mutex::new(());

fn queue_lane() -> MutexGuard<'static, ()> {
    QUEUE_LANE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct RiseDiagnostics {
    is_installed: Mutex<bool>,
    subscription: Mutex<Option<DiagnosticsSubscription>>,
}

impl RiseDiagnostics {
    pub const ENVELOPE_SCHEMA: i64 = 1;
    const PAYLOAD_EXTENSION: &'static str = "json";

    fn new() -> Self {
        Self {
            is_installed: Mutex::new(false),
            subscription: Mutex::new(None),
        }
    }

    pub fn shared() -> &'static RiseDiagnostics {
        &SHARED
    }

    /// the app data directory, what ios calls application support
    pub fn application_support_directory() -> Option<&'static Path> {
        APPLICATION_SUPPORT.as_deref()
    }

    pub fn install(&self) {
        {
            let mut is_installed = self.is_installed.lock().unwrap_or_else(PoisonError::into_inner);
            if *is_installed {
                return;
            }
            *is_installed = true;
        }
        let Some(directory) = self.directory() else {
            tracing::warn!(target: "riseonly", "no diagnostics directory; crash reports are off");
            return;
        };
        let sink: PayloadSink = Arc::new(|payloads: Vec<DiagnosticPayload>| {
            RiseDiagnostics::shared().did_receive(payloads);
        });
        match install(&directory, sink) {
            Ok(subscription) => {
                *self.subscription.lock().unwrap_or_else(PoisonError::into_inner) = Some(subscription);
            }
            Err(reason) => {
                tracing::warn!(target: "riseonly", %reason, "crash and metric reports are off");
            }
        }
    }

    /// application support and not the cache dir, caches get evicted under disk pressure
    /// exactly when a disk write diagnostic matters most
    /// new directory is excluded from backup
    fn directory(&self) -> Option<PathBuf> {
        let data_dir = Self::application_support_directory()?;
        match prepare_directory(data_dir) {
            Ok(directory) => Some(directory.path),
            Err(reason) => {
                tracing::warn!(target: "riseonly", %reason, "diagnostics directory unavailable");
                None
            }
        }
    }

    pub fn pending_payload_urls(&self) -> Vec<PathBuf> {
        let Some(directory) = self.directory() else {
            return Vec::new();
        };
        Self::stored_files(&directory).into_iter().map(|file| file.url).collect()
    }

    pub fn discard_payload(&self, url: PathBuf) {
        let runtime = RiseAccountRuntime::current();
        drop(runtime.spawn_blocking(move || {
            let _lane = queue_lane();
            let _ = FileManager.remove_item(&url);
        }));
    }

    fn did_receive(&self, payloads: Vec<DiagnosticPayload>) {
        let received_at = Date::now();
        for payload in payloads {
            self.store(payload.json_text(), payload.kind.name(), received_at);
        }
    }

    fn store(&self, payload: String, kind: &'static str, received_at: Date) {
        let stamp = diagnostics_formatter(received_at);
        let envelope = RiseDiagnosticsEnvelope {
            schema: Self::ENVELOPE_SCHEMA,
            kind: kind.to_owned(),
            captured_at: stamp.clone(),
            app_version: app_version(),
            build_number: build_number(),
            os_version: DeviceInfo::system_version(),
            device_model: DeviceInfo::display_name(),
            locale: RiseDiagnosticsLocale::device_identifier(),
            user_id: SaiAccountCacheScope::active_user_id(),
            payload,
        };

        let runtime = RiseAccountRuntime::current();
        drop(runtime.spawn_blocking(move || {
            let _lane = queue_lane();
            let diagnostics = RiseDiagnostics::shared();
            let Some(directory) = diagnostics.directory() else {
                return;
            };
            let Ok(data) = serde_json::to_vec(&envelope) else {
                return;
            };

            let uuid = Uuid::new_v4().uuid_string();
            let name = format!(
                "{stamp}-{kind}-{}.{}",
                character_prefix(&uuid, 8),
                Self::PAYLOAD_EXTENSION
            );
            let target = directory.join(name);

            let _ = write_atomically(&target, &data);
            Self::prune(&directory);
        }));
    }

    /// only regular files are listed, tracers and panic markers write subdirectories into this root
    /// without the filter they count against the file ceiling and remove_item could delete the whole tree
    fn stored_files(directory: &Path) -> Vec<RiseDiagnosticsStoredFile> {
        let contents = FileManager
            .contents_of_directory_at(directory, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
            .unwrap_or_default();

        let mut files: Vec<RiseDiagnosticsStoredFile> = contents
            .into_iter()
            .filter_map(|url| {
                if url.extension().and_then(|extension| extension.to_str())
                    != Some(Self::PAYLOAD_EXTENSION)
                {
                    return None;
                }
                let values = FileManager.attributes_of_item(&url).ok()?;
                if values.r#type != FileAttributeType::TypeRegular {
                    return None;
                }
                Some(RiseDiagnosticsStoredFile {
                    url,
                    size: u64::try_from(values.size).unwrap_or(0),
                    created: values.creation_date.map_or(DISTANT_PAST, date_from_system_time),
                })
            })
            .collect();
        files.sort_by_key(|file| file.created);
        files
    }

    fn prune(directory: &Path) {
        let mut files = Self::stored_files(directory);

        while files.len() > MAX_STORED_PAYLOADS {
            let oldest = files.remove(0);
            let _ = FileManager.remove_item(&oldest.url);
        }

        let mut total: u64 = files.iter().map(|file| file.size).sum();
        while total > MAX_STORED_BYTES && !files.is_empty() {
            let oldest = files.remove(0);
            total = total.saturating_sub(oldest.size);
            let _ = FileManager.remove_item(&oldest.url);
        }
    }
}

/// versioned from the first row because the on disk queue survives the update that fixed the crash
/// so an old payload must stay readable
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseDiagnosticsEnvelope {
    pub schema: i64,
    pub kind: String,
    pub captured_at: String,
    pub app_version: String,
    pub build_number: String,
    pub os_version: String,
    pub device_model: String,
    pub locale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub payload: String,
}

struct RiseDiagnosticsStoredFile {
    url: PathBuf,
    size: u64,
    created: Date,
}

const DISTANT_PAST: Date = Date::from_ms(i64::MIN);

fn date_from_system_time(time: SystemTime) -> Date {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => Date::from_ms(i64::try_from(after.as_millis()).unwrap_or(i64::MAX)),
        Err(before) => {
            Date::from_ms(i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |ms| -ms))
        }
    }
}

fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut staging = url.as_os_str().to_owned();
    staging.push(".tmp");
    let staging = PathBuf::from(staging);
    std::fs::write(&staging, data)?;
    std::fs::rename(&staging, url)
}

/// app version trimmed or 0.0.0 if its missing
fn app_version() -> String {
    let value = AppInfo::current(AppEnvironment::current().as_str())
        .map(|info| trimming_whitespaces_and_newlines(&info.short_version).to_owned())
        .unwrap_or_default();
    if value.is_empty() { "0.0.0".to_owned() } else { value }
}

/// build number from the bundle or 0 if its missing
fn build_number() -> String {
    let value = AppInfo::current(AppEnvironment::current().as_str())
        .map(|info| trimming_whitespaces_and_newlines(&info.build).to_owned())
        .unwrap_or_default();
    if value.is_empty() { "0".to_owned() } else { value }
}

fn utc_calendar() -> Calendar {
    Calendar {
        time_zone: TimeZone::gmt(),
        ..Calendar::gregorian()
    }
}

/// iso 8601 basic form in utc, eg 20261104T134900
/// no colons since they are awkward for shell tooling
pub(crate) fn diagnostics_formatter(date: Date) -> String {
    let components = utc_calendar().date_components(
        &[
            CalendarComponent::Year,
            CalendarComponent::Month,
            CalendarComponent::Day,
            CalendarComponent::Hour,
            CalendarComponent::Minute,
            CalendarComponent::Second,
        ],
        &date,
    );
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}",
        components.year.unwrap_or(0),
        components.month.unwrap_or(0),
        components.day.unwrap_or(0),
        components.hour.unwrap_or(0),
        components.minute.unwrap_or(0),
        components.second.unwrap_or(0),
    )
}

#[cfg(test)]
#[path = "rise_diagnostics_tests.rs"]
mod tests;
