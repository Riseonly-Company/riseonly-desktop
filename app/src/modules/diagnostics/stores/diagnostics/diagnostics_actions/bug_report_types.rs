use rise_canon::{RiseBugReportSurface, RiseDiagnosticReportKind, RiseFeedbackKind};
use rise_catalog::DIAGNOSTICS_FEEDBACK_KINDS_DEFAULT;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;

use crate::modules::diagnostics::engine::core::rise_bug_report_upload::BugReportAttachment;
use crate::modules::diagnostics::engine::rise_bug_report_submitter::BugReportDraftPayload;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_wire_types::BugReportRecord;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BugReportRoute {
    Idle,
    Sheet,
    ClosingSheetForPicker,
    ClosingSheetForDismissal,
    Picker,
    ClosingPickerForSheet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BugReportDraftKind {
    Bug,
    Feedback,
}

impl BugReportDraftKind {
    pub const ALL_CASES: [BugReportDraftKind; 2] = [BugReportDraftKind::Bug, BugReportDraftKind::Feedback];

    pub const fn raw_value(self) -> &'static str {
        match self {
            BugReportDraftKind::Bug => "bug",
            BugReportDraftKind::Feedback => "feedback",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|kind| kind.raw_value() == raw)
    }

    pub const fn wire(self) -> &'static str {
        match self {
            BugReportDraftKind::Bug => RiseDiagnosticReportKind::UserReport.wire(),
            BugReportDraftKind::Feedback => RiseDiagnosticReportKind::Feedback.wire(),
        }
    }

    pub const fn page(self) -> BugReportSheetPage {
        match self {
            BugReportDraftKind::Bug => BugReportSheetPage::Bug,
            BugReportDraftKind::Feedback => BugReportSheetPage::Feedback,
        }
    }

    pub fn from_kind(kind: RiseDiagnosticReportKind) -> Option<Self> {
        match kind {
            RiseDiagnosticReportKind::UserReport => Some(BugReportDraftKind::Bug),
            RiseDiagnosticReportKind::Feedback => Some(BugReportDraftKind::Feedback),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BugReportSheetPage {
    Root,
    Feedback,
    Bug,
    Areas,
}

impl BugReportSheetPage {
    pub const fn raw_value(self) -> &'static str {
        match self {
            BugReportSheetPage::Root => "root",
            BugReportSheetPage::Feedback => "feedback",
            BugReportSheetPage::Bug => "bug",
            BugReportSheetPage::Areas => "areas",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        [
            BugReportSheetPage::Root,
            BugReportSheetPage::Feedback,
            BugReportSheetPage::Bug,
            BugReportSheetPage::Areas,
        ]
        .into_iter()
        .find(|page| page.raw_value() == raw)
    }

    pub fn stack(self) -> Vec<BugReportSheetPage> {
        match self {
            BugReportSheetPage::Root => vec![BugReportSheetPage::Root],
            BugReportSheetPage::Feedback => vec![BugReportSheetPage::Root, BugReportSheetPage::Feedback],
            BugReportSheetPage::Bug => vec![BugReportSheetPage::Root, BugReportSheetPage::Bug],
            BugReportSheetPage::Areas => vec![BugReportSheetPage::Root],
        }
    }

    pub const fn form_kind(self) -> Option<BugReportDraftKind> {
        match self {
            BugReportSheetPage::Feedback => Some(BugReportDraftKind::Feedback),
            BugReportSheetPage::Bug => Some(BugReportDraftKind::Bug),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BugReportDraft {
    pub client_report_id: String,
    pub kind: BugReportDraftKind,
    pub feedback_kind: RiseFeedbackKind,
    pub editing_report_id: Option<String>,
    pub parent_report_id: Option<String>,
    pub message: String,
    pub pages: Vec<RiseBugReportSurface>,
    pub seeded_pages: Vec<RiseBugReportSurface>,
    pub component: String,
    pub screenshots: Vec<BugReportAttachment>,
    pub video: Option<BugReportAttachment>,
}

impl BugReportDraft {
    pub fn new(client_report_id: String, kind: BugReportDraftKind) -> Self {
        Self {
            client_report_id,
            kind,
            feedback_kind: DIAGNOSTICS_FEEDBACK_KINDS_DEFAULT,
            editing_report_id: None,
            parent_report_id: None,
            message: String::new(),
            pages: Vec::new(),
            seeded_pages: Vec::new(),
            component: String::new(),
            screenshots: Vec::new(),
            video: None,
        }
    }

    pub fn temporary_files(&self) -> Vec<Url> {
        self.screenshots
            .iter()
            .chain(self.video.iter())
            .flat_map(|attachment| [attachment.url.clone(), attachment.preview_url.clone()])
            .flatten()
            .collect()
    }

    pub fn payload(&self) -> BugReportDraftPayload {
        let trimmed_component = trimming_whitespaces_and_newlines(&self.component);
        BugReportDraftPayload {
            client_report_id: self.client_report_id.clone(),
            kind: self.kind.wire().to_owned(),
            feedback_kind: (self.kind == BugReportDraftKind::Feedback)
                .then(|| self.feedback_kind.wire().to_owned()),
            parent_report_id: self.parent_report_id.clone(),
            message: trimming_whitespaces_and_newlines(&self.message).to_owned(),
            pages: self.pages.iter().map(|page| page.wire().to_owned()).collect(),
            component: (self.kind == BugReportDraftKind::Bug && !trimmed_component.is_empty())
                .then(|| trimmed_component.to_owned()),
            screenshots: self.screenshots.clone(),
            video: self.video.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BugReportSubmissionState {
    Editing,
    Sending { progress: f64 },
    Sent,
    Failed { message: String },
}

impl BugReportSubmissionState {
    pub fn is_sending(&self) -> bool {
        matches!(self, BugReportSubmissionState::Sending { .. })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BugReportListState {
    #[default]
    Idle,
    Loading,
    Loaded,
    Failed {
        message: String,
    },
}

impl BugReportListState {
    pub fn is_loading(&self) -> bool {
        *self == BugReportListState::Loading
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BugReportListSlice {
    pub records: Vec<BugReportRecord>,
    pub state: BugReportListState,
    pub have_more: bool,
}
