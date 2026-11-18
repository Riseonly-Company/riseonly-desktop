use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_canon::{RiseBugReportPlatform, RiseBugReportSurface, RiseFeedbackKind, RiseGuideSignal};
use rise_catalog::DIAGNOSTICS_FEEDBACK_KINDS_DEFAULT;
use rise_core::foundation::foundation_compare::localized_case_insensitive_contains;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::modules::diagnostics::engine::bug_report_draft_persistence::{
    BugReportDraftPersistence, BugReportPersistedDraft, BugReportPersistedState,
};
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::{
    BugReportAttachment, BugReportAttachmentKind, BugReportUploadedAttachment, RiseBugReportUpload,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_types::{
    BugReportDraft, BugReportDraftKind, BugReportListSlice, BugReportListState, BugReportRoute, BugReportSheetPage,
    BugReportSubmissionState,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_wire_types::{
    BugReportAttachmentURL, BugReportRecord,
};

static MESSAGE_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("report_submit_diagnostic_report", "/message"));

static COMPONENT_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("report_submit_diagnostic_report", "/component"));

type DraftsBeforeLesson = (
    HashMap<BugReportDraftKind, BugReportDraft>,
    BugReportDraftKind,
    BugReportSheetPage,
    HashSet<BugReportDraftKind>,
);

pub struct BugReportServicesStore {
    route: BugReportRoute,
    state: BugReportSubmissionState,
    page: BugReportSheetPage,
    active_kind: BugReportDraftKind,
    page_query: String,
    is_confirming_discard: bool,
    drafts: HashMap<BugReportDraftKind, BugReportDraft>,
    editing_draft: Option<BugReportDraft>,
    viewed_record: Option<BugReportRecord>,
    opened_status_comment: Option<String>,
    message_was_valid_kinds: HashSet<BugReportDraftKind>,
    persistence_loaded: bool,
    drafts_before_lesson: Option<DraftsBeforeLesson>,
    my_reports: HashMap<BugReportDraftKind, BugReportListSlice>,
    my_reports_tab: i64,
    deleting_report_id: Option<String>,
    loaded_draft: Option<BugReportDraft>,
}

impl BugReportServicesStore {
    pub const MY_REPORTS_PAGE_SIZE: i64 = 30;

    pub const TAB_ORDER: [BugReportDraftKind; 2] = [BugReportDraftKind::Bug, BugReportDraftKind::Feedback];

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            route: BugReportRoute::Idle,
            state: BugReportSubmissionState::Editing,
            page: BugReportSheetPage::Root,
            active_kind: BugReportDraftKind::Bug,
            page_query: String::new(),
            is_confirming_discard: false,
            drafts: Self::fresh_drafts(),
            editing_draft: None,
            viewed_record: None,
            opened_status_comment: None,
            message_was_valid_kinds: HashSet::new(),
            persistence_loaded: false,
            drafts_before_lesson: None,
            my_reports: Self::fresh_my_reports(),
            my_reports_tab: 0,
            deleting_report_id: None,
            loaded_draft: None,
        }
    }

    fn fresh_draft(kind: BugReportDraftKind) -> BugReportDraft {
        BugReportDraft::new(Uuid::new_v4().uuid_string(), kind)
    }

    fn fresh_drafts() -> HashMap<BugReportDraftKind, BugReportDraft> {
        HashMap::from([
            (BugReportDraftKind::Bug, Self::fresh_draft(BugReportDraftKind::Bug)),
            (BugReportDraftKind::Feedback, Self::fresh_draft(BugReportDraftKind::Feedback)),
        ])
    }

    fn fresh_my_reports() -> HashMap<BugReportDraftKind, BugReportListSlice> {
        HashMap::from([
            (BugReportDraftKind::Bug, BugReportListSlice::default()),
            (BugReportDraftKind::Feedback, BugReportListSlice::default()),
        ])
    }

    pub fn route(&self) -> BugReportRoute {
        self.route
    }

    pub fn state(&self) -> &BugReportSubmissionState {
        &self.state
    }

    pub fn page(&self) -> BugReportSheetPage {
        self.page
    }

    pub fn active_kind(&self) -> BugReportDraftKind {
        self.active_kind
    }

    pub fn page_query(&self) -> &str {
        &self.page_query
    }

    pub fn is_confirming_discard(&self) -> bool {
        self.is_confirming_discard
    }

    pub fn my_reports(&self) -> &HashMap<BugReportDraftKind, BugReportListSlice> {
        &self.my_reports
    }

    pub fn my_reports_tab(&self) -> i64 {
        self.my_reports_tab
    }

    pub fn deleting_report_id(&self) -> Option<&str> {
        self.deleting_report_id.as_deref()
    }

    pub fn loaded_draft(&self) -> Option<&BugReportDraft> {
        self.loaded_draft.as_ref()
    }

    pub fn message_rule() -> RiseFieldRule {
        *MESSAGE_RULE
    }

    pub fn component_rule() -> RiseFieldRule {
        *COMPONENT_RULE
    }

    pub fn draft(&self) -> BugReportDraft {
        self.editing_draft
            .clone()
            .or_else(|| self.drafts.get(&self.active_kind).cloned())
            .unwrap_or_else(|| Self::fresh_draft(self.active_kind))
    }

    pub fn draft_for(&self, kind: BugReportDraftKind) -> BugReportDraft {
        self.drafts.get(&kind).cloned().unwrap_or_else(|| Self::fresh_draft(kind))
    }

    pub fn mounts_sheet(&self) -> bool {
        matches!(
            self.route,
            BugReportRoute::Sheet | BugReportRoute::ClosingSheetForPicker | BugReportRoute::ClosingSheetForDismissal
        )
    }

    pub fn is_sheet_presented(&self) -> bool {
        self.route == BugReportRoute::Sheet
    }

    pub fn mounts_picker(&self) -> bool {
        self.route == BugReportRoute::Picker || self.route == BugReportRoute::ClosingPickerForSheet
    }

    pub fn is_picker_presented(&self) -> bool {
        self.route == BugReportRoute::Picker
    }

    pub fn has_interactive_surface(&self) -> bool {
        self.route != BugReportRoute::Idle
    }

    pub fn sheet_root_page(&self) -> BugReportSheetPage {
        if self.is_editing() || self.is_follow_up() {
            self.active_kind.page()
        } else {
            BugReportSheetPage::Root
        }
    }

    pub fn restored_page_stack(&self) -> Vec<String> {
        let mut stack = self.page.stack();
        if self.page == BugReportSheetPage::Areas {
            stack.push(self.active_kind.page());
            stack.push(BugReportSheetPage::Areas);
        }
        if self.sheet_root_page() != BugReportSheetPage::Root {
            stack.retain(|page| *page != BugReportSheetPage::Root);
        }
        stack.into_iter().map(|page| page.raw_value().to_owned()).collect()
    }

    fn message_was_valid(&self) -> bool {
        self.editing_draft.is_some() || self.message_was_valid_kinds.contains(&self.active_kind)
    }

    pub fn message_error(&self) -> Option<String> {
        if !self.message_was_valid() {
            return None;
        }
        let draft = self.draft();
        let raw = trimming_whitespaces_and_newlines(&draft.message);
        if raw.is_empty() {
            return None;
        }
        Self::message_rule().error_text(raw)
    }

    pub fn component_error(&self) -> Option<String> {
        let draft = self.draft();
        if draft.kind != BugReportDraftKind::Bug {
            return None;
        }
        let raw = trimming_whitespaces_and_newlines(&draft.component);
        if raw.is_empty() {
            return None;
        }
        Self::component_rule().error_text(raw)
    }

    pub fn message_remaining(&self) -> Option<usize> {
        Self::message_rule().remaining(&self.draft().message)
    }

    pub fn message_minimum() -> usize {
        Self::message_rule().minimum().unwrap_or(0)
    }

    pub fn message_shortfall(&self) -> Option<usize> {
        Self::message_rule().shortfall(&self.draft().message)
    }

    pub fn message_length(&self) -> usize {
        Self::message_minimum().saturating_sub(self.message_shortfall().unwrap_or(0))
    }

    pub fn is_message_valid(&self) -> bool {
        Self::message_rule().is_satisfied(trimming_whitespaces_and_newlines(&self.draft().message))
    }

    pub fn can_submit(&self) -> bool {
        if self.state != BugReportSubmissionState::Editing || self.component_error().is_some() {
            return false;
        }
        self.is_message_valid()
    }

    pub fn viewed_record(&self) -> Option<&BugReportRecord> {
        self.viewed_record.as_ref()
    }

    pub fn opened_status_comment(&self) -> Option<&str> {
        self.opened_status_comment.as_deref()
    }

    pub fn is_read_only(&self) -> bool {
        self.viewed_record.is_some()
    }

    pub fn can_edit_draft(&self) -> bool {
        if self.is_read_only() {
            return false;
        }
        match self.state {
            BugReportSubmissionState::Editing | BugReportSubmissionState::Failed { .. } => true,
            BugReportSubmissionState::Sending { .. } | BugReportSubmissionState::Sent => false,
        }
    }

    pub fn can_send(&self) -> bool {
        if self.is_read_only() || self.component_error().is_some() {
            return false;
        }
        match self.state {
            BugReportSubmissionState::Editing | BugReportSubmissionState::Failed { .. } => self.is_message_valid(),
            BugReportSubmissionState::Sending { .. } | BugReportSubmissionState::Sent => false,
        }
    }

    pub fn screenshot_slots_left(&self) -> i64 {
        0_i64.max(RiseBugReportUpload::maximum_screenshots() - self.draft().screenshots.len() as i64)
    }

    pub fn can_attach_more(&self) -> bool {
        self.screenshot_slots_left() > 0 || self.draft().video.is_none()
    }

    pub fn page_slots_left(&self) -> i64 {
        0_i64.max(RiseBugReportUpload::maximum_pages() - self.draft().pages.len() as i64)
    }

    pub fn is_editing(&self) -> bool {
        self.editing_draft.as_ref().is_some_and(|draft| draft.editing_report_id.is_some())
    }

    pub fn is_follow_up(&self) -> bool {
        self.editing_draft.as_ref().is_some_and(|draft| draft.parent_report_id.is_some())
    }

    pub fn has_content(&self) -> bool {
        if self.is_editing() {
            return Some(&self.draft()) != self.loaded_draft.as_ref();
        }
        Self::draft_has_content(&self.draft())
    }

    pub fn has_any_draft_content(&self) -> bool {
        self.drafts.values().any(Self::draft_has_content)
    }

    fn draft_has_content(draft: &BugReportDraft) -> bool {
        !trimming_whitespaces_and_newlines(&draft.message).is_empty()
            || !trimming_whitespaces_and_newlines(&draft.component).is_empty()
            || !draft.screenshots.is_empty()
            || draft.video.is_some()
            || draft.pages != draft.seeded_pages
            || draft.fix_platforms.len() != RiseBugReportPlatform::ALL.len()
    }

    pub fn visible_pages(&self) -> Vec<RiseBugReportSurface> {
        let query = trimming_whitespaces_and_newlines(&self.page_query);
        if query.is_empty() {
            return Self::ordered_surfaces().to_vec();
        }
        Self::ordered_surfaces()
            .iter()
            .copied()
            .filter(|surface| localized_case_insensitive_contains(&tr(surface.i18n_key()), query))
            .collect()
    }

    pub fn ordered_surfaces() -> &'static [RiseBugReportSurface] {
        &RiseBugReportSurface::ALL
    }

    pub fn ordered_feedback_kinds() -> &'static [RiseFeedbackKind] {
        &RiseFeedbackKind::ALL
    }

    pub fn apply_route(&mut self, route: BugReportRoute, cx: &mut Context<Self>) {
        self.route = route;
        self.publish_guide_facts(cx);
        cx.notify();
    }

    fn publish_guide_facts(&self, cx: &mut Context<Self>) {
        let is_open = self.route == BugReportRoute::Sheet;
        let is_bug_form = is_open && self.page == BugReportSheetPage::Bug;
        cx.defer(move |cx| {
            GuideFacts::set(RiseGuideSignal::BugReportOpen, is_open, cx);
            GuideFacts::set(RiseGuideSignal::BugReportBugForm, is_bug_form, cx);
        });
    }

    pub fn begin_practice_report(&mut self, cx: &mut Context<Self>) {
        self.activate_persistence(cx);
        if self.drafts_before_lesson.is_some() {
            return;
        }
        self.drafts_before_lesson = Some((
            std::mem::replace(&mut self.drafts, Self::fresh_drafts()),
            self.active_kind,
            self.page,
            std::mem::take(&mut self.message_was_valid_kinds),
        ));
        self.active_kind = BugReportDraftKind::Bug;
        self.page = BugReportSheetPage::Root;
        self.state = BugReportSubmissionState::Editing;
        self.publish_guide_facts(cx);
        cx.notify();
    }

    pub fn end_practice_report(&mut self, cx: &mut Context<Self>) {
        let Some((drafts, active_kind, page, valid)) = self.drafts_before_lesson.take() else {
            return;
        };
        self.drafts = drafts;
        self.active_kind = active_kind;
        self.page = page;
        self.message_was_valid_kinds = valid;
        self.state = BugReportSubmissionState::Editing;
        self.persist(cx);
        self.publish_guide_facts(cx);
        cx.notify();
    }

    pub fn apply_state(&mut self, state: BugReportSubmissionState, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }

    pub fn apply_page(&mut self, page: BugReportSheetPage, cx: &mut Context<Self>) {
        if let Some(kind) = page.form_kind()
            && self.editing_draft.is_none()
        {
            self.active_kind = kind;
        }
        if page != BugReportSheetPage::Areas {
            self.page_query = String::new();
        }
        cx.notify();
        if page == self.page {
            return;
        }
        self.page = page;
        self.persist(cx);
        self.publish_guide_facts(cx);
    }

    pub fn apply_page_query(&mut self, page_query: &str, cx: &mut Context<Self>) {
        if page_query == self.page_query {
            return;
        }
        self.page_query = page_query.to_owned();
        cx.notify();
    }

    pub fn apply_is_confirming_discard(&mut self, is_confirming_discard: bool, cx: &mut Context<Self>) {
        self.is_confirming_discard = is_confirming_discard;
        cx.notify();
    }

    pub fn mutate_draft(&mut self, transform: impl FnOnce(&mut BugReportDraft), cx: &mut Context<Self>) {
        if !self.can_edit_draft() {
            return;
        }
        let current = self.draft();
        let mut next = current.clone();
        transform(&mut next);
        if next == current {
            return;
        }
        if self.editing_draft.is_some() {
            self.editing_draft = Some(next);
        } else {
            let kind = next.kind;
            if !self.message_was_valid_kinds.contains(&kind)
                && Self::message_rule().is_satisfied(trimming_whitespaces_and_newlines(&next.message))
            {
                self.message_was_valid_kinds.insert(kind);
            }
            self.drafts.insert(kind, next);
            self.persist(cx);
        }
        cx.notify();
    }

    pub fn start_draft(
        &mut self,
        kind: BugReportDraftKind,
        surface: Option<RiseBugReportSurface>,
        cx: &mut Context<Self>,
    ) {
        let seeded: Vec<RiseBugReportSurface> = surface.into_iter().collect();
        self.drafts.insert(
            kind,
            BugReportDraft { pages: seeded.clone(), seeded_pages: seeded, ..Self::fresh_draft(kind) },
        );
        self.message_was_valid_kinds.remove(&kind);
        if self.editing_draft.is_none() && self.active_kind == kind {
            self.state = BugReportSubmissionState::Editing;
        }
        self.is_confirming_discard = false;
        self.persist(cx);
        cx.notify();
    }

    pub fn resume_drafts(&mut self, surface: Option<RiseBugReportSurface>, cx: &mut Context<Self>) {
        for kind in BugReportDraftKind::ALL_CASES {
            let mut next = self.draft_for(kind);
            if next.pages.is_empty()
                && let Some(surface) = surface
            {
                next.pages = vec![surface];
                next.seeded_pages = vec![surface];
                self.drafts.insert(kind, next);
            }
        }
        self.state = BugReportSubmissionState::Editing;
        self.page_query = String::new();
        self.is_confirming_discard = false;
        cx.notify();
    }

    pub fn finish_editing(&mut self, cx: &mut Context<Self>) {
        self.editing_draft = None;
        self.viewed_record = None;
        self.opened_status_comment = None;
        self.loaded_draft = None;
        self.state = BugReportSubmissionState::Editing;
        self.is_confirming_discard = false;
        cx.notify();
    }

    pub fn start_editing(&mut self, record: &BugReportRecord, cx: &mut Context<Self>) {
        let mut next = BugReportDraft {
            feedback_kind: record.feedback_kind.unwrap_or(DIAGNOSTICS_FEEDBACK_KINDS_DEFAULT),
            editing_report_id: Some(record.id.clone()),
            message: record.message.clone(),
            pages: record.pages.clone(),
            seeded_pages: record.pages.clone(),
            component: record.component.clone().unwrap_or_default(),
            fix_platforms: record.fix_platforms.clone(),
            ..BugReportDraft::new(Uuid::new_v4().uuid_string(), record.kind)
        };
        next.screenshots = record
            .screenshot_file_urls
            .iter()
            .zip(record.screenshot_file_ids.iter())
            .map(|(url, id)| {
                BugReportAttachment::new(
                    id.clone(),
                    BugReportAttachmentKind::Screenshot,
                    None,
                    id.clone(),
                    "image/jpeg".to_owned(),
                    None,
                    BugReportAttachmentURL::absolute(url).and_then(|absolute| Url::from_string(&absolute)),
                    false,
                    Some(BugReportUploadedAttachment {
                        file_url: url.clone(),
                        file_id: id.clone(),
                        duration_ms: None,
                    }),
                )
            })
            .collect();
        if let (Some(video_url), Some(video_id)) = (record.video_file_url.as_ref(), record.video_file_id.as_ref()) {
            next.video = Some(BugReportAttachment::new(
                video_id.clone(),
                BugReportAttachmentKind::Video,
                None,
                video_id.clone(),
                "video/mp4".to_owned(),
                record.video_duration_ms,
                None,
                false,
                Some(BugReportUploadedAttachment {
                    file_url: video_url.clone(),
                    file_id: video_id.clone(),
                    duration_ms: record.video_duration_ms,
                }),
            ));
        }
        self.editing_draft = Some(next.clone());
        self.loaded_draft = Some(next);
        self.opened_status_comment = record.status_comment.clone();
        self.active_kind = record.kind;
        self.page = record.kind.page();
        self.state = BugReportSubmissionState::Editing;
        self.page_query = String::new();
        self.is_confirming_discard = false;
        cx.notify();
    }

    pub fn start_viewing(&mut self, record: &BugReportRecord, cx: &mut Context<Self>) {
        self.start_editing(record, cx);
        self.viewed_record = Some(record.clone());
    }

    pub fn start_follow_up(&mut self, record: &BugReportRecord, cx: &mut Context<Self>) {
        self.viewed_record = None;
        self.opened_status_comment = None;
        self.editing_draft = Some(BugReportDraft {
            parent_report_id: Some(record.id.clone()),
            pages: record.pages.clone(),
            seeded_pages: record.pages.clone(),
            ..BugReportDraft::new(Uuid::new_v4().uuid_string(), BugReportDraftKind::Bug)
        });
        self.loaded_draft = None;
        self.active_kind = BugReportDraftKind::Bug;
        self.page = BugReportSheetPage::Bug;
        self.state = BugReportSubmissionState::Editing;
        self.page_query = String::new();
        self.is_confirming_discard = false;
        cx.notify();
    }

    pub fn activate_persistence(&mut self, cx: &mut Context<Self>) {
        if self.persistence_loaded {
            return;
        }
        self.persistence_loaded = true;
        let Some(saved) = BugReportDraftPersistence::shared(cx).update(cx, |persistence, _| persistence.load()) else {
            return;
        };
        for (kind, persisted) in saved.drafts {
            let next = BugReportDraft {
                feedback_kind: persisted.feedback_kind,
                pages: persisted.pages,
                seeded_pages: persisted.seeded_pages,
                component: persisted.component,
                message: persisted.message,
                fix_platforms: persisted.fix_platforms,
                ..BugReportDraft::new(persisted.client_report_id, kind)
            };
            if Self::message_rule().is_satisfied(trimming_whitespaces_and_newlines(&next.message)) {
                self.message_was_valid_kinds.insert(kind);
            }
            self.drafts.insert(kind, next);
        }
        self.active_kind = saved.active_kind;
        self.page = saved.page;
        cx.notify();
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        if !self.persistence_loaded || self.drafts_before_lesson.is_some() {
            return;
        }
        let mut state =
            BugReportPersistedState { page: self.page, active_kind: self.active_kind, ..BugReportPersistedState::default() };
        for (kind, draft) in &self.drafts {
            if Self::draft_has_content(draft) {
                state.drafts.insert(
                    *kind,
                    BugReportPersistedDraft {
                        client_report_id: draft.client_report_id.clone(),
                        kind: *kind,
                        feedback_kind: draft.feedback_kind,
                        message: draft.message.clone(),
                        component: draft.component.clone(),
                        pages: draft.pages.clone(),
                        seeded_pages: draft.seeded_pages.clone(),
                        fix_platforms: draft.fix_platforms.clone(),
                    },
                );
            }
        }
        let is_empty = state.drafts.is_empty() && self.page == BugReportSheetPage::Root;
        BugReportDraftPersistence::shared(cx).update(cx, |persistence, cx| {
            if is_empty {
                persistence.clear();
            } else {
                persistence.save(state, cx);
            }
        });
    }

    pub fn flush_persistence(&mut self, cx: &mut Context<Self>) {
        BugReportDraftPersistence::shared(cx).update(cx, |persistence, cx| persistence.flush(cx));
    }

    pub fn slice_for(&self, kind: BugReportDraftKind) -> BugReportListSlice {
        self.my_reports.get(&kind).cloned().unwrap_or_default()
    }

    pub fn apply_list_state(&mut self, list_state: BugReportListState, kind: BugReportDraftKind, cx: &mut Context<Self>) {
        let mut slice = self.slice_for(kind);
        slice.state = list_state;
        self.my_reports.insert(kind, slice);
        cx.notify();
    }

    pub fn apply_records(
        &mut self,
        records: Vec<BugReportRecord>,
        have_more: bool,
        append: bool,
        kind: BugReportDraftKind,
        cx: &mut Context<Self>,
    ) {
        let mut slice = self.slice_for(kind);
        if append {
            let mut seen: HashSet<String> = slice.records.iter().map(|record| record.id.clone()).collect();
            for record in records {
                if seen.insert(record.id.clone()) {
                    slice.records.push(record);
                }
            }
        } else {
            slice.records = records;
        }
        slice.have_more = have_more;
        slice.state = BugReportListState::Loaded;
        self.my_reports.insert(kind, slice);
        cx.notify();
    }

    pub fn replace_my_report(&mut self, record: BugReportRecord, cx: &mut Context<Self>) {
        let kind = record.kind;
        let mut slice = self.slice_for(kind);
        let Some(index) = slice.records.iter().position(|existing| existing.id == record.id) else {
            return;
        };
        slice.records[index] = record;
        self.my_reports.insert(kind, slice);
        cx.notify();
    }

    pub fn remove_my_report(&mut self, id: &str, cx: &mut Context<Self>) {
        for kind in BugReportDraftKind::ALL_CASES {
            let mut slice = self.slice_for(kind);
            if !slice.records.iter().any(|record| record.id == id) {
                continue;
            }
            slice.records.retain(|record| record.id != id);
            self.my_reports.insert(kind, slice);
        }
        cx.notify();
    }

    pub fn apply_deleting_report_id(&mut self, deleting_report_id: Option<String>, cx: &mut Context<Self>) {
        self.deleting_report_id = deleting_report_id;
        cx.notify();
    }

    pub fn apply_my_reports_tab(&mut self, my_reports_tab: i64, cx: &mut Context<Self>) {
        let clamped = 0_i64.max(my_reports_tab).min(Self::TAB_ORDER.len() as i64 - 1);
        if clamped == self.my_reports_tab {
            return;
        }
        self.my_reports_tab = clamped;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.route = BugReportRoute::Idle;
        self.editing_draft = None;
        self.viewed_record = None;
        self.opened_status_comment = None;
        self.loaded_draft = None;
        self.state = BugReportSubmissionState::Editing;
        self.page = BugReportSheetPage::Root;
        self.active_kind = BugReportDraftKind::Bug;
        self.page_query = String::new();
        self.is_confirming_discard = false;
        self.message_was_valid_kinds = HashSet::new();
        self.persistence_loaded = false;
        self.drafts = Self::fresh_drafts();
        self.my_reports = Self::fresh_my_reports();
        self.my_reports_tab = 0;
        self.deleting_report_id = None;
        BugReportDraftPersistence::shared(cx).update(cx, |persistence, _| persistence.reset_account_state());
        cx.notify();
    }
}
