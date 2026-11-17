use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_limits::service_limit::API_GATEWAY_REQUEST_MAX_STRING_CHARS;
use rise_platform::crash_diagnostics::{
    READING_FAILED, available_disk_mb, process_uptime_seconds, resident_memory_mb,
};
use rise_platform::power::system_low_power_mode;
use rise_platform::process_info::{active_processor_count, physical_memory, physical_memory_mb, thermal_reading};
use serde::Serialize;

use rise_diagnostics::rise_breadcrumbs::RiseBreadcrumbs;
use crate::core::diagnostics::rise_diagnostics::RiseDiagnostics;
use crate::core::diagnostics::rise_diagnostics_rpc::RiseDiagnosticsLocale;

const PAYLOAD_CEILING: usize = API_GATEWAY_REQUEST_MAX_STRING_CHARS as usize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseBugReportEnvelope {
    breadcrumbs: Vec<RiseBugReportEnvelopeBreadcrumb>,
    pub captured_at_ms: i64,
    pub context: RiseBugReportEnvelopeContext,
    dropped_breadcrumbs: usize,
    pub metrics: RiseBugReportEnvelopeMetrics,
    pub schema: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseBugReportEnvelopeMetrics {
    pub active_processor_count: i64,
    #[serde(rename = "availableDiskMB")]
    pub available_disk_mb: i64,
    pub low_power_mode: bool,
    #[serde(rename = "physicalMemoryMB")]
    pub physical_memory_mb: i64,
    pub process_uptime_seconds: i64,
    #[serde(rename = "residentMemoryMB")]
    pub resident_memory_mb: i64,
    pub thermal_state: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseBugReportEnvelopeContext {
    pub is_debug_build: bool,
    pub locale: String,
    pub preferred_languages: Vec<String>,
    pub socket_connected: bool,
    pub time_zone: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseBugReportEnvelopeBreadcrumb {
    pub at_ms: i64,
    pub channel: String,
    pub message: String,
}

impl RiseBugReportEnvelope {
    pub const CURRENT_SCHEMA: i64 = 2;

    /// built off the main thread because every reader is a system query
    /// call from a blocking task, never the gpui main thread or an async worker
    pub fn capture(socket_connected: bool) -> RiseBugReportEnvelope {
        RiseBugReportEnvelope {
            schema: Self::CURRENT_SCHEMA,
            captured_at_ms: Date::now().ms(),
            metrics: RiseBugReportEnvelopeMetrics {
                thermal_state: Self::thermal_state_name(),
                low_power_mode: system_low_power_mode(),
                resident_memory_mb: resident_memory_mb(),
                physical_memory_mb: Self::physical_memory_mb(),
                available_disk_mb: Self::available_disk_mb(),
                process_uptime_seconds: process_uptime_seconds(),
                active_processor_count: i64::try_from(active_processor_count()).unwrap_or(i64::MAX),
            },
            context: RiseBugReportEnvelopeContext {
                locale: RiseDiagnosticsLocale::device_identifier(),
                time_zone: TimeZone::current().identifier(),
                preferred_languages: rise_platform::device_locale::current()
                    .languages
                    .into_iter()
                    .take(3)
                    .collect(),
                socket_connected,
                is_debug_build: Self::is_debug_build(),
            },
            breadcrumbs: RiseBreadcrumbs::snapshot()
                .into_iter()
                .map(|entry| RiseBugReportEnvelopeBreadcrumb {
                    at_ms: entry.at_ms,
                    channel: entry.channel.raw_value().to_owned(),
                    message: entry.message,
                })
                .collect(),
            dropped_breadcrumbs: 0,
        }
    }

    pub fn breadcrumbs(&self) -> &[RiseBugReportEnvelopeBreadcrumb] {
        &self.breadcrumbs
    }

    pub fn dropped_breadcrumbs(&self) -> usize {
        self.dropped_breadcrumbs
    }

    /// wire text that fits the gateway, keys sorted so a retry hashes the same
    /// too big drops oldest entries and encodes the count, limit counts chars not bytes
    pub fn encoded_payload(&self) -> serde_json::Result<String> {
        let options = JSONWritingOptions {
            sorted_keys: true,
            ..JSONWritingOptions::default()
        };

        let mut candidate = self.clone();
        loop {
            let value = serde_json::to_value(&candidate)?;
            let Some(data) = JSONSerialization::data_with_json_object(&value, options) else {
                return Err(serde::ser::Error::custom("bug report envelope is not a JSON object"));
            };
            let text = String::from_utf8(data).map_err(serde::ser::Error::custom)?;
            if text.chars().count() <= PAYLOAD_CEILING || candidate.breadcrumbs.is_empty() {
                return Ok(text);
            }
            let drop = (candidate.breadcrumbs.len() / 4).max(1);
            candidate.breadcrumbs.drain(..drop);
            candidate.dropped_breadcrumbs += drop;
        }
    }

    fn is_debug_build() -> bool {
        cfg!(debug_assertions)
    }

    fn thermal_state_name() -> String {
        thermal_reading().state.name().to_owned()
    }

    fn physical_memory_mb() -> i64 {
        match physical_memory() {
            Ok(bytes) => i64::try_from(physical_memory_mb(bytes)).unwrap_or(i64::MAX),
            Err(_) => READING_FAILED,
        }
    }

    /// free disk space in mb of the volume the data directory lives on, that one explains a failed save
    fn available_disk_mb() -> i64 {
        match RiseDiagnostics::application_support_directory() {
            Some(directory) => available_disk_mb(directory),
            None => READING_FAILED,
        }
    }
}

#[cfg(test)]
#[path = "rise_bug_report_envelope_tests.rs"]
mod tests;
