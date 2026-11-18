use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseRequestBody, RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_endpoints::EndpointKind;
use rise_engine::{RiseEngine, RiseWireMethodDescriptor, RiseWireReplayPolicy};
use rise_platform::device_info::ReportedOs;

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::rise_engine_bridge::rise_canon_endpoints::{
    RiseCanonEndpoints, RiseEndpointClientTimeout,
};

pub type RiseEndpointKind = EndpointKind;

/// the report a person wrote as it goes on the wire
/// split from the device half, valid together or absent together
/// report service refuses a mismatch both ways
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseBugReportFields {
    pub client_report_id: String,
    pub parent_report_id: Option<String>,
    pub message: String,
    pub pages: Vec<String>,
    pub component: Option<String>,
    pub screenshot_file_urls: Vec<String>,
    pub screenshot_file_ids: Vec<String>,
    pub video_file_url: Option<String>,
    pub video_file_id: Option<String>,
    pub video_duration_ms: Option<i64>,
    pub feedback_kind: Option<String>,
    pub fix_platforms: Vec<String>,
}

/// convenience init for submitting a diagnostic report
pub trait SubmitDiagnosticReportDiagnostics: Sized {
    /// one stored payload, device fields come from the capture time envelope
    /// kind from payload or user_report, bug half flattened in or left out
    #[allow(clippy::too_many_arguments)]
    fn diagnostic(
        kind: String,
        payload: String,
        app_version: String,
        build_number: String,
        os_version: String,
        device_model: String,
        locale: Option<String>,
        occurred_at_ms: Option<i64>,
        bug: Option<&RiseBugReportFields>,
    ) -> Self;
}

impl SubmitDiagnosticReportDiagnostics for rise_request::report::SubmitDiagnosticReport {
    fn diagnostic(
        kind: String,
        payload: String,
        app_version: String,
        build_number: String,
        os_version: String,
        device_model: String,
        locale: Option<String>,
        occurred_at_ms: Option<i64>,
        bug: Option<&RiseBugReportFields>,
    ) -> Self {
        let screenshots = bug.is_some_and(|bug| !bug.screenshot_file_urls.is_empty());
        let reported = ReportedOs::current();
        Self {
            app_version,
            build_number,
            device_model,
            kind,
            os_version,
            payload,
            client_report_id: bug.map(|bug| bug.client_report_id.clone()),
            component: bug.and_then(|bug| bug.component.clone()),
            feedback_kind: bug.and_then(|bug| bug.feedback_kind.clone()),
            fix_platforms: bug.and_then(|bug| (!bug.fix_platforms.is_empty()).then(|| bug.fix_platforms.clone())),
            locale,
            os_arch: RiseReportedOs::field(&OS_ARCH_RULE, Some(&reported.arch)),
            os_build: RiseReportedOs::field(&OS_BUILD_RULE, reported.build.as_deref()),
            os_environment: RiseReportedOs::field(&OS_ENVIRONMENT_RULE, reported.environment.as_deref()),
            os_name: RiseReportedOs::field(&OS_NAME_RULE, Some(&reported.name)),
            message: bug.map(|bug| bug.message.clone()),
            occurred_at_ms,
            pages: bug.and_then(|bug| (!bug.pages.is_empty()).then(|| bug.pages.clone())),
            parent_report_id: bug.and_then(|bug| bug.parent_report_id.clone()),
            screenshot_file_ids: bug
                .filter(|_| screenshots)
                .map(|bug| bug.screenshot_file_ids.clone()),
            screenshot_file_urls: bug
                .filter(|_| screenshots)
                .map(|bug| bug.screenshot_file_urls.clone()),
            video_duration_ms: bug.and_then(|bug| bug.video_duration_ms),
            video_file_id: bug.and_then(|bug| bug.video_file_id.clone()),
            video_file_url: bug.and_then(|bug| bug.video_file_url.clone()),
        }
    }
}

fn submit_rule(path: &str) -> RiseFieldRule {
    RiseFieldRule::new(<rise_request::report::SubmitDiagnosticReport as RiseRequestBody>::CONTRACT, path)
}

static OS_NAME_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| submit_rule("/os_name"));
static OS_BUILD_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| submit_rule("/os_build"));
static OS_ARCH_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| submit_rule("/os_arch"));
static OS_ENVIRONMENT_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| submit_rule("/os_environment"));

pub enum RiseReportedOs {}

impl RiseReportedOs {
    /// one os field of a report, trimmed to the canon ceiling and none when empty
    fn field(rule: &RiseFieldRule, value: Option<&str>) -> Option<String> {
        let clamped = rule.clamped(value?);
        let value = trimming_whitespaces_and_newlines(&clamped);
        (!value.is_empty()).then(|| value.to_owned())
    }
}

static LOCALE_RULE: LazyLock<RiseFieldRule> = LazyLock::new(|| {
    RiseFieldRule::new(
        <rise_request::report::SubmitDiagnosticReport as RiseRequestBody>::CONTRACT,
        "/locale",
    )
});

pub enum RiseDiagnosticsLocale {}

impl RiseDiagnosticsLocale {
    /// device language in the form canon accepts
    /// keywords like ru_KZ@calendar=gregorian are too long and would reject the whole report so theyre cut
    pub fn current() -> Option<String> {
        Self::clamped(&Self::device_identifier())
    }

    /// same thing for a value already stored in an on disk envelope
    pub fn clamped(identifier: &str) -> Option<String> {
        let without_keywords = identifier.split('@').next().unwrap_or(identifier);
        let clamped = LOCALE_RULE.clamped(without_keywords);
        let value = trimming_whitespaces_and_newlines(&clamped);
        if value.is_empty() { None } else { Some(value.to_owned()) }
    }

    pub fn device_identifier() -> String {
        let device = rise_platform::device_locale::current();
        match (device.languages.first(), device.region.as_deref()) {
            (Some(language), Some(region)) => format!("{language}_{region}"),
            (Some(language), None) => language.clone(),
            (None, _) => String::new(),
        }
    }
}

static SUBMIT_DIAGNOSTIC_REPORT: LazyLock<RiseDiagnosticsEndpoint> = LazyLock::new(|| {
    RiseDiagnosticsEndpoint::new(
        rise_wire_methods::report::SUBMIT_DIAGNOSTIC_REPORT,
        RiseEndpointKind::IdempotentMutation,
    )
});

static GET_MY_DIAGNOSTIC_REPORTS: LazyLock<RiseDiagnosticsEndpoint> = LazyLock::new(|| {
    RiseDiagnosticsEndpoint::new(rise_wire_methods::report::GET_MY_DIAGNOSTIC_REPORTS, RiseEndpointKind::Read)
});

static UPDATE_DIAGNOSTIC_REPORT: LazyLock<RiseDiagnosticsEndpoint> = LazyLock::new(|| {
    RiseDiagnosticsEndpoint::new(rise_wire_methods::report::UPDATE_DIAGNOSTIC_REPORT, RiseEndpointKind::Mutation)
});

static DELETE_DIAGNOSTIC_REPORT: LazyLock<RiseDiagnosticsEndpoint> = LazyLock::new(|| {
    RiseDiagnosticsEndpoint::new(rise_wire_methods::report::DELETE_DIAGNOSTIC_REPORT, RiseEndpointKind::Mutation)
});

pub enum RiseDiagnosticsEndpoints {}

impl RiseDiagnosticsEndpoints {
    /// replay is safe, no client_report_id means auto diagnostics deduped on payload sha256
    /// with it a persons report deduped by that id
    /// camelCase wire method only, else a stand in endpoint
    pub fn submit_diagnostic_report() -> &'static RiseDiagnosticsEndpoint {
        &SUBMIT_DIAGNOSTIC_REPORT
    }

    pub fn get_my_diagnostic_reports() -> &'static RiseDiagnosticsEndpoint {
        &GET_MY_DIAGNOSTIC_REPORTS
    }

    pub fn update_diagnostic_report() -> &'static RiseDiagnosticsEndpoint {
        &UPDATE_DIAGNOSTIC_REPORT
    }

    pub fn delete_diagnostic_report() -> &'static RiseDiagnosticsEndpoint {
        &DELETE_DIAGNOSTIC_REPORT
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseDiagnosticsEndpoint {
    pub method: String,
    pub timeout: Duration,
    pub replay_policy: RiseWireReplayPolicy,
}

impl RiseDiagnosticsEndpoint {
    /// deadline, kind and idempotency field come from the canon catalogue
    /// expecting only checks that this declaration and canon agree
    #[track_caller]
    pub fn new(canonical: &str, expecting: RiseEndpointKind) -> Self {
        let entry = RiseCanonEndpoints::required(rise_wire_methods::report::SERVICE, canonical);
        debug_assert!(
            entry.kind == expecting,
            "report.{canonical}: canon declares {}, not {}",
            entry.kind.name(),
            expecting.name()
        );
        let replay_policy = if let Some(field) = entry.idempotency_field {
            RiseWireReplayPolicy::IdempotencyKey {
                field: field.to_owned(),
            }
        } else if entry.kind == RiseEndpointKind::Read {
            RiseWireReplayPolicy::ReadOnly
        } else {
            RiseWireReplayPolicy::Never
        };
        Self {
            method: entry.method.to_owned(),
            timeout: entry.client_timeout(),
            replay_policy,
        }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(rise_wire_methods::report::SERVICE, self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(self.replay_policy.clone())
    }
}

#[derive(Clone)]
pub struct RiseDiagnosticsRPC {
    engine: Arc<RiseEngine>,
}

impl RiseDiagnosticsRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn submit_diagnostic_report(
        &self,
        request: &rise_request::report::SubmitDiagnosticReport,
    ) -> anyhow::Result<rise_response::report::SubmitDiagnosticReport> {
        self.engine
            .call_descriptor_response(
                &RiseDiagnosticsEndpoints::submit_diagnostic_report().descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await
    }

    pub async fn get_my_diagnostic_reports(
        &self,
        request: &rise_request::report::GetMyDiagnosticReports,
    ) -> anyhow::Result<rise_response::report::GetMyDiagnosticReports> {
        self.engine
            .call_descriptor_response(
                &RiseDiagnosticsEndpoints::get_my_diagnostic_reports().descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await
    }

    pub async fn update_diagnostic_report(
        &self,
        request: &rise_request::report::UpdateDiagnosticReport,
    ) -> anyhow::Result<rise_response::report::UpdateDiagnosticReport> {
        self.engine
            .call_descriptor_response(
                &RiseDiagnosticsEndpoints::update_diagnostic_report().descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await
    }

    pub async fn delete_diagnostic_report(
        &self,
        request: &rise_request::report::DeleteDiagnosticReport,
    ) -> anyhow::Result<rise_response::report::DeleteDiagnosticReport> {
        self.engine
            .call_descriptor_response(
                &RiseDiagnosticsEndpoints::delete_diagnostic_report().descriptor(),
                request,
                None,
                RiseWireRequestMetadata::default(),
            )
            .await
    }
}

#[cfg(test)]
#[path = "rise_diagnostics_rpc_tests.rs"]
mod tests;
