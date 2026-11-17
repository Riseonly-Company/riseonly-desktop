use rise_canon::{RiseBugReportStatus, RiseBugReportSurface, RiseDiagnosticReportKind, RiseFeedbackKind, rise_response, rise_view};
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::UrlComponents;

use crate::core::stores::http::http_service::HttpService;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_types::BugReportDraftKind;

pub type BugReportWireDTO = rise_view::report::DiagnosticReport;
pub type MyDiagnosticReportsGatewayDTO = rise_response::report::GetMyDiagnosticReports;
pub type UpdatedDiagnosticReportGatewayDTO = rise_response::report::UpdateDiagnosticReport;

const ATTACHMENT_ROUTE: &str = "/api/file/secure/bug-report/";
const MAXIMUM_ATTACHMENT_ROUTE_CHARACTERS: usize = 256;
const HTTP_SCHEME: &str = "http";
const HTTPS_SCHEME: &str = "https";
const HTTP_PREFIX: &str = "http://";
const HTTPS_PREFIX: &str = "https://";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BugReportRecord {
    pub id: String,
    pub kind: BugReportDraftKind,
    pub feedback_kind: Option<RiseFeedbackKind>,
    pub status: RiseBugReportStatus,
    pub message: String,
    pub pages: Vec<RiseBugReportSurface>,
    pub component: Option<String>,
    pub screenshot_file_urls: Vec<String>,
    pub screenshot_file_ids: Vec<String>,
    pub video_file_url: Option<String>,
    pub video_file_id: Option<String>,
    pub video_duration_ms: Option<i64>,
    pub received_at_ms: i64,
    pub updated_at_ms: Option<i64>,
    pub attempt_count: i64,
    pub reopened_from: Option<RiseBugReportStatus>,
}

impl BugReportRecord {
    pub fn new(dto: &BugReportWireDTO) -> Option<Self> {
        if dto.id.is_empty() {
            return None;
        }
        let wire_kind = RiseDiagnosticReportKind::from_wire(&dto.kind).unwrap_or(RiseDiagnosticReportKind::UserReport);
        Some(Self {
            id: dto.id.clone(),
            kind: BugReportDraftKind::from_kind(wire_kind).unwrap_or(BugReportDraftKind::Bug),
            feedback_kind: dto.feedback_kind.as_deref().and_then(RiseFeedbackKind::from_wire),
            status: RiseBugReportStatus::from_wire(&dto.status).unwrap_or(RiseBugReportStatus::Pending),
            message: dto.message.clone().unwrap_or_default(),
            pages: dto.pages.iter().filter_map(|page| RiseBugReportSurface::from_wire(page)).collect(),
            component: dto.component.as_deref().and_then(nil_if_blank),
            screenshot_file_urls: dto.screenshot_file_urls.clone(),
            screenshot_file_ids: dto.screenshot_file_ids.clone(),
            video_file_url: dto.video_file_url.as_deref().and_then(nil_if_blank),
            video_file_id: dto.video_file_id.as_deref().and_then(nil_if_blank),
            video_duration_ms: dto.video_duration_ms,
            received_at_ms: dto.received_at,
            updated_at_ms: dto.updated_at,
            attempt_count: dto.follow_up_count.saturating_add(1).max(1),
            reopened_from: dto.reopened_from.as_deref().and_then(RiseBugReportStatus::from_wire),
        })
    }

    pub fn is_feedback(&self) -> bool {
        self.kind == BugReportDraftKind::Feedback
    }

    pub fn is_editable(&self) -> bool {
        self.status == RiseBugReportStatus::Pending || self.status == RiseBugReportStatus::Reopened
    }

    pub fn can_refile(&self) -> bool {
        self.kind == BugReportDraftKind::Bug && self.status == RiseBugReportStatus::Fixed
    }
}

pub enum BugReportAttachmentURL {}

impl BugReportAttachmentURL {
    pub fn absolute(raw: &str) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw);
        if value.is_empty() {
            return None;
        }
        if value.starts_with(HTTP_PREFIX) || value.starts_with(HTTPS_PREFIX) {
            return Some(value.to_owned());
        }
        if !value.starts_with(ATTACHMENT_ROUTE)
            || character_count(value) > MAXIMUM_ATTACHMENT_ROUTE_CHARACTERS
            || value.contains("..")
            || value.contains('%')
            || value.contains('?')
            || value.contains('#')
            || value.contains('\\')
        {
            return None;
        }
        let mut components = UrlComponents::from_url(&HttpService::resolved_api_base_url(), false)?;
        let scheme = components.scheme()?.to_lowercase();
        if scheme != HTTP_SCHEME && scheme != HTTPS_SCHEME {
            return None;
        }
        components.host()?;
        components.set_path(value);
        components.set_query(None);
        components.set_fragment(None);
        components.url().map(|url| url.absolute_string().to_owned())
    }
}

fn nil_if_blank(value: &str) -> Option<String> {
    let trimmed = trimming_whitespaces_and_newlines(value);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}
