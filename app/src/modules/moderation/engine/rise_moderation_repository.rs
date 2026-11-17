use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rise_canon::{RiseLimits, rise_request};
use rise_engine::{CancellationError, RiseEngine};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::moderation::engine::core::rise_moderation_resource_repository::{
    RiseModerationCached, RiseModerationResourceRepository,
};
use crate::modules::moderation::engine::rise_moderation_rpc::RiseModerationRPC;
use crate::modules::moderation::engine::rise_moderation_wire::{
    AppealDTO, AppealsPageDTO, ReportRowDTO, ReportsPageDTO, RiseStoredAdminSanction, RiseStoredAppealCase,
    RiseStoredModerationNote, RiseStoredMyBlocks, RiseStoredReportCase, RiseStoredUserBlock,
};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AppealStatus, ReportQueueSort, ReportStatus, ReportTargetType,
};

/// every field is part of the cache key so two filters never read each others first page
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModerationQueueFilter {
    pub status: ReportStatus,
    pub r#type: Option<ReportTargetType>,
    pub sort: ReportQueueSort,
    pub target_user_id: Option<String>,
}

impl Default for ModerationQueueFilter {
    fn default() -> Self {
        Self {
            status: ReportStatus::Active,
            r#type: None,
            sort: ReportQueueSort::DateDesc,
            target_user_id: None,
        }
    }
}

impl ModerationQueueFilter {
    pub fn cache_params(&self) -> Vec<String> {
        vec![
            self.status.wire().to_owned(),
            self.r#type.map_or(rise_moderation::ALL_FILTER_NAME, |kind| kind.wire()).to_owned(),
            self.sort.wire().to_owned(),
            self.target_user_id.clone().unwrap_or_default(),
        ]
    }
}

/// cache names of the moderation resources
enum Resource {}

impl Resource {
    const MY_REPORTS: &'static str = "my-reports";
    const MY_BLOCKS: &'static str = "my-blocks";
    const MY_APPEALS: &'static str = "my-appeals";
    const QUEUE: &'static str = "queue";
    const REVIEW: &'static str = "appeals-review";
    const REPORT_CASE: &'static str = "case";
    const APPEAL_CASE: &'static str = "appeal-case";
}

/// cache first moderation reads and writes
/// only first pages, the callers blocks and opened cases are persisted
/// a cached page two without page one would leave a hole in the list
pub struct RiseModerationRepository {
    account_id: String,
    rpc: RiseModerationRPC,
    resource: Arc<RiseModerationResourceRepository>,
    is_shutdown: AtomicBool,
}

impl RiseModerationRepository {
    pub fn my_reports_limit() -> i64 {
        RiseLimits::client_value("moderation_services_store_m_y_r_e_p_o_r_t_s_l_i_m_i_t")
    }

    pub fn queue_limit() -> i64 {
        RiseLimits::client_value("moderation_services_store_q_u_e_u_e_l_i_m_i_t")
    }

    pub fn appeals_limit() -> i64 {
        RiseLimits::client_value("moderation_services_store_a_p_p_e_a_l_s_l_i_m_i_t")
    }

    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Self {
        Self {
            rpc: RiseModerationRPC::new(engine.clone()),
            resource: Arc::new(RiseModerationResourceRepository::new(engine, account_id.clone())),
            account_id,
            is_shutdown: AtomicBool::new(false),
        }
    }

    pub async fn cached_my_reports(&self, status: Option<ReportStatus>) -> Option<RiseModerationCached<ReportsPageDTO>> {
        let params = [Self::status_param(status.map(ReportStatus::wire))];
        self.resource.cached(Resource::MY_REPORTS, &params).await.ok().flatten()
    }

    pub async fn cached_my_blocks(&self) -> Option<RiseModerationCached<RiseStoredMyBlocks>> {
        self.resource.cached(Resource::MY_BLOCKS, &[]).await.ok().flatten()
    }

    pub async fn cached_my_appeals(&self, status: Option<&str>) -> Option<RiseModerationCached<AppealsPageDTO>> {
        let params = [Self::status_param(status)];
        self.resource.cached(Resource::MY_APPEALS, &params).await.ok().flatten()
    }

    pub async fn cached_queue(&self, filter: &ModerationQueueFilter) -> Option<RiseModerationCached<ReportsPageDTO>> {
        self.resource.cached(Resource::QUEUE, &filter.cache_params()).await.ok().flatten()
    }

    pub async fn cached_review(&self, status: &str) -> Option<RiseModerationCached<AppealsPageDTO>> {
        self.resource.cached(Resource::REVIEW, &[status.to_owned()]).await.ok().flatten()
    }

    pub async fn cached_case(&self, report_id: i64) -> Option<RiseModerationCached<RiseStoredReportCase>> {
        self.resource
            .cached(Resource::REPORT_CASE, &[report_id.to_string()])
            .await
            .ok()
            .flatten()
    }

    pub async fn cached_appeal_case(&self, appeal_id: i64) -> Option<RiseModerationCached<RiseStoredAppealCase>> {
        self.resource
            .cached(Resource::APPEAL_CASE, &[appeal_id.to_string()])
            .await
            .ok()
            .flatten()
    }

    pub async fn my_reports(&self, status: Option<ReportStatus>, relative_id: Option<i64>) -> anyhow::Result<ReportsPageDTO> {
        self.check_alive()?;
        let page = self
            .rpc
            .get_my_reports(rise_request::report::GetMyReports {
                limit: Some(Self::my_reports_limit()),
                relative_id,
                status: status.map(|status| status.wire().to_owned()),
                up: Some(false),
                ..Default::default()
            })
            .await?;
        if relative_id.is_none() {
            let params = [Self::status_param(status.map(ReportStatus::wire))];
            let _ = self.resource.store(&page, Resource::MY_REPORTS, &params).await;
        }
        Ok(page)
    }

    pub async fn my_blocks(&self) -> anyhow::Result<RiseStoredMyBlocks> {
        self.check_alive()?;
        let blocks = self.rpc.get_my_blocks(rise_request::report::GetMyBlocks {}).await?;
        let _ = self.resource.store(&blocks, Resource::MY_BLOCKS, &[]).await;
        Ok(blocks)
    }

    pub async fn my_appeals(&self, status: Option<&str>, relative_id: Option<i64>) -> anyhow::Result<AppealsPageDTO> {
        self.check_alive()?;
        let page = self
            .rpc
            .get_my_appeals(rise_request::report::GetMyAppeals {
                limit: Some(Self::appeals_limit()),
                relative_id,
                status: status.map(str::to_owned),
                up: Some(false),
            })
            .await?;
        if relative_id.is_none() {
            let _ = self
                .resource
                .store(&page, Resource::MY_APPEALS, &[Self::status_param(status)])
                .await;
        }
        Ok(page)
    }

    pub async fn blocked_account_appeal_status(&self, block: &RiseStoredUserBlock) -> anyhow::Result<Option<AppealStatus>> {
        let page = match self.my_appeals(None, None).await {
            Ok(page) => page,
            Err(error) => {
                match self.cached_my_appeals(None).await {
                    Some(cached) => cached.value,
                    None => return Err(error),
                }
            }
        };
        self.check_alive()?;
        let appeal = page
            .appeals
            .iter()
            .map(|item| &item.appeal)
            .filter(|appeal| appeal.appellant_id == self.account_id && appeal.concerns(block))
            .max_by_key(|appeal| appeal.id);
        Ok(appeal.and_then(|appeal| AppealStatus::from_wire(&appeal.status)))
    }

    pub async fn queue(&self, filter: &ModerationQueueFilter, relative_id: Option<i64>) -> anyhow::Result<ReportsPageDTO> {
        self.check_alive()?;
        let page = self
            .rpc
            .get_reports(rise_request::report::GetReports {
                limit: Some(Self::queue_limit()),
                relative_id,
                report_type: filter.r#type.map(|kind| kind.wire().to_owned()),
                sort: Some(filter.sort.wire().to_owned()),
                status: Some(filter.status.wire().to_owned()),
                target_user_id: filter.target_user_id.clone(),
                up: Some(false),
            })
            .await?;
        if relative_id.is_none() {
            let _ = self.resource.store(&page, Resource::QUEUE, &filter.cache_params()).await;
        }
        Ok(page)
    }

    pub async fn review(&self, status: &str, relative_id: Option<i64>) -> anyhow::Result<AppealsPageDTO> {
        self.check_alive()?;
        let page = self
            .rpc
            .get_appeals(rise_request::report::GetAppeals {
                limit: Some(Self::appeals_limit()),
                relative_id,
                status: Some(status.to_owned()),
                up: Some(false),
            })
            .await?;
        if relative_id.is_none() {
            let _ = self.resource.store(&page, Resource::REVIEW, &[status.to_owned()]).await;
        }
        Ok(page)
    }

    pub async fn report_case(&self, report_id: i64) -> anyhow::Result<RiseStoredReportCase> {
        self.check_alive()?;
        let value = self
            .rpc
            .get_report_case(rise_request::report::GetReportCase {
                report_id: Some(report_id),
            })
            .await?;
        let _ = self
            .resource
            .store(&value, Resource::REPORT_CASE, &[report_id.to_string()])
            .await;
        Ok(value)
    }

    pub async fn appeal_case(&self, appeal_id: i64) -> anyhow::Result<RiseStoredAppealCase> {
        self.check_alive()?;
        let value = self
            .rpc
            .get_appeal_case(rise_request::report::GetAppealCase {
                appeal_id: Some(appeal_id),
            })
            .await?;
        let _ = self
            .resource
            .store(&value, Resource::APPEAL_CASE, &[appeal_id.to_string()])
            .await;
        Ok(value)
    }

    pub async fn store_case(&self, value: &RiseStoredReportCase) {
        let _ = self
            .resource
            .store(value, Resource::REPORT_CASE, &[value.report.id.to_string()])
            .await;
    }

    pub async fn store_appeal_case(&self, value: &RiseStoredAppealCase) {
        let _ = self
            .resource
            .store(value, Resource::APPEAL_CASE, &[value.appeal.id.to_string()])
            .await;
    }

    pub async fn create_report(&self, request: rise_request::report::CreateReport) -> anyhow::Result<ReportRowDTO> {
        self.check_alive()?;
        let report = self.rpc.create_report(request).await?;
        self.forget_memory(Resource::MY_REPORTS);
        Ok(report)
    }

    pub async fn resolve_report(&self, request: rise_request::report::ResolveReport) -> anyhow::Result<ReportRowDTO> {
        self.check_alive()?;
        let report = self.rpc.resolve_report(request).await?;
        self.forget_memory(Resource::QUEUE);
        Ok(report)
    }

    pub async fn create_appeal(&self, request: rise_request::report::CreateAppeal) -> anyhow::Result<Option<AppealDTO>> {
        self.check_alive()?;
        let appeal = self.rpc.create_appeal(request).await?;
        self.forget_memory(Resource::MY_APPEALS);
        self.forget_memory(Resource::MY_BLOCKS);
        Ok(appeal)
    }

    pub async fn resolve_appeal(&self, request: rise_request::report::ResolveAppeal) -> anyhow::Result<Option<AppealDTO>> {
        self.check_alive()?;
        let appeal = self.rpc.resolve_appeal(request).await?;
        self.forget_memory(Resource::REVIEW);
        Ok(appeal)
    }

    pub async fn add_note(&self, request: rise_request::report::AddReportNote) -> anyhow::Result<RiseStoredModerationNote> {
        self.check_alive()?;
        self.rpc.add_report_note(request).await
    }

    pub async fn admin_sanction(
        &self,
        request: rise_request::report::AdminSanction,
    ) -> anyhow::Result<RiseStoredAdminSanction> {
        self.check_alive()?;
        self.rpc.admin_sanction(request).await
    }

    pub async fn reopen_report(&self, request: rise_request::report::ReopenReport) -> anyhow::Result<ReportRowDTO> {
        self.check_alive()?;
        let report = self.rpc.reopen_report(request).await?;
        self.forget_memory(Resource::QUEUE);
        Ok(report)
    }

    pub fn forget_queue(&self) {
        self.forget_memory(Resource::QUEUE);
    }

    pub fn forget_personal(&self) {
        self.forget_memory(Resource::MY_REPORTS);
        self.forget_memory(Resource::MY_BLOCKS);
        self.forget_memory(Resource::MY_APPEALS);
    }

    pub async fn shutdown(&self) {
        if self.is_shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        self.resource.shutdown();
    }

    fn check_alive(&self) -> Result<(), CancellationError> {
        if self.is_shutdown.load(Ordering::SeqCst) { Err(CancellationError) } else { Ok(()) }
    }

    /// memory only
    /// persisted first page gets fixed by the refresh after every mutation
    /// a stale page beats an empty one
    fn forget_memory(&self, name: &str) {
        self.resource.forget_memory(name);
    }

    fn status_param(status: Option<&str>) -> String {
        status.unwrap_or(rise_moderation::ALL_FILTER_NAME).to_owned()
    }
}
