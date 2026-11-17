use std::fmt;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::{RiseRequestJSON, RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseWireError, RiseWireMethodDescriptor, RiseWireReplayPolicy};
use rise_i18n::tr;

use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};
use crate::modules::moderation::engine::rise_moderation_wire::{
    AppealDTO, AppealsPageDTO, ReportRowDTO, ReportsPageDTO, RiseStoredAdminSanction, RiseStoredAppealCase,
    RiseStoredModerationNote, RiseStoredMyBlocks, RiseStoredReportCase,
};

/// one report-service method
/// deadline and replay policy come from the canon catalogue and not from the call site
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseModerationEndpoint {
    pub method: String,
    pub timeout: Duration,
    pub replay_policy: RiseWireReplayPolicy,
}

impl RiseModerationEndpoint {
    /// endpoint from a canonical name and the kind it expects
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

static CREATE_REPORT: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::CREATE_REPORT, RiseEndpointKind::Mutation)
});
static GET_MY_REPORTS: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::GET_MY_REPORTS, RiseEndpointKind::Read)
});
static GET_REPORTS: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::GET_REPORTS, RiseEndpointKind::Read)
});
static GET_REPORT_BY_ID: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::GET_REPORT_BY_ID, RiseEndpointKind::Read)
});
static RESOLVE_REPORT: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::RESOLVE_REPORT, RiseEndpointKind::Mutation)
});
static GET_MY_BLOCKS: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::GET_MY_BLOCKS, RiseEndpointKind::Read)
});
static CREATE_APPEAL: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::CREATE_APPEAL, RiseEndpointKind::Mutation)
});
static GET_MY_APPEALS: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::GET_MY_APPEALS, RiseEndpointKind::Read)
});
static GET_APPEALS: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::GET_APPEALS, RiseEndpointKind::Read)
});
static RESOLVE_APPEAL: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::RESOLVE_APPEAL, RiseEndpointKind::Mutation)
});
static GET_REPORT_CASE: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::GET_REPORT_CASE, RiseEndpointKind::Read)
});
static GET_APPEAL_CASE: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::GET_APPEAL_CASE, RiseEndpointKind::Read)
});
static ADD_REPORT_NOTE: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::ADD_REPORT_NOTE, RiseEndpointKind::Mutation)
});
static ADMIN_SANCTION: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::ADMIN_SANCTION, RiseEndpointKind::Mutation)
});
static REOPEN_REPORT: LazyLock<RiseModerationEndpoint> = LazyLock::new(|| {
    RiseModerationEndpoint::new(rise_wire_methods::report::REOPEN_REPORT, RiseEndpointKind::Mutation)
});

pub enum RiseModerationEndpoints {}

impl RiseModerationEndpoints {
    pub fn create_report() -> &'static RiseModerationEndpoint {
        &CREATE_REPORT
    }

    pub fn get_my_reports() -> &'static RiseModerationEndpoint {
        &GET_MY_REPORTS
    }

    pub fn get_reports() -> &'static RiseModerationEndpoint {
        &GET_REPORTS
    }

    pub fn get_report_by_id() -> &'static RiseModerationEndpoint {
        &GET_REPORT_BY_ID
    }

    pub fn resolve_report() -> &'static RiseModerationEndpoint {
        &RESOLVE_REPORT
    }

    pub fn get_my_blocks() -> &'static RiseModerationEndpoint {
        &GET_MY_BLOCKS
    }

    pub fn create_appeal() -> &'static RiseModerationEndpoint {
        &CREATE_APPEAL
    }

    pub fn get_my_appeals() -> &'static RiseModerationEndpoint {
        &GET_MY_APPEALS
    }

    pub fn get_appeals() -> &'static RiseModerationEndpoint {
        &GET_APPEALS
    }

    pub fn resolve_appeal() -> &'static RiseModerationEndpoint {
        &RESOLVE_APPEAL
    }

    pub fn get_report_case() -> &'static RiseModerationEndpoint {
        &GET_REPORT_CASE
    }

    pub fn get_appeal_case() -> &'static RiseModerationEndpoint {
        &GET_APPEAL_CASE
    }

    pub fn add_report_note() -> &'static RiseModerationEndpoint {
        &ADD_REPORT_NOTE
    }

    pub fn admin_sanction() -> &'static RiseModerationEndpoint {
        &ADMIN_SANCTION
    }

    pub fn reopen_report() -> &'static RiseModerationEndpoint {
        &REOPEN_REPORT
    }
}

pub trait ModerationEvidence {
    /// evidence lists are sent only when non empty, both halves together
    fn moderation_evidence(&self) -> Option<Vec<RiseRequestJSON>>;
}

impl ModerationEvidence for [String] {
    fn moderation_evidence(&self) -> Option<Vec<RiseRequestJSON>> {
        if self.is_empty() {
            None
        } else {
            Some(self.iter().cloned().map(RiseRequestJSON::String).collect())
        }
    }
}

#[derive(Clone)]
pub struct RiseModerationRPC {
    engine: Arc<RiseEngine>,
}

impl RiseModerationRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    pub async fn create_report(&self, request: rise_request::report::CreateReport) -> anyhow::Result<ReportRowDTO> {
        let reply: rise_response::report::CreateReport =
            self.call(RiseModerationEndpoints::create_report(), &request).await?;
        Ok(ReportRowDTO::from(Self::required(reply)?))
    }

    pub async fn get_my_reports(&self, request: rise_request::report::GetMyReports) -> anyhow::Result<ReportsPageDTO> {
        let reply: rise_response::report::GetMyReports =
            self.call(RiseModerationEndpoints::get_my_reports(), &request).await?;
        Ok(ReportsPageDTO::from(reply))
    }

    pub async fn get_reports(&self, request: rise_request::report::GetReports) -> anyhow::Result<ReportsPageDTO> {
        let reply: rise_response::report::GetReports = self.call(RiseModerationEndpoints::get_reports(), &request).await?;
        Ok(ReportsPageDTO::from(reply))
    }

    pub async fn get_report_by_id(&self, request: rise_request::report::GetReportById) -> anyhow::Result<ReportRowDTO> {
        let reply: rise_response::report::GetReportById =
            self.call(RiseModerationEndpoints::get_report_by_id(), &request).await?;
        Ok(ReportRowDTO::from(Self::required(reply)?))
    }

    pub async fn resolve_report(&self, request: rise_request::report::ResolveReport) -> anyhow::Result<ReportRowDTO> {
        let reply: rise_response::report::ResolveReport =
            self.call(RiseModerationEndpoints::resolve_report(), &request).await?;
        Ok(ReportRowDTO::from(Self::required(reply)?))
    }

    pub async fn get_my_blocks(&self, request: rise_request::report::GetMyBlocks) -> anyhow::Result<RiseStoredMyBlocks> {
        let reply: rise_response::report::GetMyBlocks =
            self.call(RiseModerationEndpoints::get_my_blocks(), &request).await?;
        Ok(RiseStoredMyBlocks::from(reply))
    }

    pub async fn create_appeal(&self, request: rise_request::report::CreateAppeal) -> anyhow::Result<Option<AppealDTO>> {
        let reply: rise_response::report::CreateAppeal =
            self.call(RiseModerationEndpoints::create_appeal(), &request).await?;
        Ok(reply.map(AppealDTO::from))
    }

    pub async fn get_my_appeals(&self, request: rise_request::report::GetMyAppeals) -> anyhow::Result<AppealsPageDTO> {
        let reply: rise_response::report::GetMyAppeals =
            self.call(RiseModerationEndpoints::get_my_appeals(), &request).await?;
        Ok(AppealsPageDTO::from(reply))
    }

    pub async fn get_appeals(&self, request: rise_request::report::GetAppeals) -> anyhow::Result<AppealsPageDTO> {
        let reply: rise_response::report::GetAppeals = self.call(RiseModerationEndpoints::get_appeals(), &request).await?;
        Ok(AppealsPageDTO::from(reply))
    }

    pub async fn resolve_appeal(&self, request: rise_request::report::ResolveAppeal) -> anyhow::Result<Option<AppealDTO>> {
        let reply: rise_response::report::ResolveAppeal =
            self.call(RiseModerationEndpoints::resolve_appeal(), &request).await?;
        Ok(reply.map(AppealDTO::from))
    }

    pub async fn get_report_case(
        &self,
        request: rise_request::report::GetReportCase,
    ) -> anyhow::Result<RiseStoredReportCase> {
        let reply: rise_response::report::GetReportCase =
            self.call(RiseModerationEndpoints::get_report_case(), &request).await?;
        let reply = Self::required(reply)?;
        RiseStoredReportCase::from_canon(reply).ok_or_else(|| RiseWireError::ResponseDecodingFailed.into())
    }

    pub async fn get_appeal_case(
        &self,
        request: rise_request::report::GetAppealCase,
    ) -> anyhow::Result<RiseStoredAppealCase> {
        let reply: rise_response::report::GetAppealCase =
            self.call(RiseModerationEndpoints::get_appeal_case(), &request).await?;
        let reply = Self::required(reply)?;
        RiseStoredAppealCase::from_canon(reply).ok_or_else(|| RiseWireError::ResponseDecodingFailed.into())
    }

    pub async fn add_report_note(
        &self,
        request: rise_request::report::AddReportNote,
    ) -> anyhow::Result<RiseStoredModerationNote> {
        let reply: rise_response::report::AddReportNote =
            self.call(RiseModerationEndpoints::add_report_note(), &request).await?;
        Ok(RiseStoredModerationNote::from(Self::required(reply)?))
    }

    pub async fn admin_sanction(
        &self,
        request: rise_request::report::AdminSanction,
    ) -> anyhow::Result<RiseStoredAdminSanction> {
        let reply: rise_response::report::AdminSanction =
            self.call(RiseModerationEndpoints::admin_sanction(), &request).await?;
        Ok(RiseStoredAdminSanction::from(reply))
    }

    pub async fn reopen_report(&self, request: rise_request::report::ReopenReport) -> anyhow::Result<ReportRowDTO> {
        let reply: rise_response::report::ReopenReport =
            self.call(RiseModerationEndpoints::reopen_report(), &request).await?;
        Ok(ReportRowDTO::from(Self::required(reply)?))
    }

    async fn call<Request, Response>(&self, endpoint: &RiseModerationEndpoint, request: &Request) -> anyhow::Result<Response>
    where
        Request: serde::Serialize + Sync,
        Response: serde::de::DeserializeOwned,
    {
        self.engine
            .call_descriptor_response(&endpoint.descriptor(), request, None, RiseWireRequestMetadata::default())
            .await
    }

    /// a reply the canon declares optional that came back empty is a broken reply
    fn required<Value>(value: Option<Value>) -> Result<Value, RiseWireError> {
        value.ok_or(RiseWireError::ResponseDecodingFailed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseModerationError {
    NoEngine,
    StaleAccount,
    MissingActiveSession,
    AlreadySubmitting,
    TooManyAttachments,
    VideoTooLong,
}

impl RiseModerationError {
    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::NoEngine | Self::StaleAccount | Self::MissingActiveSession => tr("error_fetching_data"),
            Self::AlreadySubmitting => tr("report_failed_title"),
            Self::TooManyAttachments => tr("report_evidence_limit_reached"),
            Self::VideoTooLong => tr("bug_report_video_too_long"),
        })
    }
}

impl fmt::Display for RiseModerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for RiseModerationError {}
