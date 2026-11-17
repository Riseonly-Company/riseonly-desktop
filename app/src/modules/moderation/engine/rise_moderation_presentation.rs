use std::collections::{HashMap, HashSet};

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::moderation::engine::core::rise_moderation_resource_repository::RiseModerationCacheSource;
use crate::modules::moderation::engine::rise_moderation_repository::ModerationQueueFilter;
use crate::modules::moderation::engine::rise_moderation_wire::{
    AppealDTO, AppealItemDTO, AppealsPageDTO, ModerationUserDTO, ReportResolutionEvent, ReportRowDTO,
    ReportsPageDTO, RiseStoredAppealCase, RiseStoredAutoStanding, RiseStoredModerationNote, RiseStoredMyBlocks,
    RiseStoredReportCase, RiseStoredSanctionHistoryItem, RiseStoredUserBlock, RiseStoredWarningState,
};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AppealReviewFilter, AppealStatus, AppealStatusFilter, ReportStatus, UserBlockType,
};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ModerationLoadState {
    #[default]
    Idle,
    Loading,
    Loaded,
    Failed(String),
}

impl ModerationLoadState {
    pub fn is_loading(&self) -> bool {
        *self == ModerationLoadState::Loading
    }

    pub fn error_text(&self) -> Option<&str> {
        match self {
            ModerationLoadState::Failed(message) => Some(message),
            _ => None,
        }
    }
}

/// everything the moderation screens read, published per account and session
/// domain activates it for its account and deactivates on shutdown
/// a snapshot from a session no longer active is dropped at the gate
pub struct RiseModerationPresentation {
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
    is_active: bool,
    is_stale: bool,

    my_reports_filter: Option<ReportStatus>,
    my_reports: Vec<ReportRowDTO>,
    my_reports_state: ModerationLoadState,
    my_reports_has_more: bool,
    my_reports_cursor: Option<i64>,
    my_reports_total: i64,
    is_loading_more_my_reports: bool,
    my_reports_load_more_error: Option<String>,

    my_blocks: Vec<RiseStoredUserBlock>,
    my_warnings: RiseStoredWarningState,
    my_history: Vec<RiseStoredSanctionHistoryItem>,
    my_auto_standings: Vec<RiseStoredAutoStanding>,
    my_blocks_state: ModerationLoadState,

    my_appeals_filter: AppealStatusFilter,
    my_appeals: Vec<AppealItemDTO>,
    my_pending_appeals: HashMap<i64, AppealDTO>,
    my_appeals_state: ModerationLoadState,
    my_appeals_has_more: bool,
    my_appeals_cursor: Option<i64>,
    my_appeals_total: i64,
    is_loading_more_my_appeals: bool,

    queue_filter: ModerationQueueFilter,
    queue: Vec<ReportRowDTO>,
    queue_state: ModerationLoadState,
    queue_has_more: bool,
    queue_cursor: Option<i64>,
    queue_total: i64,
    is_loading_more_queue: bool,

    cases: HashMap<i64, RiseStoredReportCase>,
    case_states: HashMap<i64, ModerationLoadState>,

    review_filter: AppealReviewFilter,
    review: Vec<AppealItemDTO>,
    review_state: ModerationLoadState,
    review_has_more: bool,
    review_cursor: Option<i64>,
    review_total: i64,
    is_loading_more_review: bool,

    appeal_cases: HashMap<i64, RiseStoredAppealCase>,
    appeal_case_states: HashMap<i64, ModerationLoadState>,

    resolving_reports: HashSet<i64>,
    noting_reports: HashSet<i64>,
    reopening_reports: HashSet<i64>,
    resolving_appeals: HashSet<i64>,
    is_sanctioning: bool,

    last_error: Option<String>,
}

impl RiseModerationPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            account_id: None,
            account_epoch: None,
            session_id: None,
            is_active: false,
            is_stale: false,
            my_reports_filter: None,
            my_reports: Vec::new(),
            my_reports_state: ModerationLoadState::Idle,
            my_reports_has_more: false,
            my_reports_cursor: None,
            my_reports_total: 0,
            is_loading_more_my_reports: false,
            my_reports_load_more_error: None,
            my_blocks: Vec::new(),
            my_warnings: RiseStoredWarningState::EMPTY,
            my_history: Vec::new(),
            my_auto_standings: Vec::new(),
            my_blocks_state: ModerationLoadState::Idle,
            my_appeals_filter: AppealStatusFilter::All,
            my_appeals: Vec::new(),
            my_pending_appeals: HashMap::new(),
            my_appeals_state: ModerationLoadState::Idle,
            my_appeals_has_more: false,
            my_appeals_cursor: None,
            my_appeals_total: 0,
            is_loading_more_my_appeals: false,
            queue_filter: ModerationQueueFilter::default(),
            queue: Vec::new(),
            queue_state: ModerationLoadState::Idle,
            queue_has_more: false,
            queue_cursor: None,
            queue_total: 0,
            is_loading_more_queue: false,
            cases: HashMap::new(),
            case_states: HashMap::new(),
            review_filter: AppealReviewFilter::Pending,
            review: Vec::new(),
            review_state: ModerationLoadState::Idle,
            review_has_more: false,
            review_cursor: None,
            review_total: 0,
            is_loading_more_review: false,
            appeal_cases: HashMap::new(),
            appeal_case_states: HashMap::new(),
            resolving_reports: HashSet::new(),
            noting_reports: HashSet::new(),
            reopening_reports: HashSet::new(),
            resolving_appeals: HashSet::new(),
            is_sanctioning: false,
            last_error: None,
        }
    }

    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }

    pub fn account_epoch(&self) -> Option<&str> {
        self.account_epoch.as_deref()
    }

    pub fn session_id(&self) -> Option<Uuid> {
        self.session_id
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn is_stale(&self) -> bool {
        self.is_stale
    }

    pub fn my_reports_filter(&self) -> Option<ReportStatus> {
        self.my_reports_filter
    }

    pub fn my_reports(&self) -> &[ReportRowDTO] {
        &self.my_reports
    }

    pub fn my_reports_state(&self) -> &ModerationLoadState {
        &self.my_reports_state
    }

    pub fn my_reports_has_more(&self) -> bool {
        self.my_reports_has_more
    }

    pub fn my_reports_cursor(&self) -> Option<i64> {
        self.my_reports_cursor
    }

    pub fn my_reports_total(&self) -> i64 {
        self.my_reports_total
    }

    pub fn is_loading_more_my_reports(&self) -> bool {
        self.is_loading_more_my_reports
    }

    pub fn my_reports_load_more_error(&self) -> Option<&str> {
        self.my_reports_load_more_error.as_deref()
    }

    pub fn my_blocks(&self) -> &[RiseStoredUserBlock] {
        &self.my_blocks
    }

    pub fn my_warnings(&self) -> &RiseStoredWarningState {
        &self.my_warnings
    }

    pub fn my_history(&self) -> &[RiseStoredSanctionHistoryItem] {
        &self.my_history
    }

    pub fn my_auto_standings(&self) -> &[RiseStoredAutoStanding] {
        &self.my_auto_standings
    }

    pub fn my_blocks_state(&self) -> &ModerationLoadState {
        &self.my_blocks_state
    }

    pub fn my_appeals_filter(&self) -> AppealStatusFilter {
        self.my_appeals_filter
    }

    pub fn my_appeals(&self) -> &[AppealItemDTO] {
        &self.my_appeals
    }

    pub fn my_appeals_state(&self) -> &ModerationLoadState {
        &self.my_appeals_state
    }

    pub fn my_appeals_has_more(&self) -> bool {
        self.my_appeals_has_more
    }

    pub fn my_appeals_cursor(&self) -> Option<i64> {
        self.my_appeals_cursor
    }

    pub fn my_appeals_total(&self) -> i64 {
        self.my_appeals_total
    }

    pub fn is_loading_more_my_appeals(&self) -> bool {
        self.is_loading_more_my_appeals
    }

    pub fn queue_filter(&self) -> &ModerationQueueFilter {
        &self.queue_filter
    }

    pub fn queue(&self) -> &[ReportRowDTO] {
        &self.queue
    }

    pub fn queue_state(&self) -> &ModerationLoadState {
        &self.queue_state
    }

    pub fn queue_has_more(&self) -> bool {
        self.queue_has_more
    }

    pub fn queue_cursor(&self) -> Option<i64> {
        self.queue_cursor
    }

    pub fn queue_total(&self) -> i64 {
        self.queue_total
    }

    pub fn is_loading_more_queue(&self) -> bool {
        self.is_loading_more_queue
    }

    pub fn cases(&self) -> &HashMap<i64, RiseStoredReportCase> {
        &self.cases
    }

    pub fn case_states(&self) -> &HashMap<i64, ModerationLoadState> {
        &self.case_states
    }

    pub fn review_filter(&self) -> AppealReviewFilter {
        self.review_filter
    }

    pub fn review(&self) -> &[AppealItemDTO] {
        &self.review
    }

    pub fn review_state(&self) -> &ModerationLoadState {
        &self.review_state
    }

    pub fn review_has_more(&self) -> bool {
        self.review_has_more
    }

    pub fn review_cursor(&self) -> Option<i64> {
        self.review_cursor
    }

    pub fn review_total(&self) -> i64 {
        self.review_total
    }

    pub fn is_loading_more_review(&self) -> bool {
        self.is_loading_more_review
    }

    pub fn appeal_cases(&self) -> &HashMap<i64, RiseStoredAppealCase> {
        &self.appeal_cases
    }

    pub fn appeal_case_states(&self) -> &HashMap<i64, ModerationLoadState> {
        &self.appeal_case_states
    }

    pub fn resolving_reports(&self) -> &HashSet<i64> {
        &self.resolving_reports
    }

    pub fn noting_reports(&self) -> &HashSet<i64> {
        &self.noting_reports
    }

    pub fn reopening_reports(&self) -> &HashSet<i64> {
        &self.reopening_reports
    }

    pub fn resolving_appeals(&self) -> &HashSet<i64> {
        &self.resolving_appeals
    }

    pub fn is_sanctioning(&self) -> bool {
        self.is_sanctioning
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn activate(&mut self, account_id: &str, account_epoch: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) {
            self.clear_all();
        }
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.is_active = true;
        self.is_stale = false;
        cx.notify();
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.is_active = false;
        self.session_id = None;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.clear_all();
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.is_active = false;
        cx.notify();
    }

    pub fn mark_stale_after_reconnect(&mut self, cx: &mut Context<Self>) {
        self.is_stale = true;
        cx.notify();
    }

    pub fn clear_stale(&mut self, cx: &mut Context<Self>) {
        self.is_stale = false;
        cx.notify();
    }

    fn clear_all(&mut self) {
        self.my_reports_filter = None;
        self.my_reports = Vec::new();
        self.my_reports_state = ModerationLoadState::Idle;
        self.my_reports_load_more_error = None;
        self.my_reports_has_more = false;
        self.my_reports_cursor = None;
        self.my_reports_total = 0;
        self.is_loading_more_my_reports = false;
        self.my_blocks = Vec::new();
        self.my_warnings = RiseStoredWarningState::EMPTY;
        self.my_history = Vec::new();
        self.my_auto_standings = Vec::new();
        self.my_blocks_state = ModerationLoadState::Idle;
        self.my_appeals_filter = AppealStatusFilter::All;
        self.my_appeals = Vec::new();
        self.my_pending_appeals = HashMap::new();
        self.my_appeals_state = ModerationLoadState::Idle;
        self.my_appeals_has_more = false;
        self.my_appeals_cursor = None;
        self.my_appeals_total = 0;
        self.is_loading_more_my_appeals = false;
        self.queue_filter = ModerationQueueFilter::default();
        self.queue = Vec::new();
        self.queue_state = ModerationLoadState::Idle;
        self.queue_has_more = false;
        self.queue_cursor = None;
        self.queue_total = 0;
        self.is_loading_more_queue = false;
        self.cases = HashMap::new();
        self.case_states = HashMap::new();
        self.review_filter = AppealReviewFilter::Pending;
        self.review = Vec::new();
        self.review_state = ModerationLoadState::Idle;
        self.review_has_more = false;
        self.review_cursor = None;
        self.review_total = 0;
        self.is_loading_more_review = false;
        self.appeal_cases = HashMap::new();
        self.appeal_case_states = HashMap::new();
        self.resolving_reports = HashSet::new();
        self.noting_reports = HashSet::new();
        self.reopening_reports = HashSet::new();
        self.resolving_appeals = HashSet::new();
        self.is_sanctioning = false;
    }

    pub fn set_my_reports_filter(&mut self, status: Option<ReportStatus>, cx: &mut Context<Self>) {
        self.my_reports_filter = status;
        self.my_reports = Vec::new();
        self.my_reports_cursor = None;
        self.my_reports_has_more = false;
        self.my_reports_total = 0;
        self.my_reports_load_more_error = None;
        cx.notify();
    }

    pub fn set_my_reports_state(&mut self, state: ModerationLoadState, cx: &mut Context<Self>) {
        self.my_reports_state = state;
        cx.notify();
    }

    pub fn set_loading_more_my_reports(&mut self, value: bool, cx: &mut Context<Self>) {
        self.is_loading_more_my_reports = value;
        if value {
            self.my_reports_load_more_error = None;
        }
        cx.notify();
    }

    pub fn fail_more_my_reports(&mut self, message: String, cx: &mut Context<Self>) {
        if !self.is_loading_more_my_reports {
            return;
        }
        self.is_loading_more_my_reports = false;
        self.my_reports_load_more_error = Some(message);
        cx.notify();
    }

    pub fn apply_my_reports(&mut self, page: ReportsPageDTO, append: bool, cx: &mut Context<Self>) {
        if append {
            let mut seen: HashSet<i64> = self.my_reports.iter().map(|row| row.id).collect();
            for row in page.reports {
                if seen.insert(row.id) {
                    self.my_reports.push(row);
                }
            }
        } else {
            self.my_reports = page.reports;
        }
        self.my_reports_has_more = page.is_have_more_bot_or_top;
        self.my_reports_cursor = page.relative_id.or_else(|| self.my_reports.last().map(|row| row.id));
        self.my_reports_total = page.total.max(Self::count(self.my_reports.len()));
        self.my_reports_state = ModerationLoadState::Loaded;
        self.is_loading_more_my_reports = false;
        self.my_reports_load_more_error = None;
        cx.notify();
    }

    pub fn fail_my_reports(&mut self, message: String, cx: &mut Context<Self>) {
        self.is_loading_more_my_reports = false;
        if self.my_reports.is_empty() {
            self.my_reports_state = ModerationLoadState::Failed(message);
        } else {
            self.my_reports_state = ModerationLoadState::Loaded;
            self.last_error = Some(message);
        }
        cx.notify();
    }

    pub fn prepend_my_report(&mut self, report: ReportRowDTO, cx: &mut Context<Self>) {
        if self.my_reports_state != ModerationLoadState::Loaded {
            return;
        }
        let matches_filter = self.my_reports_filter.is_none_or(|filter| filter.wire() == report.status);
        if !matches_filter || self.my_reports.iter().any(|row| row.id == report.id) {
            return;
        }
        self.my_reports.insert(0, report);
        self.my_reports_total += 1;
        cx.notify();
    }

    pub fn seed_blocks(&mut self, page: RiseStoredMyBlocks, source: RiseModerationCacheSource, cx: &mut Context<Self>) {
        if self.my_blocks_state != ModerationLoadState::Idle {
            return;
        }
        self.my_blocks = page.blocks;
        self.my_warnings = page.warnings;
        self.my_history = page.history;
        self.my_auto_standings = page.auto_standings;
        if source != RiseModerationCacheSource::Miss {
            self.my_blocks_state = ModerationLoadState::Loaded;
        }
        cx.notify();
    }

    pub fn set_blocks_state(&mut self, state: ModerationLoadState, cx: &mut Context<Self>) {
        self.my_blocks_state = state;
        cx.notify();
    }

    pub fn apply_blocks(&mut self, page: RiseStoredMyBlocks, cx: &mut Context<Self>) {
        self.my_blocks = page.blocks;
        self.my_warnings = page.warnings;
        self.my_history = page.history;
        self.my_auto_standings = page.auto_standings;
        self.my_blocks_state = ModerationLoadState::Loaded;
        cx.notify();
    }

    pub fn fail_blocks(&mut self, message: String, cx: &mut Context<Self>) {
        if self.my_blocks.is_empty() {
            self.my_blocks_state = ModerationLoadState::Failed(message);
        } else {
            self.my_blocks_state = ModerationLoadState::Loaded;
            self.last_error = Some(message);
        }
        cx.notify();
    }

    pub fn mark_block_appealed(&mut self, block_id: &str, appealed: bool, cx: &mut Context<Self>) {
        self.my_blocks = self
            .my_blocks
            .iter()
            .map(|block| if block.id() == block_id { block.with_can_appeal(!appealed) } else { block.clone() })
            .collect();
        cx.notify();
    }

    /// true if this sanction has a pending appeal among the ones seen so far, whatever filter the tab shows
    pub fn has_pending_appeal(&self, block: &RiseStoredUserBlock) -> bool {
        self.my_pending_appeals.values().any(|appeal| appeal.concerns(block))
    }

    /// the full block on the caller if one is in force, what the blocked page shows
    pub fn active_full_block(&self) -> Option<RiseStoredUserBlock> {
        self.active_blocks()
            .into_iter()
            .find(|block| block.resolved_type() == UserBlockType::Full)
    }

    /// sanctions still in force
    pub fn active_blocks(&self) -> Vec<RiseStoredUserBlock> {
        let now = Date::now().ms();
        self.my_blocks
            .iter()
            .filter(|block| block.block_end_timestamp == 0 || block.block_end_timestamp > now)
            .cloned()
            .collect()
    }

    pub fn active_auto_mutes(&self) -> Vec<RiseStoredAutoStanding> {
        let now = Date::now().ms();
        self.my_auto_standings
            .iter()
            .filter(|standing| standing.is_muted(now))
            .cloned()
            .collect()
    }

    pub fn visible_auto_standings(&self) -> Vec<RiseStoredAutoStanding> {
        let now = Date::now().ms();
        self.my_auto_standings
            .iter()
            .filter(|standing| standing.strikes > 0 || standing.is_muted(now))
            .cloned()
            .collect()
    }

    pub fn set_my_appeals_filter(&mut self, filter: AppealStatusFilter, cx: &mut Context<Self>) {
        self.my_appeals_filter = filter;
        self.my_appeals = Vec::new();
        self.my_appeals_cursor = None;
        self.my_appeals_has_more = false;
        self.my_appeals_total = 0;
        cx.notify();
    }

    pub fn set_my_appeals_state(&mut self, state: ModerationLoadState, cx: &mut Context<Self>) {
        self.my_appeals_state = state;
        cx.notify();
    }

    pub fn set_loading_more_my_appeals(&mut self, value: bool, cx: &mut Context<Self>) {
        self.is_loading_more_my_appeals = value;
        cx.notify();
    }

    pub fn apply_my_appeals(&mut self, page: AppealsPageDTO, append: bool, cx: &mut Context<Self>) {
        if !append
            && (self.my_appeals_filter == AppealStatusFilter::All || self.my_appeals_filter == AppealStatusFilter::Pending)
        {
            self.my_pending_appeals = HashMap::new();
        }
        for row in &page.appeals {
            self.track_pending(&row.appeal);
        }
        if append {
            let mut seen: HashSet<i64> = self.my_appeals.iter().map(AppealItemDTO::id).collect();
            for row in page.appeals {
                if seen.insert(row.id()) {
                    self.my_appeals.push(row);
                }
            }
        } else {
            self.my_appeals = page.appeals;
        }
        self.my_appeals_has_more = page.is_have_more_bot_or_top;
        self.my_appeals_cursor = page.relative_id.or_else(|| self.my_appeals.last().map(AppealItemDTO::id));
        self.my_appeals_total = page.total.max(Self::count(self.my_appeals.len()));
        self.my_appeals_state = ModerationLoadState::Loaded;
        self.is_loading_more_my_appeals = false;
        cx.notify();
    }

    pub fn fail_my_appeals(&mut self, message: String, cx: &mut Context<Self>) {
        self.is_loading_more_my_appeals = false;
        if self.my_appeals.is_empty() {
            self.my_appeals_state = ModerationLoadState::Failed(message);
        } else {
            self.my_appeals_state = ModerationLoadState::Loaded;
            self.last_error = Some(message);
        }
        cx.notify();
    }

    pub fn set_queue_filter(&mut self, filter: ModerationQueueFilter, cx: &mut Context<Self>) {
        self.queue_filter = filter;
        self.queue = Vec::new();
        self.queue_cursor = None;
        self.queue_has_more = false;
        self.queue_total = 0;
        cx.notify();
    }

    pub fn set_queue_state(&mut self, state: ModerationLoadState, cx: &mut Context<Self>) {
        self.queue_state = state;
        cx.notify();
    }

    pub fn set_loading_more_queue(&mut self, value: bool, cx: &mut Context<Self>) {
        self.is_loading_more_queue = value;
        cx.notify();
    }

    pub fn apply_queue(&mut self, page: ReportsPageDTO, append: bool, cx: &mut Context<Self>) {
        self.apply_queue_page(page, append);
        cx.notify();
    }

    fn apply_queue_page(&mut self, page: ReportsPageDTO, append: bool) {
        if append {
            let mut seen: HashSet<i64> = self.queue.iter().map(|row| row.id).collect();
            let mut targets: HashSet<String> = self.queue.iter().filter_map(Self::queue_target_key).collect();
            for row in page.reports {
                let fresh_target = Self::queue_target_key(&row).is_none_or(|key| targets.insert(key));
                if seen.insert(row.id) && fresh_target {
                    self.queue.push(row);
                }
            }
        } else {
            self.queue = page.reports;
        }
        self.queue_has_more = page.is_have_more_bot_or_top;
        self.queue_cursor = page.relative_id.or_else(|| self.queue.last().map(|row| row.id));
        self.queue_total = page.total.max(Self::count(self.queue.len()));
        self.queue_state = ModerationLoadState::Loaded;
        self.is_loading_more_queue = false;
    }

    pub fn apply_queue_head(&mut self, page: ReportsPageDTO, cx: &mut Context<Self>) {
        if page.reports.is_empty() || !page.is_have_more_bot_or_top || self.queue.len() <= page.reports.len() {
            self.apply_queue_page(page, false);
            cx.notify();
            return;
        }
        let fresh_ids: HashSet<i64> = page.reports.iter().map(|row| row.id).collect();
        let fresh_targets: HashSet<String> = page.reports.iter().filter_map(Self::queue_target_key).collect();
        let boundary = page
            .reports
            .iter()
            .rev()
            .find_map(|row| self.queue.iter().position(|existing| existing.id == row.id));
        let Some(boundary) = boundary else {
            self.apply_queue_page(page, false);
            cx.notify();
            return;
        };
        let tail: Vec<ReportRowDTO> = self.queue[(boundary + 1)..]
            .iter()
            .filter(|row| {
                !fresh_ids.contains(&row.id)
                    && Self::queue_target_key(row).is_none_or(|key| !fresh_targets.contains(&key))
            })
            .cloned()
            .collect();
        if tail.is_empty() {
            self.apply_queue_page(page, false);
            cx.notify();
            return;
        }
        let total = page.total;
        let mut queue = page.reports;
        queue.extend(tail);
        self.queue = queue;
        self.queue_total = total.max(Self::count(self.queue.len()));
        self.queue_state = ModerationLoadState::Loaded;
        self.is_loading_more_queue = false;
        cx.notify();
    }

    pub fn fail_queue(&mut self, message: String, cx: &mut Context<Self>) {
        self.is_loading_more_queue = false;
        if self.queue.is_empty() {
            self.queue_state = ModerationLoadState::Failed(message);
        } else {
            self.queue_state = ModerationLoadState::Loaded;
            self.last_error = Some(message);
        }
        cx.notify();
    }

    /// a report changed status
    /// the row leaves a queue filtered on the old status and every other copy is replaced
    pub fn replace_report(&mut self, report: ReportRowDTO, cx: &mut Context<Self>) {
        if let Some(index) = self.queue.iter().position(|row| row.id == report.id) {
            if self.queue_filter.status.wire() == report.status {
                self.queue[index] = report.clone();
            } else {
                self.queue.remove(index);
                self.queue_total = (self.queue_total - 1).max(0);
                if self.queue_cursor == Some(report.id) {
                    self.queue_cursor = self.queue.last().map(|row| row.id);
                }
            }
        }
        if let Some(index) = self.my_reports.iter().position(|row| row.id == report.id) {
            self.my_reports[index] = report.clone();
        }
        if let Some(existing) = self.cases.get(&report.id) {
            let updated = existing.with_report(report.clone());
            self.cases.insert(report.id, updated);
        }
        cx.notify();
    }

    pub fn apply_my_report_resolution(&mut self, resolution: &ReportResolutionEvent, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.my_reports.iter().position(|row| row.id == resolution.report_id) else {
            return false;
        };
        let report = self.my_reports[index].with_resolution(resolution);
        if self.my_reports_filter.is_none_or(|filter| filter.wire() == report.status) {
            self.my_reports[index] = report;
        } else {
            self.my_reports.remove(index);
            self.my_reports_total = (self.my_reports_total - 1).max(0);
            if self.my_reports_cursor == Some(report.id) {
                self.my_reports_cursor = self.my_reports.last().map(|row| row.id);
            }
        }
        cx.notify();
        true
    }

    pub fn case_state(&self, report_id: i64) -> ModerationLoadState {
        self.case_states.get(&report_id).cloned().unwrap_or_default()
    }

    pub fn set_case_state(&mut self, report_id: i64, state: ModerationLoadState, cx: &mut Context<Self>) {
        self.case_states.insert(report_id, state);
        cx.notify();
    }

    pub fn apply_case(&mut self, value: RiseStoredReportCase, cx: &mut Context<Self>) {
        let report_id = value.report.id;
        self.cases.insert(report_id, value);
        self.case_states.insert(report_id, ModerationLoadState::Loaded);
        cx.notify();
    }

    pub fn fail_case(&mut self, report_id: i64, message: String, cx: &mut Context<Self>) {
        if self.cases.contains_key(&report_id) {
            self.case_states.insert(report_id, ModerationLoadState::Loaded);
            self.last_error = Some(message);
        } else {
            self.case_states.insert(report_id, ModerationLoadState::Failed(message));
        }
        cx.notify();
    }

    pub fn append_note(&mut self, note: RiseStoredModerationNote, cx: &mut Context<Self>) {
        let Some(existing) = self.cases.get(&note.report_id) else {
            return;
        };
        if existing.notes.iter().any(|current| current.id == note.id) {
            return;
        }
        let report_id = note.report_id;
        let mut notes = existing.notes.clone();
        notes.push(note);
        let updated = existing.with_notes(notes);
        self.cases.insert(report_id, updated);
        cx.notify();
    }

    pub fn set_resolving(&mut self, report_id: i64, value: bool, cx: &mut Context<Self>) {
        Self::toggle(&mut self.resolving_reports, report_id, value);
        cx.notify();
    }

    pub fn set_noting(&mut self, report_id: i64, value: bool, cx: &mut Context<Self>) {
        Self::toggle(&mut self.noting_reports, report_id, value);
        cx.notify();
    }

    pub fn set_reopening(&mut self, report_id: i64, value: bool, cx: &mut Context<Self>) {
        Self::toggle(&mut self.reopening_reports, report_id, value);
        cx.notify();
    }

    pub fn set_sanctioning(&mut self, value: bool, cx: &mut Context<Self>) {
        self.is_sanctioning = value;
        cx.notify();
    }

    pub fn set_review_filter(&mut self, filter: AppealReviewFilter, cx: &mut Context<Self>) {
        self.review_filter = filter;
        self.review = Vec::new();
        self.review_cursor = None;
        self.review_has_more = false;
        self.review_total = 0;
        cx.notify();
    }

    pub fn set_review_state(&mut self, state: ModerationLoadState, cx: &mut Context<Self>) {
        self.review_state = state;
        cx.notify();
    }

    pub fn set_loading_more_review(&mut self, value: bool, cx: &mut Context<Self>) {
        self.is_loading_more_review = value;
        cx.notify();
    }

    pub fn apply_review(&mut self, page: AppealsPageDTO, append: bool, cx: &mut Context<Self>) {
        if append {
            let mut seen: HashSet<i64> = self.review.iter().map(AppealItemDTO::id).collect();
            for row in page.appeals {
                if seen.insert(row.id()) {
                    self.review.push(row);
                }
            }
        } else {
            self.review = page.appeals;
        }
        self.review_has_more = page.is_have_more_bot_or_top;
        self.review_cursor = page.relative_id.or_else(|| self.review.last().map(AppealItemDTO::id));
        self.review_total = page.total.max(Self::count(self.review.len()));
        self.review_state = ModerationLoadState::Loaded;
        self.is_loading_more_review = false;
        cx.notify();
    }

    pub fn fail_review(&mut self, message: String, cx: &mut Context<Self>) {
        self.is_loading_more_review = false;
        if self.review.is_empty() {
            self.review_state = ModerationLoadState::Failed(message);
        } else {
            self.review_state = ModerationLoadState::Loaded;
            self.last_error = Some(message);
        }
        cx.notify();
    }

    pub fn replace_appeal(&mut self, appeal: AppealDTO, cx: &mut Context<Self>) {
        if let Some(index) = self.review.iter().position(|row| row.id() == appeal.id) {
            if self.review_filter.raw_value() == appeal.status {
                let updated = AppealItemDTO::from_item(&self.review[index], appeal.clone());
                self.review[index] = updated;
            } else {
                self.review.remove(index);
                self.review_total = (self.review_total - 1).max(0);
            }
        }
        if let Some(index) = self.my_appeals.iter().position(|row| row.id() == appeal.id) {
            let updated = AppealItemDTO::from_item(&self.my_appeals[index], appeal.clone());
            self.my_appeals[index] = updated;
        }
        self.track_pending(&appeal);
        if let Some(existing) = self.appeal_cases.get(&appeal.id) {
            let updated = existing.with_appeal(appeal.clone());
            self.appeal_cases.insert(appeal.id, updated);
        }
        cx.notify();
    }

    pub fn appeal_case_state(&self, appeal_id: i64) -> ModerationLoadState {
        self.appeal_case_states.get(&appeal_id).cloned().unwrap_or_default()
    }

    pub fn set_appeal_case_state(&mut self, appeal_id: i64, state: ModerationLoadState, cx: &mut Context<Self>) {
        self.appeal_case_states.insert(appeal_id, state);
        cx.notify();
    }

    pub fn apply_appeal_case(&mut self, value: RiseStoredAppealCase, cx: &mut Context<Self>) {
        let appeal_id = value.appeal.id;
        if let Some(report_case) = value.report_case.clone() {
            let report_id = report_case.report.id;
            self.cases.insert(report_id, report_case);
            self.case_states.insert(report_id, ModerationLoadState::Loaded);
        }
        self.appeal_cases.insert(appeal_id, value);
        self.appeal_case_states.insert(appeal_id, ModerationLoadState::Loaded);
        cx.notify();
    }

    pub fn fail_appeal_case(&mut self, appeal_id: i64, message: String, cx: &mut Context<Self>) {
        if self.appeal_cases.contains_key(&appeal_id) {
            self.appeal_case_states.insert(appeal_id, ModerationLoadState::Loaded);
            self.last_error = Some(message);
        } else {
            self.appeal_case_states.insert(appeal_id, ModerationLoadState::Failed(message));
        }
        cx.notify();
    }

    pub fn set_resolving_appeal(&mut self, appeal_id: i64, value: bool, cx: &mut Context<Self>) {
        Self::toggle(&mut self.resolving_appeals, appeal_id, value);
        cx.notify();
    }

    pub fn prepend_my_appeal(&mut self, appeal: AppealDTO, block: Option<&RiseStoredUserBlock>, cx: &mut Context<Self>) {
        self.track_pending(&appeal);
        if self.my_appeals_state != ModerationLoadState::Loaded {
            cx.notify();
            return;
        }
        let matches_filter =
            self.my_appeals_filter == AppealStatusFilter::All || self.my_appeals_filter.raw_value() == appeal.status;
        if matches_filter && !self.my_appeals.iter().any(|row| row.id() == appeal.id) {
            self.my_appeals.insert(0, AppealItemDTO::from_block(appeal, block));
            self.my_appeals_total += 1;
        }
        cx.notify();
    }

    fn track_pending(&mut self, appeal: &AppealDTO) {
        if appeal.resolved_status() == AppealStatus::Pending {
            self.my_pending_appeals.insert(appeal.id, appeal.clone());
        } else {
            self.my_pending_appeals.remove(&appeal.id);
        }
    }

    pub fn consume_last_error(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let last_error = self.last_error.take();
        if last_error.is_some() {
            cx.notify();
        }
        last_error
    }

    fn toggle(set: &mut HashSet<i64>, id: i64, value: bool) {
        if value {
            set.insert(id);
        } else {
            set.remove(&id);
        }
    }

    /// object a queue case groups its reports by, in the servers precedence order
    fn queue_target_key(row: &ReportRowDTO) -> Option<String> {
        if let Some(id) = row.post_id {
            return Some(format!("post:{id}"));
        }
        if let Some(id) = row.comment_id {
            return Some(format!("comment:{id}"));
        }
        if let Some(id) = row.message_id.as_deref() {
            return Some(format!("message:{}:{id}", row.chat_id.as_deref().unwrap_or_default()));
        }
        if let Some(id) = row.chat_id.as_deref() {
            return Some(format!("chat:{id}"));
        }
        if let Some(id) = row.user_id.as_deref() {
            return Some(format!("user:{id}"));
        }
        if let Some(id) = row.vacancy_id {
            return Some(format!("vacancy:{id}"));
        }
        if let Some(id) = row.track_id.as_deref() {
            return Some(format!("track:{id}"));
        }
        if let Some(id) = row.artist_id.as_deref() {
            return Some(format!("artist:{id}"));
        }
        if let Some(id) = row.story_id {
            return Some(format!("story:{id}"));
        }
        row.bot_id.as_deref().map(|id| format!("bot:{id}"))
    }

    fn count(len: usize) -> i64 {
        i64::try_from(len).unwrap_or(i64::MAX)
    }
}

impl AppealItemDTO {
    /// same context with a newer appeal inside
    pub fn from_item(item: &AppealItemDTO, appeal: AppealDTO) -> Self {
        Self::new(
            appeal,
            item.report_type.clone(),
            item.report_reason.clone(),
            item.snapshot_title.clone(),
            item.snapshot_text.clone(),
            item.original_decision.clone(),
            item.block_type.clone(),
            item.resolved_by_id.clone(),
            item.appellant.clone(),
        )
    }

    pub fn from_block(appeal: AppealDTO, block: Option<&RiseStoredUserBlock>) -> Self {
        Self::new(
            appeal,
            block.and_then(|block| block.report_type.clone()),
            block.and_then(|block| block.report_reason.clone()),
            block.and_then(|block| block.snapshot_title.clone()),
            None,
            block.and_then(|block| block.reason.clone()),
            block.map(|block| block.block_type.clone()),
            None,
            None,
        )
    }

    /// plain constructor for the appeal context
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        appeal: AppealDTO,
        report_type: Option<String>,
        report_reason: Option<String>,
        snapshot_title: Option<String>,
        snapshot_text: Option<String>,
        original_decision: Option<String>,
        block_type: Option<String>,
        resolved_by_id: Option<String>,
        appellant: Option<ModerationUserDTO>,
    ) -> Self {
        Self {
            appeal,
            report_type,
            report_reason,
            snapshot_title,
            snapshot_text,
            original_decision,
            block_type,
            resolved_by_id,
            appellant,
        }
    }
}
