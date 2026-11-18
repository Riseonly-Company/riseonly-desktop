use std::collections::HashSet;
use std::path::Path;

use gpui::{App, Context, Entity};
use rise_canon::{RiseBugReportPlatform, RiseBugReportSurface, RiseFeedbackKind, RiseGuideSignal};
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;

use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::{
    BugReportAttachment, BugReportAttachmentKind, RiseBugReportUpload,
};
use crate::modules::diagnostics::engine::rise_bug_report_failure::RiseBugReportFailure;
use crate::modules::diagnostics::engine::rise_bug_report_submitter::RiseBugReportSubmitter;
use crate::modules::diagnostics::shared::bug_report_shake_preference::BugReportShakePreference;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_actions::BugReportActionsStore;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_types::{
    BugReportDraftKind, BugReportListState, BugReportRoute, BugReportSheetPage, BugReportSubmissionState,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_actions::bug_report_wire_types::BugReportRecord;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_services::bug_report_services::BugReportServicesStore;
use crate::modules::diagnostics::stores::diagnostics_guide_sandbox_hooks::diagnostics_guide_sandbox_hooks;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerItem, MediaPickerItemKind, MediaPickerResult,
};

pub struct BugReportInteractionsStore {
    actions: Entity<BugReportActionsStore>,
    services: Entity<BugReportServicesStore>,
    linked_report_id: Option<String>,
    linked_report_loads: HashSet<BugReportDraftKind>,
    linked_report_pages: usize,
}

impl BugReportInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            actions: BugReportActionsStore::shared(cx),
            services: BugReportServicesStore::shared(cx),
            linked_report_id: None,
            linked_report_loads: HashSet::new(),
            linked_report_pages: 0,
        }
    }

    fn route(&self, cx: &App) -> BugReportRoute {
        self.services.read(cx).route()
    }

    fn apply_route(&self, route: BugReportRoute, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| services.apply_route(route, cx));
    }

    fn apply_state(&self, state: BugReportSubmissionState, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| services.apply_state(state, cx));
    }

    fn apply_page(&self, page: BugReportSheetPage, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| services.apply_page(page, cx));
    }

    fn finish_editing(&self, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| services.finish_editing(cx));
    }

    fn start_draft(&self, kind: BugReportDraftKind, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| services.start_draft(kind, None, cx));
    }

    fn is_editing_or_follow_up(&self, cx: &App) -> bool {
        let services = self.services.read(cx);
        services.is_editing() || services.is_follow_up()
    }

    fn is_authenticated(cx: &mut Context<Self>) -> bool {
        AuthServicesStore::shared(cx).read(cx).is_authenticated()
    }

    fn send_toast(title: String, kind: NotifierType, cx: &mut App) {
        Notifier::send(
            title,
            NotifierSendOptions { r#type: kind, position: NotifierPosition::Bottom, ..NotifierSendOptions::default() },
            cx,
        );
    }

    pub fn present_from_shake(&mut self, surface: RiseBugReportSurface, cx: &mut Context<Self>) {
        if !BugReportShakePreference::is_enabled() {
            return;
        }
        self.present(surface, cx);
    }

    pub fn present(&mut self, surface: RiseBugReportSurface, cx: &mut Context<Self>) {
        if self.route(cx) != BugReportRoute::Idle {
            return;
        }
        if !Self::is_authenticated(cx) {
            return;
        }
        if self.services.read(cx).state().is_sending() {
            return;
        }

        self.services.update(cx, |services, cx| services.activate_persistence(cx));
        if self.is_editing_or_follow_up(cx) {
            self.discard_temporary_files(cx);
            self.finish_editing(cx);
            self.apply_page(BugReportSheetPage::Root, cx);
        }
        self.services.update(cx, |services, cx| services.resume_drafts(Some(surface), cx));
        self.apply_route(BugReportRoute::Sheet, cx);
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Screen, || {
            format!("report sheet opened at {}", surface.wire())
        });
    }

    pub fn set_sheet_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if presented || self.route(cx) != BugReportRoute::Sheet {
            return;
        }
        if self.services.read(cx).state().is_sending() {
            self.actions.update(cx, |actions, _| actions.cancel_submission_action());
            self.apply_state(BugReportSubmissionState::Editing, cx);
        }
        self.apply_route(BugReportRoute::ClosingSheetForDismissal, cx);
    }

    pub fn sheet_did_dismiss(&mut self, cx: &mut Context<Self>) {
        match self.route(cx) {
            BugReportRoute::ClosingSheetForPicker => self.apply_route(BugReportRoute::Picker, cx),
            BugReportRoute::ClosingSheetForDismissal => {
                if *self.services.read(cx).state() == BugReportSubmissionState::Sent {
                    self.discard_temporary_files(cx);
                    if self.is_editing_or_follow_up(cx) {
                        self.finish_editing(cx);
                    } else {
                        let kind = self.services.read(cx).active_kind();
                        self.start_draft(kind, cx);
                    }
                    self.apply_page(BugReportSheetPage::Root, cx);
                } else if self.is_editing_or_follow_up(cx) {
                    self.discard_temporary_files(cx);
                    self.finish_editing(cx);
                    self.apply_page(BugReportSheetPage::Root, cx);
                }
                self.apply_state(BugReportSubmissionState::Editing, cx);
                self.apply_route(BugReportRoute::Idle, cx);
                self.services.update(cx, |services, cx| services.flush_persistence(cx));
            }
            _ => {}
        }
    }

    pub fn sheet_presentation_refused(&mut self, cx: &mut Context<Self>) {
        if self.route(cx) != BugReportRoute::Sheet {
            return;
        }
        self.apply_route(BugReportRoute::Idle, cx);
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Screen, || {
            "report sheet refused: another modal is up".to_owned()
        });
    }

    pub fn page_did_change(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(page) = BugReportSheetPage::from_raw_value(id) else {
            return;
        };
        self.apply_page(page, cx);
    }

    pub fn request_discard(&mut self, cx: &mut Context<Self>) {
        let services = self.services.read(cx);
        if services.state().is_sending() || services.has_content() {
            self.services.update(cx, |services, cx| services.apply_is_confirming_discard(true, cx));
            return;
        }
        self.discard_draft(cx);
    }

    pub fn cancel_discard(&mut self, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| services.apply_is_confirming_discard(false, cx));
    }

    pub fn discard_draft(&mut self, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| services.apply_is_confirming_discard(false, cx));
        self.actions.update(cx, |actions, _| actions.cancel_submission_action());
        self.discard_temporary_files(cx);
        if self.is_editing_or_follow_up(cx) {
            self.finish_editing(cx);
        } else {
            let kind = self.services.read(cx).active_kind();
            self.start_draft(kind, cx);
        }
        if self.route(cx) != BugReportRoute::Sheet {
            return;
        }
        self.apply_route(BugReportRoute::ClosingSheetForDismissal, cx);
    }

    fn discard_temporary_files(&self, cx: &mut Context<Self>) {
        let draft = self.services.read(cx).draft();
        BugReportAttachmentFactory::remove_temporary_files(&draft.temporary_files());
        RiseBugReportSubmitter::shared().forget(&draft.client_report_id);
    }

    pub fn update_message(&mut self, raw: &str, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| {
            services.mutate_draft(|draft| draft.message = BugReportServicesStore::message_rule().clamped(raw), cx);
        });
        let draft = self.services.read(cx).draft();
        let is_ready =
            BugReportServicesStore::message_rule().is_satisfied(trimming_whitespaces_and_newlines(&draft.message));
        cx.defer(move |cx| GuideFacts::set(RiseGuideSignal::BugReportMessageReady, is_ready, cx));
    }

    pub fn update_component(&mut self, raw: &str, cx: &mut Context<Self>) {
        let single = raw.replace('\n', " ");
        self.services.update(cx, |services, cx| {
            services
                .mutate_draft(|draft| draft.component = BugReportServicesStore::component_rule().clamped(&single), cx);
        });
    }

    pub fn select_feedback_kind(&mut self, kind: RiseFeedbackKind, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| services.mutate_draft(|draft| draft.feedback_kind = kind, cx));
    }

    pub fn toggle_platform(&mut self, platform: RiseBugReportPlatform, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| {
            services.mutate_draft(
                |draft| {
                    if let Some(index) = draft.fix_platforms.iter().position(|picked| *picked == platform) {
                        if draft.fix_platforms.len() > 1 {
                            draft.fix_platforms.remove(index);
                        }
                        return;
                    }
                    draft.fix_platforms.push(platform);
                },
                cx,
            );
        });
    }

    pub fn update_page_query(&mut self, raw: &str, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| services.apply_page_query(raw, cx));
    }

    pub fn toggle_page(&mut self, surface: RiseBugReportSurface, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| {
            services.mutate_draft(
                |draft| {
                    if let Some(index) = draft.pages.iter().position(|page| *page == surface) {
                        draft.pages.remove(index);
                        return;
                    }
                    if draft.pages.len() as i64 >= RiseBugReportUpload::maximum_pages() {
                        return;
                    }
                    draft.pages.push(surface);
                },
                cx,
            );
        });
    }

    pub fn open_attachment_picker(&mut self, cx: &mut Context<Self>) {
        if self.route(cx) != BugReportRoute::Sheet {
            return;
        }
        let services = self.services.read(cx);
        if services.state().is_sending() || !services.can_attach_more() {
            return;
        }
        self.apply_route(BugReportRoute::ClosingSheetForPicker, cx);
    }

    pub fn set_picker_presented(&mut self, presented: bool, cx: &mut Context<Self>) {
        if presented || self.route(cx) != BugReportRoute::Picker {
            return;
        }
        self.apply_route(BugReportRoute::ClosingPickerForSheet, cx);
    }

    pub fn picker_did_dismiss(&mut self, cx: &mut Context<Self>) {
        if self.route(cx) != BugReportRoute::ClosingPickerForSheet {
            return;
        }
        self.apply_route(BugReportRoute::Sheet, cx);
    }

    pub fn attach(&mut self, result: MediaPickerResult, cx: &mut Context<Self>) {
        if !result.was_cancelled {
            self.attach_items(&result, cx);
        }
        if self.route(cx) == BugReportRoute::Picker {
            self.apply_route(BugReportRoute::ClosingPickerForSheet, cx);
        }
    }

    fn attach_items(&mut self, result: &MediaPickerResult, cx: &mut Context<Self>) {
        let attachments: Vec<BugReportAttachment> =
            result.items.iter().filter_map(BugReportAttachmentFactory::make).collect();
        let mut rejected_video = false;
        let mut trimmed_video = false;
        self.services.update(cx, |services, cx| {
            services.mutate_draft(
                |draft| {
                    for attachment in attachments {
                        match attachment.kind {
                            BugReportAttachmentKind::Screenshot => {
                                if draft.screenshots.len() as i64 >= RiseBugReportUpload::maximum_screenshots() {
                                    continue;
                                }
                                draft.screenshots.push(attachment);
                            }
                            BugReportAttachmentKind::Video => {
                                if draft.video.is_some() {
                                    continue;
                                }
                                if let Some(duration) = attachment.duration_ms
                                    && duration > RiseBugReportUpload::maximum_video_duration_ms()
                                {
                                    rejected_video = true;
                                    continue;
                                }
                                if attachment.was_trimmed_to_ceiling {
                                    trimmed_video = true;
                                }
                                draft.video = Some(attachment);
                            }
                        }
                    }
                },
                cx,
            );
        });
        if rejected_video || trimmed_video {
            Self::send_toast(
                tr("bug_report_video_too_long"),
                if rejected_video { NotifierType::Error } else { NotifierType::Warning },
                cx,
            );
        }
    }

    pub fn remove_attachment(&mut self, id: &str, cx: &mut Context<Self>) {
        let services = self.services.read(cx);
        if !services.can_edit_draft() {
            return;
        }
        let draft = services.draft();
        let removed: Vec<Url> = draft
            .screenshots
            .iter()
            .chain(draft.video.iter())
            .filter(|attachment| attachment.id == id)
            .flat_map(|attachment| [attachment.url.clone(), attachment.preview_url.clone()])
            .flatten()
            .collect();
        self.services.update(cx, |services, cx| {
            services.mutate_draft(
                |draft| {
                    draft.screenshots.retain(|attachment| attachment.id != id);
                    if draft.video.as_ref().is_some_and(|video| video.id == id) {
                        draft.video = None;
                    }
                },
                cx,
            );
        });
        BugReportAttachmentFactory::remove_temporary_files(&removed);
    }

    fn progress_handler(&self) -> impl Fn(f64, &mut App) + 'static + use<> {
        let services = self.services.downgrade();
        move |fraction: f64, cx: &mut App| {
            let _ = services.update(cx, |services, cx| {
                if services.state().is_sending() {
                    services.apply_state(BugReportSubmissionState::Sending { progress: fraction }, cx);
                }
            });
        }
    }

    pub fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.services.read(cx).can_submit() {
            return;
        }
        let draft = self.services.read(cx).draft();

        if (diagnostics_guide_sandbox_hooks().owns_bug_report_scene)(cx) && draft.editing_report_id.is_none() {
            self.apply_state(BugReportSubmissionState::Sent, cx);
            GuideSignalBus::emit(RiseGuideSignal::BugReportSubmitted);
            if self.route(cx) == BugReportRoute::Sheet {
                self.apply_route(BugReportRoute::ClosingSheetForDismissal, cx);
            }
            return;
        }

        if let Some(report_id) = draft.editing_report_id.clone() {
            self.save(report_id, cx);
            return;
        }

        let payload = draft.payload();
        let kind = draft.kind;
        let was_follow_up = payload.parent_report_id.is_some();
        self.apply_state(BugReportSubmissionState::Sending { progress: 0.0 }, cx);

        let on_progress = self.progress_handler();
        let this = cx.weak_entity();
        self.actions.update(cx, |actions, cx| {
            actions.submit_action(
                payload,
                on_progress,
                move |result, cx: &mut App| {
                    let _ = this.update(cx, |this, cx| this.submit_did_finish(result, kind, was_follow_up, cx));
                },
                cx,
            );
        });
    }

    fn submit_did_finish(
        &mut self,
        result: anyhow::Result<bool>,
        kind: BugReportDraftKind,
        was_follow_up: bool,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(_) => {
                self.apply_state(BugReportSubmissionState::Sent, cx);
                if was_follow_up {
                    self.refresh_my_reports_kind(kind, cx);
                } else {
                    self.services
                        .update(cx, |services, cx| services.apply_list_state(BugReportListState::Idle, kind, cx));
                }
                Self::send_toast(
                    tr(if kind == BugReportDraftKind::Feedback { "feedback_sent" } else { "bug_report_sent" }),
                    NotifierType::Success,
                    cx,
                );
                if self.route(cx) == BugReportRoute::Sheet {
                    self.apply_route(BugReportRoute::ClosingSheetForDismissal, cx);
                } else {
                    self.discard_temporary_files(cx);
                    if self.is_editing_or_follow_up(cx) {
                        self.finish_editing(cx);
                    } else {
                        self.start_draft(kind, cx);
                    }
                    self.apply_state(BugReportSubmissionState::Editing, cx);
                    self.apply_route(BugReportRoute::Idle, cx);
                }
            }
            Err(error) => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || {
                    format!("report submit failed: {}", RiseBugReportFailure::diagnostic(&error))
                });
                let fallback_key = if kind == BugReportDraftKind::Feedback { "feedback_failed" } else { "bug_report_failed" };
                self.apply_state(
                    BugReportSubmissionState::Failed { message: RiseBugReportFailure::message(&error, fallback_key) },
                    cx,
                );
            }
        }
    }

    fn save(&mut self, report_id: String, cx: &mut Context<Self>) {
        let payload = self.services.read(cx).draft().payload();
        self.apply_state(BugReportSubmissionState::Sending { progress: 0.0 }, cx);

        let on_progress = self.progress_handler();
        let this = cx.weak_entity();
        self.actions.update(cx, |actions, cx| {
            actions.update_action(
                report_id,
                payload,
                on_progress,
                move |result, cx: &mut App| {
                    let _ = this.update(cx, |this, cx| this.save_did_finish(result, cx));
                },
                cx,
            );
        });
    }

    fn save_did_finish(&mut self, result: anyhow::Result<Option<BugReportRecord>>, cx: &mut Context<Self>) {
        match result {
            Ok(record) => {
                self.apply_state(BugReportSubmissionState::Sent, cx);
                if let Some(record) = record {
                    self.services.update(cx, |services, cx| services.replace_my_report(record, cx));
                }
                Self::send_toast(tr("bug_report_saved"), NotifierType::Success, cx);
                if self.route(cx) == BugReportRoute::Sheet {
                    self.apply_route(BugReportRoute::ClosingSheetForDismissal, cx);
                } else {
                    self.discard_temporary_files(cx);
                    self.finish_editing(cx);
                    self.apply_route(BugReportRoute::Idle, cx);
                }
            }
            Err(error) => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || {
                    format!("report update failed: {}", RiseBugReportFailure::diagnostic(&error))
                });
                self.apply_state(
                    BugReportSubmissionState::Failed {
                        message: RiseBugReportFailure::message(&error, "bug_report_failed"),
                    },
                    cx,
                );
            }
        }
    }

    pub fn select_my_reports_tab(&mut self, index: i64, cx: &mut Context<Self>) {
        self.services.update(cx, |services, cx| services.apply_my_reports_tab(index, cx));
    }

    pub fn open_linked_report(&mut self, id: &str, cx: &mut Context<Self>) {
        self.linked_report_id = Some(id.to_lowercase());
        self.linked_report_pages = 0;
        self.linked_report_loads = BugReportDraftKind::ALL_CASES.into_iter().collect();
        self.refresh_my_reports(cx);
    }

    fn linked_report_page_arrived(&mut self, kind: BugReportDraftKind, cx: &mut Context<Self>) {
        let Some(id) = self.linked_report_id.clone() else {
            return;
        };
        self.linked_report_loads.remove(&kind);
        for kind in BugReportDraftKind::ALL_CASES {
            let slice = self.services.read(cx).slice_for(kind);
            let Some(record) = slice.records.iter().find(|record| record.id.to_lowercase() == id).cloned() else {
                continue;
            };
            self.linked_report_id = None;
            self.linked_report_loads.clear();
            if let Some(tab) = BugReportServicesStore::TAB_ORDER.iter().position(|ordered| *ordered == kind) {
                self.select_my_reports_tab(tab as i64, cx);
            }
            self.open_report(&record, cx);
            return;
        }
        if !self.linked_report_loads.is_empty() {
            return;
        }
        let more: Vec<BugReportDraftKind> = BugReportDraftKind::ALL_CASES
            .into_iter()
            .filter(|kind| self.services.read(cx).slice_for(*kind).have_more)
            .collect();
        if more.is_empty() || self.linked_report_pages >= Self::LINKED_REPORT_PAGE_LIMIT {
            self.linked_report_id = None;
            return;
        }
        self.linked_report_pages += 1;
        self.linked_report_loads = more.iter().copied().collect();
        for kind in more {
            let offset = self.services.read(cx).slice_for(kind).records.len() as i64;
            self.load_my_reports(kind, offset, true, cx);
        }
    }

    const LINKED_REPORT_PAGE_LIMIT: usize = 10;

    pub fn ensure_my_reports_loaded(&mut self, kind: BugReportDraftKind, cx: &mut Context<Self>) {
        if self.services.read(cx).slice_for(kind).state.is_loading() {
            return;
        }
        self.load_my_reports(kind, 0, false, cx);
    }

    pub fn refresh_my_reports(&mut self, cx: &mut Context<Self>) {
        for kind in BugReportDraftKind::ALL_CASES {
            self.refresh_my_reports_kind(kind, cx);
        }
    }

    pub fn refresh_my_reports_kind(&mut self, kind: BugReportDraftKind, cx: &mut Context<Self>) {
        self.load_my_reports(kind, 0, false, cx);
    }

    pub fn load_more_my_reports(&mut self, kind: BugReportDraftKind, cx: &mut Context<Self>) {
        let slice = self.services.read(cx).slice_for(kind);
        if !slice.have_more || slice.state.is_loading() {
            return;
        }
        let offset = (slice.records.len() as i64 - Self::MY_REPORTS_PAGE_OVERLAP).max(0);
        self.load_my_reports(kind, offset, true, cx);
    }

    const MY_REPORTS_PAGE_OVERLAP: i64 = 5;

    fn load_my_reports(&mut self, kind: BugReportDraftKind, offset: i64, append: bool, cx: &mut Context<Self>) {
        if !Self::is_authenticated(cx) {
            return;
        }
        let had_list = self.services.read(cx).slice_for(kind).state == BugReportListState::Loaded;
        if !append && !had_list {
            self.services
                .update(cx, |services, cx| services.apply_list_state(BugReportListState::Loading, kind, cx));
        }

        let this = cx.weak_entity();
        self.actions.update(cx, |actions, cx| {
            actions.load_my_reports_action(
                kind,
                offset,
                move |result, cx: &mut App| {
                    let _ = this.update(cx, |this, cx| this.my_reports_did_load(result, kind, offset, append, had_list, cx));
                },
                cx,
            );
        });
    }

    fn my_reports_did_load(
        &mut self,
        result: anyhow::Result<(Vec<BugReportRecord>, bool)>,
        kind: BugReportDraftKind,
        offset: i64,
        append: bool,
        had_list: bool,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok((records, have_more)) => {
                if append && offset > 0 && !records.is_empty() {
                    let slice = self.services.read(cx).slice_for(kind);
                    let overlaps = records.iter().any(|record| slice.records.iter().any(|known| known.id == record.id));
                    if !overlaps {
                        tracing::warn!(target: "riseonly::X1862", kind = kind.wire(), offset, loaded = slice.records.len(), "my reports page seam lost, reloading");
                        self.load_my_reports(kind, 0, false, cx);
                        return;
                    }
                }
                self.services
                    .update(cx, |services, cx| services.apply_records(records, have_more, append, kind, cx));
                self.linked_report_page_arrived(kind, cx);
            }
            Err(error) => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || {
                    format!("my reports ({}) failed: {}", kind.wire(), RiseBugReportFailure::diagnostic(&error))
                });
                let list_state = if !append && !had_list {
                    BugReportListState::Failed {
                        message: RiseBugReportFailure::message(&error, "error_fetching_data"),
                    }
                } else {
                    BugReportListState::Loaded
                };
                self.services.update(cx, |services, cx| services.apply_list_state(list_state, kind, cx));
                self.linked_report_page_arrived(kind, cx);
            }
        }
    }

    pub fn present_edit(&mut self, record: &BugReportRecord, cx: &mut Context<Self>) {
        if self.route(cx) != BugReportRoute::Idle {
            return;
        }
        if !Self::is_authenticated(cx) {
            return;
        }
        if self.services.read(cx).state().is_sending() {
            return;
        }
        if !record.is_editable() {
            Self::send_toast(
                tr(if record.is_feedback() { "feedback_locked" } else { "bug_report_locked" }),
                NotifierType::Warning,
                cx,
            );
            return;
        }
        self.services.update(cx, |services, cx| {
            services.activate_persistence(cx);
            services.start_editing(record, cx);
        });
        self.apply_route(BugReportRoute::Sheet, cx);
    }

    pub fn open_report(&mut self, record: &BugReportRecord, cx: &mut Context<Self>) {
        if record.is_editable() {
            self.present_edit(record, cx);
            return;
        }
        self.present_view(record, cx);
    }

    pub fn present_view(&mut self, record: &BugReportRecord, cx: &mut Context<Self>) {
        if self.route(cx) != BugReportRoute::Idle {
            return;
        }
        if !Self::is_authenticated(cx) {
            return;
        }
        if self.services.read(cx).state().is_sending() {
            return;
        }
        self.services.update(cx, |services, cx| {
            services.activate_persistence(cx);
            services.start_viewing(record, cx);
        });
        self.apply_route(BugReportRoute::Sheet, cx);
    }

    pub fn refile_viewed_report(&mut self, cx: &mut Context<Self>) {
        if self.route(cx) != BugReportRoute::Sheet {
            return;
        }
        let Some(record) = self.services.read(cx).viewed_record().filter(|record| record.can_refile()).cloned() else {
            return;
        };
        self.services.update(cx, |services, cx| services.start_follow_up(&record, cx));
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Screen, || format!("bug report refiled on {}", record.id));
    }

    pub fn present_refile(&mut self, record: &BugReportRecord, cx: &mut Context<Self>) {
        if self.route(cx) != BugReportRoute::Idle {
            return;
        }
        if !Self::is_authenticated(cx) {
            return;
        }
        if self.services.read(cx).state().is_sending() {
            return;
        }
        if !record.can_refile() {
            Self::send_toast(tr("bug_report_locked"), NotifierType::Warning, cx);
            return;
        }
        self.services.update(cx, |services, cx| {
            services.activate_persistence(cx);
            services.start_follow_up(record, cx);
        });
        self.apply_route(BugReportRoute::Sheet, cx);
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Screen, || format!("bug report refiled on {}", record.id));
    }

    pub fn delete_report(&mut self, id: &str, kind: BugReportDraftKind, cx: &mut Context<Self>) {
        if self.services.read(cx).deleting_report_id().is_some() {
            return;
        }
        let snapshot = self.services.read(cx).slice_for(kind);
        self.services.update(cx, |services, cx| {
            services.apply_deleting_report_id(Some(id.to_owned()), cx);
            services.remove_my_report(id, cx);
        });

        let this = cx.weak_entity();
        let report_id = id.to_owned();
        self.actions.update(cx, |actions, cx| {
            actions.delete_action(
                report_id,
                move |result, cx: &mut App| {
                    let _ = this.update(cx, |this, cx| {
                        this.services.update(cx, |services, cx| services.apply_deleting_report_id(None, cx));
                        if let Err(error) = result {
                            this.services.update(cx, |services, cx| {
                                services.apply_records(snapshot.records, snapshot.have_more, false, kind, cx);
                            });
                            Self::send_toast(
                                RiseBugReportFailure::message(&error, "error_fetching_data"),
                                NotifierType::Error,
                                cx,
                            );
                            this.refresh_my_reports_kind(kind, cx);
                        }
                    });
                },
                cx,
            );
        });
    }

    pub fn send_or_retry(&mut self, cx: &mut Context<Self>) {
        if matches!(self.services.read(cx).state(), BugReportSubmissionState::Failed { .. }) {
            self.retry(cx);
            return;
        }
        self.submit(cx);
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.services.read(cx).state(), BugReportSubmissionState::Failed { .. }) {
            return;
        }
        self.apply_state(BugReportSubmissionState::Editing, cx);
        self.submit(cx);
    }

    pub fn account_did_change(&mut self, cx: &mut Context<Self>) {
        self.actions.update(cx, |actions, _| actions.cancel_submission_action());
        self.services.update(cx, |services, cx| services.reset_account_state(cx));
        RiseBreadcrumbs::reset();
    }
}

pub enum BugReportAttachmentFactory {}

impl BugReportAttachmentFactory {
    pub fn make(item: &MediaPickerItem) -> Option<BugReportAttachment> {
        match item.kind {
            MediaPickerItemKind::Image => {
                if item.is_file {
                    return None;
                }
                Some(BugReportAttachment::new(
                    item.id.clone(),
                    BugReportAttachmentKind::Screenshot,
                    Some(item.url.clone()),
                    Self::filename(item, "jpg"),
                    Self::content_type(&item.suggested_file_name, false).to_owned(),
                    None,
                    Some(item.url.clone()),
                    false,
                    None,
                ))
            }
            MediaPickerItemKind::Video => {
                let payload = item.video_payload.as_ref();
                let seconds = payload.map(|payload| payload.end_seconds - payload.start_seconds);
                let trimmed = payload
                    .is_some_and(|payload| payload.duration - (payload.end_seconds - payload.start_seconds) > 0.5);
                Some(BugReportAttachment::new(
                    item.id.clone(),
                    BugReportAttachmentKind::Video,
                    Some(payload.map_or_else(|| item.url.clone(), |payload| payload.url.clone())),
                    Self::filename(item, "mp4"),
                    Self::content_type(&item.suggested_file_name, true).to_owned(),
                    seconds.map(|seconds| (seconds.max(0.0) * 1000.0) as i64),
                    payload.and_then(|payload| payload.poster_url.clone()),
                    trimmed,
                    None,
                ))
            }
            MediaPickerItemKind::File => None,
        }
    }

    fn filename(item: &MediaPickerItem, fallback_extension: &str) -> String {
        let suggested = trimming_whitespaces(&item.suggested_file_name);
        if !suggested.is_empty() {
            return suggested.to_owned();
        }
        format!("bug-report-{}.{}", item.id, fallback_extension)
    }

    pub fn remove_temporary_files(urls: &[Url]) {
        let temporary = temporary_directory();
        for url in urls {
            if !url.is_file_url() {
                continue;
            }
            let Some(path) = url.standardized().to_file_path() else {
                continue;
            };
            if !path.starts_with(&temporary) {
                continue;
            }
            let _ = FileManager.remove_item(&path);
        }
    }

    fn content_type(filename: &str, is_video: bool) -> &'static str {
        let extension = Path::new(filename)
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "png" => "image/png",
            "heic" => "image/heic",
            "heif" => "image/heif",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "mov" => "video/quicktime",
            "m4v" => "video/x-m4v",
            "mp4" => "video/mp4",
            _ => {
                if is_video {
                    "video/mp4"
                } else {
                    "image/jpeg"
                }
            }
        }
    }
}
