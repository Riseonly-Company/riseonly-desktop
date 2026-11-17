use rise_core::foundation::foundation_date::Date;

use super::{RiseDiagnosticsUploader, diagnostics_upload_formatter};
use crate::core::diagnostics::rise_diagnostics::{
    RiseDiagnostics, RiseDiagnosticsEnvelope, diagnostics_formatter,
};

fn envelope(kind: &str, payload: &str) -> RiseDiagnosticsEnvelope {
    RiseDiagnosticsEnvelope {
        schema: RiseDiagnostics::ENVELOPE_SCHEMA,
        kind: kind.to_owned(),
        captured_at: "20261104T134905".to_owned(),
        app_version: "1.0.0".to_owned(),
        build_number: "1".to_owned(),
        os_version: "15.0".to_owned(),
        device_model: "Mac".to_owned(),
        locale: "en_US".to_owned(),
        user_id: Some("7".to_owned()),
        payload: payload.to_owned(),
    }
}

#[test]
fn metric_envelope_is_a_metric_report() {
    assert_eq!(RiseDiagnosticsUploader::report_kind(&envelope("metric", "{}")), Some("metric"));
}

#[test]
fn unreadable_diagnostic_payload_is_filed_as_a_crash() {
    assert_eq!(RiseDiagnosticsUploader::report_kind(&envelope("diagnostic", "not json")), Some("crash"));
}

#[test]
fn crash_wins_over_every_other_array() {
    let payload = r#"{"hangDiagnostics":[{}],"crashDiagnostics":[{}]}"#;
    assert_eq!(RiseDiagnosticsUploader::report_kind(&envelope("diagnostic", payload)), Some("crash"));
}

#[test]
fn first_non_empty_array_in_priority_order_names_the_kind() {
    let payload = r#"{"crashDiagnostics":[],"cpuExceptionDiagnostics":[{}],"appLaunchDiagnostics":[{}]}"#;
    assert_eq!(
        RiseDiagnosticsUploader::report_kind(&envelope("diagnostic", payload)),
        Some("cpu_exception")
    );
}

#[test]
fn payload_without_diagnostic_arrays_has_nothing_to_report() {
    let payload = r#"{"crashDiagnostics":[],"timeStampBegin":"x"}"#;
    assert_eq!(RiseDiagnosticsUploader::report_kind(&envelope("diagnostic", payload)), None);
}

#[test]
fn captured_at_round_trips_through_the_writer_form() {
    let date = Date::from_ms(1_793_886_545_000);
    let stamp = diagnostics_formatter(date);
    assert_eq!(RiseDiagnosticsUploader::occurred_at_ms(&stamp), Some(date.ms()));
}

#[test]
fn colon_form_and_garbage_are_rejected() {
    assert_eq!(diagnostics_upload_formatter("2026-11-04T13:49:05Z"), None);
    assert_eq!(diagnostics_upload_formatter("20261304T134905"), None);
    assert_eq!(diagnostics_upload_formatter("2026110xT134905"), None);
}
