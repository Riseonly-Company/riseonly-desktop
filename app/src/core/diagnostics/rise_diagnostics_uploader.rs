use std::sync::{Arc, LazyLock};

use rise_canon::rise_key::report_diagnostics::camel as diagnostics_keys;
use rise_canon::rise_request::report::SubmitDiagnosticReport;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_calendar::{Calendar, DateComponents, TimeZone};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_json_serialization::JSONSerialization;
use rise_engine::RiseEngine;
use serde_json::Value;

use crate::core::diagnostics::rise_diagnostics::{RiseDiagnostics, RiseDiagnosticsEnvelope};
use crate::core::diagnostics::rise_diagnostics_rpc::{
    RiseDiagnosticsLocale, RiseDiagnosticsRPC, SubmitDiagnosticReportDiagnostics,
};

static SHARED: LazyLock<RiseDiagnosticsUploader> = LazyLock::new(RiseDiagnosticsUploader::new);

#[derive(Default)]
struct RiseDiagnosticsUploaderState {
    is_draining: bool,
}

pub struct RiseDiagnosticsUploader {
    state: RiseIsolated<RiseDiagnosticsUploaderState>,
}

struct DrainingReset<'a> {
    state: &'a RiseIsolated<RiseDiagnosticsUploaderState>,
}

impl Drop for DrainingReset<'_> {
    fn drop(&mut self) {
        self.state.with(|state| state.is_draining = false);
    }
}

impl RiseDiagnosticsUploader {
    const PAYLOADS_PER_DRAIN: usize = 6;

    fn new() -> Self {
        Self {
            state: RiseIsolated::new(RiseDiagnosticsUploaderState::default()),
        }
    }

    pub fn shared() -> &'static RiseDiagnosticsUploader {
        &SHARED
    }

    /// safe on every reconnect, with an empty queue its one directory read
    pub async fn drain(&self, engine: Arc<RiseEngine>) {
        let started = self.state.with(|state| {
            if state.is_draining {
                return false;
            }
            state.is_draining = true;
            true
        });
        if !started {
            return;
        }
        let _reset = DrainingReset { state: &self.state };

        let rpc = RiseDiagnosticsRPC::new(engine);
        let diagnostics = RiseDiagnostics::shared();
        let pending = diagnostics.pending_payload_urls();
        let mut submitted = 0;

        for url in pending {
            if submitted >= Self::PAYLOADS_PER_DRAIN {
                break;
            }
            let Ok(data) = std::fs::read(&url) else {
                continue;
            };

            let Ok(envelope) = serde_json::from_slice::<RiseDiagnosticsEnvelope>(&data) else {
                diagnostics.discard_payload(url);
                continue;
            };

            let Some(kind) = Self::report_kind(&envelope) else {
                diagnostics.discard_payload(url);
                continue;
            };

            if envelope.user_id.as_deref().is_none_or(str::is_empty) {
                continue;
            }

            let request = SubmitDiagnosticReport::diagnostic(
                kind.to_owned(),
                envelope.payload.clone(),
                envelope.app_version.clone(),
                envelope.build_number.clone(),
                envelope.os_version.clone(),
                envelope.device_model.clone(),
                RiseDiagnosticsLocale::clamped(&envelope.locale),
                Self::occurred_at_ms(&envelope.captured_at),
                None,
            );

            match rpc.submit_diagnostic_report(&request).await {
                Ok(response) => {
                    let _ = response.duplicate;
                    diagnostics.discard_payload(url);
                    submitted += 1;
                }
                Err(_) => return,
            }
        }
    }

    /// one payload can carry several diagnostic arrays so this is a priority order and not a lookup
    /// crashes come first
    fn report_kind(envelope: &RiseDiagnosticsEnvelope) -> Option<&'static str> {
        if envelope.kind == "metric" {
            return Some("metric");
        }

        let Some(Value::Object(root)) = JSONSerialization::json_object(envelope.payload.as_bytes())
        else {
            return Some("crash");
        };

        let by_priority: [(&str, &'static str); 5] = [
            (diagnostics_keys::CRASH_DIAGNOSTICS, "crash"),
            (diagnostics_keys::HANG_DIAGNOSTICS, "hang"),
            (diagnostics_keys::CPU_EXCEPTION_DIAGNOSTICS, "cpu_exception"),
            (diagnostics_keys::DISK_WRITE_EXCEPTION_DIAGNOSTICS, "disk_write_exception"),
            (diagnostics_keys::APP_LAUNCH_DIAGNOSTICS, "app_launch"),
        ];
        by_priority.into_iter().find_map(|(key, kind)| match root.get(key) {
            Some(Value::Array(entries)) if !entries.is_empty() => Some(kind),
            _ => None,
        })
    }

    /// from the envelope capture time and not the upload clock
    /// the system delivers once per 24 hours so arrival time would cluster every incident
    fn occurred_at_ms(captured_at: &str) -> Option<i64> {
        diagnostics_upload_formatter(captured_at).map(|date| date.ms())
    }
}

/// must match the writer form exactly
/// the envelope uses colon free utc which a default iso parser rejects
fn diagnostics_upload_formatter(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    if bytes.len() != 15 || bytes[8] != b'T' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = text.get(range)?;
        if !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let (month, day) = (number(4..6)?, number(6..8)?);
    let (hour, minute, second) = (number(9..11)?, number(11..13)?, number(13..15)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let calendar = Calendar {
        time_zone: TimeZone::gmt(),
        ..Calendar::gregorian()
    };
    calendar.date_from_components(&DateComponents {
        year: Some(number(0..4)?),
        month: Some(month),
        day: Some(day),
        hour: Some(hour),
        minute: Some(minute),
        second: Some(second),
        ..DateComponents::default()
    })
}

#[cfg(test)]
#[path = "rise_diagnostics_uploader_tests.rs"]
mod tests;
